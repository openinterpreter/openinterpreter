use crate::request::ToolKinds;
use crate::request::convert_request;
use crate::stream::spawn_chat_stream;
use bytes::Bytes;
use codex_api::ApiError;
use codex_api::AuthProvider;
use codex_api::Compression;
use codex_api::Provider;
use codex_api::RequestTelemetry;
use codex_api::ResponseStream;
use codex_api::ResponsesOptions;
use codex_api::SharedAuthProvider;
use codex_api::SseTelemetry;
use codex_api::build_session_headers;
use codex_client::HttpTransport;
use codex_client::Request;
use codex_client::RequestBody;
use codex_client::RequestCompression;
use codex_client::TransportError;
use codex_client::run_with_retry;
use futures::stream;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use serde_json::Value;
use std::sync::Arc;
use tracing::instrument;

/// Internal extra-header key carrying a per-request chat-completions upstream
/// URL override. This never leaves the process as a real header; the compat
/// client reads it to pick the upstream before building the provider request.
pub const CHAT_WIRE_UPSTREAM_URL_HEADER: &str = "x-codex-chat-wire-upstream-url";

const OPENROUTER_DEFAULT_TOKEN_LIMIT: u64 = 1024;

pub struct ChatCompletionsCompatClient<T: HttpTransport> {
    transport: T,
    provider: Provider,
    auth: SharedAuthProvider,
    request_telemetry: Option<Arc<dyn RequestTelemetry>>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

impl<T: HttpTransport> ChatCompletionsCompatClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            transport,
            provider,
            auth,
            request_telemetry: None,
            sse_telemetry: None,
        }
    }

    pub fn with_telemetry(
        mut self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        self.request_telemetry = request;
        self.sse_telemetry = sse;
        self
    }

    #[instrument(
        name = "chat_wire_compat.stream_request",
        level = "info",
        skip_all,
        fields(http.method = "POST", api.path = "chat/completions")
    )]
    pub async fn stream_request(
        &self,
        request: codex_api::ResponsesApiRequest,
        options: ResponsesOptions,
    ) -> Result<ResponseStream, ApiError> {
        let (body, tool_kinds) = convert_request(&request)?;
        self.stream_chat_request_value(
            serde_json::to_value(body).map_err(|error| ApiError::Stream(error.to_string()))?,
            tool_kinds,
            options,
        )
        .await
    }

    #[instrument(
        name = "chat_wire_compat.stream_chat_request_value",
        level = "info",
        skip_all,
        fields(http.method = "POST", api.path = "chat/completions")
    )]
    pub async fn stream_chat_request_value(
        &self,
        mut body: Value,
        tool_kinds: ToolKinds,
        options: ResponsesOptions,
    ) -> Result<ResponseStream, ApiError> {
        let ResponsesOptions {
            session_id,
            thread_id,
            session_source,
            mut extra_headers,
            compression,
            turn_state: _,
        } = options;
        let provider_base_url = extra_headers
            .get(CHAT_WIRE_UPSTREAM_URL_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or(self.provider.base_url.as_str());
        let is_openrouter = is_openrouter_base_url(provider_base_url);

        // Drop request fields that the target chat provider does not accept. Some
        // harnesses emit provider-specific extensions (e.g. kimi-cli's
        // `prompt_cache_key`, deepseek-tui's `reasoning_content`) that strict
        // OpenAI-compatible upstreams like Groq reject outright.
        sanitize_chat_body_for_provider(&mut body, provider_base_url);

        if let Some(ref thread_id) = thread_id {
            insert_header(&mut extra_headers, "x-client-request-id", thread_id);
        }
        extra_headers.extend(build_session_headers(session_id, thread_id));
        if let Some(subagent) = subagent_header(&session_source) {
            insert_header(&mut extra_headers, "x-openai-subagent", &subagent);
        }

        let request_compression = match compression {
            Compression::None => RequestCompression::None,
            Compression::Zstd => RequestCompression::Zstd,
        };

        let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
        if !streaming {
            let response_result = self
                .execute_with(
                    Method::POST,
                    "chat/completions",
                    extra_headers.clone(),
                    Some(body.clone()),
                    move |request| {
                        request.compression = request_compression;
                    },
                )
                .await;
            let response = match response_result {
                Ok(response) => response,
                Err(error) => {
                    if let Some(retry_body) =
                        openrouter_affordability_retry_body(is_openrouter, &body, &error)
                    {
                        self.execute_with(
                            Method::POST,
                            "chat/completions",
                            extra_headers,
                            Some(retry_body),
                            move |request| {
                                request.compression = request_compression;
                            },
                        )
                        .await?
                    } else {
                        return Err(error);
                    }
                }
            };
            let sse = synthetic_sse_from_chat_completion_response(&response.body)?;
            let bytes = stream::once(async move { Ok(Bytes::from(sse)) });
            return Ok(spawn_chat_stream(
                Box::pin(bytes),
                self.provider.stream_idle_timeout,
                self.sse_telemetry.clone(),
                tool_kinds,
            ));
        }

        let stream_result = self
            .stream_with(
                Method::POST,
                "chat/completions",
                extra_headers.clone(),
                Some(body.clone()),
                move |request| {
                    request.headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("text/event-stream"),
                    );
                    request.compression = request_compression;
                },
            )
            .await;
        let stream_response = match stream_result {
            Ok(response) => response,
            Err(error) => {
                if let Some(retry_body) =
                    openrouter_affordability_retry_body(is_openrouter, &body, &error)
                {
                    self.stream_with(
                        Method::POST,
                        "chat/completions",
                        extra_headers,
                        Some(retry_body),
                        move |request| {
                            request.headers.insert(
                                http::header::ACCEPT,
                                HeaderValue::from_static("text/event-stream"),
                            );
                            request.compression = request_compression;
                        },
                    )
                    .await?
                } else {
                    return Err(error);
                }
            }
        };

        Ok(spawn_chat_stream(
            stream_response.bytes,
            self.provider.stream_idle_timeout,
            self.sse_telemetry.clone(),
            tool_kinds,
        ))
    }

    async fn execute_with<C>(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
        configure: C,
    ) -> Result<codex_client::Response, ApiError>
    where
        C: Fn(&mut Request),
    {
        let make_request = || {
            let mut request = self.make_request(&method, path, &extra_headers, body.as_ref());
            configure(&mut request);
            request
        };

        run_with_retry(
            self.provider.retry.to_policy(),
            make_request,
            |request, attempt| async move {
                let start = std::time::Instant::now();
                let result = self.transport.execute(request).await;
                if let Some(telemetry) = self.request_telemetry.as_ref() {
                    let (status, error) = match &result {
                        Ok(response) => (Some(response.status), None),
                        Err(error) => (http_status(error), Some(error)),
                    };
                    telemetry.on_request(attempt, status, error, start.elapsed());
                }
                result
            },
        )
        .await
        .map_err(ApiError::Transport)
    }

    async fn stream_with<C>(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
        configure: C,
    ) -> Result<codex_client::StreamResponse, ApiError>
    where
        C: Fn(&mut Request),
    {
        let make_request = || {
            let mut request = self.make_request(&method, path, &extra_headers, body.as_ref());
            configure(&mut request);
            request
        };

        run_with_retry(
            self.provider.retry.to_policy(),
            make_request,
            |request, attempt| async move {
                let start = std::time::Instant::now();
                let result = self.transport.stream(request).await;
                if let Some(telemetry) = self.request_telemetry.as_ref() {
                    let (status, error) = match &result {
                        Ok(response) => (Some(response.status), None),
                        Err(error) => (http_status(error), Some(error)),
                    };
                    telemetry.on_request(attempt, status, error, start.elapsed());
                }
                result
            },
        )
        .await
        .map_err(ApiError::Transport)
    }

    fn make_request(
        &self,
        method: &Method,
        path: &str,
        extra_headers: &HeaderMap,
        body: Option<&Value>,
    ) -> Request {
        let mut request = self.provider.build_request(method.clone(), path);
        request.headers.extend(extra_headers.clone());
        if let Some(body) = body {
            request.body = Some(RequestBody::Json(body.clone()));
        }
        add_auth_headers(self.auth.as_ref(), &mut request.headers);
        request
    }
}

