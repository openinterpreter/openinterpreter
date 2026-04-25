#![cfg(not(target_os = "windows"))]

mod common;

use std::path::Path;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use codex_tui::STARTUP_TRACE_PATH_ENV_VAR;
use codex_utils_cargo_bin::repo_root;
use serde::Deserialize;
use serde_json::json;
use tempfile::NamedTempFile;
use tempfile::TempDir;

use crate::common::TmuxSession;
use crate::common::resolve_interpreter_bin;
use crate::common::tmux_is_available;
use codex_server_cli::home::INTERPRETER_DISABLE_SYSTEM_IMPORT_ENV_VAR;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_first_run_uses_open_interpreter_provider_picker() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping onboarding branding test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_trusted_project_config(home.path(), &repo_root)?;

    let session = launch_interpreter(home.path(), &repo_root).await?;
    let pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Choose a provider to get started.")
        })
        .await?;

    assert!(pane.contains("Welcome to Open Interpreter"));
    assert!(pane.contains("OpenAI"));
    assert!(pane.contains("OpenRouter"));
    assert!(pane.contains("Groq"));
    assert!(!pane.contains("Welcome to Codex"));
    assert!(!pane.contains("OpenAI's command-line coding agent"));
    Ok(())
}

#[derive(Debug, Deserialize)]
struct StartupTraceEvent {
    event: String,
    unix_time_ms: u128,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_first_run_renders_provider_picker_from_real_tui_startup() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping onboarding startup test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_trusted_project_config(home.path(), &repo_root)?;
    let startup_trace = NamedTempFile::new()?;

    let session =
        launch_interpreter_with_trace(home.path(), &repo_root, startup_trace.path()).await?;
    let pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Choose a provider to get started.")
                && pane.contains("OpenAI")
                && pane.contains("OpenRouter")
        })
        .await?;
    let first_frame_seen_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before unix epoch")?
        .as_millis();

    assert!(pane.contains("Welcome to Open Interpreter."));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let startup_trace = read_startup_trace(startup_trace.path())?;
    assert!(
        startup_trace
            .iter()
            .any(|event| event.event == "interpreter.tui.delegate.enter"),
        "startup should delegate into the real TUI path: {startup_trace:?}"
    );
    assert!(
        startup_trace
            .iter()
            .any(|event| event.event == "tui.notification_backend.ready"),
        "startup should initialize the real TUI terminal path: {startup_trace:?}"
    );
    assert!(
        startup_trace
            .iter()
            .any(|event| event.event == "tui.notification_backend.ready"
                && event.unix_time_ms <= first_frame_seen_at),
        "first onboarding frame should appear after the TUI terminal probes complete"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_interpreter_home_beats_stale_codex_home() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping configured-home precedence test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let interpreter_home = TempDir::new()?;
    let stale_codex_home = TempDir::new()?;
    write_ready_config(interpreter_home.path(), &repo_root)?;
    write_trusted_project_config(stale_codex_home.path(), &repo_root)?;

    let session = launch_interpreter_with_env(
        interpreter_home.path(),
        &repo_root,
        &[(
            "CODEX_HOME".to_string(),
            stale_codex_home.path().display().to_string(),
        )],
    )
    .await?;
    let pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Open Interpreter")
                && pane.contains("/model to change")
                && pane.contains("gpt-5.4-mini")
        })
        .await?;

    assert!(!pane.contains("Choose a provider to get started."));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_interpreter_home_with_codex_import_shows_trust_then_provider_onboarding()
-> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping imported-home onboarding test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home_root = TempDir::new()?;
    let interpreter_home = home_root.path().join(".openinterpreter");
    let codex_home = home_root.path().join(".codex");
    std::fs::create_dir_all(&codex_home)?;
    write_codex_import_config(
        &codex_home,
        "groq",
        "qwen/qwen3-32b",
        r#"
[model_providers.groq]
name = "groq"
base_url = "https://api.groq.com/openai/v1"
env_key = "GROQ_API_KEY"
"#,
    )?;

    let session = launch_interpreter_with_import(
        &interpreter_home,
        &repo_root,
        &[
            ("HOME".to_string(), home_root.path().display().to_string()),
            ("GROQ_API_KEY".to_string(), "test-groq-key".to_string()),
        ],
    )
    .await?;
    let trust_pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Do you trust the contents of this directory?")
        })
        .await?;

    assert!(trust_pane.contains("Untrusted directories can contain prompt injection."));
    session.send_enter()?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    session.type_like_user("groq").await?;
    let pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Choose a provider to get started.")
                && pane.contains("Filter providers:")
                && pane.contains("Groq")
                && pane.contains("Imported model: qwen/qwen3-32b")
        })
        .await?;

    assert!(pane.contains("Ready"));
    assert!(!pane.contains("Explain this codebase"));
    assert!(!pane.contains("(current)"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_interpreter_home_marks_anthropic_ready_after_trust_prompt() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping anthropic onboarding readiness test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home_root = TempDir::new()?;
    let interpreter_home = home_root.path().join(".openinterpreter");

    let session = launch_interpreter_with_import(
        &interpreter_home,
        &repo_root,
        &[
            ("HOME".to_string(), home_root.path().display().to_string()),
            (
                "ANTHROPIC_API_KEY".to_string(),
                "test-anthropic-key".to_string(),
            ),
        ],
    )
    .await?;
    let trust_pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Do you trust the contents of this directory?")
        })
        .await?;

    assert!(trust_pane.contains("Untrusted directories can contain prompt injection."));
    session.send_enter()?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    session.type_like_user("anthropic").await?;
    let pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Choose a provider to get started.")
                && pane.contains("Filter providers:")
                && pane.contains("Anthropic")
                && pane.contains("Harness: claude-code")
        })
        .await?;

    assert!(pane.contains("Ready"));
    Ok(())
}

