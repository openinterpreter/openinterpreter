use codex_http_client::build_reqwest_client_with_custom_ca;
use codex_protocol::models::FunctionCallOutputContentItem;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::boxed_tool_output;

const MEDIA_UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const ASF_HEADER: [u8; 16] = [
    0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce, 0x6c,
];

#[derive(Clone, Copy)]
pub(super) enum ReadMode {
    Default,
    ImageOptionsRequested,
}

#[derive(Deserialize)]
struct VideoUploadResponse {
    id: String,
}

pub(super) async fn handle(
    invocation: &ToolInvocation,
    path: &Path,
    data: &[u8],
    mime_type: &'static str,
    read_mode: ReadMode,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    if matches!(read_mode, ReadMode::ImageOptionsRequested) {
        return Err(FunctionCallError::RespondToModel(
            "region and full_resolution apply only to image files.".to_string(),
        ));
    }

    let base_url = invocation
        .turn
        .provider
        .runtime_base_url()
        .await
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "Failed to resolve the video upload endpoint: {error}"
            ))
        })?
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "The current provider does not expose a video upload endpoint.".to_string(),
            )
        })?;
    let auth = invocation.turn.provider.api_auth().await.map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "Failed to resolve credentials for video upload: {error}"
        ))
    })?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("video");
    let boundary = format!("----open-interpreter-kimi-video-{}", Uuid::new_v4());
    let body = multipart_body(&boundary, filename, mime_type, data);
    let client =
        build_reqwest_client_with_custom_ca(reqwest::Client::builder()).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "failed to build Kimi video upload client");
            reqwest::Client::new()
        });
    let request = client
        .post(format!("{}/files", base_url.trim_end_matches('/')))
        .timeout(MEDIA_UPLOAD_TIMEOUT)
        .headers(auth.to_auth_headers())
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body);
    let response = tokio::select! {
        _ = invocation.cancellation_token.cancelled() => {
            return Err(FunctionCallError::RespondToModel(
                "Video upload was cancelled.".to_string(),
            ));
        }
        response = request.send() => response,
    }
    .map_err(|error| {
        FunctionCallError::RespondToModel(format!("Failed to upload video: {error}"))
    })?;
    let status = response.status();
    let response_body = response.bytes().await.map_err(|error| {
        FunctionCallError::RespondToModel(format!(
            "Failed to read the video upload response: {error}"
        ))
    })?;
    if !status.is_success() {
        let body = String::from_utf8_lossy(&response_body)
            .chars()
            .take(4_096)
            .collect::<String>();
        return Err(FunctionCallError::RespondToModel(format!(
            "Video upload failed with status {status}: {body}"
        )));
    }
    let uploaded: VideoUploadResponse =
        serde_json::from_slice(&response_body).map_err(|error| {
            FunctionCallError::RespondToModel(format!(
                "Failed to decode the video upload response: {error}"
            ))
        })?;
    if uploaded.id.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "The video upload response did not include a file id.".to_string(),
        ));
    }

    let absolute_path = path.to_string_lossy();
    let video_url = format!("ms://{}", uploaded.id);
    let note = format!(
        "<system>Read video file. Mime type: {mime_type}. Size: {} bytes. If you generate or edit images or videos via commands or scripts, read the result back immediately before continuing.</system>",
        data.len()
    );
    Ok(boxed_tool_output(FunctionToolOutput::from_content(
        vec![
            FunctionCallOutputContentItem::InputText {
                text: format!("<video path=\"{absolute_path}\">"),
            },
            FunctionCallOutputContentItem::InputVideo {
                video_url,
                id: Some(uploaded.id),
            },
            FunctionCallOutputContentItem::InputText {
                text: "</video>".to_string(),
            },
            FunctionCallOutputContentItem::InputText { text: note },
        ],
        /*success*/ Some(true),
    )))
}

fn multipart_body(boundary: &str, filename: &str, mime_type: &str, data: &[u8]) -> Vec<u8> {
    let filename = filename
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "_")
        .replace('\n', "_");
    let mut body = Vec::with_capacity(data.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(
        format!(
            "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\nvideo\r\n--{boundary}--\r\n"
        )
        .as_bytes(),
    );
    body
}

pub(super) fn mime_type(path: &Path, data: &[u8]) -> Option<&'static str> {
    sniff_mime_type(data).or_else(|| {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "mp4" => Some("video/mp4"),
            "mpg" | "mpeg" => Some("video/mpeg"),
            "mkv" => Some("video/x-matroska"),
            "avi" => Some("video/x-msvideo"),
            "mov" => Some("video/quicktime"),
            "ogv" => Some("video/ogg"),
            "wmv" => Some("video/x-ms-wmv"),
            "webm" => Some("video/webm"),
            "m4v" => Some("video/x-m4v"),
            "flv" => Some("video/x-flv"),
            "3gp" => Some("video/3gpp"),
            "3g2" => Some("video/3gpp2"),
            _ => None,
        }
    })
}

fn sniff_mime_type(data: &[u8]) -> Option<&'static str> {
    let header = &data[..data.len().min(512)];
    if header.starts_with(b"RIFF") && header.get(8..12) == Some(b"AVI ") {
        return Some("video/x-msvideo");
    }
    if header.starts_with(b"FLV") {
        return Some("video/x-flv");
    }
    if header.starts_with(&ASF_HEADER) {
        return Some("video/x-ms-wmv");
    }
    if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let lowered = String::from_utf8_lossy(header).to_ascii_lowercase();
        if lowered.contains("webm") {
            return Some("video/webm");
        }
        if lowered.contains("matroska") {
            return Some("video/x-matroska");
        }
    }
    if header.get(4..8) == Some(b"ftyp") {
        let brand = header
            .get(8..12)?
            .iter()
            .copied()
            .take_while(|byte| !byte.is_ascii_whitespace() && *byte != 0)
            .map(|byte| byte.to_ascii_lowercase())
            .collect::<Vec<_>>();
        return match brand.as_slice() {
            b"isom" | b"iso2" | b"iso5" | b"mp41" | b"mp42" | b"avc1" | b"mp4v" => {
                Some("video/mp4")
            }
            b"m4v" => Some("video/x-m4v"),
            b"qt" => Some("video/quicktime"),
            b"3gp4" | b"3gp5" | b"3gp6" | b"3gp7" => Some("video/3gpp"),
            b"3g2" => Some("video/3gpp2"),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
#[path = "kimi_code_video_tests.rs"]
mod tests;
