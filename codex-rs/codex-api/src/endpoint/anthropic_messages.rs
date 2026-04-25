use crate::anthropic::AnthropicMessageRequest;
use crate::auth::SharedAuthProvider;
use crate::common::ResponseStream;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::sse::spawn_anthropic_response_stream;
use crate::telemetry::SseTelemetry;
use codex_client::HttpTransport;
use codex_client::RequestCompression;
use codex_client::RequestTelemetry;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use std::sync::Arc;
use tracing::instrument;

pub struct AnthropicMessagesClient<T: HttpTransport> {
    session: EndpointSession<T>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

const ANTHROPIC_MESSAGES_PATH: &str = "v1/messages";

impl<T: HttpTransport> AnthropicMessagesClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
            sse_telemetry: None,
        }
    }

    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
            sse_telemetry: sse,
        }
    }

    #[instrument(
        name = "anthropic_messages.stream_request",
        level = "info",
        skip_all,
        fields(
            transport = "anthropic_http",
            http.method = "POST",
            api.path = "v1/messages"
        )
    )]
    pub async fn stream_request(
        &self,
        request: AnthropicMessageRequest,
        extra_headers: HeaderMap,
    ) -> Result<ResponseStream, ApiError> {
        self.stream(request, extra_headers).await
    }

    fn path() -> &'static str {
        ANTHROPIC_MESSAGES_PATH
    }

    #[instrument(
        name = "anthropic_messages.stream",
        level = "info",
        skip_all,
        fields(
            transport = "anthropic_http",
            http.method = "POST",
            api.path = "v1/messages"
        )
    )]
    async fn stream(
        &self,
        request: AnthropicMessageRequest,
        extra_headers: HeaderMap,
    ) -> Result<ResponseStream, ApiError> {
        let body = serde_json::to_value(&request).map_err(|err| {
            ApiError::Stream(format!("failed to encode anthropic request: {err}"))
        })?;
        let stream_response = self
            .session
            .stream_with(
                Method::POST,
                Self::path(),
                extra_headers,
                Some(body),
                |req| {
                    req.headers.remove("originator");
                    req.headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("application/json"),
                    );
                    req.compression = RequestCompression::None;
                },
            )
            .await?;

        Ok(spawn_anthropic_response_stream(
            stream_response,
            self.session.provider().stream_idle_timeout,
            self.sse_telemetry.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn anthropic_messages_path_matches_anthropic_v1_endpoint() {
        assert_eq!(ANTHROPIC_MESSAGES_PATH, "v1/messages");
    }
}
