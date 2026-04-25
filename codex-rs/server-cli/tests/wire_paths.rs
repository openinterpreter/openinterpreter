#![cfg(not(target_os = "windows"))]

mod common;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::repo_root;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;
use tokio::select;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

use crate::common::resolve_interpreter_bin;

const TEST_MODEL: &str = "gpt-5.4-mini";

#[derive(Clone, Copy)]
enum PromptExitMode {
    TerminateClientOnMarker,
}

struct InterpreterPromptRequest<'a> {
    home: &'a Path,
    repo_root: &'a Path,
    extra_args: &'a [&'a str],
    prompt: &'a str,
    expected_output: &'a str,
    exit_mode: PromptExitMode,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DaemonLockfile {
    pid: u32,
    websocket_url: String,
    server_bin: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_profile_responses_routes_through_local_daemon() -> Result<()> {
    let responses_server = MockServer::start().await;
    mount_models(&responses_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses_assistant_sse("INTERPRETERRESPONSESOK")),
        )
        .expect(1)
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

    let output = run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "responses", "--no-alt-screen"],
        prompt: "Decode the base64 string SU5URVJQUkVURVJSRVNQT05TRVNPSw== and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "INTERPRETERRESPONSESOK",
        exit_mode: PromptExitMode::TerminateClientOnMarker,
    })
    .await?;

    assert!(
        output.contains("INTERPRETERRESPONSESOK"),
        "expected marker in TUI output, got: {output}"
    );

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
async fn interpreter_profile_chat_routes_through_chat_completions_proxy() -> Result<()> {
    let chat_server = MockServer::start().await;
    mount_models(&chat_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(
                    chat_completions_sse("INTERPRETERCHATOK"),
                    "text/event-stream",
                ),
        )
        .expect(1)
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

    let output = run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "chat", "--no-alt-screen"],
        prompt: "Decode the base64 string SU5URVJQUkVURVJDSEFUT0s= and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "INTERPRETERCHATOK",
        exit_mode: PromptExitMode::TerminateClientOnMarker,
    })
    .await?;

    assert!(
        output.contains("INTERPRETERCHATOK"),
        "expected marker in TUI output, got: {output}"
    );

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
async fn interpreter_reuses_one_daemon_across_responses_and_chat_profiles() -> Result<()> {
    let responses_server = MockServer::start().await;
    mount_models(&responses_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses_assistant_sse("RESPONSESDAEMONOK")),
        )
        .expect(1)
        .mount(&responses_server)
        .await;

    let chat_server = MockServer::start().await;
    mount_models(&chat_server).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(chat_completions_sse("CHATDAEMONOK"), "text/event-stream"),
        )
        .expect(1)
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

    run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "responses", "--no-alt-screen"],
        prompt: "Decode the base64 string UkVTUE9OU0VTREFFTU9OT0s= and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "RESPONSESDAEMONOK",
        exit_mode: PromptExitMode::TerminateClientOnMarker,
    })
    .await?;
    let first_lockfile = read_daemon_lockfile(home.path())?;

    run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        extra_args: &["--profile", "chat", "--no-alt-screen"],
        prompt: "Decode the base64 string Q0hBVERBRU1PTk9L and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "CHATDAEMONOK",
        exit_mode: PromptExitMode::TerminateClientOnMarker,
    })
    .await?;
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

async fn run_interpreter_prompt(request: InterpreterPromptRequest<'_>) -> Result<String> {
    let InterpreterPromptRequest {
        home,
        repo_root,
        extra_args,
        prompt,
        expected_output,
        exit_mode,
    } = request;
    let interpreter = resolve_interpreter_bin()?;
    let mut env = HashMap::new();
    env.insert(
        "OPEN_INTERPRETER_HOME".to_string(),
        home.display().to_string(),
    );
    env.insert("RUST_LOG".to_string(), "trace".to_string());

    let mut args = vec![
        "-C".to_string(),
        repo_root.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
    ];
    args.extend(extra_args.iter().map(ToString::to_string));
    args.push(prompt.to_string());

    let spawned = codex_utils_pty::spawn_pty_process(
        interpreter.to_string_lossy().as_ref(),
        &args,
        repo_root,
        &env,
        &None,
        codex_utils_pty::TerminalSize::default(),
    )
    .await?;

    let codex_utils_pty::SpawnedProcess {
        session,
        stdout_rx,
        stderr_rx,
        exit_rx,
    } = spawned;
    let mut output_rx = codex_utils_pty::combine_output_receivers(stdout_rx, stderr_rx);
    let mut exit_rx = exit_rx;
    let writer_tx = session.writer_sender();
    let mut output = Vec::new();
    let mut screen = vt100::Parser::new(24, 80, 0);
    let mut exit_requested = false;

    let exit_code = timeout(Duration::from_secs(90), async {
        loop {
            select! {
                result = output_rx.recv() => match result {
                    Ok(chunk) => {
                        if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                            let _ = writer_tx.send(b"\x1b[1;1R".to_vec()).await;
                        }
                        output.extend_from_slice(&chunk);
                        screen.process(&chunk);

                        if !exit_requested
                            && visible_output_contains(&output, &screen, expected_output)
                        {
                            exit_requested = true;
                            match exit_mode {
                                PromptExitMode::TerminateClientOnMarker => {
                                    session.terminate();
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break exit_rx.await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                },
                result = &mut exit_rx => break result,
            }
        }
    })
    .await;

    let exit_code = match exit_code {
        Ok(Ok(code)) => code,
        Ok(Err(err)) => return Err(err.into()),
        Err(_) => {
            session.terminate();
            let output = String::from_utf8_lossy(&output);
            let screen = screen.screen().contents();
            anyhow::bail!(
                "timed out waiting for interpreter output; output: {output}\nvisible screen:\n{screen}"
            );
        }
    };

    while let Ok(chunk) = output_rx.try_recv() {
        output.extend_from_slice(&chunk);
    }

    let output = String::from_utf8_lossy(&output).to_string();
    match exit_mode {
        PromptExitMode::TerminateClientOnMarker => anyhow::ensure!(
            exit_requested,
            "client terminated before the expected marker was observed; exit code: {exit_code}; output: {output}"
        ),
    }
    anyhow::ensure!(
        visible_output_contains(output.as_bytes(), &screen, expected_output),
        "expected `{expected_output}` in output, got raw: {output}\nvisible screen:\n{}",
        screen.screen().contents()
    );
    Ok(output)
}

fn visible_output_contains(
    raw_output: &[u8],
    screen: &vt100::Parser,
    expected_output: &str,
) -> bool {
    String::from_utf8_lossy(raw_output).contains(expected_output)
        || screen.screen().contents().contains(expected_output)
}
