use std::fs;

use agent_client_protocol::AcpAgent;
use agent_client_protocol::Agent;
use agent_client_protocol::Client;
use agent_client_protocol::ConnectionTo;
use agent_client_protocol::schema::CloseSessionRequest;
use agent_client_protocol::schema::ContentBlock;
use agent_client_protocol::schema::InitializeRequest;
use agent_client_protocol::schema::ListSessionsRequest;
use agent_client_protocol::schema::NewSessionRequest;
use agent_client_protocol::schema::PromptRequest;
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::SessionConfigKind;
use agent_client_protocol::schema::SessionConfigSelectOptions;
use agent_client_protocol::schema::SessionModeId;
use agent_client_protocol::schema::SessionNotification;
use agent_client_protocol::schema::SessionUpdate;
use agent_client_protocol::schema::SetSessionConfigOptionRequest;
use agent_client_protocol::schema::SetSessionModeRequest;
use agent_client_protocol::schema::StopReason;
use agent_client_protocol::schema::TextContent;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tokio::time::Instant;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path_regex;

#[tokio::test]
async fn acp_subcommand_serves_basic_session_protocol_over_stdio() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/responses$"))
        .respond_with(sse_response(sse(vec![
            response_created_event("resp-1"),
            assistant_message_event("msg-1", "hello from acp"),
            completed_event("resp-1"),
        ])))
        .mount(&server)
        .await;

    fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{}/v1"
