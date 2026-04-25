use anyhow::Result;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_provider_models_cache_with_models;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput as V2UserInput;
use codex_models_manager::bundled_models_response;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

const NON_CODEX_CHAT_MODEL: &str = "gpt-5.4";

#[cfg(windows)]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
#[cfg(not(windows))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn chat_completions_sse(text: &str) -> String {
    format!(
        concat!(
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.4\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",",
            "\"content\":\"{text}\"}},\"finish_reason\":null}}]}}\n\n",
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.4\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],",
            "\"usage\":{{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}}}\n\n",
            "data: [DONE]\n\n"
        ),
        text = text.replace('"', "\\\"")
    )
}

fn create_config_toml(
    codex_home: &std::path::Path,
    server_a_uri: &str,
    server_b_uri: &str,
) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "{NON_CODEX_CHAT_MODEL}"
approval_policy = "never"
sandbox_mode = "read-only"

model_provider = "chat_a"

[model_providers.chat_a]
name = "chat_a"
base_url = "{server_a_uri}/v1"
env_key = "PATH"
wire_api = "chat"
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false

[model_providers.chat_b]
name = "chat_b"
base_url = "{server_b_uri}/v1"
env_key = "PATH"
wire_api = "chat"
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#
        ),
    )
}

async fn start_thread(mcp: &mut McpProcess, expected_model_provider: &str) -> Result<String> {
    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model_provider: Some(expected_model_provider.to_string()),
            ..Default::default()
        })
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let ThreadStartResponse {
        thread,
        model_provider,
        ..
    } = to_response(response)?;
    assert_eq!(model_provider, expected_model_provider);
    Ok(thread.id)
}

async fn run_turn(mcp: &mut McpProcess, thread_id: &str, input: &str) -> Result<()> {
    let request_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.to_string(),
            input: vec![V2UserInput::Text {
                text: input.to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    Ok(())
}

#[tokio::test]
async fn chat_wire_keeps_provider_routing_thread_scoped_within_one_app_server() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server_a = MockServer::start().await;
    let server_b = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(chat_completions_sse("from_a"), "text/event-stream"),
        )
        .expect(2)
        .mount(&server_a)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(chat_completions_sse("from_b"), "text/event-stream"),
        )
        .expect(1)
        .mount(&server_b)
        .await;

    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server_a.uri(), &server_b.uri())?;
    let bundled_models = bundled_models_response()?.models;
    write_provider_models_cache_with_models(codex_home.path(), "chat_a", bundled_models.clone())?;
    write_provider_models_cache_with_models(codex_home.path(), "chat_b", bundled_models)?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_a_id = start_thread(&mut mcp, "chat_a").await?;
    let thread_b_id = start_thread(&mut mcp, "chat_b").await?;

    run_turn(&mut mcp, &thread_a_id, "route thread a").await?;
    run_turn(&mut mcp, &thread_b_id, "route thread b").await?;
    run_turn(&mut mcp, &thread_a_id, "route thread a again").await?;

    let server_a_requests = server_a.received_requests().await.unwrap_or_default();
    let server_b_requests = server_b.received_requests().await.unwrap_or_default();
    let chat_a_requests = server_a_requests
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .count();
    let chat_b_requests = server_b_requests
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .count();
    assert_eq!(chat_a_requests, 2);
    assert_eq!(chat_b_requests, 1);

    Ok(())
}
