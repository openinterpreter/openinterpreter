#![cfg(not(target_os = "windows"))]

mod common;

use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::repo_root;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::Mutex;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

use crate::common::resolve_exec_bin;
use crate::common::resolve_interpreter_bin;

const TEST_MODEL: &str = "gpt-5.4-mini";
static EXEC_WIRE_PATHS_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DaemonLockfile {
    pid: u32,
    websocket_url: String,
    server_bin: String,
}

struct InterpreterExecRequest<'a> {
    home: &'a Path,
    repo_root: &'a Path,
    extra_args: &'a [&'a str],
    prompt: &'a str,
    expected_output: &'a str,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_exec_profile_responses_routes_through_local_daemon() -> Result<()> {
    let _guard = EXEC_WIRE_PATHS_TEST_LOCK.lock().await;
    let responses_server = MockServer::start().await;
    mount_models(&responses_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses_assistant_sse("INTERPRETEREXECRESPONSESOK")),
        )
        .mount(&responses_server)
        .await;

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_test_config(
        home.path(),
        &repo_root,
        &responses_server.uri(),
        &responses_server.uri(),
    )?;

    let last_message = run_interpreter_exec(InterpreterExecRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "responses"],
        prompt: "Decode the base64 string SU5URVJQUkVURVJFWEVDUkVTUE9OU0VTT0s= and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "INTERPRETEREXECRESPONSESOK",
    })?;

    assert_eq!(last_message.trim(), "INTERPRETEREXECRESPONSESOK");

    let requests = responses_server
        .received_requests()
        .await
        .unwrap_or_default();
    let response_calls = requests
        .iter()
        .filter(|request| request.url.path() == "/v1/responses")
        .count();
    assert_eq!(response_calls, 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_exec_profile_chat_routes_through_chat_completions_proxy() -> Result<()> {
    let _guard = EXEC_WIRE_PATHS_TEST_LOCK.lock().await;
    let chat_server = MockServer::start().await;
    mount_models(&chat_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(
                    chat_completions_sse("INTERPRETEREXECCHATOK"),
                    "text/event-stream",
                ),
        )
        .mount(&chat_server)
        .await;

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_test_config(
        home.path(),
        &repo_root,
        &chat_server.uri(),
        &chat_server.uri(),
    )?;

    let last_message = run_interpreter_exec(InterpreterExecRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "chat"],
        prompt: "Decode the base64 string SU5URVJQUkVURVJFWEVDQ0hBVE9L and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "INTERPRETEREXECCHATOK",
    })?;

    assert_eq!(last_message.trim(), "INTERPRETEREXECCHATOK");

    let requests = chat_server.received_requests().await.unwrap_or_default();
    let chat_calls = requests
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .count();
    let response_calls = requests
        .iter()
        .filter(|request| request.url.path() == "/v1/responses")
        .count();
    assert_eq!(chat_calls, 1);
    assert_eq!(response_calls, 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_exec_reuses_one_daemon_across_responses_and_chat_profiles() -> Result<()> {
    let _guard = EXEC_WIRE_PATHS_TEST_LOCK.lock().await;
    let responses_server = MockServer::start().await;
    mount_models(&responses_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses_assistant_sse("EXECRESPONSESDAEMONOK")),
        )
        .mount(&responses_server)
        .await;

    let chat_server = MockServer::start().await;
    mount_models(&chat_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(
                    chat_completions_sse("EXECCHATDAEMONOK"),
                    "text/event-stream",
                ),
        )
        .mount(&chat_server)
        .await;

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_test_config(
        home.path(),
        &repo_root,
        &responses_server.uri(),
        &chat_server.uri(),
    )?;

    run_interpreter_exec(InterpreterExecRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "responses"],
        prompt: "Decode the base64 string RVhFQ1JFU1BPTlNFU0RBRU1PTk9L and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "EXECRESPONSESDAEMONOK",
    })?;
    let first_lockfile = read_daemon_lockfile(home.path())?;

    run_interpreter_exec(InterpreterExecRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "chat"],
        prompt: "Decode the base64 string RVhFQ0NIQVREQUVNT05PSw== and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "EXECCHATDAEMONOK",
    })?;
    let second_lockfile = read_daemon_lockfile(home.path())?;

    assert_eq!(first_lockfile, second_lockfile);

    let response_requests = responses_server
        .received_requests()
        .await
        .unwrap_or_default();
    let chat_requests = chat_server.received_requests().await.unwrap_or_default();
    assert_eq!(
        response_requests
            .iter()
            .filter(|request| request.url.path() == "/v1/responses")
            .count(),
        1
    );
    assert_eq!(
        chat_requests
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .count(),
        1
    );

    Ok(())
}

