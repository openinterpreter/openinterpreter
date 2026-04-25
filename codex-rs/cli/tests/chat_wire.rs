use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

const NON_CODEX_CHAT_MODEL: &str = "gpt-5.4";

fn single_tool_call_chat_completions_sse(name: &str, call_id: &str, arguments: &str) -> String {
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

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

fn repo_root() -> std::path::PathBuf {
    #[expect(clippy::expect_used)]
    codex_utils_cargo_bin::repo_root().expect("failed to resolve repo root")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_routes_chat_wire_via_chat_completions() -> Result<()> {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-5.4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hi from chat\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-5.4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":3,\"total_tokens\":4}}\n\n",
        "data: [DONE]\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse, "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new()?;
    let provider_override = format!(
        "model_providers.mock={{ name = \"mock\", base_url = \"{}/v1\", env_key = \"PATH\", wire_api = \"chat\" }}",
        server.uri()
    );
    let mut cmd = codex_command(home.path())?;
    cmd.timeout(Duration::from_secs(30));
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(&provider_override)
        .arg("-c")
        .arg("model_provider=\"mock\"")
        .arg("-m")
        .arg(NON_CODEX_CHAT_MODEL)
        .arg("-C")
        .arg(repo_root())
        .arg("hello over chat?");
    cmd.env("OPENAI_API_KEY", "dummy");

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout,
        stderr
    );
    assert!(stdout.contains("hi from chat"));

    let requests = server.received_requests().await.unwrap_or_default();
    let chat_requests: Vec<_> = requests
        .into_iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .collect();
    assert_eq!(chat_requests.len(), 1);
    let request_body: Value = chat_requests[0]
        .body_json()
        .expect("chat request body should be valid json");
    assert_eq!(request_body["model"].as_str(), Some(NON_CODEX_CHAT_MODEL));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_chat_wire_web_search_fails_gracefully() -> Result<()> {
    let server = MockServer::start().await;
    let call_id = "web-search-1";
    let search_args = r#"{"query":"latest rust release notes"}"#;
    let responses = [
        single_tool_call_chat_completions_sse("web_search", call_id, search_args),
        concat!(
            "data: {\"id\":\"chatcmpl-2\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-5.4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"I can't browse the web through this provider, but I can still help from local context.\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-2\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-5.4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":12,\"total_tokens\":13}}\n\n",
            "data: [DONE]\n\n"
        )
        .to_string(),
    ];
    for body in responses {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(body, "text/event-stream"),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }

    let home = TempDir::new()?;
    let provider_override = format!(
        "model_providers.mock={{ name = \"mock\", base_url = \"{}/v1\", env_key = \"PATH\", wire_api = \"chat\" }}",
        server.uri()
    );
    let mut cmd = codex_command(home.path())?;
    cmd.timeout(Duration::from_secs(30));
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-c")
        .arg(&provider_override)
        .arg("-c")
        .arg("model_provider=\"mock\"")
        .arg("-m")
        .arg(NON_CODEX_CHAT_MODEL)
        .arg("-C")
        .arg(repo_root())
        .arg("search the web for the latest Rust release notes");
    cmd.env("OPENAI_API_KEY", "dummy");

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout,
        stderr
    );
    assert!(stdout.contains("I can't browse the web through this provider"));

    let requests = server.received_requests().await.unwrap_or_default();
    let chat_requests: Vec<_> = requests
        .into_iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .collect();
    assert_eq!(chat_requests.len(), 2);
    let first_body: Value = chat_requests[0]
        .body_json()
        .expect("initial web search request body should be valid json");
    assert_eq!(first_body["model"].as_str(), Some(NON_CODEX_CHAT_MODEL));

    let second_body: Value = chat_requests[1]
        .body_json()
        .expect("web search follow-up request body should be valid json");
    let last_message = second_body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .expect("web search follow-up request should include a last message");
    assert_eq!(last_message["role"].as_str(), Some("tool"));
    assert!(
        last_message["content"]
            .as_str()
            .is_some_and(|content| content.contains("unsupported call: web_search"))
    );

    Ok(())
}
