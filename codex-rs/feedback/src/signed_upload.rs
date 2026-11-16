//! Direct GCS originals for large interactive feedback only. No object bytes
//! pass through the feedback receiver or the model context.

use std::time::Instant;

use anyhow::Result;
use anyhow::anyhow;
use anyhow::ensure;
use bytes::Bytes;
use codex_http_client::RouteAwareClientPool;
use sentry::protocol::Attachment;
use sentry::types::Dsn;
use sha2::Digest;
use sha2::Sha256;
use url::Url;

pub(super) const DIRECT_UPLOAD_THRESHOLD: usize = 7 * 1024 * 1024;

fn validate_grant(
    grant: &serde_json::Value,
    report_id: &str,
    size: usize,
) -> Result<(String, Url)> {
    let path = grant["path"]
        .as_str()
        .ok_or_else(|| anyhow!("invalid feedback signed grant path"))?;
    ensure!(
        path.starts_with(&format!("{report_id}/"))
            && path.len() == report_id.len() + 33
            && path[report_id.len() + 1..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "invalid feedback signed grant path"
    );
    ensure!(
        grant["expires_in"] == 600
            && grant["method"] == "PUT"
            && grant["content_type"] == "application/octet-stream"
            && grant["content_length"] == size,
        "invalid feedback signed grant bounds"
    );
    let signed_url = Url::parse(
        grant["url"]
            .as_str()
            .ok_or_else(|| anyhow!("invalid signed URL"))?,
    )
    .map_err(|_| anyhow!("invalid signed URL"))?;
    ensure!(
        signed_url.scheme() == "https"
            && signed_url.host_str() == Some("storage.googleapis.com")
            && signed_url.username().is_empty()
            && signed_url.password().is_none(),
        "feedback signed URL destination is not GCS"
    );
    Ok((path.to_owned(), signed_url))
}

pub(super) async fn send_large_attachment(
    client_pool: &RouteAwareClientPool,
    dsn: &Dsn,
    report_id: &str,
    capability: &str,
    attachment: Attachment,
    deadline: Instant,
) -> Result<()> {
    ensure!(
        attachment.buffer.len() <= 200 * 1024 * 1024,
        "feedback attachment exceeds limit"
    );
    let mut endpoint = dsn.envelope_api_url();
    endpoint.set_path(&format!("/v0/feedback/uploads/{report_id}"));
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let digest = format!("{:x}", Sha256::digest(&attachment.buffer));
    let body = serde_json::json!({
        "name": attachment.filename,
        "content_type": attachment.content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
        "bytes": attachment.buffer.len(),
        "sha256": digest,
    });
    let request_timeout = deadline.saturating_duration_since(Instant::now());
    ensure!(
        !request_timeout.is_zero(),
        "feedback upload deadline exceeded"
    );
    let response = client_pool
        .post(endpoint.clone())
        .header("X-Feedback-Upload-Capability", capability)
        .json(&body)
        .timeout(request_timeout)
        .send()
        .await
        .map_err(|_| anyhow!("feedback signed grant request failed"))?;
    ensure!(
        response.status().is_success(),
        "feedback signed grant was rejected"
    );
    let response_body = response
        .bytes()
        .await
        .map_err(|_| anyhow!("feedback signed grant response failed"))?;
    ensure!(
        response_body.len() < 4096,
        "feedback signed grant response exceeds limit"
    );
    let grant: serde_json::Value = serde_json::from_slice(&response_body)
        .map_err(|_| anyhow!("invalid feedback signed grant"))?;
    let (path, signed_url) = validate_grant(&grant, report_id, attachment.buffer.len())?;

    let request_timeout = deadline.saturating_duration_since(Instant::now());
    ensure!(
        !request_timeout.is_zero(),
        "feedback upload deadline exceeded"
    );
    let length = attachment.buffer.len();
    let response = client_pool
        .put(signed_url)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", length.to_string())
        .body(Bytes::from(attachment.buffer))
        .timeout(request_timeout)
        .send()
        .await
        .map_err(|_| anyhow!("direct feedback upload failed"))?;
    ensure!(
        response.status().is_success(),
        "direct feedback upload was rejected"
    );

    let mut finalize = endpoint;
    finalize.set_path(&format!("/v0/feedback/uploads/{report_id}/finalize"));
    let request_timeout = deadline.saturating_duration_since(Instant::now());
    ensure!(
        !request_timeout.is_zero(),
        "feedback upload deadline exceeded"
    );
    let response = client_pool
        .post(finalize)
        .header("X-Feedback-Upload-Capability", capability)
        .json(&serde_json::json!({
            "name": body["name"], "content_type": body["content_type"],
            "bytes": body["bytes"], "sha256": body["sha256"], "path": path,
        }))
        .timeout(request_timeout)
        .send()
        .await
        .map_err(|_| anyhow!("feedback attachment finalize failed"))?;
    ensure!(
        response.status().is_success(),
        "feedback attachment finalize was rejected"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_grant;
    use serde_json::json;

    #[test]
    fn grant_must_bind_exact_report_size_and_google_storage_origin() {
        let report = "a".repeat(32);
        let path = format!("{report}/{}", "b".repeat(32));
        let mut grant = json!({"path": path, "url": "https://storage.googleapis.com/bucket/object?X-Goog-Signature=secret",
            "expires_in": 600, "method": "PUT", "content_type": "application/octet-stream", "content_length": 9 * 1024 * 1024});
        assert!(validate_grant(&grant, &report, 9 * 1024 * 1024).is_ok());
        grant["url"] = json!("https://storage.googleapis.com.evil.test/steal");
        assert!(validate_grant(&grant, &report, 9 * 1024 * 1024).is_err());
        grant["url"] = json!("https://storage.googleapis.com/bucket/object");
        assert!(validate_grant(&grant, "c", 9 * 1024 * 1024).is_err());
        assert!(validate_grant(&grant, &report, 8 * 1024 * 1024).is_err());
    }
}