async fn mount_models(server: &MockServer) {
    let body = minimal_models_response();
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(body),
        )
        .up_to_n_times(10)
        .mount(server)
        .await;
}

fn minimal_models_response() -> Value {
    json!({
        "models": [{
            "slug": TEST_MODEL,
            "display_name": "GPT 5.4 Mini",
            "description": "Test model",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                {
                    "effort": "medium",
                    "description": "Medium"
                }
            ],
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
    })
}

fn responses_assistant_sse(text: &str) -> String {
    sse(&[
        json!({
            "type": "response.created",
            "response": {
                "id": "resp-1"
            }
        }),
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "id": "msg-1",
                "content": [{
                    "type": "output_text",
                    "text": text
                }]
            }
        }),
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp-1"
            }
        }),
    ])
}

fn chat_completions_sse(text: &str) -> String {
    format!(
        concat!(
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"{TEST_MODEL}\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",",
            "\"content\":\"{text}\"}},\"finish_reason\":null}}]}}\n\n",
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"{TEST_MODEL}\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],",
            "\"usage\":{{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}}}\n\n",
            "data: [DONE]\n\n"
        ),
        TEST_MODEL = TEST_MODEL,
        text = text.replace('"', "\\\""),
    )
}

fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

fn write_test_config(
    home: &Path,
    repo_root: &Path,
    responses_base_url: &str,
    chat_base_url: &str,
) -> Result<()> {
    let repo_root_display = repo_root.display();
    let config_contents = format!(
        r#"
model = "{TEST_MODEL}"
model_provider = "mock_responses"
approval_policy = "never"
sandbox_mode = "read-only"

[profiles.responses]
model = "{TEST_MODEL}"
model_provider = "mock_responses"

[profiles.chat]
model = "{TEST_MODEL}"
model_provider = "mock_chat"

[projects."{repo_root_display}"]
trust_level = "trusted"

[model_providers.mock_responses]
name = "Mock Responses"
base_url = "{responses_base_url}/v1"
env_key = "PATH"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false

[model_providers.mock_chat]
name = "Mock Chat"
base_url = "{chat_base_url}/v1"
env_key = "PATH"
wire_api = "chat"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#
    );
    let config_path = home.join("config.toml");
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

fn read_daemon_lockfile(home: &Path) -> Result<DaemonLockfile> {
    let lockfile_path = home.join("tmp").join("interpreter").join("app-server.json");
    let content = std::fs::read_to_string(&lockfile_path)
        .with_context(|| format!("failed to read {}", lockfile_path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", lockfile_path.display()))
}

fn run_interpreter_exec(request: InterpreterExecRequest<'_>) -> Result<String> {
    let InterpreterExecRequest {
        home,
        repo_root,
        extra_args,
        prompt,
        expected_output,
    } = request;
    let interpreter = resolve_interpreter_bin()?;
    let exec_bin = resolve_exec_bin()?;
    let last_message_path = home.join("last-message.txt");
    let trace_path = home.join("exec-forward-trace.txt");

    let output = Command::new(&interpreter)
        .current_dir(repo_root)
        .env_remove("CODEX_HOME")
        .env("OPEN_INTERPRETER_HOME", home)
        .env("OPEN_INTERPRETER_EXEC_BIN", &exec_bin)
        .env("OPEN_INTERPRETER_EXEC_TRACE_PATH", &trace_path)
        .env("RUST_LOG", "trace")
        .arg("exec")
        .arg("-C")
        .arg(repo_root)
        .arg("-c")
        .arg("analytics.enabled=false")
        .args(extra_args)
        .arg("--output-last-message")
        .arg(&last_message_path)
        .arg(prompt)
        .output()
        .with_context(|| format!("failed to launch {}", interpreter.display()))?;

    let trace = std::fs::read_to_string(&trace_path).unwrap_or_default();
    anyhow::ensure!(
        output.status.success(),
        "interpreter exec exited with status {}; stdout: {}\nstderr: {}\ntrace: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        trace
    );

    let last_message = std::fs::read_to_string(&last_message_path)
        .with_context(|| format!("failed to read {}", last_message_path.display()))?;
    anyhow::ensure!(
        last_message.contains(expected_output),
        "expected `{expected_output}` in last message, got: {last_message}"
    );
    Ok(last_message)
}
