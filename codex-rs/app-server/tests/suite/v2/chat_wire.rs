use anyhow::Context;
use anyhow::Result;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_provider_models_cache_with_models;
use codex_app_server_protocol::DynamicToolCallOutputContentItem;
use codex_app_server_protocol::DynamicToolCallParams;
use codex_app_server_protocol::DynamicToolCallResponse;
use codex_app_server_protocol::DynamicToolCallStatus;
use codex_app_server_protocol::DynamicToolSpec;
use codex_app_server_protocol::ItemCompletedNotification;
use codex_app_server_protocol::ItemStartedNotification;
use codex_app_server_protocol::JSONRPCNotification;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput as V2UserInput;
use codex_models_manager::bundled_models_response;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::Respond;
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

fn chat_completions_tool_call_sse(call_id: &str, function_name: &str, arguments: &str) -> String {
    let tool_call_delta = json!({
        "id": "chatcmpl-tool-1",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": NON_CODEX_CHAT_MODEL,
        "choices": [
            {
                "index": 0,
                "delta": {
                    "tool_calls": [
                        {
                            "index": 0,
                            "id": call_id,
                            "function": {
                                "name": function_name,
                                "arguments": arguments
                            }
                        }
                    ]
                },
                "finish_reason": null
            }
        ]
    });
    let tool_call_done = json!({
        "id": "chatcmpl-tool-1",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": NON_CODEX_CHAT_MODEL,
        "choices": [
            {
                "index": 0,
                "delta": {},
                "finish_reason": "tool_calls"
            }
        ]
    });
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        tool_call_delta, tool_call_done
    )
}

struct ChatCompletionsSeqResponder {
    num_calls: AtomicUsize,
    responses: Vec<String>,
}

impl Respond for ChatCompletionsSeqResponder {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let call_num = self.num_calls.fetch_add(1, Ordering::SeqCst);
        let response = self
            .responses
            .get(call_num)
            .unwrap_or_else(|| panic!("no chat completions response for call {call_num}"));
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_raw(response.clone(), "text/event-stream")
    }
}

async fn mount_chat_completions_sequence(server: &MockServer, responses: Vec<String>) {
    let expected_calls = responses.len() as u64;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ChatCompletionsSeqResponder {
            num_calls: AtomicUsize::new(0),
            responses,
        })
        .expect(expected_calls)
        .mount(server)
        .await;
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

async fn wait_for_dynamic_tool_started(
    mcp: &mut McpProcess,
    call_id: &str,
) -> Result<ItemStartedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/started"),
        )
        .await??;
        let Some(params) = notification.params else {
            continue;
        };
        let started: ItemStartedNotification = serde_json::from_value(params)?;
        if matches!(&started.item, ThreadItem::DynamicToolCall { id, .. } if id == call_id) {
            return Ok(started);
        }
    }
}

async fn wait_for_dynamic_tool_completed(
    mcp: &mut McpProcess,
    call_id: &str,
) -> Result<ItemCompletedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/completed"),
        )
        .await??;
        let Some(params) = notification.params else {
            continue;
        };
        let completed: ItemCompletedNotification = serde_json::from_value(params)?;
        if matches!(&completed.item, ThreadItem::DynamicToolCall { id, .. } if id == call_id) {
            return Ok(completed);
        }
    }
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

fn chat_completion_request_bodies(server_requests: &[wiremock::Request]) -> Result<Vec<Value>> {
    server_requests
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| {
            request
                .body_json()
                .context("chat request body should be JSON")
        })
        .collect()
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

