use std::collections::HashMap;
use std::io::Cursor;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use async_stream::stream;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::header::CACHE_CONTROL;
use axum::http::header::CONTENT_ENCODING;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use bytes::BytesMut;
use codex_api::ApiError;
use codex_api::Compression;
use codex_api::ReqwestTransport;
use codex_api::ResponseEvent;
use codex_api::ResponseStream;
use codex_api::ResponsesApiRequest;
use codex_api::ResponsesOptions;
use codex_client::TransportError;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::OnceCell;
use tracing::warn;
use url::Url;

use crate::client::ChatCompletionsCompatClient;

const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_millis(300_000);
const PROXY_BIND_ADDR: &str = "127.0.0.1:0";
pub const CHAT_WIRE_UPSTREAM_URL_HEADER: &str = "x-codex-chat-wire-upstream-url";
const CHAT_WIRE_PROXY_TRACE_PATH_ENV_VAR: &str = "CODEX_CHAT_WIRE_PROXY_TRACE_PATH";

static LOCAL_RESPONSES_PROXY: OnceCell<LocalResponsesProxy> = OnceCell::const_new();

#[derive(Clone)]
struct LocalResponsesProxy {
    base_url: String,
}

#[derive(Clone)]
struct ProxyState {
    client: Client,
}

#[derive(Clone, Default)]
struct ForwardedAuth {
    bearer_token: Option<String>,
    account_id: Option<String>,
}

impl codex_api::AuthProvider for ForwardedAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        if let Some(token) = self.bearer_token.as_ref()
            && let Ok(header) = HeaderValue::from_str(&format!("Bearer {token}"))
        {
            headers.insert(AUTHORIZATION, header);
        }
        if let Some(account_id) = self.account_id.as_ref()
            && let Ok(header) = HeaderValue::from_str(account_id)
        {
            headers.insert("ChatGPT-Account-ID", header);
        }
    }
}

pub async fn ensure_local_responses_proxy() -> Result<String> {
    let proxy = LOCAL_RESPONSES_PROXY
        .get_or_try_init(spawn_local_responses_proxy)
        .await?;
    Ok(proxy.base_url.clone())
}

pub fn build_chat_completions_upstream_url(
    base_url: &str,
    query_params: Option<&HashMap<String, String>>,
) -> Result<String> {
    let mut url = Url::parse(base_url).with_context(|| format!("parsing base URL `{base_url}`"))?;
    {
        let current_path = url.path().trim_end_matches('/');
        let next_path = if current_path.is_empty() {
            "/chat/completions".to_string()
        } else {
            format!("{current_path}/chat/completions")
        };
        url.set_path(&next_path);
    }
    if let Some(query_params) = query_params {
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query_params {
                pairs.append_pair(key, value);
            }
        }
    }
    Ok(url.to_string())
}

async fn spawn_local_responses_proxy() -> Result<LocalResponsesProxy> {
    let listener = TcpListener::bind(PROXY_BIND_ADDR)
        .await
        .context("binding local chat-wire responses proxy")?;
    let addr = listener
        .local_addr()
        .context("reading local chat-wire responses proxy address")?;
    let state = Arc::new(ProxyState {
        client: Client::builder()
            .build()
            .context("building local chat-wire responses proxy reqwest client")?,
    });
    let app = Router::new()
        .route("/v1/responses", post(handle_responses))
        .route("/v1/responses/compact", post(handle_compact))
        .with_state(state);

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            warn!("local chat-wire responses proxy stopped unexpectedly: {err}");
        }
    });

    Ok(LocalResponsesProxy {
        base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
    })
}

async fn handle_responses(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match stream_via_proxy(state.as_ref(), headers, body).await {
        Ok(stream) => response_stream_to_sse(stream),
        Err(response) => response,
    }
}

