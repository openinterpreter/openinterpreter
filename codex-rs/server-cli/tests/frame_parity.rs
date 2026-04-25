#![cfg(not(target_os = "windows"))]

mod common;

use std::collections::HashMap;
use std::io::ErrorKind;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::repo_root;
use pretty_assertions::assert_eq;
use serde::Deserialize;
use tempfile::TempDir;
use tokio::time::sleep;

use crate::common::MockResponsesServer;
use crate::common::TmuxSession;
use crate::common::is_session_ready_screen;
use crate::common::resolve_codex_bin;
use crate::common::resolve_interpreter_bin;
use crate::common::tmux_is_available;

const TEST_MODEL: &str = "gpt-5.4-mini";
const EXPECTED_OUTPUT: &str = "FRAMEPARITYOK";
const TEST_PROMPT: &str = "Decode the base64 string RlJBTUVQQVJJVFlPSw== and reply with exactly the decoded ASCII text and nothing else.";

#[derive(Clone, Copy)]
enum CliFlavor {
    Codex,
    Interpreter,
}

struct CapturedFrames {
    ready: String,
    submitted: String,
    working: String,
    completed: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppServerLockfile {
    pid: u32,
    websocket_url: String,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_and_interpreter_render_matching_ready_working_and_completed_frames() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping frame parity test because tmux is not available");
        return Ok(());
    }

    let responses_server =
        MockResponsesServer::start(TEST_MODEL, EXPECTED_OUTPUT, Duration::from_millis(1500))
            .await?;

    let repo_root = repo_root()?;
    let codex_home = TempDir::new()?;
    let interpreter_home = TempDir::new()?;
    write_test_config(codex_home.path(), &repo_root, responses_server.base_url())?;
    write_test_config(
        interpreter_home.path(),
        &repo_root,
        responses_server.base_url(),
    )?;

    let codex_frames = capture_frames(CliFlavor::Codex, codex_home.path(), &repo_root).await?;
    let interpreter_frames =
        capture_frames(CliFlavor::Interpreter, interpreter_home.path(), &repo_root).await?;

    let response_calls = responses_server.response_call_count();
    assert_eq!(response_calls, 2);

    assert!(codex_frames.ready.contains("OpenAI Codex"));
    assert!(interpreter_frames.ready.contains("Open Interpreter"));
    assert!(!interpreter_frames.ready.contains("OpenAI Codex"));

    assert_eq!(
        normalize_screen(&codex_frames.ready),
        normalize_screen(&interpreter_frames.ready)
    );
    assert!(
        normalized_prompt_text(&codex_frames.submitted).contains(&collapse_whitespace(TEST_PROMPT))
    );
    assert!(
        normalized_prompt_text(&interpreter_frames.submitted)
            .contains(&collapse_whitespace(TEST_PROMPT))
    );
    assert_eq!(
        normalize_screen(&codex_frames.working),
        normalize_screen(&interpreter_frames.working)
    );
    assert_eq!(
        normalize_screen(&codex_frames.completed),
        normalize_screen(&interpreter_frames.completed)
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_reuses_one_daemon_across_different_workdirs() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping daemon reuse test because tmux is not available");
        return Ok(());
    }

    let responses_server =
        MockResponsesServer::start(TEST_MODEL, EXPECTED_OUTPUT, Duration::from_millis(250)).await?;
    let repo_root = repo_root()?;
    let alternate_workdir = repo_root.join("codex-rs");
    let interpreter_home = TempDir::new()?;
    write_test_config(
        interpreter_home.path(),
        &repo_root,
        responses_server.base_url(),
    )?;

    let first_session = launch_interpreter_session(
        "interpreter-daemon-workdir-a",
        interpreter_home.path(),
        &repo_root,
    )?;
    first_session
        .wait_for_screen(Duration::from_secs(30), is_session_ready_screen)
        .await?;
    let first_lockfile =
        wait_for_lockfile(interpreter_home.path(), Duration::from_secs(30)).await?;

    let second_session = launch_interpreter_session(
        "interpreter-daemon-workdir-b",
        interpreter_home.path(),
        &alternate_workdir,
    )?;
    second_session
        .wait_for_screen(Duration::from_secs(30), is_session_ready_screen)
        .await?;
    let second_lockfile =
        wait_for_lockfile(interpreter_home.path(), Duration::from_secs(30)).await?;

    assert_eq!(first_lockfile, second_lockfile);

    Ok(())
}

