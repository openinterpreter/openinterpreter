mod common;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::repo_root;
use serde::Deserialize;
use tempfile::TempDir;
use tokio::select;
use tokio::time::sleep;
use tokio::time::timeout;

use crate::common::resolve_interpreter_bin;

#[derive(Clone, Copy)]
enum PromptExitMode {
    InterruptOnMarker,
    TerminateClientOnMarker,
}

struct InterpreterPromptRequest<'a> {
    home: &'a Path,
    repo_root: &'a Path,
    api_key_env: &'a str,
    api_key: &'a str,
    extra_args: &'a [&'a str],
    prompt: &'a str,
    expected_output: &'a str,
    trace_path: Option<&'a Path>,
    exit_mode: PromptExitMode,
}

#[tokio::test]
#[ignore = "live OpenAI Responses smoke test"]
async fn interpreter_can_use_real_openai_responses_via_local_daemon() -> Result<()> {
    if cfg!(windows) {
        return Ok(());
    }

    let Some(api_key) = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping live Responses smoke test because OPENAI_API_KEY is not set");
        return Ok(());
    };

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_live_config(home.path(), &repo_root)?;

    let output = run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        api_key_env: "OPENAI_API_KEY",
        api_key: &api_key,
        extra_args: &["--profile", "responses", "--no-alt-screen"],
        prompt: "Decode the base64 string SU5URVJQUkVURVJSRVNQT05TRVNPSw== and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "INTERPRETERRESPONSESOK",
        trace_path: None,
        exit_mode: PromptExitMode::InterruptOnMarker,
    })
    .await?;

    assert!(
        output.contains("INTERPRETERRESPONSESOK"),
        "expected decoded response marker in output, got: {output}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "live OpenAI Chat Completions via proxy smoke test"]
async fn interpreter_can_use_real_openai_chat_completions_via_local_proxy() -> Result<()> {
    if cfg!(windows) {
        return Ok(());
    }

    let Some(api_key) = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping live chat smoke test because OPENAI_API_KEY is not set");
        return Ok(());
    };

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_live_config(home.path(), &repo_root)?;
    let trace_path = home.path().join("chat-wire-proxy-trace.log");

    let output = run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        api_key_env: "OPENAI_API_KEY",
        api_key: &api_key,
        extra_args: &["--profile", "chat", "--no-alt-screen"],
        prompt: "Decode the base64 string SU5URVJQUkVURVJDSEFUT0s= and reply with exactly the decoded ASCII text and nothing else.",
        expected_output: "INTERPRETERCHATOK",
        trace_path: Some(trace_path.as_path()),
        exit_mode: PromptExitMode::TerminateClientOnMarker,
    })
    .await?;

    assert!(
        output.contains("INTERPRETERCHATOK"),
        "expected decoded chat marker in output, got: {output}"
    );

    let trace = std::fs::read_to_string(&trace_path)
        .with_context(|| format!("failed to read {}", trace_path.display()))?;
    assert!(
        trace.contains("https://api.openai.com/v1/chat/completions"),
        "expected proxy trace to mention Chat Completions upstream, got: {trace}"
    );
    assert!(
        trace.contains("gpt-5.4-mini"),
        "expected proxy trace to mention gpt-5.4-mini, got: {trace}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "live OpenAI smoke test proving responses and chat reuse one local daemon"]
async fn interpreter_responses_and_chat_reuse_one_local_daemon() -> Result<()> {
    if cfg!(windows) {
        return Ok(());
    }

    let Some(api_key) = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping live daemon reuse smoke test because OPENAI_API_KEY is not set");
        return Ok(());
    };

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_live_config(home.path(), &repo_root)?;

    run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        api_key_env: "OPENAI_API_KEY",
        api_key: &api_key,
        extra_args: &["--profile", "responses", "--no-alt-screen"],
        prompt: "Reply with exactly RESPONSESDAEMONOK and nothing else.",
        expected_output: "RESPONSESDAEMONOK",
        trace_path: None,
        exit_mode: PromptExitMode::TerminateClientOnMarker,
    })
    .await?;

    let first_lockfile = read_daemon_lockfile(home.path())?;

    run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        api_key_env: "OPENAI_API_KEY",
        api_key: &api_key,
        extra_args: &["--profile", "chat", "--no-alt-screen"],
        prompt: "Reply with exactly CHATDAEMONOK and nothing else.",
        expected_output: "CHATDAEMONOK",
        trace_path: None,
        exit_mode: PromptExitMode::TerminateClientOnMarker,
    })
    .await?;

    let second_lockfile = read_daemon_lockfile(home.path())?;
    assert_eq!(first_lockfile, second_lockfile);

    Ok(())
}

