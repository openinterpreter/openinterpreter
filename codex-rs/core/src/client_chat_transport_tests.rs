use super::ModelClient;
use super::Prompt;
use crate::client_common::ResponseEvent;
use crate::responses_metadata::CodexResponsesMetadata;
use crate::test_support::TestCodexResponsesRequestKind;
use crate::test_support::responses_metadata as test_responses_metadata;
use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;
use codex_login::auth::AgentIdentityAuthPolicy;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::create_oss_provider_with_base_url;
use codex_otel::SessionTelemetry;
use codex_protocol::ThreadId;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::error::CodexErr;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::SessionSource;
use codex_rollout_trace::InferenceTraceContext;
use codex_tools::Harness;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

const TEST_INSTALLATION_ID: &str = "11111111-1111-4111-8111-111111111111";

fn test_model_info(slug: &str) -> ModelInfo {
    serde_json::from_value(json!({
        "slug": slug,
        "display_name": slug,
        "description": "desc",
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": [
            {"effort": "medium", "description": "medium"}
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "upgrade": null,
        "model_messages": null,
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": null,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_image_detail_original": false,
        "context_window": 272000,
        "auto_compact_token_limit": null,
        "experimental_supported_tools": []
    }))
    .expect("deserialize test model info")
}

fn test_session_telemetry() -> SessionTelemetry {
    SessionTelemetry::new(
        ThreadId::new(),
        "gpt-test",
        "gpt-test",
        /*account_id*/ None,
        /*account_email*/ None,
        /*auth_mode*/ None,
        "test-originator".to_string(),
        /*log_user_prompts*/ false,
        "test-terminal".to_string(),
        SessionSource::Cli,
    )
}

fn user_prompt(text: &str) -> Prompt {
    Prompt {
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        ..Prompt::default()
    }
}

fn chat_sse(chunks: Vec<Value>) -> String {
    let mut body = chunks
        .into_iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect::<String>();
    body.push_str("data: [DONE]\n\n");
    body
}

fn assistant_text_sse(id: &str, model: &str, text: &str) -> String {
    chat_sse(vec![
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": { "role": "assistant", "content": text },
                "finish_reason": null
            }]
        }),
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }]
        }),
    ])
}

fn bash_tool_call_sse(id: &str, model: &str, call_id: &str) -> String {
    let arguments = json!({ "command": "pwd" }).to_string();
    chat_sse(vec![
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "tool_calls": [{
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": "Bash",
                            "arguments": arguments
                        }
                    }]
                },
                "finish_reason": null
            }]
        }),
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        }),
    ])
}

fn chat_model_client(base_url: &str, harness: Harness) -> ModelClient {
    let provider = create_oss_provider_with_base_url(base_url, WireApi::Chat);
    ModelClient::new(
        /*auth_manager*/ None,
        AgentIdentityAuthPolicy::JwtOnly,
        ThreadId::new(),
        provider,
        SessionSource::Cli,
        "test_originator".to_string(),
        /*model_verbosity*/ None,
        /*content_item_kinds_enabled*/ false,
        /*enable_request_compression*/ false,
        /*include_timing_metrics*/ false,
        /*beta_features_header*/ None,
        /*concurrent_reasoning_summaries_enabled*/ false,
        /*attestation_provider*/ None,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
    )
    .with_harness(harness, /*harness_guidance*/ true)
}

fn messages_model_client(base_url: &str, harness: Harness) -> ModelClient {
    let provider = create_oss_provider_with_base_url(base_url, WireApi::Messages);
    ModelClient::new(
        /*auth_manager*/ None,
        AgentIdentityAuthPolicy::JwtOnly,
        ThreadId::new(),
        provider,
        SessionSource::Cli,
        "test_originator".to_string(),
        /*model_verbosity*/ None,
        /*content_item_kinds_enabled*/ false,
        /*enable_request_compression*/ false,
        /*include_timing_metrics*/ false,
        /*beta_features_header*/ None,
        /*concurrent_reasoning_summaries_enabled*/ false,
        /*attestation_provider*/ None,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
    )
    .with_harness(harness, /*harness_guidance*/ true)
}

fn anthropic_text_sse(model: &str, text: &str) -> String {
    format!(
        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"{model}\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":1,\"output_tokens\":0}}}}}}\n\nevent: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}\n\nevent: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":1}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
    )
}

fn responses_metadata_for(client: &ModelClient) -> CodexResponsesMetadata {
    let thread_id = client.state.thread_id.to_string();
    test_responses_metadata(
        TEST_INSTALLATION_ID,
        &thread_id,
        &thread_id,
        /*turn_id*/ Some("turn-1"),
        format!("{thread_id}:0"),
        &client.state.session_source,
        /*parent_thread_id*/ None,
        TestCodexResponsesRequestKind::Turn,
    )
}

async fn collect_stream_events(
    client: ModelClient,
    model_slug: &str,
    prompt: Prompt,
) -> Result<Vec<ResponseEvent>, CodexErr> {
    let metadata = responses_metadata_for(&client);
    let mut session = client.new_session();
    let mut stream = session
        .stream(
            &prompt,
            &test_model_info(model_slug),
            &test_session_telemetry(),
            /*effort*/ None,
            ReasoningSummary::None,
            /*service_tier*/ None,
            &metadata,
            &InferenceTraceContext::disabled(),
        )
        .await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    Ok(events)
}

