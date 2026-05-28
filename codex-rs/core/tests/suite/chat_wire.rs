use anyhow::Result;
use codex_core::compact::SUMMARIZATION_PROMPT;
use codex_core::compact::SUMMARY_PREFIX;
use codex_features::Feature;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use core_test_support::wait_for_event_match;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio::time::sleep;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::Respond;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

const NON_CODEX_CHAT_MODEL: &str = "gpt-5.4";

fn chat_completions_sse(text: &str) -> String {
    chat_completions_sse_with_total_tokens(text, /*total_tokens*/ 18)
}

fn chat_completions_sse_with_total_tokens(text: &str, total_tokens: i64) -> String {
    format!(
        concat!(
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.4\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",",
            "\"content\":\"{text}\"}},\"finish_reason\":null}}]}}\n\n",
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.4\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],",
            "\"usage\":{{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":{total_tokens}}}}}\n\n",
            "data: [DONE]\n\n"
        ),
        text = text.replace('"', "\\\""),
        total_tokens = total_tokens
    )
}

fn chat_provider(server: &MockServer) -> ModelProviderInfo {
    ModelProviderInfo {
        name: "mock-chat".into(),
        base_url: Some(format!("{}/v1", server.uri())),
        env_key: Some("PATH".into()),
        env_key_instructions: None,
        experimental_bearer_token: None,
        auth: None,
        aws: None,
        wire_api: WireApi::Chat,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: Some(1),
        stream_max_retries: Some(1),
        stream_idle_timeout_ms: Some(2_000),
        websocket_connect_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: false,
    }
}

fn chat_completion_requests(
    requests: &[wiremock::Request],
) -> impl Iterator<Item = &wiremock::Request> {
    requests
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
}

fn request_body_contains_text(request: &wiremock::Request, text: &str) -> bool {
    String::from_utf8_lossy(&request.body).contains(&json_fragment(text))
}

fn json_fragment(text: &str) -> String {
    serde_json::to_string(text)
        .unwrap_or_else(|error| panic!("serialize text to JSON: {error}"))
        .trim_matches('"')
        .to_string()
}

fn single_tool_call_chat_completions_sse(name: &str, call_id: &str, arguments: &str) -> String {
    let name = Value::String(name.to_string());
    let call_id = Value::String(call_id.to_string());
    let arguments = Value::String(arguments.to_string());
    let chunk = json!({
        "id": "chatcmpl-tool-1",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": NON_CODEX_CHAT_MODEL,
        "choices": [{
            "index": 0,
            "delta": {
                "role": "assistant",
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments,
                    }
                }]
            },
            "finish_reason": null
        }]
    });
    let finished = json!({
        "id": "chatcmpl-tool-1",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": NON_CODEX_CHAT_MODEL,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "tool_calls"
        }]
    });
    format!("data: {chunk}\n\ndata: {finished}\n\ndata: [DONE]\n\n")
}