async fn handle_compact(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let upstream_url = match upstream_url_from_headers(&headers) {
        Ok(url) => url,
        Err(response) => return response,
    };
    let decoded_body = match decode_request_body(&headers, body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let payload = match serde_json::from_slice::<CompactionInputOwned>(&decoded_body) {
        Ok(payload) => payload,
        Err(error) => {
            return invalid_request_response(format!(
                "failed to decode `/v1/responses/compact` request: {error}"
            ));
        }
    };
    let request = ResponsesApiRequest {
        model: payload.model,
        instructions: payload.instructions,
        input: payload.input,
        tools: payload.tools,
        tool_choice: "auto".to_string(),
        parallel_tool_calls: payload.parallel_tool_calls,
        reasoning: payload.reasoning.map(ReasoningOwned::into_reasoning),
        store: false,
        stream: true,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: payload.text.map(TextControlsOwned::into_text_controls),
        client_metadata: None,
    };
    trace_proxy_request(upstream_url.as_str(), request.model.as_str());
    let stream = match stream_request_to_upstream(
        state.as_ref(),
        request,
        upstream_url.as_str(),
        forwarded_headers(&headers),
        auth_from_headers(&headers),
    )
    .await
    {
        Ok(stream) => stream,
        Err(response) => return response,
    };
    match collect_output_items(stream).await {
        Ok(output) => Json(json!({ "output": output })).into_response(),
        Err(error) => invalid_request_response(error),
    }
}

async fn stream_via_proxy(
    state: &ProxyState,
    headers: HeaderMap,
    body: Bytes,
) -> std::result::Result<ResponseStream, Response> {
    let upstream_url = upstream_url_from_headers(&headers)?;
    let decoded_body = decode_request_body(&headers, body)?;
    let request: ResponsesApiRequest =
        serde_json::from_slice::<ResponsesApiRequestOwned>(&decoded_body)
            .map(ResponsesApiRequestOwned::into_request)
            .map_err(|error| {
                invalid_request_response(format!(
                    "failed to decode `/v1/responses` request: {error}"
                ))
            })?;
    trace_proxy_request(upstream_url.as_str(), request.model.as_str());
    stream_request_to_upstream(
        state,
        request,
        upstream_url.as_str(),
        forwarded_headers(&headers),
        auth_from_headers(&headers),
    )
    .await
}

async fn stream_request_to_upstream(
    state: &ProxyState,
    request: ResponsesApiRequest,
    upstream_url: &str,
    extra_headers: HeaderMap,
    auth: ForwardedAuth,
) -> std::result::Result<ResponseStream, Response> {
    let provider =
        provider_for_upstream_url(upstream_url, extra_headers.clone()).map_err(|error| {
            invalid_request_response(format!("invalid upstream chat completions URL: {error}"))
        })?;
    let mut sanitized_request = request;
    sanitize_request_for_chat_completions(&mut sanitized_request);
    let client = ChatCompletionsCompatClient::new(
        ReqwestTransport::new(state.client.clone()),
        provider,
        auth,
    );
    client
        .stream_request(
            sanitized_request,
            ResponsesOptions {
                conversation_id: None,
                session_source: None,
                extra_headers,
                compression: Compression::None,
                turn_state: None,
            },
        )
        .await
        .map_err(upstream_api_error_response)
}

fn response_stream_to_sse(mut stream: ResponseStream) -> Response {
    let body = Body::from_stream(stream! {
        while let Some(event) = stream.next().await {
            match event
                .map_err(api_error_to_io_error)
                .and_then(|event| serialize_response_event(&event))
            {
                Ok(Some(frame)) => yield Ok::<_, std::io::Error>(frame),
                Ok(None) => {}
                Err(error) => {
                    yield Err::<bytes::Bytes, _>(error);
                    break;
                }
            }
        }
    });
    let mut response = body.into_response();
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

async fn collect_output_items(
    mut stream: ResponseStream,
) -> std::result::Result<Vec<codex_protocol::models::ResponseItem>, String> {
    let mut items = Vec::new();
    while let Some(event) = stream.next().await {
        match event.map_err(|error| error.to_string())? {
            ResponseEvent::OutputItemDone(item) => items.push(item),
            ResponseEvent::Completed { .. } => break,
            ResponseEvent::Created
            | ResponseEvent::OutputItemAdded(_)
            | ResponseEvent::ServerModel(_)
            | ResponseEvent::ServerReasoningIncluded(_)
            | ResponseEvent::OutputTextDelta(_)
            | ResponseEvent::ReasoningSummaryDelta { .. }
            | ResponseEvent::ReasoningContentDelta { .. }
            | ResponseEvent::ReasoningSummaryPartAdded { .. }
            | ResponseEvent::ToolCallInputDelta { .. }
            | ResponseEvent::RateLimits(_)
            | ResponseEvent::ModelsEtag(_) => {}
        }
    }
    Ok(items)
}

fn upstream_url_from_headers(headers: &HeaderMap) -> std::result::Result<String, Response> {
    headers
        .get(CHAT_WIRE_UPSTREAM_URL_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| {
            invalid_request_response(format!(
                "missing required `{CHAT_WIRE_UPSTREAM_URL_HEADER}` header"
            ))
        })
}

fn decode_request_body(headers: &HeaderMap, body: Bytes) -> std::result::Result<Vec<u8>, Response> {
    let is_zstd = headers
        .get(CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|entry| entry.trim().eq_ignore_ascii_case("zstd"))
        });
    if !is_zstd {
        return Ok(body.to_vec());
    }
    zstd::stream::decode_all(Cursor::new(body)).map_err(|error| {
        invalid_request_response(format!("failed to decode zstd request body: {error}"))
    })
}

