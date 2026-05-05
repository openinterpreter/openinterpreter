use crate::request::ToolKinds;
use crate::request::convert_request;
use crate::stream::spawn_chat_stream;
use codex_api::ApiError;
use codex_api::AuthProvider;
use codex_api::Compression;
use codex_api::Provider;
use codex_api::RequestTelemetry;
use codex_api::ResponseStream;
use codex_api::ResponsesOptions;
use codex_api::SseTelemetry;
use codex_api::build_conversation_headers;
use codex_client::HttpTransport;
use codex_client::Request;
use codex_client::RequestBody;
use codex_client::RequestCompression;
use codex_client::TransportError;
use codex_client::run_with_retry;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use serde_json::Value;
use std::sync::Arc;
use tracing::instrument;

pub struct ChatCompletionsCompatClient<T: HttpTransport, A: AuthProvider> {
    transport: T,
    provider: Provider,
    auth: A,
    request_telemetry: Option<Arc<dyn RequestTelemetry>>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

impl<T: HttpTransport, A: AuthProvider> ChatCompletionsCompatClient<T, A> {
    pub fn new(transport: T, provider: Provider, auth: A) -> Self {
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
        body: Value,
        tool_kinds: ToolKinds,
        options: ResponsesOptions,
    ) -> Result<ResponseStream, ApiError> {
        let ResponsesOptions {
            conversation_id,
            session_source,
            mut extra_headers,
            compression,
            turn_state: _,
        } = options;

        if let Some(ref conversation_id) = conversation_id {
            insert_header(&mut extra_headers, "x-client-request-id", conversation_id);
        }
        extra_headers.extend(build_conversation_headers(conversation_id));
        if let Some(subagent) = subagent_header(&session_source) {
            insert_header(&mut extra_headers, "x-openai-subagent", &subagent);
        }

        let request_compression = match compression {
            Compression::None => RequestCompression::None,
            Compression::Zstd => RequestCompression::Zstd,
        };

        let stream_response = self
            .stream_with(
                Method::POST,
                "chat/completions",
                extra_headers,
                Some(body),
                move |request| {
                    request.headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("text/event-stream"),
                    );
                    request.compression = request_compression;
                },
            )
            .await?;

        Ok(spawn_chat_stream(
            stream_response.bytes,
            self.provider.stream_idle_timeout,
            self.sse_telemetry.clone(),
            tool_kinds,
        ))
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
        add_auth_headers(&self.auth, &mut request.headers);
        request
    }
}

fn add_auth_headers<A: AuthProvider>(auth: &A, headers: &mut HeaderMap) {
    auth.add_auth_headers(headers);
}

fn http_status(error: &TransportError) -> Option<http::StatusCode> {
    match error {
        TransportError::Http { status, .. } => Some(*status),
        TransportError::RetryLimit
        | TransportError::Timeout
        | TransportError::Network(_)
        | TransportError::Build(_) => None,
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
    use async_trait::async_trait;
    use codex_api::Compression;
    use codex_api::ResponsesApiRequest;
    use codex_api::ResponsesOptions;
    use codex_api::common::OpenAiVerbosity;
    use codex_api::common::TextControls;
    use codex_api::provider::RetryConfig;
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

    #[derive(Debug, Default)]
    struct RecordingTransport {
        last_request: Mutex<Option<Request>>,
    }

    #[async_trait]
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
                end_turn: None,
                phase: None,
            }],
            tools: Vec::new(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            text: Some(TextControls {
                verbosity: Some(OpenAiVerbosity::Low),
                format: None,
            }),
        }
    }

    #[tokio::test]
    async fn stream_request_marks_thread_spawn_subagents_on_chat_requests() {
        let transport = RecordingTransport::default();
        let client = ChatCompletionsCompatClient::new(transport, test_provider(), StaticAuth);
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
                    conversation_id: Some("conv-123".to_string()),
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
}
