use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use codex_http_client::ClientRouteClass;
use codex_http_client::HttpClientFactory;
use codex_http_client::RouteAwareClientPool;
use codex_protocol::protocol::SessionSource;
use http::StatusCode;
use sentry::protocol::Attachment;
use sentry::protocol::Envelope;
use sentry::protocol::EnvelopeHeaders;
use sentry::protocol::EnvelopeItem;
use sentry::types::Dsn;
use sentry::types::Uuid;

use crate::FEEDBACK_ENDPOINT_DSN;
use crate::FeedbackAttachment;
use crate::FeedbackSnapshot;
use crate::MAX_DECODED_UPLOAD_BYTES;
use crate::MAX_EVENT_BYTES;
use crate::MAX_UPLOAD_BYTES;
use crate::upload;

// Background parts must tolerate slow links without inheriting the interactive
// upload's ten-second budget. The report API can cancel the request at any time.
const REPORT_PART_UPLOAD_TIMEOUT: Duration = Duration::from_secs(/*secs*/ 300);

/// The upstream HTTP result, not proof of durable storage in Sentry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedbackDelivery {
    Accepted {
        retry_after: Option<Duration>,
    },
    Rejected {
        status: u16,
        retry_after: Option<Duration>,
    },
    /// No response was received. The upstream may still have accepted the envelope.
    Unconfirmed,
}

/// Sends persisted report parts without retries or request diagnostics.
pub struct FeedbackTransport {
    client_pool: RouteAwareClientPool,
    dsn: Dsn,
}

impl FeedbackTransport {
    pub fn new(http_client_factory: HttpClientFactory) -> Result<Self> {
        Ok(Self {
            client_pool: RouteAwareClientPool::new_without_redirects_or_request_logging(
                http_client_factory,
                ClientRouteClass::Other,
            ),
            // Persisted report parts still use the legacy whole-envelope
            // transport. They cannot switch to a receiver with an 8 MiB body
            // ceiling until the report API persists a signed-upload capability
            // and finalizes each large original. Interactive feedback has that
            // protocol; silently routing this transport would lose 64 MiB logs.
            dsn: FEEDBACK_ENDPOINT_DSN
                .parse()
                .map_err(|_| anyhow!("invalid feedback DSN"))?,
        })
    }

    /// Make one attempt. Callers own retry decisions and must retain the same bytes.
    pub async fn send(&self, envelope: Vec<u8>) -> FeedbackDelivery {
        let response = upload::envelope_request(
            &self.client_pool,
            &self.dsn,
            envelope.into(),
            REPORT_PART_UPLOAD_TIMEOUT,
            upload::EnvelopeKind::Attachment,
        )
        .send()
        .await;
        let Ok(response) = response else {
            return FeedbackDelivery::Unconfirmed;
        };
        let status = response.status();
        let headers = response.headers();
        let mut retry_after = headers
            .get_all("Retry-After")
            .iter()
            .map(|value| {
                upload::parse_retry_after(value.to_str().unwrap_or_default())
                    .unwrap_or(upload::DEFAULT_RATE_LIMIT)
            })
            .max();
        // Report events and attachments share one conservative cooldown. Honor quotas on
        // successful responses too; ignore quotas for unrelated Sentry categories.
        // https://develop.sentry.dev/sdk/foundations/transport/rate-limiting/
        retry_after = retry_after.max(upload::sentry_rate_limit_delay(headers));
        if status == StatusCode::TOO_MANY_REQUESTS && retry_after.is_none() {
            retry_after = Some(upload::DEFAULT_RATE_LIMIT);
        }
        if status.is_success() {
            FeedbackDelivery::Accepted { retry_after }
        } else {
            FeedbackDelivery::Rejected {
                status: status.as_u16(),
                retry_after,
            }
        }
    }
}

impl FeedbackSnapshot {
    /// Prepare only the core report, using the caller's stable UUID as its Sentry event ID.
    pub fn prepare_report_event(
        &self,
        report_id: &str,
        classification: &str,
        reason: Option<&str>,
        tags: Option<&BTreeMap<String, String>>,
        session_source: Option<&SessionSource>,
    ) -> Result<Vec<u8>> {
        let mut event = self.feedback_event(classification, reason, tags, session_source);
        // Auth tags must come from this report's caller, not metadata retained
        // from an earlier account in the process-wide tracing layer.
        event.tags.retain(|key, _| {
            !matches!(key.as_str(), "account_id" | "chatgpt_user_id")
                || tags.is_some_and(|tags| tags.contains_key(key))
        });
        event.event_id =
            Uuid::parse_str(report_id).map_err(|_| anyhow!("invalid feedback report ID"))?;
        let mut envelope = Envelope::new();
        envelope.add_item(EnvelopeItem::Event(event));
        prepare_envelope(envelope, MAX_EVENT_BYTES)
    }
}

/// Prepare one whole attachment linked to an existing report, without replaying its event.
/// The caller must apply consent and the report's cumulative attachment budget first.
pub fn prepare_report_attachment(
    report_id: &str,
    attachment: FeedbackAttachment,
) -> Result<Vec<u8>> {
    let event_id = Uuid::parse_str(report_id).map_err(|_| anyhow!("invalid feedback report ID"))?;
    let mut envelope = Envelope::new().with_headers(EnvelopeHeaders::new().with_event_id(event_id));
    envelope.add_item(EnvelopeItem::Attachment(Attachment {
        buffer: attachment.buffer,
        filename: attachment.filename,
        content_type: attachment.content_type,
        ty: None,
    }));
    prepare_envelope(envelope, MAX_DECODED_UPLOAD_BYTES)
}

fn prepare_envelope(envelope: Envelope, max_decoded_bytes: usize) -> Result<Vec<u8>> {
    let (bytes, decoded_bytes) =
        upload::encode_envelope(&envelope).context("failed to serialize feedback envelope")?;
    anyhow::ensure!(
        bytes.len() <= MAX_UPLOAD_BYTES && decoded_bytes <= max_decoded_bytes,
        "feedback envelope exceeds the size limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::FEEDBACK_ENDPOINT_DSN;
    use super::FeedbackTransport;
    use super::prepare_report_attachment;
    use crate::FeedbackAttachment;
    use crate::MAX_UPLOAD_BYTES;
    use codex_http_client::HttpClientFactory;
    use codex_http_client::OutboundProxyPolicy;

    #[test]
    fn persisted_report_transport_remains_on_compatible_envelope_receiver() {
        let transport =
            FeedbackTransport::new(HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault))
                .unwrap();
        let expected: sentry::types::Dsn = FEEDBACK_ENDPOINT_DSN.parse().unwrap();
        assert_eq!(
            transport.dsn.envelope_api_url(),
            expected.envelope_api_url()
        );
    }

    #[test]
    fn persisted_report_rejects_wire_body_above_legacy_receiver_limit() {
        let mut seed = 0x1234_5678_9abc_def0_u64;
        let bytes = (0..MAX_UPLOAD_BYTES + 1024)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        let result = prepare_report_attachment(
            "12345678-1234-1234-1234-123456789abc",
            FeedbackAttachment {
                filename: "incompressible.log".to_string(),
                content_type: None,
                buffer: bytes,
            },
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "feedback envelope exceeds the size limit"
        );
    }
}
