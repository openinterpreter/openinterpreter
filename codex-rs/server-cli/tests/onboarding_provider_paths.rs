#![cfg(not(target_os = "windows"))]

mod common;

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::repo_root;
use tempfile::TempDir;

use crate::common::TmuxSession;
use crate::common::resolve_interpreter_bin;
use crate::common::tmux_is_available;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_first_run_provider_paths_are_clear_and_shared() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping onboarding provider paths test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;

    assert_provider_flow(&repo_root, "api key", |pane| {
        pane.contains("OpenAI (API key) API key")
            && pane.contains("Leave blank if this endpoint does not require an API key")
    })
    .await?;
    assert_provider_flow(&repo_root, "openrouter", |pane| {
        pane.contains("OpenRouter API key")
            && pane.contains("Leave blank if this endpoint does not require an API key")
    })
    .await?;
    assert_provider_flow(&repo_root, "groq", |pane| {
        pane.contains("Groq API key")
            && pane.contains("Leave blank if this endpoint does not require an API key")
    })
    .await?;
    assert_provider_flow(
        &repo_root,
        "lm studio",
        provider_path_predicate("LM Studio"),
    )
    .await?;
    assert_provider_flow(&repo_root, "ollama", provider_path_predicate("Ollama")).await?;
    assert_provider_flow(&repo_root, "compatible", |pane| {
        pane.contains("Provider name")
            && pane.contains("Choose the name that should appear in /model and config")
    })
    .await?;

    Ok(())
}

async fn assert_provider_flow<F>(repo_root: &Path, query: &str, predicate: F) -> Result<()>
where
    F: Fn(&str) -> bool,
{
    let home = TempDir::new()?;
    write_trusted_project_config(home.path(), repo_root)?;

    let session = launch_interpreter(home.path(), repo_root).await?;
    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Choose a provider to get started.") && pane.contains("Filter providers")
        })
        .await?;

    session.type_like_user(query).await?;
    session.send_enter()?;

    let pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| predicate(pane))
        .await?;
    assert!(
        predicate(&pane),
        "unexpected provider flow for `{query}`:\n{pane}"
    );

    Ok(())
}

fn provider_path_predicate(provider_name: &'static str) -> impl Fn(&str) -> bool {
    move |pane| {
        pane.contains(&format!("{provider_name} is unavailable"))
            || pane.contains(&format!("Select Model for {provider_name}"))
    }
}

async fn launch_interpreter(home: &Path, repo_root: &Path) -> Result<TmuxSession> {
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

    TmuxSession::start(
        "interpreter-onboarding-provider-paths",
        binary.as_path(),
        &args,
        repo_root,
        &env,
    )
}

fn write_trusted_project_config(home: &Path, repo_root: &Path) -> Result<()> {
    let config_contents = format!(
        r#"
model = "gpt-5.4-mini"
approval_policy = "never"
sandbox_mode = "read-only"

[projects."{}"]
trust_level = "trusted"
"#,
        repo_root.display()
    );
    let config_path = home.join("config.toml");
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}
