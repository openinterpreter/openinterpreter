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
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header::CACHE_CONTROL;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use bytes::Bytes;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::sync::oneshot;

#[derive(Clone)]
struct MockState {
    model: String,
    response_text: String,
    response_pause: Duration,
    response_calls: Arc<AtomicUsize>,
    response_requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

pub(crate) struct MockResponsesServer {
    base_url: String,
    response_calls: Arc<AtomicUsize>,
    response_requests: Arc<Mutex<Vec<serde_json::Value>>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockResponsesServer {
    pub(crate) async fn start(
        model: impl Into<String>,
        response_text: impl Into<String>,
        response_pause: Duration,
    ) -> Result<Self> {
        let response_calls = Arc::new(AtomicUsize::new(0));
        let response_requests = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            model: model.into(),
            response_text: response_text.into(),
            response_pause,
            response_calls: response_calls.clone(),
            response_requests: response_requests.clone(),
        };
        let app = Router::new()
            .route("/v1/models", get(handle_models))
            .route("/v1/responses", post(handle_responses))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding mock responses server")?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            if let Err(err) = server.await {
                panic!("mock responses server failed: {err}");
            }
        });

        Ok(Self {
            base_url,
            response_calls,
            response_requests,
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn response_call_count(&self) -> usize {
        self.response_calls.load(Ordering::SeqCst)
    }

    pub(crate) async fn response_requests(&self) -> Vec<serde_json::Value> {
        self.response_requests.lock().await.clone()
    }
}

impl Drop for MockResponsesServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.task.abort();
    }
}

async fn handle_models(State(state): State<MockState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "models": [{
            "slug": state.model,
            "display_name": "GPT 5.4 Mini",
            "description": "Test model",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [{
                "effort": "medium",
                "description": "Medium"
            }],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "availability_nux": null,
            "upgrade": null,
            "base_instructions": "Be terse.",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "default_reasoning_summary": "auto",
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "web_search_tool_type": "text",
            "truncation_policy": {
                "mode": "bytes",
                "limit": 10000
            },
            "supports_parallel_tool_calls": true,
            "supports_image_detail_original": false,
            "context_window": null,
            "auto_compact_token_limit": null,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "input_modalities": ["text", "image"],
            "supports_search_tool": false
        }]
    }))
}

async fn handle_responses(State(state): State<MockState>, body: AxumBytes) -> impl IntoResponse {
    state.response_calls.fetch_add(1, Ordering::SeqCst);
    if let Ok(request_body) = serde_json::from_slice::<serde_json::Value>(&body) {
        state.response_requests.lock().await.push(request_body);
    }

    let response_text = state.response_text.clone();
    let response_pause = state.response_pause;
    let body_stream = stream! {
        yield Ok::<Bytes, Infallible>(Bytes::from(sse_event(serde_json::json!({
            "type": "response.created",
            "response": { "id": "resp-1" }
        }))));
        tokio::time::sleep(response_pause).await;
        yield Ok(Bytes::from(sse_event(serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "id": "msg-1",
                "content": [{
                    "type": "output_text",
                    "text": response_text
                }]
            }
        }))));
        tokio::time::sleep(Duration::from_millis(150)).await;
        yield Ok(Bytes::from(sse_event(serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp-1" }
        }))));
    };

    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_static("text/event-stream")),
            (CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        Body::from_stream(body_stream),
    )
}

fn sse_event(event: serde_json::Value) -> String {
    format!("data: {event}\n\n")
}