async fn mount_chat_sse_sequence(server: &MockServer, bodies: Vec<String>) {
    struct SeqResponder {
        num_calls: AtomicUsize,
        responses: Vec<String>,
    }

    impl Respond for SeqResponder {
        fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
            let call_num = self.num_calls.fetch_add(1, Ordering::SeqCst);
            match self.responses.get(call_num) {
                Some(body) => ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body.clone()),
                None => panic!("no response for {call_num}"),
            }
        }
    }

    let num_calls = bodies.len();
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(SeqResponder {
            num_calls: AtomicUsize::new(0),
            responses: bodies,
        })
        .up_to_n_times(num_calls as u64)
        .expect(num_calls as u64)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_turn_uses_chat_completions_endpoint() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(
                    chat_completions_sse("hello from chat wire"),
                    "text/event-stream",
                ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let TestCodex {
        codex, cwd, config, ..
    } = test_codex()
        .with_config({
            let provider = chat_provider(&server);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
            }
        })
        .build(&server)
        .await?;

    let model = config
        .model
        .clone()
        .unwrap_or_else(|| "gpt-5.2-codex".to_string());

    codex
        .submit(Op::UserTurn {
            environments: None,
            items: vec![UserInput::Text {
                text: "route me through chat completions".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            approvals_reviewer: None,
            sandbox_policy: SandboxPolicy::new_read_only_policy(),
            permission_profile: None,
            model: model.clone(),
            effort: config.model_reasoning_effort,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let first_delta = wait_for_event_match(&codex, |event| match event {
        EventMsg::AgentMessageContentDelta(event) => Some(event.delta.clone()),
        _ => None,
    })
    .await;
    assert_eq!(first_delta, "hello from chat wire");

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let requests = server.received_requests().await.unwrap_or_default();
    let chat_requests: Vec<_> = chat_completion_requests(&requests).collect();
    assert_eq!(chat_requests.len(), 1);

    let body: Value = chat_requests[0]
        .body_json()
        .expect("chat request body should be valid json");
    assert_eq!(body["model"].as_str(), Some(model.as_str()));
    assert_eq!(body["stream"].as_bool(), Some(true));
    assert_eq!(body["messages"][0]["role"].as_str(), Some("system"));
    assert_eq!(
        body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str),
        Some("user")
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_flattens_namespace_tools_for_chat_completions() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(chat_completions_sse("ok"), "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut builder = test_codex().with_config({
        let provider = chat_provider(&server);
        move |config| {
            config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
            config.model_provider = provider;
        }
    });
    let base_test = builder.build(&server).await?;
    let new_thread = base_test
        .thread_manager
        .start_thread_with_tools(
            base_test.config.clone(),
            vec![DynamicToolSpec {
                namespace: Some("codex_app".to_string()),
                name: "geo_lookup".to_string(),
                description: "Look up a city".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"],
                    "additionalProperties": false
                }),
                defer_loading: false,
            }],
            /*persist_extended_history*/ false,
        )
        .await?;
    let mut test = base_test;
    test.codex = new_thread.thread;
    test.session_configured = new_thread.session_configured;

    test.submit_turn("prepare to use the namespaced tool")
        .await?;

    let requests = server.received_requests().await.unwrap_or_default();
    let chat_requests: Vec<_> = chat_completion_requests(&requests).collect();
    assert_eq!(chat_requests.len(), 1);

    let body: Value = chat_requests[0]
        .body_json()
        .expect("chat request body should be valid json");
    let tools = body["tools"]
        .as_array()
        .expect("chat request should include tools");
    assert!(
        tools
            .iter()
            .all(|tool| tool["type"].as_str() == Some("function")),
        "chat completions tools should all be function tools: {tools:?}"
    );
    assert!(
        tools.iter().all(|tool| tool.get("tools").is_none()),
        "chat completions tools should not include Responses namespace payloads: {tools:?}"
    );
    let dynamic_tool = tools
        .iter()
        .find(|tool| tool["function"]["name"].as_str() == Some("codex_app_geo_lookup"))
        .expect("namespaced dynamic tool should be flattened");
    assert_eq!(
        dynamic_tool["function"]["description"].as_str(),
        Some("Look up a city")
    );
    assert_eq!(
        dynamic_tool["function"]["parameters"],
        json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": ["city"],
            "additionalProperties": false
        })
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_keeps_provider_routing_session_scoped() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server_a = MockServer::start().await;
    let server_b = MockServer::start().await;
    for server in [&server_a, &server_b] {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(chat_completions_sse("ok"), "text/event-stream"),
            )
            .expect(1)
            .mount(server)
            .await;
    }

    let test_a = test_codex()
        .with_config({
            let provider = chat_provider(&server_a);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
            }
        })
        .build(&server_a)
        .await?;
    let test_b = test_codex()
        .with_config({
            let provider = chat_provider(&server_b);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
            }
        })
        .build(&server_b)
        .await?;

    test_a.submit_turn("server a").await?;
    test_b.submit_turn("server b").await?;

    let server_a_requests = server_a.received_requests().await.unwrap_or_default();
    let server_b_requests = server_b.received_requests().await.unwrap_or_default();
    assert_eq!(chat_completion_requests(&server_a_requests).count(), 1);
    assert_eq!(chat_completion_requests(&server_b_requests).count(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_manual_compact_uses_local_prompt_and_preserves_summary() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;
    mount_chat_sse_sequence(
        &server,
        vec![
            chat_completions_sse("FIRST_REPLY"),
            chat_completions_sse("MANUAL_CHAT_SUMMARY"),
            chat_completions_sse("AFTER_COMPACT_REPLY"),
        ],
    )
    .await;

    let test = test_codex()
        .with_config({
            let provider = chat_provider(&server);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
                config.compact_prompt = Some(SUMMARIZATION_PROMPT.to_string());
            }
        })
        .build(&server)
        .await?;

    test.submit_turn("hello compact over chat wire").await?;

    test.codex.submit(Op::Compact).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    test.submit_turn("after compact").await?;

    let requests = server.received_requests().await.unwrap_or_default();
    let chat_requests: Vec<_> = chat_completion_requests(&requests).collect();
    assert_eq!(chat_requests.len(), 3);

    let compact_index = chat_requests
        .iter()
        .enumerate()
        .find_map(|(idx, request)| {
            request_body_contains_text(request, SUMMARIZATION_PROMPT).then_some(idx)
        })
        .expect("compact request missing");
    let compact_body: Value = chat_requests[compact_index]
        .body_json()
        .expect("compact request body should be valid json");
    assert_eq!(
        compact_body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str),
        Some(SUMMARIZATION_PROMPT)
    );

    let follow_up_index = chat_requests
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, request)| {
            (request_body_contains_text(request, "after compact")
                && !request_body_contains_text(request, SUMMARIZATION_PROMPT))
            .then_some(idx)
        })
        .expect("follow-up request missing");
    let follow_up_body: Value = chat_requests[follow_up_index]
        .body_json()
        .expect("follow-up request body should be valid json");
    let follow_up_body = follow_up_body.to_string();
    assert!(follow_up_body.contains(SUMMARY_PREFIX));
    assert!(follow_up_body.contains("MANUAL_CHAT_SUMMARY"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_auto_compact_uses_local_prompt_and_preserves_summary() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;
    mount_chat_sse_sequence(
        &server,
        vec![
            chat_completions_sse_with_total_tokens("FIRST_REPLY", /*total_tokens*/ 70_000),
            chat_completions_sse_with_total_tokens("SECOND_REPLY", /*total_tokens*/ 330_000),
            chat_completions_sse_with_total_tokens("AUTO_CHAT_SUMMARY", /*total_tokens*/ 200),
            chat_completions_sse_with_total_tokens(
                "AFTER_AUTO_COMPACT_REPLY",
                /*total_tokens*/ 120,
            ),
        ],
    )
    .await;

    let test = test_codex()
        .with_config({
            let provider = chat_provider(&server);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
                config.compact_prompt = Some(SUMMARIZATION_PROMPT.to_string());
                config.model_auto_compact_token_limit = Some(200_000);
            }
        })
        .build(&server)
        .await?;

    test.submit_turn("first long turn").await?;
    test.submit_turn("second long turn").await?;
    test.submit_turn("after auto compact").await?;

    let requests = server.received_requests().await.unwrap_or_default();
    let chat_requests: Vec<_> = chat_completion_requests(&requests).collect();
    assert_eq!(chat_requests.len(), 4);

    let compact_index = chat_requests
        .iter()
        .enumerate()
        .find_map(|(idx, request)| {
            request_body_contains_text(request, SUMMARIZATION_PROMPT).then_some(idx)
        })
        .expect("auto compact request missing");
    let compact_body: Value = chat_requests[compact_index]
        .body_json()
        .expect("auto compact request body should be valid json");
    assert_eq!(
        compact_body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str),
        Some(SUMMARIZATION_PROMPT)
    );

    let follow_up_index = chat_requests
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, request)| {
            (request_body_contains_text(request, "after auto compact")
                && !request_body_contains_text(request, SUMMARIZATION_PROMPT))
            .then_some(idx)
        })
        .expect("post-auto-compact request missing");
    let follow_up_body: Value = chat_requests[follow_up_index]
        .body_json()
        .expect("post-auto-compact request body should be valid json");
    let follow_up_body = follow_up_body.to_string();
    assert!(follow_up_body.contains(SUMMARY_PREFIX));
    assert!(follow_up_body.contains("AUTO_CHAT_SUMMARY"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_shell_timeout_round_trips_tool_output() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;
    let call_id = "shell-timeout";
    let timeout_ms = 50u64;
    let args = json!({
        "command": ["/bin/sh", "-c", "yes line | head -n 400; sleep 1"],
        "timeout_ms": timeout_ms,
    })
    .to_string();
    mount_chat_sse_sequence(
        &server,
        vec![
            single_tool_call_chat_completions_sse("shell", call_id, &args),
            chat_completions_sse("done"),
        ],
    )
    .await;

    let test = test_codex()
        .with_config({
            let provider = chat_provider(&server);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
            }
        })
        .build(&server)
        .await?;

    test.submit_turn_with_policies(
        "run a long command",
        AskForApproval::Never,
        SandboxPolicy::DangerFullAccess,
    )
    .await?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let requests = loop {
        let requests = server.received_requests().await.unwrap_or_default();
        if chat_completion_requests(&requests).count() >= 2 || Instant::now() >= deadline {
            break requests;
        }
        sleep(Duration::from_millis(50)).await;
    };
    let chat_requests: Vec<_> = chat_completion_requests(&requests).collect();
    assert_eq!(chat_requests.len(), 2);

    let second_body: Value = chat_requests[1]
        .body_json()
        .expect("tool follow-up request body should be valid json");
    let last_message = second_body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .expect("tool follow-up request should include a last message");
    assert_eq!(last_message["role"].as_str(), Some("tool"));
    assert_eq!(last_message["tool_call_id"].as_str(), Some(call_id));
    let tool_content = last_message["content"]
        .as_str()
        .expect("tool message content should be a string");
    assert!(
        tool_content.contains("command timed out")
            || tool_content.to_ascii_lowercase().contains("signal"),
        "expected timeout or signal output, got {tool_content:?}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_web_search_fails_gracefully_and_turn_continues() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;
    let call_id = "web-search-1";
    let args = json!({
        "query": "latest rust release notes",
    })
    .to_string();
    mount_chat_sse_sequence(
        &server,
        vec![
            single_tool_call_chat_completions_sse("web_search", call_id, &args),
            chat_completions_sse("I can't browse the web through this provider, but I can keep helping from local context."),
        ],
    )
    .await;

    let test = test_codex()
        .with_config({
            let provider = chat_provider(&server);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
            }
        })
        .build(&server)
        .await?;

    test.submit_turn("search the web for the latest Rust release notes")
        .await?;

    let requests = server.received_requests().await.unwrap_or_default();
    let chat_requests: Vec<_> = chat_completion_requests(&requests).collect();
    assert_eq!(chat_requests.len(), 2);

    let second_body: Value = chat_requests[1]
        .body_json()
        .expect("web search follow-up request body should be valid json");
    let last_message = second_body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .expect("web search follow-up request should include a last message");
    assert_eq!(last_message["role"].as_str(), Some("tool"));
    let tool_content = last_message["content"]
        .as_str()
        .expect("web search tool message content should be a string");
    assert!(
        tool_content.contains("unsupported call: web_search"),
        "expected graceful unsupported web_search tool output, got {tool_content:?}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_wire_spawn_agent_flow_marks_child_request_as_subagent() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = MockServer::start().await;
    let call_id = "spawn-call-1";
    let child_prompt = "child: do work over chat wire";
    let spawn_args = json!({
        "message": child_prompt,
    })
    .to_string();
    mount_chat_sse_sequence(
        &server,
        vec![
            single_tool_call_chat_completions_sse("spawn_agent", call_id, &spawn_args),
            chat_completions_sse("child done"),
            chat_completions_sse("parent done"),
        ],
    )
    .await;

    let test = test_codex()
        .with_config({
            let provider = chat_provider(&server);
            move |config| {
                config.model = Some(NON_CODEX_CHAT_MODEL.to_string());
                config.model_provider = provider;
                #[allow(clippy::expect_used)]
                config
                    .features
                    .enable(Feature::Collab)
                    .expect("test config should allow feature update");
            }
        })
        .build(&server)
        .await?;

    test.submit_turn("spawn a helper").await?;

    let deadline = Instant::now() + Duration::from_secs(2);
    let child_request = loop {
        let requests = server.received_requests().await.unwrap_or_default();
        let maybe_child_request = requests.into_iter().find(|request| {
            request.url.path() == "/v1/chat/completions"
                && request_body_contains_text(request, child_prompt)
                && !request_body_contains_text(request, call_id)
        });
        if let Some(child_request) = maybe_child_request {
            break child_request;
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for spawned child chat request");
        }
        sleep(Duration::from_millis(10)).await;
    };

    assert_eq!(
        child_request
            .headers
            .get("x-openai-subagent")
            .and_then(|value| value.to_str().ok()),
        Some("collab_spawn")
    );

    Ok(())
}
