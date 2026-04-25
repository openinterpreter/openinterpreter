#![cfg(not(target_os = "windows"))]

mod common;

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::repo_root;
use tempfile::TempDir;

use crate::common::MockResponsesServer;
use crate::common::TmuxSession;
use crate::common::is_session_ready_screen;
use crate::common::resolve_interpreter_bin;
use crate::common::tmux_is_available;

const TEST_MODEL: &str = "mock-compatible-model";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn onboarding_provider_model_selection_persists_for_next_chat() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping onboarding model selection test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_trusted_project_config(home.path(), &repo_root)?;
    let mock_server =
        MockResponsesServer::start(TEST_MODEL, "ONBOARDINGMODELSELECTIONOK", Duration::ZERO)
            .await?;

    let extra_env: Vec<(String, String)> = Vec::new();

    let first_session =
        launch_interpreter_with_extra_env(home.path(), &repo_root, &extra_env).await?;
    let provider_picker = first_session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Choose a provider to get started.") && pane.contains("Filter providers")
        })
        .await?;
    assert!(provider_picker.contains("OpenAI (ChatGPT sign-in)"));

    first_session.type_like_user("compatible").await?;
    first_session.send_enter()?;
    first_session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Provider name")
        })
        .await?;
    first_session.type_like_user("Local Bench").await?;
    first_session.send_enter()?;

    first_session
        .wait_for_screen(Duration::from_secs(15), |pane| pane.contains("base URL"))
        .await?;
    first_session
        .type_like_user(&format!("{}/v1", mock_server.base_url()))
        .await?;
    first_session.send_enter()?;

    first_session
        .wait_for_screen(Duration::from_secs(15), |pane| pane.contains("API key"))
        .await?;
    first_session.send_enter()?;

    first_session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Select Model for Local Bench") && pane.contains(TEST_MODEL)
        })
        .await?;
    first_session.send_enter()?;

    let mut selection_outcome = first_session
        .wait_for_screen(Duration::from_secs(10), |pane| {
            pane.contains("Choose a model for this new chat.")
                || pane.contains("Local Bench model name")
                || pane.contains("Select Reasoning Effort")
                || (is_session_ready_screen(pane) && pane.contains(TEST_MODEL))
        })
        .await?;
    if selection_outcome.contains("Choose a model for this new chat.") {
        first_session.send_enter()?;
        selection_outcome = first_session
            .wait_for_screen(Duration::from_secs(10), |pane| {
                pane.contains("Local Bench model name")
                    || pane.contains("Select Reasoning Effort")
                    || (is_session_ready_screen(pane) && pane.contains(TEST_MODEL))
            })
            .await?;
    }
    if selection_outcome.contains("Local Bench model name")
        || selection_outcome.contains("Select Reasoning Effort")
    {
        first_session.send_enter()?;
    }

    let first_ready = first_session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            is_session_ready_screen(pane) && pane.contains(TEST_MODEL)
        })
        .await?;
    assert!(!first_ready.contains("Choose a provider for Open Interpreter."));

    let config_text = std::fs::read_to_string(home.path().join("config.toml"))
        .context("read persisted onboarding config")?;
    eprintln!("persisted config.toml after onboarding:\n{config_text}");
    assert!(
        config_text.contains("model_provider = \"compatible_local_bench\""),
        "expected persisted provider selection in config.toml, got:\n{config_text}"
    );
    assert!(
        config_text.contains(&format!("model = \"{TEST_MODEL}\"")),
        "expected persisted model selection in config.toml, got:\n{config_text}"
    );
    assert!(
        !config_text.contains("model = \"gpt-5.4-mini\""),
        "expected old default model to be replaced in config.toml, got:\n{config_text}"
    );
    assert!(
        !config_text.contains("\nprofile = "),
        "expected no active profile override in config.toml, got:\n{config_text}"
    );

    first_session.send_ctrl_c()?;
    first_session.wait_for_exit(Duration::from_secs(15)).await?;

    let second_session =
        launch_interpreter_with_extra_env(home.path(), &repo_root, &extra_env).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let second_ready = second_session
        .pane_text()
        .context("capture second launch pane after onboarding")?;

    assert!(!second_ready.contains("Choose a provider for Open Interpreter."));
    assert!(!second_ready.contains("Choose a model for this new chat."));

    assert!(
        second_ready.contains(TEST_MODEL),
        "expected second launch ready screen to show {TEST_MODEL}, got:\n{second_ready}"
    );

    Ok(())
}

async fn launch_interpreter_with_extra_env(
    home: &Path,
    repo_root: &Path,
    extra_env: &[(String, String)],
) -> Result<TmuxSession> {
    let binary = resolve_interpreter_bin()?;
    let args = vec![
        "--no-alt-screen".to_string(),
        "-C".to_string(),
        repo_root.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
    ];
    let mut env = vec![
        ("HOME".to_string(), home.display().to_string()),
        (
            "OPEN_INTERPRETER_HOME".to_string(),
            home.display().to_string(),
        ),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    env.extend_from_slice(extra_env);

    TmuxSession::start(
        "interpreter-onboarding-model-selection",
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