async fn capture_frames(
    flavor: CliFlavor,
    home: &std::path::Path,
    repo_root: &std::path::Path,
) -> Result<CapturedFrames> {
    let binary = match flavor {
        CliFlavor::Codex => resolve_codex_bin()?,
        CliFlavor::Interpreter => resolve_interpreter_bin()?,
    };

    let mut env = HashMap::new();
    env.insert("RUST_LOG".to_string(), "trace".to_string());
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    match flavor {
        CliFlavor::Codex => {
            env.insert("CODEX_HOME".to_string(), home.display().to_string());
        }
        CliFlavor::Interpreter => {
            env.insert(
                "OPEN_INTERPRETER_HOME".to_string(),
                home.display().to_string(),
            );
        }
    }

    let log_dir = home.join("logs");
    let args = vec![
        "--no-alt-screen".to_string(),
        "-C".to_string(),
        repo_root.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
        "-c".to_string(),
        format!("log_dir=\"{}\"", log_dir.display()),
    ];
    let env = env.into_iter().collect::<Vec<_>>();
    let session = TmuxSession::start(
        match flavor {
            CliFlavor::Codex => "codex-frame-parity",
            CliFlavor::Interpreter => "interpreter-frame-parity",
        },
        binary.as_path(),
        &args,
        repo_root,
        &env,
    )?;

    let ready = session
        .wait_for_screen(Duration::from_secs(30), is_session_ready_screen)
        .await?;

    session.type_like_user(TEST_PROMPT).await?;
    let typed = session
        .wait_for_screen(Duration::from_secs(10), |pane| {
            normalized_prompt_text(pane).contains(&collapse_whitespace(TEST_PROMPT))
        })
        .await?;
    sleep(Duration::from_millis(200)).await;
    session.send_enter()?;

    let submitted = session
        .wait_for_screen(Duration::from_secs(10), |pane| {
            normalized_prompt_text(pane).contains(&collapse_whitespace(TEST_PROMPT))
                && normalize_screen(pane) != normalize_screen(&typed)
        })
        .await?;

    sleep(Duration::from_millis(300)).await;
    let working = session.pane_text()?;
    anyhow::ensure!(
        !working.contains(EXPECTED_OUTPUT),
        "expected mid-turn frame before completion, but completion already rendered:\n{working}"
    );

    let completed = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains(EXPECTED_OUTPUT)
        })
        .await?;

    Ok(CapturedFrames {
        ready,
        submitted,
        working,
        completed,
    })
}

fn write_test_config(
    home: &std::path::Path,
    repo_root: &std::path::Path,
    responses_base_url: &str,
) -> Result<()> {
    let config_contents = format!(
        r#"
model = "{TEST_MODEL}"
model_provider = "mock_responses"
approval_policy = "never"
sandbox_mode = "read-only"

[features]
apps = false
plugins = false

[projects."{}"]
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
"#,
        repo_root.display()
    );
    let config_path = home.join("config.toml");
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

fn launch_interpreter_session(
    session_name: &str,
    home: &std::path::Path,
    workdir: &std::path::Path,
) -> Result<TmuxSession> {
    let binary = resolve_interpreter_bin()?;
    let args = vec![
        "--no-alt-screen".to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
        "-c".to_string(),
        format!("log_dir=\"{}\"", home.join("logs").display()),
    ];
    let env = vec![
        (
            "OPEN_INTERPRETER_HOME".to_string(),
            home.display().to_string(),
        ),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];

    TmuxSession::start(session_name, binary.as_path(), &args, workdir, &env)
}

async fn wait_for_lockfile(home: &std::path::Path, timeout: Duration) -> Result<AppServerLockfile> {
    let lockfile_path = home.join("tmp").join("interpreter").join("app-server.json");
    let start = tokio::time::Instant::now();
    loop {
        match std::fs::read_to_string(&lockfile_path) {
            Ok(content) => {
                return serde_json::from_str(&content)
                    .with_context(|| format!("parse {}", lockfile_path.display()));
            }
            Err(err) if err.kind() == ErrorKind::NotFound && start.elapsed() <= timeout => {
                sleep(Duration::from_millis(50)).await;
            }
            Err(err) => {
                return Err(err).with_context(|| format!("read {}", lockfile_path.display()));
            }
        }
    }
}

fn normalize_screen(screen: &str) -> String {
    let mut lines = screen
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_end();
            if trimmed.trim_start().starts_with("Tip:")
                || trimmed.contains("https://chatgpt.com/codex?app-landing-page=true")
            {
                return None;
            }

            let normalized =
                if trimmed.contains("OpenAI Codex") || trimmed.contains("Open Interpreter") {
                    "│ >_ <product> (v#.#.#) │".to_string()
                } else if trimmed.starts_with('›')
                    && !trimmed.contains(TEST_PROMPT)
                    && !trimmed.contains(EXPECTED_OUTPUT)
                {
                    "› <placeholder>".to_string()
                } else {
                    trimmed.to_string()
                };

            Some(
                normalized
                    .chars()
                    .map(|character| {
                        if character.is_ascii_digit() {
                            '#'
                        } else {
                            character
                        }
                    })
                    .collect::<String>(),
            )
        })
        .collect::<Vec<_>>();

    lines.dedup();

    while matches!(lines.last(), Some(last) if last.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

fn normalized_prompt_text(screen: &str) -> String {
    collapse_whitespace(screen)
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