fn auth_from_headers(headers: &HeaderMap) -> ForwardedAuth {
    ForwardedAuth {
        bearer_token: headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::to_string),
        account_id: headers
            .get("ChatGPT-Account-ID")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
    }
}

fn forwarded_headers(headers: &HeaderMap) -> HeaderMap {
    let mut forwarded = HeaderMap::new();
    for (name, value) in headers {
        if should_skip_forwarded_header(name.as_str()) {
            continue;
        }
        let _ = forwarded.append(name.clone(), value.clone());
    }
    forwarded
}

fn should_skip_forwarded_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "chatgpt-account-id"
            | "host"
            | "content-length"
            | "content-encoding"
            | "connection"
            | "transfer-encoding"
            | CHAT_WIRE_UPSTREAM_URL_HEADER
    )
}

fn provider_for_upstream_url(
    upstream_url: &str,
    headers: HeaderMap,
) -> Result<codex_api::Provider> {
    let mut url = Url::parse(upstream_url)
        .with_context(|| format!("parsing upstream URL `{upstream_url}`"))?;
    let mut query_params = HashMap::new();
    if let Some(query) = url.query() {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            query_params.insert(key.into_owned(), value.into_owned());
        }
    }
    url.set_query(None);
    let path = url.path().trim_end_matches('/').to_string();
    let Some(base_path) = path.strip_suffix("/chat/completions") else {
        anyhow::bail!("upstream URL must end with `/chat/completions`");
    };
    let next_path = if base_path.is_empty() { "/" } else { base_path };
    url.set_path(next_path);
    Ok(codex_api::Provider {
        name: "Chat Wire Upstream".to_string(),
        base_url: url.to_string().trim_end_matches('/').to_string(),
        query_params: (!query_params.is_empty()).then_some(query_params),
        headers,
        retry: codex_api::provider::RetryConfig {
            max_attempts: 1,
            base_delay: Duration::from_millis(200),
            retry_429: false,
            retry_5xx: false,
            retry_transport: false,
        },
        stream_idle_timeout: DEFAULT_STREAM_IDLE_TIMEOUT,
    })
}

fn sanitize_request_for_chat_completions(request: &mut ResponsesApiRequest) {
    request.reasoning = None;
}

fn trace_proxy_request(upstream_url: &str, model: &str) {
    let Ok(path) = std::env::var(CHAT_WIRE_PROXY_TRACE_PATH_ENV_VAR) else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = writeln!(file, "{model}\t{upstream_url}");
}