/// Remove request fields that specific strict OpenAI-compatible chat providers
/// reject. These are optional, provider-specific extensions, so dropping them
/// for upstreams that don't support them keeps every harness working without
/// changing behavior on the providers that rely on them.
fn sanitize_chat_body_for_provider(body: &mut Value, base_url: &str) {
    let host = base_url.to_ascii_lowercase();
    let is_deepseek = host.contains("api.deepseek.com");
    let is_openrouter = is_openrouter_base_url(&host);

    if is_deepseek && let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if let Some(message_obj) = message.as_object_mut()
                && message_obj.get("role").and_then(Value::as_str) == Some("developer")
            {
                message_obj.insert("role".to_string(), Value::String("user".to_string()));
            }
        }
    }

    if is_openrouter {
        cap_chat_token_limits(body, OPENROUTER_DEFAULT_TOKEN_LIMIT);
        ensure_chat_token_limit(body, OPENROUTER_DEFAULT_TOKEN_LIMIT);
    }

    // Groq's chat completions endpoint is strict about unknown fields.
    let is_groq = host.contains("api.groq.com");
    if !is_groq {
        return;
    }
    cap_chat_token_limits(body, /*cap*/ 32_768);
    if let Some(obj) = body.as_object_mut() {
        // `prompt_cache_key` is a Kimi/OpenAI extension (kimi-cli, kimi-code).
        obj.remove("prompt_cache_key");
        // `thinking` is a Kimi/Anthropic-style toggle (kimi-cli on thinking-toggle
        // models); Groq's chat completions endpoint rejects it.
        obj.remove("thinking");
        // These optional OpenAI chat extensions are not accepted by Groq's
        // strict OpenAI-compatible endpoint.
        obj.remove("parallel_tool_calls");
        obj.remove("response_format");
        obj.remove("reasoning_effort");
        obj.remove("service_tier");
        obj.remove("store");
        obj.remove("verbosity");
        // `reasoning_content` echoed back on assistant messages is a DeepSeek
        // extension (deepseek-tui); Groq rejects it on assistant tool calls.
        if let Some(messages) = obj.get_mut("messages").and_then(Value::as_array_mut) {
            for message in messages {
                if let Some(message_obj) = message.as_object_mut() {
                    message_obj.remove("reasoning_content");
                }
            }
        }
    }
}

