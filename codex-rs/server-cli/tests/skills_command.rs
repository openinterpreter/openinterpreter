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
use crate::common::resolve_interpreter_bin;
use crate::common::tmux_is_available;

const TEST_MODEL: &str = "gpt-5.4-mini";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interpreter_skills_command_opens_app_server_backed_skill_list() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping /skills tmux test because tmux is not available");
        return Ok(());
    }

    let responses_server =
        MockResponsesServer::start(TEST_MODEL, "SKILLSCOMMANDOK", Duration::from_millis(250))
            .await?;
    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    write_test_config(home.path(), &repo_root, responses_server.base_url())?;

    let session = launch_interpreter(home.path(), &repo_root)?;
    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Open Interpreter") && pane.contains('›')
        })
        .await?;

    session.type_like_user("/skills").await?;
    session.send_enter()?;

    let menu = session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("List skills") && pane.contains("Enable/Disable Skills")
        })
        .await?;
    assert!(
        menu.contains("Tip: press $ to open this list directly."),
        "expected /skills menu tip, got:\n{menu}"
    );

    session.send_enter()?;

    let skills_popup = session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("[Skill]") && pane.contains("Press enter to insert or esc to close")
        })
        .await?;
    assert!(
        skills_popup.contains("[Skill]"),
        "expected app-server-backed skills popup, got:\n{skills_popup}"
    );

    Ok(())
}

fn launch_interpreter(home: &Path, repo_root: &Path) -> Result<TmuxSession> {
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

    TmuxSession::start(
        "interpreter-skills-command",
        binary.as_path(),
        &args,
        repo_root,
        &env,
    )
}

fn write_test_config(home: &Path, repo_root: &Path, responses_base_url: &str) -> Result<()> {
    let config_contents = format!(
        r#"
model = "{TEST_MODEL}"
model_provider = "mock_responses"
approval_policy = "never"
sandbox_mode = "read-only"

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