fn serialize_response_event(event: &ResponseEvent) -> std::io::Result<Option<bytes::Bytes>> {
    let payload = match event {
        ResponseEvent::Created => Some(json!({
            "type": "response.created",
            "response": { "id": "chat-wire-proxy" }
        })),
        ResponseEvent::OutputItemDone(item) => Some(json!({
            "type": "response.output_item.done",
            "item": item
        })),
        ResponseEvent::OutputItemAdded(item) => Some(json!({
            "type": "response.output_item.added",
            "item": item
        })),
        ResponseEvent::Completed {
            response_id,
            token_usage,
        } => Some(json!({
            "type": "response.completed",
            "response": {
                "id": response_id,
                "usage": token_usage.as_ref().map(token_usage_payload).unwrap_or_else(|| json!({
                    "input_tokens": 0,
                    "input_tokens_details": null,
                    "output_tokens": 0,
                    "output_tokens_details": null,
                    "total_tokens": 0
                })),
            }
        })),
        ResponseEvent::OutputTextDelta(delta) => Some(json!({
            "type": "response.output_text.delta",
            "delta": delta
        })),
        ResponseEvent::ReasoningSummaryDelta {
            delta,
            summary_index,
        } => Some(json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": delta,
            "summary_index": summary_index,
        })),
        ResponseEvent::ReasoningContentDelta {
            delta,
            content_index,
        } => Some(json!({
            "type": "response.reasoning_text.delta",
            "delta": delta,
            "content_index": content_index,
        })),
        ResponseEvent::ReasoningSummaryPartAdded { summary_index } => Some(json!({
            "type": "response.reasoning_summary_part.added",
            "summary_index": summary_index,
        })),
        ResponseEvent::ServerModel(_)
        | ResponseEvent::ServerReasoningIncluded(_)
        | ResponseEvent::ToolCallInputDelta { .. }
        | ResponseEvent::RateLimits(_)
        | ResponseEvent::ModelsEtag(_) => None,
    };
    let Some(payload) = payload else {
        return Ok(None);
    };
    let kind = payload
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| std::io::Error::other("missing response event type"))?;
    let mut frame = BytesMut::new();
    frame.extend_from_slice(format!("event: {kind}\n").as_bytes());
    frame.extend_from_slice(b"data: ");
    frame.extend_from_slice(payload.to_string().as_bytes());
    frame.extend_from_slice(b"\n\n");
    Ok(Some(frame.freeze()))
}

fn token_usage_payload(token_usage: &codex_protocol::protocol::TokenUsage) -> Value {
    json!({
        "input_tokens": token_usage.input_tokens,
        "input_tokens_details": {
            "cached_tokens": token_usage.cached_input_tokens,
        },
        "output_tokens": token_usage.output_tokens,
        "output_tokens_details": {
            "reasoning_tokens": token_usage.reasoning_output_tokens,
        },
        "total_tokens": token_usage.total_tokens,
    })
}

fn upstream_api_error_response(error: ApiError) -> Response {
    match error {
        ApiError::Transport(TransportError::Http { status, body, .. }) => {
            let mut response = body.unwrap_or_default().into_response();
            *response.status_mut() = status;
            response
        }
        other => invalid_request_response(other.to_string()),
    }
}