fn cap_chat_token_limits(body: &mut Value, cap: u64) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    for key in ["max_completion_tokens", "max_tokens"] {
        if let Some(value) = obj.get_mut(key)
            && value.as_u64().is_some_and(|tokens| tokens > cap)
        {
            *value = Value::Number(cap.into());
        }
    }
}

fn ensure_chat_token_limit(body: &mut Value, limit: u64) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    if obj.contains_key("max_completion_tokens") || obj.contains_key("max_tokens") {
        return;
    }
    obj.insert("max_tokens".to_string(), Value::Number(limit.into()));
}

fn openrouter_affordability_retry_body(
    is_openrouter: bool,
    original_body: &Value,
    error: &ApiError,
) -> Option<Value> {
    let affordable_limit = openrouter_affordable_token_limit(is_openrouter, error)?;
    let mut retry_body = original_body.clone();
    cap_chat_token_limits(&mut retry_body, affordable_limit);
    ensure_chat_token_limit(&mut retry_body, affordable_limit);
    Some(retry_body)
}

fn openrouter_affordable_token_limit(is_openrouter: bool, error: &ApiError) -> Option<u64> {
    if !is_openrouter {
        return None;
    }
    let ApiError::Transport(TransportError::Http { status, body, .. }) = error else {
        return None;
    };
    if *status != http::StatusCode::PAYMENT_REQUIRED {
        return None;
    }
    let body = body.as_deref()?;
    parse_openrouter_affordable_token_limit(body)
}