#[tokio::test]
async fn chat_wire_dynamic_tool_round_trip_preserves_namespace_and_outputs() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let call_id = "dyn-call-chat-1";
    let tool_namespace = "codex_app";
    let tool_name = "demo_tool";
    let flattened_tool_name = "codex_app_demo_tool";
    let tool_args = json!({ "city": "Paris" });
    let tool_call_arguments = serde_json::to_string(&tool_args)?;

    let server = MockServer::start().await;
    mount_chat_completions_sequence(
        &server,
        vec![
            chat_completions_tool_call_sse(call_id, flattened_tool_name, &tool_call_arguments),
            chat_completions_sse("Done"),
        ],
    )
    .await;

    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri(), &server.uri())?;
    let bundled_models = bundled_models_response()?.models;
    write_provider_models_cache_with_models(codex_home.path(), "chat_a", bundled_models.clone())?;
    write_provider_models_cache_with_models(codex_home.path(), "chat_b", bundled_models)?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let dynamic_tool = DynamicToolSpec {
        namespace: Some(tool_namespace.to_string()),
        name: tool_name.to_string(),
        description: "Demo dynamic tool".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": ["city"],
            "additionalProperties": false,
        }),
        defer_loading: false,
    };

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            dynamic_tools: Some(vec![dynamic_tool]),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;
    let thread_id = thread.id.clone();

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "Run the dynamic tool".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;
    let turn_id = turn.id.clone();

    let started = wait_for_dynamic_tool_started(&mut mcp, call_id).await?;
    assert_eq!(started.thread_id, thread_id);
    assert_eq!(started.turn_id, turn_id.clone());
    let ThreadItem::DynamicToolCall {
        id,
        namespace,
        tool,
        arguments,
        status,
        content_items,
        success,
        duration_ms,
    } = started.item
    else {
        panic!("expected dynamic tool call item");
    };
    assert_eq!(id, call_id);
    assert_eq!(namespace.as_deref(), Some(tool_namespace));
    assert_eq!(tool, tool_name);
    assert_eq!(arguments, tool_args);
    assert_eq!(status, DynamicToolCallStatus::InProgress);
    assert_eq!(content_items, None);
    assert_eq!(success, None);
    assert_eq!(duration_ms, None);

    let request = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_request_message(),
    )
    .await??;
    let (request_id, params) = match request {
        ServerRequest::DynamicToolCall { request_id, params } => (request_id, params),
        other => panic!("expected DynamicToolCall request, got {other:?}"),
    };
    assert_eq!(
        params,
        DynamicToolCallParams {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            call_id: call_id.to_string(),
            namespace: Some(tool_namespace.to_string()),
            tool: tool_name.to_string(),
            arguments: tool_args.clone(),
        }
    );

    mcp.send_response(
        request_id,
        serde_json::to_value(DynamicToolCallResponse {
            content_items: vec![DynamicToolCallOutputContentItem::InputText {
                text: "dynamic-ok".to_string(),
            }],
            success: true,
        })?,
    )
    .await?;

    let completed = wait_for_dynamic_tool_completed(&mut mcp, call_id).await?;
    assert_eq!(completed.thread_id, thread_id);
    assert_eq!(completed.turn_id, turn_id);
    let ThreadItem::DynamicToolCall {
        status,
        content_items,
        success,
        ..
    } = completed.item
    else {
        panic!("expected dynamic tool call item");
    };
    assert_eq!(status, DynamicToolCallStatus::Completed);
    assert_eq!(
        content_items,
        Some(vec![DynamicToolCallOutputContentItem::InputText {
            text: "dynamic-ok".to_string(),
        }])
    );
    assert_eq!(success, Some(true));

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let server_requests = server.received_requests().await.unwrap_or_default();
    let bodies = chat_completion_request_bodies(&server_requests)?;
    assert_eq!(bodies.len(), 2);

    let first_tools = bodies[0]["tools"]
        .as_array()
        .context("first chat request should include tools")?;
    assert!(
        first_tools
            .iter()
            .all(|tool| tool["type"].as_str() == Some("function")),
        "chat completions tools should all be function tools: {first_tools:?}"
    );
    assert!(
        first_tools.iter().all(|tool| tool.get("tools").is_none()),
        "chat completions tools should not include Responses namespace payloads: {first_tools:?}"
    );
    let chat_tool = first_tools
        .iter()
        .find(|tool| tool["function"]["name"].as_str() == Some(flattened_tool_name))
        .context("namespaced dynamic tool should be flattened for chat completions")?;
    assert_eq!(
        chat_tool["function"]["parameters"],
        json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": ["city"],
            "additionalProperties": false,
        })
    );

    let follow_up_messages = bodies[1]["messages"]
        .as_array()
        .context("follow-up chat request should include messages")?;
    let assistant_tool_call = follow_up_messages
        .iter()
        .find(|message| {
            message["role"].as_str() == Some("assistant") && message["tool_calls"].is_array()
        })
        .context("follow-up chat request should preserve assistant tool call history")?;
    assert_eq!(
        assistant_tool_call["tool_calls"][0]["id"].as_str(),
        Some(call_id)
    );
    assert_eq!(
        assistant_tool_call["tool_calls"][0]["function"]["name"].as_str(),
        Some(flattened_tool_name)
    );
    assert_eq!(
        assistant_tool_call["tool_calls"][0]["function"]["arguments"].as_str(),
        Some(tool_call_arguments.as_str())
    );

    let tool_output_message = follow_up_messages
        .iter()
        .find(|message| {
            message["role"].as_str() == Some("tool")
                && message["tool_call_id"].as_str() == Some(call_id)
        })
        .context("follow-up chat request should include tool output message")?;
    assert_eq!(tool_output_message["content"].as_str(), Some("dynamic-ok"));

    Ok(())
}
