use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use async_stream::stream;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::body::Bytes as AxumBytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header::CACHE_CONTROL;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use bytes::Bytes;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::sync::oneshot;

const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const TITLE_RESPONSE_TEXT: &str = "{\"title\":\"Write output file\"}";

#[derive(Clone)]
struct MockState {
    api_key: String,
    model: String,
    response_text: String,
    response_pause: Duration,
    message_calls: Arc<AtomicUsize>,
    message_requests: Arc<Mutex<Vec<serde_json::Value>>>,
    scripted_responses: Arc<Mutex<VecDeque<MockAnthropicResponse>>>,
}

pub(crate) struct MockAnthropicServer {
    base_url: String,
    message_calls: Arc<AtomicUsize>,
    message_requests: Arc<Mutex<Vec<serde_json::Value>>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Debug)]
pub(crate) enum MockAnthropicResponse {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

impl MockAnthropicResponse {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub(crate) fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }
}

impl MockAnthropicServer {
    pub(crate) async fn start(
        api_key: impl Into<String>,
        model: impl Into<String>,
        response_text: impl Into<String>,
        response_pause: Duration,
    ) -> Result<Self> {
        Self::start_with_script(api_key, model, response_text, response_pause, Vec::new()).await
    }

    pub(crate) async fn start_scripted(
        api_key: impl Into<String>,
        model: impl Into<String>,
        scripted_responses: Vec<MockAnthropicResponse>,
        response_pause: Duration,
    ) -> Result<Self> {
        Self::start_with_script(
            api_key,
            model,
            String::new(),
            response_pause,
            scripted_responses,
        )
        .await
    }

    async fn start_with_script(
        api_key: impl Into<String>,
        model: impl Into<String>,
        response_text: impl Into<String>,
        response_pause: Duration,
        scripted_responses: Vec<MockAnthropicResponse>,
    ) -> Result<Self> {
        let message_calls = Arc::new(AtomicUsize::new(0));
        let message_requests = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            api_key: api_key.into(),
            model: model.into(),
            response_text: response_text.into(),
            response_pause,
            message_calls: message_calls.clone(),
            message_requests: message_requests.clone(),
            scripted_responses: Arc::new(Mutex::new(VecDeque::from(scripted_responses))),
        };
        let app = Router::new()
            .route("/v1/models", get(handle_models))
            .route("/v1/messages", post(handle_messages))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding mock anthropic server")?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            if let Err(err) = server.await {
                panic!("mock anthropic server failed: {err}");
            }
        });

        Ok(Self {
            base_url,
            message_calls,
            message_requests,
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn message_call_count(&self) -> usize {
        self.message_calls.load(Ordering::SeqCst)
    }

    pub(crate) async fn message_requests(&self) -> Vec<serde_json::Value> {
        self.message_requests.lock().await.clone()
    }
}

impl Drop for MockAnthropicServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.task.abort();
    }
}

async fn handle_models(State(state): State<MockState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(response) = validate_anthropic_request(&state, &headers, /*require_beta*/ false) {
        return response;
    }

    Json(serde_json::json!({
        "data": [{
            "type": "model",
            "id": state.model,
            "display_name": "Mock Claude",
            "created_at": "2026-04-19T00:00:00Z",
            "max_input_tokens": 200000,
            "max_tokens": 64000,
            "capabilities": model_capabilities(state.model.as_str())
        }],
        "has_more": false
    }))
    .into_response()
}