fn parse_openrouter_affordable_token_limit(body: &str) -> Option<u64> {
    let marker = "can only afford ";
    let start = body.find(marker)? + marker.len();
    let rest = &body[start..];
    let mut digits = String::new();
    for ch in rest.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if (ch == ',' && !digits.is_empty())
            || (digits.is_empty() && ch.is_ascii_whitespace())
        {
            continue;
        } else {
            break;
        }
    }
    let limit = digits.parse::<u64>().ok()?;
    (limit > 0).then_some(limit)
}

fn is_openrouter_base_url(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("openrouter.ai")
}

fn synthetic_sse_from_chat_completion_response(body: &[u8]) -> Result<String, ApiError> {
    let response: Value =
        serde_json::from_slice(body).map_err(|error| ApiError::Stream(error.to_string()))?;
    let id = response
        .get("id")
        .cloned()
        .unwrap_or_else(|| Value::String("chatcmpl-compat".to_string()));
    let model = response.get("model").cloned().unwrap_or(Value::Null);
    let usage = response.get("usage").cloned();
    let choices = response
        .get("choices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut chunks = Vec::new();
    for choice in choices {
        let index = choice
            .get("index")
            .cloned()
            .unwrap_or(Value::Number(0.into()));
        let message = choice.get("message").cloned().unwrap_or(Value::Null);
        let mut delta = serde_json::Map::new();
        if let Some(content) = message.get("content") {
            delta.insert("content".to_string(), content.clone());
        }
        if let Some(reasoning_content) = message.get("reasoning_content") {
            delta.insert("reasoning_content".to_string(), reasoning_content.clone());
        }
        if let Some(tool_calls) = message.get("tool_calls") {
            delta.insert("tool_calls".to_string(), tool_calls.clone());
        }
        chunks.push(serde_json::json!({
            "id": id,
            "model": model,
            "choices": [{
                "index": index,
                "delta": delta,
                "finish_reason": Value::Null,
            }],
        }));
        chunks.push(serde_json::json!({
            "id": id,
            "model": model,
            "choices": [{
                "index": index,
                "delta": {},
                "finish_reason": choice.get("finish_reason").cloned().unwrap_or(Value::Null),
            }],
            "usage": usage,
        }));
    }

    let mut sse = String::new();
    for chunk in chunks {
        sse.push_str("data: ");
        sse.push_str(&chunk.to_string());
        sse.push_str("\n\n");
    }
    sse.push_str("data: [DONE]\n\n");
    Ok(sse)
}

fn add_auth_headers(auth: &dyn AuthProvider, headers: &mut HeaderMap) {
    auth.add_auth_headers(headers);
}

fn http_status(error: &TransportError) -> Option<http::StatusCode> {
    match error {
        TransportError::Http { status, .. } => Some(*status),
        TransportError::RetryLimit
        | TransportError::Timeout
        | TransportError::Connection(_)
        | TransportError::Network(_)
        | TransportError::Build(_)
        | TransportError::ResponseTooLarge { .. } => None,
    }
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) {
    if let (Ok(header_name), Ok(header_value)) = (
        name.parse::<http::HeaderName>(),
        HeaderValue::from_str(value),
    ) {
        headers.insert(header_name, header_value);
    }
}

fn subagent_header(source: &Option<codex_protocol::protocol::SessionSource>) -> Option<String> {
    let codex_protocol::protocol::SessionSource::SubAgent(sub) = source.as_ref()? else {
        return None;
    };
    match sub {
        codex_protocol::protocol::SubAgentSource::Review => Some("review".to_string()),
        codex_protocol::protocol::SubAgentSource::Compact => Some("compact".to_string()),
        codex_protocol::protocol::SubAgentSource::MemoryConsolidation => {
            Some("memory_consolidation".to_string())
        }
        codex_protocol::protocol::SubAgentSource::ThreadSpawn { .. } => {
            Some("collab_spawn".to_string())
        }
        codex_protocol::protocol::SubAgentSource::Other(label) => Some(label.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_api::Compression;
    use codex_api::OpenAiVerbosity;
    use codex_api::ResponsesApiRequest;
    use codex_api::ResponsesOptions;
    use codex_api::RetryConfig;
    use codex_api::TextControls;
    use codex_client::RequestBody;
    use codex_client::Response;
    use codex_protocol::ThreadId;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::SessionSource;
    use codex_protocol::protocol::SubAgentSource;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    use std::time::Duration;

    #[test]
    fn response_too_large_has_no_http_status() {
        assert_eq!(
            http_status(&TransportError::ResponseTooLarge { max_bytes: 1 }),
            None
        );
    }

    #[derive(Debug, Default)]
    struct RecordingTransport {
        last_request: Mutex<Option<Request>>,
    }

    impl HttpTransport for RecordingTransport {
        async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
            unreachable!("chat wire compat tests only use streaming requests")
        }

        async fn stream(
            &self,
            req: Request,
        ) -> Result<codex_client::StreamResponse, TransportError> {
            *self.last_request.lock().expect("record last request") = Some(req);
            Ok(codex_client::StreamResponse {
                status: http::StatusCode::OK,
                headers: HeaderMap::new(),
                bytes: Box::pin(futures::stream::empty()),
            })
        }
    }

    #[derive(Debug, Default)]
    struct OpenRouterRetryTransport {
        requests: Mutex<Vec<Request>>,
        stream_attempts: Mutex<u32>,
    }

    impl HttpTransport for OpenRouterRetryTransport {
        async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
            unreachable!("openrouter retry test only uses streaming requests")
        }

        async fn stream(
            &self,
            req: Request,
        ) -> Result<codex_client::StreamResponse, TransportError> {
            self.requests.lock().expect("record requests").push(req);
            let mut attempts = self.stream_attempts.lock().expect("stream attempts");
            if *attempts == 0 {
                *attempts += 1;
                return Err(TransportError::Http {
                    status: http::StatusCode::PAYMENT_REQUIRED,
                    url: Some("https://openrouter.ai/api/v1/chat/completions".to_string()),
                    headers: None,
                    body: Some(
                        r#"{"error":{"message":"This endpoint requested up to 1024 tokens, but can only afford 140. Please try again with fewer max_tokens."}}"#
                            .to_string(),
                    ),
                });
            }
            *attempts += 1;
            Ok(codex_client::StreamResponse {
                status: http::StatusCode::OK,
                headers: HeaderMap::new(),
                bytes: Box::pin(futures::stream::empty()),
            })
        }
    }

    struct StaticAuth;

    impl AuthProvider for StaticAuth {
        fn add_auth_headers(&self, headers: &mut HeaderMap) {
            headers.insert(
                http::header::AUTHORIZATION,
                "Bearer test-token".parse().expect("valid header value"),
            );
            headers.insert(
                "ChatGPT-Account-ID",
                "acct_123".parse().expect("valid header value"),
            );
        }
    }

    fn test_provider() -> Provider {
        Provider {
            name: "mock-chat".to_string(),
            base_url: "https://example.com/v1".to_string(),
            query_params: Some(HashMap::from([(
                "api-version".to_string(),
                "2026-03-01".to_string(),
            )])),
            headers: HeaderMap::new(),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(1),
                retry_429: false,
                retry_5xx: false,
                retry_transport: false,
            },
            stream_idle_timeout: Duration::from_secs(1),
        }
    }

    fn openrouter_test_provider() -> Provider {
        Provider {
            base_url: "https://openrouter.ai/api/v1".to_string(),
            ..test_provider()
        }
    }

    #[test]
    fn deepseek_sanitizer_maps_developer_messages_to_user() {
        let mut body = serde_json::json!({
            "model": "deepseek-v4-flash",
            "messages": [
                {"role": "system", "content": "system"},
                {"role": "developer", "content": "developer"},
                {"role": "user", "content": "user"}
            ],
            "stream": true
        });

        sanitize_chat_body_for_provider(&mut body, "https://api.deepseek.com/v1");

        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "user");
    }

    #[test]
    fn groq_sanitizer_removes_openai_only_fields() {
        let mut body = serde_json::json!({
            "model": "openai/gpt-oss-120b",
            "messages": [
                {"role": "assistant", "content": "", "reasoning_content": "hidden"}
            ],
            "stream": true,
            "parallel_tool_calls": true,
            "prompt_cache_key": "turn-1",
            "response_format": {"type": "json_object"},
            "reasoning_effort": "high",
            "service_tier": "auto",
            "store": false,
            "thinking": {"type": "enabled"},
            "tool_choice": "auto",
            "verbosity": "low",
            "max_completion_tokens": 384000,
            "max_tokens": 64000
        });

        sanitize_chat_body_for_provider(&mut body, "https://api.groq.com/openai/v1");

        assert_eq!(
            body,
            serde_json::json!({
                "model": "openai/gpt-oss-120b",
                "messages": [
                    {"role": "assistant", "content": ""}
                ],
                "stream": true,
                "tool_choice": "auto",
                "max_completion_tokens": 32768,
                "max_tokens": 32768
            })
        );
    }

    #[test]
    fn openrouter_sanitizer_caps_large_token_limits() {
        let mut body = serde_json::json!({
            "model": "inclusionai/ling-2.6-1t",
            "messages": [
                {"role": "user", "content": "hello"}
            ],
            "stream": true,
            "max_completion_tokens": 32768,
            "max_tokens": 32768
        });

        sanitize_chat_body_for_provider(&mut body, "https://openrouter.ai/api/v1");

        assert_eq!(body["max_completion_tokens"], serde_json::json!(1024));
        assert_eq!(body["max_tokens"], serde_json::json!(1024));
    }

    #[test]
    fn openrouter_sanitizer_inserts_token_limit_when_absent() {
        let mut body = serde_json::json!({
            "model": "inclusionai/ling-2.6-1t",
            "messages": [
                {"role": "user", "content": "hello"}
            ],
            "stream": true
        });

        sanitize_chat_body_for_provider(&mut body, "https://openrouter.ai/api/v1");

        assert_eq!(body["max_tokens"], serde_json::json!(1024));
    }

    #[test]
    fn openrouter_affordability_retry_body_caps_to_reported_limit() {
        let body = serde_json::json!({
            "model": "moonshotai/kimi-k2.7-code",
            "messages": [
                {"role": "user", "content": "hello"}
            ],
            "stream": true,
            "max_completion_tokens": 1024,
            "max_tokens": 1024
        });
        let error = ApiError::Transport(TransportError::Http {
            status: http::StatusCode::PAYMENT_REQUIRED,
            url: None,
            headers: None,
            body: Some(
                r#"{"error":{"message":"requested up to 1,024 tokens, but can only afford 140"}}"#
                    .to_string(),
            ),
        });

        let retry_body =
            openrouter_affordability_retry_body(/*is_openrouter*/ true, &body, &error)
                .expect("retry body");

        assert_eq!(retry_body["max_completion_tokens"], serde_json::json!(140));
        assert_eq!(retry_body["max_tokens"], serde_json::json!(140));
    }

    #[test]
    fn synthetic_sse_from_non_streaming_response_preserves_tool_calls() {
        let body = serde_json::json!({
            "id": "chatcmpl-test",
            "model": "kimi-k2.6",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "run pwd",
                    "tool_calls": [{
                        "index": 0,
                        "id": "bash:0",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"pwd\"}"
                        }
                    }]
                }
            }]
        });

        let sse = synthetic_sse_from_chat_completion_response(&body.to_string().into_bytes())
            .expect("synthetic sse");

        assert!(sse.contains("\"tool_calls\""));
        assert!(sse.contains("\"finish_reason\":\"tool_calls\""));
        assert!(sse.contains("\"id\":\"bash:0\""));
    }

    fn test_request() -> ResponsesApiRequest {
        ResponsesApiRequest {
            model: "gpt-5.2-codex".to_string(),
            instructions: "be terse".to_string(),
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hello".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }],
            tools: Some(crate::request::responses_api_tools(Vec::new())),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: Some(TextControls {
                verbosity: Some(OpenAiVerbosity::Low),
                format: None,
            }),
        }
    }

    #[tokio::test]
    async fn stream_request_marks_thread_spawn_subagents_on_chat_requests() {
        let transport = RecordingTransport::default();
        let client =
            ChatCompletionsCompatClient::new(transport, test_provider(), Arc::new(StaticAuth));
        let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: ThreadId::new(),
            depth: 1,
            agent_path: None,
            agent_nickname: Some("helper".to_string()),
            agent_role: Some("worker".to_string()),
        });

        let _stream = client
            .stream_request(
                test_request(),
                ResponsesOptions {
                    session_id: None,
                    thread_id: Some("conv-123".to_string()),
                    session_source: Some(session_source),
                    extra_headers: HeaderMap::new(),
                    compression: Compression::None,
                    turn_state: Some(Arc::new(OnceLock::new())),
                },
            )
            .await
            .expect("chat request should stream");

        let recorded = client
            .transport
            .last_request
            .lock()
            .expect("recorded request")
            .clone()
            .expect("chat request should be recorded");
        assert_eq!(
            recorded.url,
            "https://example.com/v1/chat/completions?api-version=2026-03-01"
        );
        assert_eq!(
            recorded
                .headers
                .get("x-openai-subagent")
                .and_then(|value| value.to_str().ok()),
            Some("collab_spawn")
        );
        assert_eq!(
            recorded
                .headers
                .get("x-client-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("conv-123")
        );
        assert_eq!(
            recorded
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-token")
        );
        assert_eq!(
            recorded
                .headers
                .get("ChatGPT-Account-ID")
                .and_then(|value| value.to_str().ok()),
            Some("acct_123")
        );
    }

    #[tokio::test]
    async fn stream_chat_request_retries_openrouter_with_affordable_token_limit() {
        let transport = OpenRouterRetryTransport::default();
        let client = ChatCompletionsCompatClient::new(
            transport,
            openrouter_test_provider(),
            Arc::new(StaticAuth),
        );
        let body = serde_json::json!({
            "model": "moonshotai/kimi-k2.7-code",
            "messages": [
                {"role": "user", "content": "hello"}
            ],
            "stream": true,
            "max_tokens": 32768
        });

        let _stream = client
            .stream_chat_request_value(
                body,
                HashMap::new(),
                ResponsesOptions {
                    session_id: None,
                    thread_id: None,
                    session_source: None,
                    extra_headers: HeaderMap::new(),
                    compression: Compression::None,
                    turn_state: Some(Arc::new(OnceLock::new())),
                },
            )
            .await
            .expect("second openrouter request should stream");

        let requests = client.transport.requests.lock().expect("requests");
        assert_eq!(requests.len(), 2);
        let first_body = requests[0]
            .body
            .as_ref()
            .and_then(RequestBody::json)
            .expect("first json body");
        let second_body = requests[1]
            .body
            .as_ref()
            .and_then(RequestBody::json)
            .expect("second json body");
        assert_eq!(first_body["max_tokens"], serde_json::json!(1024));
        assert_eq!(second_body["max_tokens"], serde_json::json!(140));
    }
}