env_key = "TEST_ACP_API_KEY"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#,
            server.uri()
        ),
    )?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let agent = AcpAgent::from_args([
        format!("CODEX_HOME={}", codex_home.path().display()),
        "OPEN_INTERPRETER_BRAND=1".to_string(),
        "INTERPRETER_DISABLE_SYSTEM_IMPORT=1".to_string(),
        "TEST_ACP_API_KEY=dummy".to_string(),
        codex_bin.display().to_string(),
        "acp".to_string(),
    ])?;
    let updates = std::sync::Arc::new(Mutex::new(Vec::new()));
    let updates_for_connection = updates.clone();

    Client
        .builder()
        .on_receive_notification(
            {
                let updates = updates.clone();
                async move |notification: SessionNotification, _connection| {
                    updates.lock().await.push(notification.update);
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(agent, |connection: ConnectionTo<Agent>| async move {
            let updates = updates_for_connection;
            let initialize = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;

            assert_eq!(initialize.protocol_version, ProtocolVersion::V1);
            // The bridge keeps login options but must not claim to implement ACP logout.
            let capabilities = serde_json::to_value(&initialize.agent_capabilities)?;
            assert!(capabilities.pointer("/auth/logout").is_none());
            let auth_methods = serde_json::to_value(&initialize.auth_methods)?;
            assert!(auth_methods.to_string().contains("chatgpt"));
            assert!(auth_methods.to_string().contains("codex-api-key"));
            let agent_info = initialize
                .agent_info
                .expect("initialize should include agent info");
            assert_eq!(agent_info.name, "codex-acp");
            assert_eq!(agent_info.title.as_deref(), Some("Open Interpreter"));
            assert!(
                initialize
                    .agent_capabilities
                    .session_capabilities
                    .list
                    .is_some()
            );
            assert!(
                initialize
                    .agent_capabilities
                    .session_capabilities
                    .close
                    .is_some()
            );

            let new_session = connection
                .send_request(NewSessionRequest::new(cwd.path()))
                .block_task()
                .await?;
            assert!(!new_session.session_id.0.is_empty());

            let modes = new_session.modes.expect("ACP modes should be advertised");
            assert_eq!(modes.current_mode_id, SessionModeId::new("workspace-write"));
            assert_eq!(modes.available_modes.len(), 3);

            connection
                .send_request(SetSessionModeRequest::new(
                    new_session.session_id.clone(),
                    SessionModeId::new("read-only"),
                ))
                .block_task()
                .await?;

            let list = connection
                .send_request(ListSessionsRequest::new())
                .block_task()
                .await?;
            assert!(list.next_cursor.is_none() || !list.sessions.is_empty());

            let prompt = connection
                .send_request(PromptRequest::new(
                    new_session.session_id.clone(),
                    vec![ContentBlock::Text(TextContent::new("say hello"))],
                ))
                .block_task()
                .await?;
            assert_eq!(prompt.stop_reason, StopReason::EndTurn);
            wait_for_agent_message(&updates, "hello from acp").await?;
            assert_eq!(agent_message_count(&updates, "hello from acp").await, 1);
            let received_requests = server
                .received_requests()
                .await
                .expect("mock server request recording should be enabled");
            assert_eq!(received_requests.len(), 2);

            connection
                .send_request(CloseSessionRequest::new(new_session.session_id))
                .block_task()
                .await?;

            Ok(())
        })
        .await?;

    Ok(())
}

#[tokio::test]
async fn acp_session_harness_option_round_trips_through_interpreter_config() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .respond_with(sse_response(chat_sse("chatcmpl-1", "mock-model", "hello")))
        .mount(&server)
        .await;
    let config = format!(
        r#"
model = "mock-model"
model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock Chat provider for ACP harness test"
base_url = "{}/v1"
env_key = "TEST_ACP_API_KEY"
wire_api = "chat"
request_max_retries = 0
stream_max_retries = 0
"#,
        server.uri()
    );
    fs::write(codex_home.path().join("config.toml"), &config)?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let agent = AcpAgent::from_args([
        format!("CODEX_HOME={}", codex_home.path().display()),
        "OPEN_INTERPRETER_BRAND=1".to_string(),
        "INTERPRETER_DISABLE_SYSTEM_IMPORT=1".to_string(),
        "TEST_ACP_API_KEY=dummy".to_string(),
        codex_bin.display().to_string(),
        "acp".to_string(),
    ])?;

    Client
        .builder()
        .connect_with(agent, |connection: ConnectionTo<Agent>| async move {
            connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let new_session = connection
                .send_request(NewSessionRequest::new(cwd.path()))
                .block_task()
                .await?;
            let config_options = new_session
                .config_options
                .as_ref()
                .expect("ACP should advertise session config options");
            let harness = config_options
                .iter()
                .find(|option| option.id.0.as_ref() == "harness")
                .expect("ACP should advertise a harness option");
            let select = match &harness.kind {
                SessionConfigKind::Select(select) => select,
                _ => panic!("ACP harness option should be a selector"),
            };
            assert_eq!(select.current_value.0.as_ref(), "");
            let options = match &select.options {
                SessionConfigSelectOptions::Ungrouped(options) => options,
                SessionConfigSelectOptions::Grouped(_) => {
                    panic!("ACP harness options should be ungrouped")
                }
                _ => panic!("unsupported ACP harness option selector"),
            };
            assert!(
                options
                    .iter()
                    .any(|option| option.value.0.as_ref().is_empty()),
                "harness options: {options:?}"
            );

            let initial_prompt = connection
                .send_request(PromptRequest::new(
                    new_session.session_id.clone(),
                    vec![ContentBlock::Text(TextContent::new(
                        "before harness change",
                    ))],
                ))
                .block_task()
                .await?;
            assert_eq!(initial_prompt.stop_reason, StopReason::EndTurn);

            let set = connection
                .send_request(SetSessionConfigOptionRequest::new(
                    new_session.session_id.clone(),
                    "harness",
                    "",
                ))
                .block_task()
                .await?;
            let harness = set
                .config_options
                .iter()
                .find(|option| option.id.0.as_ref() == "harness")
                .expect("set response should include the harness option");
            let select = match &harness.kind {
                SessionConfigKind::Select(select) => select,
                _ => panic!("ACP harness option should be a selector"),
            };
            assert_eq!(select.current_value.0.as_ref(), "");

            let prompt = connection
                .send_request(PromptRequest::new(
                    new_session.session_id.clone(),
                    vec![ContentBlock::Text(TextContent::new("say hello"))],
                ))
                .block_task()
                .await?;
            assert_eq!(prompt.stop_reason, StopReason::EndTurn);
            assert_eq!(
                fs::read_to_string(codex_home.path().join("config.toml"))
                    .map_err(anyhow::Error::from)?,
                config
            );

            connection
                .send_request(CloseSessionRequest::new(new_session.session_id))
                .block_task()
                .await?;
            Ok(())
        })
        .await?;

    Ok(())
}

async fn agent_message_count(updates: &Mutex<Vec<SessionUpdate>>, expected: &str) -> usize {
    updates
        .lock()
        .await
        .iter()
        .filter(|update| agent_message_contains(update, expected))
        .count()
}

async fn wait_for_agent_message(
    updates: &Mutex<Vec<SessionUpdate>>,
    expected: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        {
            let updates = updates.lock().await;
            if updates
                .iter()
                .any(|update| agent_message_contains(update, expected))
            {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "expected prompt to stream an ACP agent message update, got {updates:#?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn agent_message_contains(update: &SessionUpdate, expected: &str) -> bool {
    matches!(
        update,
        SessionUpdate::AgentMessageChunk(chunk)
            if matches!(
                &chunk.content,
                ContentBlock::Text(text) if text.text.contains(expected)
            )
    )
}

fn sse_response(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_raw(body, "text/event-stream")
}

fn chat_sse(id: &str, model: &str, text: &str) -> String {
    format!(
        "data: {{\"id\":\"{id}\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"{text}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"{id}\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"{model}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

fn sse(events: Vec<serde_json::Value>) -> String {
    let mut out = String::new();
    for event in events {
        let kind = event
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("message");
        out.push_str(&format!("event: {kind}\n"));
        out.push_str(&format!("data: {event}\n\n"));
    }
    out
}

fn response_created_event(id: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "response.created",
        "response": {
            "id": id
        }
    })
}

fn assistant_message_event(id: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "id": id,
            "content": [{"type": "output_text", "text": text}]
        }
    })
}

fn completed_event(id: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "response.completed",
        "response": {
            "id": id,
            "usage": {
                "input_tokens": 0,
                "input_tokens_details": null,
                "output_tokens": 0,
                "output_tokens_details": null,
                "total_tokens": 0
            }
        }
    })
}