fn api_error_to_io_error(error: ApiError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

fn invalid_request_response(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
            }
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct ResponsesApiRequestOwned {
    model: String,
    #[serde(default)]
    instructions: String,
    input: Vec<codex_protocol::models::ResponseItem>,
    tools: Vec<Value>,
    tool_choice: String,
    parallel_tool_calls: bool,
    reasoning: Option<ReasoningOwned>,
    store: bool,
    stream: bool,
    include: Vec<String>,
    service_tier: Option<String>,
    prompt_cache_key: Option<String>,
    text: Option<TextControlsOwned>,
    client_metadata: Option<HashMap<String, String>>,
}

impl ResponsesApiRequestOwned {
    fn into_request(self) -> ResponsesApiRequest {
        ResponsesApiRequest {
            model: self.model,
            instructions: self.instructions,
            input: self.input,
            tools: self.tools,
            tool_choice: self.tool_choice,
            parallel_tool_calls: self.parallel_tool_calls,
            reasoning: self.reasoning.map(ReasoningOwned::into_reasoning),
            store: self.store,
            stream: self.stream,
            include: self.include,
            service_tier: self.service_tier,
            prompt_cache_key: self.prompt_cache_key,
            text: self.text.map(TextControlsOwned::into_text_controls),
            client_metadata: self.client_metadata,
        }
    }
}

#[derive(serde::Deserialize)]
struct CompactionInputOwned {
    model: String,
    input: Vec<codex_protocol::models::ResponseItem>,
    #[serde(default)]
    instructions: String,
    tools: Vec<Value>,
    parallel_tool_calls: bool,
    reasoning: Option<ReasoningOwned>,
    text: Option<TextControlsOwned>,
}

#[derive(serde::Deserialize)]
struct ReasoningOwned {
    effort: Option<codex_protocol::openai_models::ReasoningEffort>,
    summary: Option<codex_protocol::config_types::ReasoningSummary>,
}

impl ReasoningOwned {
    fn into_reasoning(self) -> codex_api::common::Reasoning {
        codex_api::common::Reasoning {
            effort: self.effort,
            summary: self.summary,
        }
    }
}

#[derive(serde::Deserialize)]
struct TextControlsOwned {
    verbosity: Option<String>,
    format: Option<TextFormatOwned>,
}

impl TextControlsOwned {
    fn into_text_controls(self) -> codex_api::common::TextControls {
        codex_api::common::TextControls {
            verbosity: self
                .verbosity
                .and_then(|verbosity| match verbosity.as_str() {
                    "low" => Some(codex_api::common::OpenAiVerbosity::Low),
                    "medium" => Some(codex_api::common::OpenAiVerbosity::Medium),
                    "high" => Some(codex_api::common::OpenAiVerbosity::High),
                    _ => None,
                }),
            format: self.format.map(TextFormatOwned::into_text_format),
        }
    }
}

#[derive(serde::Deserialize)]
struct TextFormatOwned {
    #[serde(rename = "type")]
    type_name: Option<String>,
    strict: bool,
    schema: Value,
    name: String,
}

impl TextFormatOwned {
    fn into_text_format(self) -> codex_api::common::TextFormat {
        let _ = self.type_name;
        codex_api::common::TextFormat {
            r#type: codex_api::common::TextFormatType::JsonSchema,
            strict: self.strict,
            schema: self.schema,
            name: self.name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::body_string_contains;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[test]
    fn build_upstream_url_preserves_query_params() {
        let url = build_chat_completions_upstream_url(
            "https://example.com/v1",
            Some(&HashMap::from([(
                "api-version".to_string(),
                "2026-03-01".to_string(),
            )])),
        )
        .expect("build upstream URL");

        assert_eq!(
            url,
            "https://example.com/v1/chat/completions?api-version=2026-03-01"
        );
    }

    #[tokio::test]
    async fn local_proxy_streams_responses_sse_from_chat_completions() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("\"stream\":true"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(
                        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n\
                         data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4}}\n\n\
                         data: [DONE]\n\n",
                    ),
            )
            .mount(&upstream)
            .await;

        let proxy_base = ensure_local_responses_proxy()
            .await
            .expect("start local responses proxy");
        let response = Client::new()
            .post(format!("{proxy_base}/responses"))
            .header(
                CHAT_WIRE_UPSTREAM_URL_HEADER,
                format!("{}/v1/chat/completions", upstream.uri()),
            )
            .json(&json!({
                "model": "gpt-5.4-mini",
                "instructions": "",
                "input": [],
                "tools": [],
                "tool_choice": "auto",
                "parallel_tool_calls": true,
                "reasoning": null,
                "store": false,
                "stream": true,
                "include": [],
                "service_tier": null,
                "prompt_cache_key": null,
                "text": null
            }))
            .send()
            .await
            .expect("proxy request should succeed");
        let body = response.text().await.expect("read proxy body");

        assert!(body.contains("event: response.created"));
        assert!(body.contains("event: response.output_text.delta"));
        assert!(body.contains("event: response.output_item.done"));
        assert!(body.contains("event: response.completed"));
    }

    #[tokio::test]
    async fn local_proxy_accepts_responses_request_without_instructions_field() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("\"stream\":true"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(
                        "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n\
                         data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4}}\n\n\
                         data: [DONE]\n\n",
                    ),
            )
            .mount(&upstream)
            .await;

        let proxy_base = ensure_local_responses_proxy()
            .await
            .expect("start local responses proxy");
        let response = Client::new()
            .post(format!("{proxy_base}/responses"))
            .header(
                CHAT_WIRE_UPSTREAM_URL_HEADER,
                format!("{}/v1/chat/completions", upstream.uri()),
            )
            .json(&json!({
                "model": "gpt-5.4-mini",
                "input": [],
                "tools": [],
                "tool_choice": "auto",
                "parallel_tool_calls": true,
                "reasoning": null,
                "store": false,
                "stream": true,
                "include": [],
                "service_tier": null,
                "prompt_cache_key": null,
                "text": null
            }))
            .send()
            .await
            .expect("proxy request should succeed");
        let body = response.text().await.expect("read proxy body");

        assert!(body.contains("event: response.completed"));
    }

    #[tokio::test]
    async fn local_proxy_compact_returns_output_items() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(
                        "data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{\"content\":\"summary\"}}]}\n\n\
                         data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":6,\"completion_tokens\":2,\"total_tokens\":8}}\n\n\
                         data: [DONE]\n\n",
                    ),
            )
            .mount(&upstream)
            .await;

        let proxy_base = ensure_local_responses_proxy()
            .await
            .expect("start local responses proxy");
        let response = Client::new()
            .post(format!("{proxy_base}/responses/compact"))
            .header(
                CHAT_WIRE_UPSTREAM_URL_HEADER,
                format!("{}/v1/chat/completions", upstream.uri()),
            )
            .json(&json!({
                "model": "gpt-5.4-mini",
                "input": [],
                "instructions": "compact this",
                "tools": [],
                "parallel_tool_calls": true,
                "reasoning": null,
                "text": null
            }))
            .send()
            .await
            .expect("proxy compact request should succeed");
        let payload: Value = response.json().await.expect("compact response json");

        assert_eq!(
            payload["output"][0]["type"],
            Value::String("message".to_string())
        );
    }

    #[tokio::test]
    async fn local_proxy_compact_accepts_missing_instructions_field() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(
                        "data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"delta\":{\"content\":\"summary\"}}]}\n\n\
                         data: {\"id\":\"chatcmpl-2\",\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":6,\"completion_tokens\":2,\"total_tokens\":8}}\n\n\
                         data: [DONE]\n\n",
                    ),
            )
            .mount(&upstream)
            .await;

        let proxy_base = ensure_local_responses_proxy()
            .await
            .expect("start local responses proxy");
        let response = Client::new()
            .post(format!("{proxy_base}/responses/compact"))
            .header(
                CHAT_WIRE_UPSTREAM_URL_HEADER,
                format!("{}/v1/chat/completions", upstream.uri()),
            )
            .json(&json!({
                "model": "gpt-5.4-mini",
                "input": [],
                "tools": [],
                "parallel_tool_calls": true,
                "reasoning": null,
                "text": null
            }))
            .send()
            .await
            .expect("proxy compact request should succeed");
        let payload: Value = response.json().await.expect("compact response json");

        assert_eq!(
            payload["output"][0]["type"],
            Value::String("message".to_string())
        );
    }
}