#[tokio::test]
#[ignore = "live Groq Chat Completions via proxy smoke test"]
async fn interpreter_can_use_real_groq_chat_completions_via_local_proxy() -> Result<()> {
    if cfg!(windows) {
        return Ok(());
    }

    let Some(api_key) = std::env::var("GROQ_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping live Groq smoke test because GROQ_API_KEY is not set");
        return Ok(());
    };

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    let repo_root_display = repo_root.display();
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            r#"
model = "llama-3.3-70b-versatile"
model_provider = "groq_chat"

[profiles.groq]
model = "llama-3.3-70b-versatile"
model_provider = "groq_chat"

[projects."{repo_root_display}"]
trust_level = "trusted"

[model_providers.groq_chat]
name = "Groq Chat Completions"
base_url = "https://api.groq.com/openai/v1"
env_key = "GROQ_API_KEY"
wire_api = "chat"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#
        ),
    )
    .with_context(|| {
        format!(
            "failed to write {}",
            home.path().join("config.toml").display()
        )
    })?;

    let output = run_interpreter_prompt(InterpreterPromptRequest {
        home: home.path(),
        repo_root: &repo_root,
        api_key_env: "GROQ_API_KEY",
        api_key: &api_key,
        extra_args: &["--profile", "groq", "--no-alt-screen"],
        prompt: "Reply with exactly INTERPRETERGROQOK and nothing else.",
        expected_output: "INTERPRETERGROQOK",
        trace_path: None,
        exit_mode: PromptExitMode::InterruptOnMarker,
    })
    .await?;

    assert!(
        output.contains("INTERPRETERGROQOK"),
        "expected Groq response marker in output, got: {output}"
    );

    Ok(())
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DaemonLockfile {
    pid: u32,
    websocket_url: String,
    server_bin: String,
}

fn write_live_config(home: &Path, repo_root: &Path) -> Result<()> {
    let repo_root_display = repo_root.display();
    let config_contents = format!(
        r#"
model = "gpt-5.4-mini"
model_provider = "openai_responses_api_key"

[profiles.responses]
model = "gpt-5.4-mini"
model_provider = "openai_responses_api_key"

[profiles.chat]
model = "gpt-5.4-mini"
model_provider = "openai_chat_completions"

[projects."{repo_root_display}"]
trust_level = "trusted"

[model_providers.openai_responses_api_key]
name = "OpenAI Responses API Key"
base_url = "https://api.openai.com/v1"
env_key = "OPENAI_API_KEY"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false

[model_providers.openai_chat_completions]
name = "OpenAI Chat Completions"
base_url = "https://api.openai.com/v1"
env_key = "OPENAI_API_KEY"
wire_api = "chat"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#
    );
    std::fs::write(home.join("config.toml"), config_contents)
        .with_context(|| format!("failed to write {}", home.join("config.toml").display()))?;
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
        api_key_env,
        api_key,
        extra_args,
        prompt,
        expected_output,
        trace_path,
        exit_mode,
    } = request;
    let interpreter = resolve_interpreter_bin()?;
    let mut env = HashMap::new();
    env.insert(
        "OPEN_INTERPRETER_HOME".to_string(),
        home.display().to_string(),
    );
    env.insert(api_key_env.to_string(), api_key.to_string());
    if let Some(trace_path) = trace_path {
        env.insert(
            "CODEX_CHAT_WIRE_PROXY_TRACE_PATH".to_string(),
            trace_path.display().to_string(),
        );
    }

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
    let interrupt_writer = writer_tx.clone();
    let mut output = Vec::new();
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

                        if !exit_requested
                            && String::from_utf8_lossy(&output).contains(expected_output)
                        {
                            exit_requested = true;
                            match exit_mode {
                                PromptExitMode::InterruptOnMarker => {
                                    for _ in 0..4 {
                                        let _ = interrupt_writer.send(vec![3]).await;
                                        sleep(Duration::from_millis(250)).await;
                                    }
                                }
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
            anyhow::bail!("timed out waiting for interpreter output; output: {output}");
        }
    };

    while let Ok(chunk) = output_rx.try_recv() {
        output.extend_from_slice(&chunk);
    }

    let output = String::from_utf8_lossy(&output).to_string();
    let interrupt_only_output = {
        let trimmed = output.trim();
        !trimmed.is_empty()
            && trimmed
                .chars()
                .all(|character| character == '^' || character == 'C' || character.is_whitespace())
    };
    match exit_mode {
        PromptExitMode::InterruptOnMarker => anyhow::ensure!(
            exit_code == 0 || exit_code == 130 || (exit_code == 1 && interrupt_only_output),
            "unexpected exit code from interpreter: {exit_code}; output: {output}"
        ),
        PromptExitMode::TerminateClientOnMarker => anyhow::ensure!(
            exit_requested,
            "client terminated before the expected marker was observed; exit code: {exit_code}; output: {output}"
        ),
    }
    anyhow::ensure!(
        output.contains(expected_output),
        "expected `{expected_output}` in output, got: {output}"
    );
    Ok(output)
}