async fn launch_interpreter(home: &Path, repo_root: &Path) -> Result<TmuxSession> {
    launch_interpreter_with_env(home, repo_root, &[]).await
}

async fn launch_interpreter_with_env(
    home: &Path,
    repo_root: &Path,
    extra_env: &[(String, String)],
) -> Result<TmuxSession> {
    launch_interpreter_internal(
        home, repo_root, extra_env, /*disable_system_import*/ true,
    )
    .await
}

async fn launch_interpreter_with_import(
    home: &Path,
    repo_root: &Path,
    extra_env: &[(String, String)],
) -> Result<TmuxSession> {
    launch_interpreter_internal(
        home, repo_root, extra_env, /*disable_system_import*/ false,
    )
    .await
}

async fn launch_interpreter_internal(
    home: &Path,
    repo_root: &Path,
    extra_env: &[(String, String)],
    disable_system_import: bool,
) -> Result<TmuxSession> {
    let binary = resolve_interpreter_bin()?;
    let args = vec![
        "--no-alt-screen".to_string(),
        "-C".to_string(),
        repo_root.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
    ];
    let env = vec![
        (
            "OPEN_INTERPRETER_HOME".to_string(),
            home.display().to_string(),
        ),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    let mut env = env;
    if disable_system_import {
        env.push((
            INTERPRETER_DISABLE_SYSTEM_IMPORT_ENV_VAR.to_string(),
            "1".to_string(),
        ));
    }
    env.extend_from_slice(extra_env);

    TmuxSession::start(
        "interpreter-onboarding",
        binary.as_path(),
        &args,
        repo_root,
        &env,
    )
}

async fn launch_interpreter_with_trace(
    home: &Path,
    repo_root: &Path,
    startup_trace_path: &Path,
) -> Result<TmuxSession> {
    let binary = resolve_interpreter_bin()?;
    let args = vec![
        "--no-alt-screen".to_string(),
        "-C".to_string(),
        repo_root.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
    ];
    let env = vec![
        (
            "OPEN_INTERPRETER_HOME".to_string(),
            home.display().to_string(),
        ),
        (
            INTERPRETER_DISABLE_SYSTEM_IMPORT_ENV_VAR.to_string(),
            "1".to_string(),
        ),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        (
            STARTUP_TRACE_PATH_ENV_VAR.to_string(),
            startup_trace_path.display().to_string(),
        ),
    ];

    TmuxSession::start(
        "interpreter-onboarding-trace",
        binary.as_path(),
        &args,
        repo_root,
        &env,
    )
}

fn read_startup_trace(path: &Path) -> Result<Vec<StartupTraceEvent>> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::Deserializer::from_str(&contents)
        .into_iter::<StartupTraceEvent>()
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to parse startup trace from {}", path.display()))
}

fn write_trusted_project_config(home: &Path, repo_root: &Path) -> Result<()> {
    let canonical_repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let trusted_project_entries = if canonical_repo_root == repo_root {
        format!(
            r#"[projects."{}"]
trust_level = "trusted"
"#,
            repo_root.display()
        )
    } else {
        format!(
            r#"[projects."{}"]
trust_level = "trusted"

[projects."{}"]
trust_level = "trusted"
"#,
            repo_root.display(),
            canonical_repo_root.display()
        )
    };
    let config_contents = format!(
        r#"
model = "gpt-5.4-mini"
approval_policy = "never"
sandbox_mode = "read-only"

{trusted_project_entries}
"#,
    );
    let config_path = home.join("config.toml");
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

fn write_ready_config(home: &Path, repo_root: &Path) -> Result<()> {
    let canonical_repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let trusted_project_entries = if canonical_repo_root == repo_root {
        format!(
            r#"[projects."{}"]
trust_level = "trusted"
"#,
            repo_root.display()
        )
    } else {
        format!(
            r#"[projects."{}"]
trust_level = "trusted"

[projects."{}"]
trust_level = "trusted"
"#,
            repo_root.display(),
            canonical_repo_root.display()
        )
    };
    let config_contents = format!(
        r#"
model = "gpt-5.4-mini"
model_provider = "openai"
model_reasoning_effort = "medium"
approval_policy = "never"
sandbox_mode = "read-only"

{trusted_project_entries}
"#,
    );
    let config_path = home.join("config.toml");
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    let auth_path = home.join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "OPENAI_API_KEY": "test-openai-key",
            "last_refresh": "2026-04-20T00:00:00Z",
        }))?,
    )
    .with_context(|| format!("failed to write {}", auth_path.display()))?;
    Ok(())
}

fn write_codex_import_config(
    codex_home: &Path,
    provider_id: &str,
    model: &str,
    provider_body: &str,
) -> Result<()> {
    let config_path = codex_home.join("config.toml");
    let config_contents = format!(
        r#"
model_provider = "{provider_id}"
model = "{model}"
{provider_body}
"#
    );
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}