async fn handle_messages(
    State(state): State<MockState>,
    headers: HeaderMap,
    uri: Uri,
    body: AxumBytes,
) -> impl IntoResponse {
    if let Err(response) = validate_anthropic_request(&state, &headers, /*require_beta*/ true) {
        return response;
    }
    if uri.query() != Some("beta=true") {
        return json_error(
            StatusCode::BAD_REQUEST,
            "expected ?beta=true on Anthropic messages endpoint",
        );
    }

    state.message_calls.fetch_add(1, Ordering::SeqCst);
    let request_body = serde_json::from_slice::<serde_json::Value>(&body).ok();
    if let Some(request_body) = request_body.clone() {
        state.message_requests.lock().await.push(request_body);
    }

    let model = state.model.clone();
    let response = if request_body.as_ref().is_some_and(is_title_request) {
        MockAnthropicResponse::text(TITLE_RESPONSE_TEXT)
    } else {
        state
            .scripted_responses
            .lock()
            .await
            .pop_front()
            .unwrap_or_else(|| MockAnthropicResponse::text(state.response_text.clone()))
    };
    let response_pause = state.response_pause;
    let body_stream = stream! {
        yield Ok::<Bytes, Infallible>(Bytes::from(sse_event(
            "message_start",
            serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": "msg_1",
                    "model": model,
                    "usage": { "input_tokens": 12 }
                }
            }),
        )));
        yield Ok(Bytes::from(sse_event("content_block_start", response_start_block(&response))));
        tokio::time::sleep(response_pause).await;
        if let Some(delta) = response_delta_block(&response) {
            yield Ok(Bytes::from(sse_event("content_block_delta", delta)));
        }
        yield Ok(Bytes::from(sse_event(
            "content_block_stop",
            serde_json::json!({
                "type": "content_block_stop",
                "index": 0
            }),
        )));
        yield Ok(Bytes::from(sse_event(
            "message_delta",
            serde_json::json!({
                "type": "message_delta",
                "usage": { "output_tokens": 3 }
            }),
        )));
        yield Ok(Bytes::from(sse_event(
            "message_stop",
            serde_json::json!({
                "type": "message_stop"
            }),
        )));
    };

    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_static("text/event-stream")),
            (CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        Body::from_stream(body_stream),
    )
        .into_response()
}

fn is_title_request(request_body: &serde_json::Value) -> bool {
    request_body
        .get("output_config")
        .and_then(|output_config| output_config.get("format"))
        .is_some()
        && request_body
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
}

fn response_start_block(response: &MockAnthropicResponse) -> serde_json::Value {
    match response {
        MockAnthropicResponse::Text(_) => serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "text",
                "text": ""
            }
        }),
        MockAnthropicResponse::ToolUse { id, name, input } => serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }
        }),
    }
}

fn response_delta_block(response: &MockAnthropicResponse) -> Option<serde_json::Value> {
    match response {
        MockAnthropicResponse::Text(response_text) => Some(serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "text_delta",
                "text": response_text
            }
        })),
        MockAnthropicResponse::ToolUse { .. } => None,
    }
}

fn validate_anthropic_request(
    state: &MockState,
    headers: &HeaderMap,
    require_beta: bool,
) -> std::result::Result<(), axum::response::Response> {
    let api_key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if api_key != state.api_key {
        return Err(json_error(
            StatusCode::UNAUTHORIZED,
            "expected x-api-key auth header",
        ));
    }

    let version = headers
        .get("anthropic-version")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if version != ANTHROPIC_API_VERSION {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "expected anthropic-version header",
        ));
    }

    if headers.contains_key("authorization") {
        return Err(json_error(
            StatusCode::UNAUTHORIZED,
            "unexpected authorization bearer header",
        ));
    }

    if require_beta {
        let beta = headers
            .get("anthropic-beta")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if beta.is_empty() {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "expected anthropic-beta header on messages requests",
            ));
        }
    }

    Ok(())
}

fn json_error(status: StatusCode, message: &str) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({
            "error": {
                "type": "invalid_request_error",
                "message": message
            }
        })),
    )
        .into_response()
}

fn sse_event(event: &str, payload: serde_json::Value) -> String {
    format!("event: {event}\ndata: {payload}\n\n")
}

fn model_capabilities(model: &str) -> serde_json::Value {
    let lower = model.to_ascii_lowercase();
    let effort_supported = !(lower.contains("haiku") || lower.contains("sonnet-4-5"));
    let adaptive_supported =
        lower.contains("opus-4-6") || lower.contains("opus-4-7") || lower.contains("sonnet-4-6");

    serde_json::json!({
        "effort": {
            "supported": effort_supported,
            "low": { "supported": effort_supported },
            "medium": { "supported": effort_supported },
            "high": { "supported": effort_supported },
            "max": { "supported": effort_supported && lower.contains("opus-4-7") }
        },
        "image_input": { "supported": true },
        "pdf_input": { "supported": true },
        "structured_outputs": { "supported": true },
        "thinking": {
            "supported": true,
            "types": {
                "enabled": { "supported": true },
                "adaptive": { "supported": adaptive_supported }
            }
        }
    })
}