async fn chat_bodies(server: &MockServer, path: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|request| request.url.path() == path)
        .map(|request| serde_json::from_slice(&request.body).expect("chat request json"))
        .collect()
}

#[tokio::test]
async fn zcode_messages_turn_reaches_anthropic_transport() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(anthropic_text_sse("glm-5.2", "hello")),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;

    collect_stream_events(
        messages_model_client(&server.uri(), Harness::ZCode),
        "glm-5.2",
        user_prompt("hello"),
    )
    .await
    .expect("ZCode Messages turn should reach the Anthropic transport");
    Ok(())
}

#[tokio::test]
async fn claude_code_messages_turn_reaches_anthropic_transport() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(anthropic_text_sse("claude-sonnet-4-6", "hello")),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;

    collect_stream_events(
        messages_model_client(&server.uri(), Harness::ClaudeCodeBare),
        "claude-sonnet-4-6",
        user_prompt("hello"),
    )
    .await
    .expect("Claude Code Messages turn should reach the Anthropic transport");
    Ok(())
}

#[tokio::test]
async fn moonshot_k3_kimi_code_tool_turn_uses_chat_completions() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(bash_tool_call_sse("chatcmpl-k3-tool", "kimi-k3", "bash:0")),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;

    let client = chat_model_client(&format!("{}/v1", server.uri()), Harness::KimiCode);
    let events = collect_stream_events(
        client,
        "kimi-k3",
        user_prompt("Run pwd using the shell and report the result. Do not modify files."),
    )
    .await
    .expect("Moonshot K3 tool turn should use the chat compatibility transport");

    let bodies = chat_bodies(&server, "/v1/chat/completions").await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["model"], "kimi-k3");
    let tools = bodies[0]["tools"]
        .as_array()
        .expect("kimi-code request should include tools");
    assert!(
        tools.iter().any(|tool| tool["function"]["name"] == "Bash"),
        "kimi-code tool surface should include Bash: {tools:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { name, .. })
                if name == "Bash"
        )),
        "expected Bash tool call in stream: {events:?}"
    );
    Ok(())
}

#[tokio::test]
async fn chat_wire_native_hello_uses_chat_completions() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(assistant_text_sse(
                    "chatcmpl-native-hello",
                    "deepseek-v4-flash",
                    "hello",
                )),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;

    let client = chat_model_client(&format!("{}/v1", server.uri()), Harness::Native);
    let events = collect_stream_events(client, "deepseek-v4-flash", user_prompt("hello"))
        .await
        .expect("chat-wire native hello should use the chat compatibility transport");

    let bodies = chat_bodies(&server, "/v1/chat/completions").await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["model"], "deepseek-v4-flash");
    assert!(
        events.iter().any(|event| matches!(
            event,
            ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. })
                if content.iter().any(|item| matches!(
                    item,
                    ContentItem::OutputText { text } if text.contains("hello")
                ))
        )),
        "expected assistant hello in stream: {events:?}"
    );
    Ok(())
}

#[tokio::test]
async fn chat_wire_minimal_hello_uses_chat_completions() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(assistant_text_sse(
                    "chatcmpl-minimal-hello",
                    "deepseek-v4-flash",
                    "hello",
                )),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;

    let client = chat_model_client(&format!("{}/v1", server.uri()), Harness::Minimal);
    collect_stream_events(client, "deepseek-v4-flash", user_prompt("hello"))
        .await
        .expect("chat-wire minimal hello should use the chat compatibility transport");

    let bodies = chat_bodies(&server, "/v1/chat/completions").await;
    assert_eq!(bodies.len(), 1);
    let messages = bodies[0]["messages"]
        .as_array()
        .expect("minimal harness should send chat messages");
    assert!(
        messages.iter().any(|message| message["role"] == "system"
            && message["content"]
                .as_str()
                .is_some_and(|text| text.contains("expert software engineer"))),
        "minimal harness should shape a system prompt: {messages:?}"
    );
    Ok(())
}

#[tokio::test]
async fn chat_wire_custom_harness_hello_uses_chat_completions() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(assistant_text_sse(
                    "chatcmpl-custom-hello",
                    "deepseek-v4-flash",
                    "hello",
                )),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;

    let client = chat_model_client(
        &format!("{}/v1", server.uri()),
        Harness::Other("persona-external".to_string()),
    );
    collect_stream_events(client, "deepseek-v4-flash", user_prompt("hello"))
        .await
        .expect("custom chat-wire harness hello should use the chat compatibility transport");

    assert_eq!(chat_bodies(&server, "/v1/chat/completions").await.len(), 1);
    Ok(())
}

#[tokio::test]
async fn open_webui_chat_wire_hello_uses_chat_completions() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(assistant_text_sse(
                    "chatcmpl-open-webui",
                    "local-model",
                    "hello",
                )),
        )
        .expect(/*requests*/ 1)
        .mount(&server)
        .await;

    let client = chat_model_client(&format!("{}/openai/v1", server.uri()), Harness::Native);
    collect_stream_events(client, "local-model", user_prompt("hello"))
        .await
        .expect("Open WebUI chat-wire hello should use the chat compatibility transport");

    assert_eq!(
        chat_bodies(&server, "/openai/v1/chat/completions")
            .await
            .len(),
        1
    );
    Ok(())
}
