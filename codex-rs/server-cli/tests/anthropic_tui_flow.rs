#![cfg(not(target_os = "windows"))]

mod common;

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::repo_root;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;

use crate::common::MockAnthropicResponse;
use crate::common::MockAnthropicServer;
use crate::common::MockResponsesServer;
use crate::common::TmuxSession;
use crate::common::is_session_ready_screen;
use crate::common::resolve_interpreter_bin;
use crate::common::tmux_is_available;

const CURRENT_MODEL: &str = "mock-compatible-model";
const IMPORTED_GROQ_MODEL: &str = "qwen/qwen3-32b";
const LIVE_CURRENT_MODEL: &str = "gpt-5.4-mini";
const ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
const ANTHROPIC_HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_API_KEY: &str = "test-anthropic-key";
const GROQ_API_KEY: &str = "test-groq-key";
const ANTHROPIC_RESPONSE: &str = "MOCK_ANTHROPIC_TUI_OK";
const LIVE_ANTHROPIC_RESPONSE: &str = "LIVE_ANTHROPIC_HAIKU_OK";

fn is_reasoning_selection_screen(pane: &str) -> bool {
    pane.contains("Select Reasoning Level")
        || pane.contains("Choose a reasoning level for this new chat.")
}

fn is_thinking_selection_screen(pane: &str) -> bool {
    pane.contains("Select Thinking Mode")
}

fn model_picker_choices(pane: &str) -> Vec<String> {
    pane.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let trimmed = trimmed
                .strip_prefix('>')
                .map(str::trim_start)
                .unwrap_or(trimmed);
            let (index, model) = trimmed.split_once(". ")?;
            index
                .chars()
                .all(|c| c.is_ascii_digit())
                .then(|| model.trim().to_string())
        })
        .collect()
}

fn preferred_live_anthropic_model(pane: &str) -> Option<String> {
    let choices = model_picker_choices(pane);
    for family in ["claude-haiku", "claude-sonnet", "claude-opus"] {
        if let Some(choice) = choices.iter().find(|choice| choice.starts_with(family)) {
            return Some(choice.clone());
        }
    }
    choices.into_iter().next()
}

fn request_tool_names(request: &serde_json::Value) -> Vec<String> {
    request
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    tool.get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tui_can_repair_stale_anthropic_provider_and_send_first_turn() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping anthropic TUI flow test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    let current_provider =
        MockResponsesServer::start(CURRENT_MODEL, "CURRENT_PROVIDER_OK", Duration::ZERO).await?;
    let anthropic_provider = MockAnthropicServer::start(
        ANTHROPIC_API_KEY,
        ANTHROPIC_MODEL,
        ANTHROPIC_RESPONSE,
        Duration::ZERO,
    )
    .await?;

    write_config(
        home.path(),
        &repo_root,
        current_provider.base_url(),
        anthropic_provider.base_url(),
    )?;

    let session = launch_interpreter(home.path(), &repo_root).await?;
    session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            is_session_ready_screen(pane) && pane.contains(CURRENT_MODEL)
        })
        .await?;

    session.type_like_user("/model").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Select Provider") && pane.contains("Anthropic")
        })
        .await?;

    session.type_like_user("anthropic").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Filter models") && pane.contains(ANTHROPIC_MODEL)
        })
        .await?;
    session.type_like_user(ANTHROPIC_MODEL).await?;
    session.send_enter()?;

    let mut selection_outcome = session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            is_reasoning_selection_screen(pane)
                || pane.contains("Enter to configure reasoning")
                || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
        })
        .await?;
    if selection_outcome.contains("Enter to configure reasoning") {
        session.send_enter()?;
        selection_outcome = session
            .wait_for_screen(Duration::from_secs(15), |pane| {
                is_reasoning_selection_screen(pane)
                    || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
            })
            .await?;
    }
    if is_reasoning_selection_screen(&selection_outcome) {
        session.send_enter()?;
    }

    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL)
        })
        .await?;

    session.type_like_user("hi").await?;
    session
        .wait_for_screen(Duration::from_secs(5), |pane| pane.contains("› hi"))
        .await?;
    session.send_enter()?;
    let final_pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains(ANTHROPIC_RESPONSE)
        })
        .await?;
    assert!(
        !final_pane.contains("Model metadata for `"),
        "expected Anthropic selection to use seeded model metadata, got:\n{final_pane}"
    );

    let config_text = std::fs::read_to_string(home.path().join("config.toml"))
        .context("read persisted anthropic config")?;
    assert!(
        config_text.contains("wire_api = \"messages\""),
        "expected anthropic provider to be repaired to messages wire api, got:\n{config_text}"
    );

    assert_eq!(anthropic_provider.message_call_count(), 1);
    let requests = anthropic_provider.message_requests().await;
    assert_eq!(requests.len(), 1);
    let request = serde_json::to_string(&requests[0]).context("serialize anthropic request")?;
    assert!(
        request.contains(ANTHROPIC_MODEL),
        "expected request to use {ANTHROPIC_MODEL}, got {request}"
    );
    assert!(
        request.contains("\"hi\""),
        "expected request to include user message, got {request}"
    );
    assert_eq!(
        requests[0].get("thinking"),
        Some(&serde_json::json!({ "type": "adaptive" }))
    );
    assert_eq!(
        requests[0].get("output_config"),
        Some(&serde_json::json!({ "effort": "medium" }))
    );
    assert_eq!(
        request_tool_names(&requests[0]),
        vec![
            "Agent".to_string(),
            "Bash".to_string(),
            "Edit".to_string(),
            "Glob".to_string(),
            "Grep".to_string(),
            "LSP".to_string(),
            "Read".to_string(),
            "TodoWrite".to_string(),
            "Write".to_string(),
        ]
    );
    assert!(
        request.contains("You are a Claude agent, built on Anthropic's Claude Agent SDK."),
        "expected Claude Agent SDK system prompt in anthropic request, got {request}"
    );
    assert!(
        !request.contains("exec_command"),
        "expected Claude Code request to avoid native Codex tools, got {request}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tui_uses_thinking_toggle_for_anthropic_models_without_effort_support() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping anthropic TUI flow test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    let current_provider =
        MockResponsesServer::start(CURRENT_MODEL, "CURRENT_PROVIDER_OK", Duration::ZERO).await?;
    let anthropic_provider = MockAnthropicServer::start(
        ANTHROPIC_API_KEY,
        ANTHROPIC_HAIKU_MODEL,
        ANTHROPIC_RESPONSE,
        Duration::ZERO,
    )
    .await?;

    write_config(
        home.path(),
        &repo_root,
        current_provider.base_url(),
        anthropic_provider.base_url(),
    )?;

    let session = launch_interpreter(home.path(), &repo_root).await?;
    session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            is_session_ready_screen(pane) && pane.contains(CURRENT_MODEL)
        })
        .await?;

    session.type_like_user("/model").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Select Provider") && pane.contains("Anthropic")
        })
        .await?;

    session.type_like_user("anthropic").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Filter models") && pane.contains(ANTHROPIC_HAIKU_MODEL)
        })
        .await?;
    session.type_like_user(ANTHROPIC_HAIKU_MODEL).await?;
    session.send_enter()?;

    let selection_outcome = session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            is_thinking_selection_screen(pane)
                || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_HAIKU_MODEL))
        })
        .await?;
    if is_thinking_selection_screen(&selection_outcome) {
        session.send_enter()?;
    }

    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            is_session_ready_screen(pane) && pane.contains(ANTHROPIC_HAIKU_MODEL)
        })
        .await?;

    session.type_like_user("hi").await?;
    session
        .wait_for_screen(Duration::from_secs(5), |pane| pane.contains("› hi"))
        .await?;
    session.send_enter()?;
    let final_pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains(ANTHROPIC_RESPONSE)
        })
        .await?;
    assert!(
        !final_pane.contains("Model metadata for `"),
        "expected provider switch to preserve Anthropic metadata, got:\n{final_pane}"
    );

    let requests = anthropic_provider.message_requests().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        request.get("thinking"),
        Some(&serde_json::json!({
            "type": "enabled",
            "budget_tokens": 31_999
        }))
    );
    assert_eq!(request.get("output_config"), None);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tui_can_send_second_prompt_after_claude_tool_turn_completes() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping anthropic TUI flow test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    let current_provider =
        MockResponsesServer::start(CURRENT_MODEL, "CURRENT_PROVIDER_OK", Duration::ZERO).await?;
    let scripted_responses = vec![
        MockAnthropicResponse::tool_use(
            "toolu_read_1",
            "Read",
            json!({
                "file_path": repo_root.join("README.md").display().to_string(),
                "offset": 1,
                "limit": 2
            }),
        ),
        MockAnthropicResponse::text("TURN1_DONE"),
        MockAnthropicResponse::text("TURN2_DONE"),
    ];
    let anthropic_provider = MockAnthropicServer::start_scripted(
        ANTHROPIC_API_KEY,
        ANTHROPIC_MODEL,
        scripted_responses,
        Duration::ZERO,
    )
    .await?;

    write_config(
        home.path(),
        &repo_root,
        current_provider.base_url(),
        anthropic_provider.base_url(),
    )?;

    let session = launch_interpreter(home.path(), &repo_root).await?;
    session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            is_session_ready_screen(pane) && pane.contains(CURRENT_MODEL)
        })
        .await?;

    session.type_like_user("/model").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Select Provider") && pane.contains("Anthropic")
        })
        .await?;

    session.type_like_user("anthropic").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Filter models") && pane.contains(ANTHROPIC_MODEL)
        })
        .await?;
    session.type_like_user(ANTHROPIC_MODEL).await?;
    session.send_enter()?;

    let mut selection_outcome = session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            is_reasoning_selection_screen(pane)
                || pane.contains("Enter to configure reasoning")
                || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
        })
        .await?;
    if selection_outcome.contains("Enter to configure reasoning") {
        session.send_enter()?;
        selection_outcome = session
            .wait_for_screen(Duration::from_secs(15), |pane| {
                is_reasoning_selection_screen(pane)
                    || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
            })
            .await?;
    }
    if is_reasoning_selection_screen(&selection_outcome) {
        session.send_enter()?;
    }

    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL)
        })
        .await?;

    session
        .type_like_user(
            "Use the Read tool once, then reply with exactly TURN1_DONE and nothing else.",
        )
        .await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(30), |pane| pane.contains("TURN1_DONE"))
        .await?;

    session
        .type_like_user("Reply with exactly TURN2_DONE and nothing else.")
        .await?;
    session.send_enter()?;
    let final_pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| pane.contains("TURN2_DONE"))
        .await?;
    assert!(
        !final_pane.contains("no replay response remaining"),
        "expected second prompt to advance cleanly after a Claude tool turn, got:\n{final_pane}"
    );

    let requests = anthropic_provider.message_requests().await;
    assert_eq!(requests.len(), 3);
    let tool_request =
        serde_json::to_string(&requests[0]).context("serialize initial anthropic tool request")?;
    assert!(
        tool_request.contains("\"name\":\"Read\""),
        "expected first Claude turn to issue a Read tool call, got {tool_request}"
    );
    let tool_result_request = serde_json::to_string(&requests[1])
        .context("serialize anthropic tool-result follow-up request")?;
    assert!(
        tool_result_request.contains("\"tool_result\""),
        "expected tool follow-up request to include the Claude tool result, got {tool_result_request}"
    );
    let second_turn_request =
        serde_json::to_string(&requests[2]).context("serialize second anthropic user request")?;
    assert!(
        second_turn_request.contains("Reply with exactly TURN2_DONE and nothing else."),
        "expected second user prompt to reach Anthropic after Claude tool completion, got {second_turn_request}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tui_switches_from_imported_groq_qwen_to_anthropic_model() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping anthropic TUI flow test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    let current_provider = MockResponsesServer::start(
        IMPORTED_GROQ_MODEL,
        "IMPORTED_GROQ_PROVIDER_OK",
        Duration::ZERO,
    )
    .await?;
    let anthropic_provider = MockAnthropicServer::start(
        ANTHROPIC_API_KEY,
        ANTHROPIC_MODEL,
        ANTHROPIC_RESPONSE,
        Duration::ZERO,
    )
    .await?;

    write_imported_groq_config(
        home.path(),
        &repo_root,
        current_provider.base_url(),
        anthropic_provider.base_url(),
    )?;

    let session = launch_interpreter_with_env(
        home.path(),
        &repo_root,
        vec![
            (
                "ANTHROPIC_API_KEY".to_string(),
                ANTHROPIC_API_KEY.to_string(),
            ),
            ("GROQ_API_KEY".to_string(), GROQ_API_KEY.to_string()),
        ],
        None,
    )
    .await?;
    session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            is_session_ready_screen(pane) && pane.contains(IMPORTED_GROQ_MODEL)
        })
        .await?;

    session.type_like_user("/model").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Select Provider") && pane.contains("Anthropic")
        })
        .await?;

    session.type_like_user("anthropic").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Filter models") && pane.contains(ANTHROPIC_MODEL)
        })
        .await?;
    session.type_like_user(ANTHROPIC_MODEL).await?;
    session.send_enter()?;

    let mut selection_outcome = session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            is_reasoning_selection_screen(pane)
                || pane.contains("Enter to configure reasoning")
                || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
        })
        .await?;
    if selection_outcome.contains("Enter to configure reasoning") {
        session.send_enter()?;
        selection_outcome = session
            .wait_for_screen(Duration::from_secs(15), |pane| {
                is_reasoning_selection_screen(pane)
                    || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
            })
            .await?;
    }
    if is_reasoning_selection_screen(&selection_outcome) {
        session.send_enter()?;
    }

    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            is_session_ready_screen(pane)
                && pane.contains(ANTHROPIC_MODEL)
                && !pane.contains(IMPORTED_GROQ_MODEL)
        })
        .await?;

    session.type_like_user("hi").await?;
    session
        .wait_for_screen(Duration::from_secs(5), |pane| pane.contains("› hi"))
        .await?;
    session.send_enter()?;
    let final_pane = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains(ANTHROPIC_RESPONSE)
        })
        .await?;
    assert!(
        !final_pane.contains("Model metadata for `"),
        "expected onboarding provider switch to preserve Anthropic metadata, got:\n{final_pane}"
    );

    let requests = anthropic_provider.message_requests().await;
    assert_eq!(requests.len(), 1);
    let request = serde_json::to_string(&requests[0]).context("serialize anthropic request")?;
    assert!(
        request.contains(ANTHROPIC_MODEL),
        "expected request to use {ANTHROPIC_MODEL}, got {request}"
    );
    assert!(
        !request.contains(IMPORTED_GROQ_MODEL),
        "expected imported Groq model to be replaced, got {request}"
    );
    assert_eq!(
        request_tool_names(&requests[0]),
        vec![
            "Agent".to_string(),
            "Bash".to_string(),
            "Edit".to_string(),
            "Glob".to_string(),
            "Grep".to_string(),
            "LSP".to_string(),
            "Read".to_string(),
            "TodoWrite".to_string(),
            "Write".to_string(),
        ]
    );
    assert!(
        !request.contains("request_user_input"),
        "expected Claude Code core tool surface after provider switch, got {request}"
    );

    let config_text = std::fs::read_to_string(home.path().join("config.toml"))
        .context("read persisted anthropic config after imported Groq switch")?;
    assert!(
        config_text.contains(&format!("model = \"{ANTHROPIC_MODEL}\"")),
        "expected persisted config model to update, got:\n{config_text}"
    );
    assert!(
        config_text.contains("model_provider = \"anthropic\""),
        "expected persisted provider to update, got:\n{config_text}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_home_onboarding_can_switch_from_imported_groq_to_anthropic() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping anthropic onboarding flow test because tmux is not available");
        return Ok(());
    }

    let repo_root = repo_root()?;
    let home_root = TempDir::new()?;
    let interpreter_home = home_root.path().join(".openinterpreter");
    let codex_home = home_root.path().join(".codex");
    std::fs::create_dir_all(&codex_home)?;
    let anthropic_provider = MockAnthropicServer::start(
        ANTHROPIC_API_KEY,
        ANTHROPIC_MODEL,
        ANTHROPIC_RESPONSE,
        Duration::ZERO,
    )
    .await?;

    write_imported_groq_codex_config(
        &codex_home,
        "https://api.groq.com/openai/v1",
        Some(anthropic_provider.base_url()),
    )?;

    let session = launch_interpreter_with_env(
        &interpreter_home,
        &repo_root,
        vec![
            ("HOME".to_string(), home_root.path().display().to_string()),
            (
                "ANTHROPIC_API_KEY".to_string(),
                ANTHROPIC_API_KEY.to_string(),
            ),
            ("GROQ_API_KEY".to_string(), GROQ_API_KEY.to_string()),
        ],
        None,
    )
    .await?;
    let onboarding_pane = session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            pane.contains("Welcome to Open Interpreter.")
                && (pane.contains("Choose a provider to get started.")
                    || pane.contains("Choose a model for this new chat."))
        })
        .await?;
    if onboarding_pane.contains("Choose a model for this new chat.") {
        session.send_escape()?;
    }
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Welcome to Open Interpreter.")
                && pane.contains("Anthropic")
                && pane.contains("Imported model: qwen/qwen3-32b")
                && !pane.contains("(current)")
        })
        .await?;

    session.type_like_user("anthropic").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            pane.contains("Filter models") && pane.contains(ANTHROPIC_MODEL)
        })
        .await?;
    session.type_like_user(ANTHROPIC_MODEL).await?;
    session.send_enter()?;

    let mut selection_outcome = session
        .wait_for_screen(Duration::from_secs(15), |pane| {
            is_reasoning_selection_screen(pane)
                || pane.contains("Enter to configure reasoning")
                || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
        })
        .await?;
    if selection_outcome.contains("Enter to configure reasoning") {
        session.send_enter()?;
        selection_outcome = session
            .wait_for_screen(Duration::from_secs(15), |pane| {
                is_reasoning_selection_screen(pane)
                    || (is_session_ready_screen(pane) && pane.contains(ANTHROPIC_MODEL))
            })
            .await?;
    }
    if is_reasoning_selection_screen(&selection_outcome) {
        session.send_enter()?;
    }

    session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            is_session_ready_screen(pane)
                && pane.contains(ANTHROPIC_MODEL)
                && !pane.contains(IMPORTED_GROQ_MODEL)
        })
        .await?;

    session.type_like_user("hi").await?;
    session
        .wait_for_screen(Duration::from_secs(5), |pane| pane.contains("› hi"))
        .await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains(ANTHROPIC_RESPONSE)
        })
        .await?;

    let requests = anthropic_provider.message_requests().await;
    assert_eq!(requests.len(), 1);
    let request =
        serde_json::to_string(&requests[0]).context("serialize onboarding anthropic request")?;
    assert!(
        request.contains(ANTHROPIC_MODEL),
        "expected onboarding request to use {ANTHROPIC_MODEL}, got {request}"
    );
    assert!(
        !request.contains(IMPORTED_GROQ_MODEL),
        "expected onboarding import model to be replaced, got {request}"
    );

    let config_text = std::fs::read_to_string(interpreter_home.join("config.toml"))
        .context("read persisted onboarding anthropic config")?;
    assert!(
        config_text.contains(&format!("model = \"{ANTHROPIC_MODEL}\"")),
        "expected onboarding config model to update, got:\n{config_text}"
    );
    assert!(
        config_text.contains("model_provider = \"anthropic\""),
        "expected onboarding provider to update, got:\n{config_text}"
    );
    assert!(
        !interpreter_home
            .join(".fresh_home_provider_onboarding")
            .exists(),
        "expected fresh-home onboarding marker to be cleared after selection"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live Anthropic TUI flow via /model using real OpenAI and Anthropic APIs"]
async fn live_tui_can_select_anthropic_model_and_send_first_turn() -> Result<()> {
    if !tmux_is_available() {
        eprintln!("skipping live anthropic TUI flow test because tmux is not available");
        return Ok(());
    }

    let Some(openai_api_key) = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping live anthropic TUI flow test because OPENAI_API_KEY is not set");
        return Ok(());
    };

    let Some(anthropic_api_key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping live anthropic TUI flow test because ANTHROPIC_API_KEY is not set");
        return Ok(());
    };

    let repo_root = repo_root()?;
    let home = TempDir::new()?;
    let log_dir = home.path().join("logs");
    std::fs::create_dir_all(&log_dir).with_context(|| format!("create {}", log_dir.display()))?;
    write_live_openai_and_anthropic_config(home.path(), &repo_root)?;

    let session = launch_interpreter_with_env(
        home.path(),
        &repo_root,
        vec![
            ("OPENAI_API_KEY".to_string(), openai_api_key),
            ("ANTHROPIC_API_KEY".to_string(), anthropic_api_key),
            ("RUST_LOG".to_string(), "trace".to_string()),
        ],
        Some(log_dir.as_path()),
    )
    .await?;

    session
        .wait_for_screen(Duration::from_secs(90), |pane| {
            is_session_ready_screen(pane) && pane.contains(LIVE_CURRENT_MODEL)
        })
        .await?;

    session.type_like_user("/model").await?;
    session.send_enter()?;
    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Select Provider") && pane.contains("Anthropic")
        })
        .await?;

    session.type_like_user("anthropic").await?;
    session.send_enter()?;
    let model_picker = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Filter models") && pane.contains("claude-")
        })
        .await?;
    let selected_model = preferred_live_anthropic_model(&model_picker)
        .context("pick a live Anthropic model from the TUI picker")?;

    session.type_like_user(selected_model.as_str()).await?;
    session.send_enter()?;
    let selection_outcome = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            is_thinking_selection_screen(pane)
                || is_reasoning_selection_screen(pane)
                || (is_session_ready_screen(pane) && pane.contains(selected_model.as_str()))
        })
        .await?;
    if is_thinking_selection_screen(&selection_outcome)
        || is_reasoning_selection_screen(&selection_outcome)
    {
        session.send_enter()?;
    }

    session
        .wait_for_screen(Duration::from_secs(90), |pane| {
            is_session_ready_screen(pane) && pane.contains(selected_model.as_str())
        })
        .await?;

    session
        .type_like_user("Reply with exactly LIVE_ANTHROPIC_HAIKU_OK and nothing else.")
        .await?;
    session.send_enter()?;
    let final_pane = session
        .wait_for_screen(Duration::from_secs(120), |pane| {
            pane.contains(LIVE_ANTHROPIC_RESPONSE)
                || pane.contains("does not support the effort parameter")
                || pane.contains("thinking.type.enabled")
        })
        .await?;

    assert!(
        !final_pane.contains("does not support the effort parameter"),
        "live Anthropic flow still sent an unsupported effort parameter:\n{final_pane}"
    );
    assert!(
        !final_pane.contains("thinking.type.enabled"),
        "live Anthropic flow still sent deprecated thinking.type.enabled:\n{final_pane}"
    );
    assert!(
        !final_pane.contains("Model metadata for `"),
        "live Anthropic flow still fell back to generic model metadata:\n{final_pane}"
    );
    assert!(
        final_pane.contains(LIVE_ANTHROPIC_RESPONSE),
        "expected live Anthropic reply marker, got:\n{final_pane}"
    );

    let config_text = std::fs::read_to_string(home.path().join("config.toml"))
        .context("read persisted live anthropic config")?;
    assert!(
        config_text.contains("harness = \"claude-code\""),
        "expected Anthropic selection to persist the Claude Code harness, got:\n{config_text}"
    );
    assert!(
        config_text.contains("wire_api = \"messages\""),
        "expected Anthropic provider to keep messages wire api, got:\n{config_text}"
    );
    assert!(
        config_text.contains(&format!("model = \"{selected_model}\"")),
        "expected live Anthropic config model to update, got:\n{config_text}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live Anthropic onboarding flow on a fresh Open Interpreter home"]
async fn live_fresh_home_onboarding_can_switch_to_anthropic_model_and_send_first_turn() -> Result<()>
{
    if !tmux_is_available() {
        eprintln!("skipping live anthropic onboarding flow test because tmux is not available");
        return Ok(());
    }

    let Some(anthropic_api_key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        eprintln!(
            "skipping live anthropic onboarding flow test because ANTHROPIC_API_KEY is not set"
        );
        return Ok(());
    };

    let repo_root = repo_root()?;
    let home_root = TempDir::new()?;
    let interpreter_home = home_root.path().join(".openinterpreter");
    let codex_home = home_root.path().join(".codex");
    let log_dir = home_root.path().join("logs");
    std::fs::create_dir_all(&codex_home)
        .with_context(|| format!("create {}", codex_home.display()))?;
    std::fs::create_dir_all(&log_dir).with_context(|| format!("create {}", log_dir.display()))?;
    write_imported_groq_codex_config(
        &codex_home,
        "https://api.groq.com/openai/v1",
        /*anthropic_provider_base_url*/ None,
    )?;

    let session = launch_interpreter_with_env(
        &interpreter_home,
        &repo_root,
        vec![
            ("HOME".to_string(), home_root.path().display().to_string()),
            ("ANTHROPIC_API_KEY".to_string(), anthropic_api_key),
            ("GROQ_API_KEY".to_string(), GROQ_API_KEY.to_string()),
            ("RUST_LOG".to_string(), "trace".to_string()),
        ],
        Some(log_dir.as_path()),
    )
    .await?;
    let onboarding_pane = session
        .wait_for_screen(Duration::from_secs(90), |pane| {
            pane.contains("Welcome to Open Interpreter.")
                && (pane.contains("Choose a provider to get started.")
                    || pane.contains("Choose a model for this new chat."))
        })
        .await?;
    if onboarding_pane.contains("Choose a model for this new chat.") {
        session.send_escape()?;
    }
    session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            pane.contains("Welcome to Open Interpreter.")
                && pane.contains("Imported model: qwen/qwen3-32b")
                && pane.contains("Anthropic")
                && pane.contains("Ready")
        })
        .await?;

    session.type_like_user("anthropic").await?;
    session.send_enter()?;
    let model_picker = session
        .wait_for_screen(Duration::from_secs(45), |pane| {
            pane.contains("Filter models") && pane.contains("claude-")
        })
        .await?;
    let selected_model = preferred_live_anthropic_model(&model_picker)
        .context("pick a live Anthropic model from the onboarding picker")?;

    session.type_like_user(selected_model.as_str()).await?;
    session.send_enter()?;
    let selection_outcome = session
        .wait_for_screen(Duration::from_secs(30), |pane| {
            is_thinking_selection_screen(pane)
                || is_reasoning_selection_screen(pane)
                || (is_session_ready_screen(pane) && pane.contains(selected_model.as_str()))
        })
        .await?;
    if is_thinking_selection_screen(&selection_outcome)
        || is_reasoning_selection_screen(&selection_outcome)
    {
        session.send_enter()?;
    }

    session
        .wait_for_screen(Duration::from_secs(90), |pane| {
            is_session_ready_screen(pane)
                && pane.contains(selected_model.as_str())
                && !pane.contains(IMPORTED_GROQ_MODEL)
        })
        .await?;

    session
        .type_like_user("Reply with exactly LIVE_ANTHROPIC_HAIKU_OK and nothing else.")
        .await?;
    session.send_enter()?;
    let final_pane = session
        .wait_for_screen(Duration::from_secs(120), |pane| {
            pane.contains(LIVE_ANTHROPIC_RESPONSE)
                || pane.contains("does not support the effort parameter")
                || pane.contains("thinking.type.enabled")
                || pane.contains("qwen/qwen3-32b")
        })
        .await?;

    assert!(
        !final_pane.contains("does not support the effort parameter"),
        "live onboarding Anthropic flow still sent an unsupported effort parameter:\n{final_pane}"
    );
    assert!(
        !final_pane.contains("thinking.type.enabled"),
        "live onboarding Anthropic flow still sent deprecated thinking.type.enabled:\n{final_pane}"
    );
    assert!(
        !final_pane.contains("qwen/qwen3-32b"),
        "live onboarding Anthropic flow still leaked the imported Groq model:\n{final_pane}"
    );
    assert!(
        !final_pane.contains("Model metadata for `"),
        "live onboarding Anthropic flow still fell back to generic model metadata:\n{final_pane}"
    );
    assert!(
        final_pane.contains(LIVE_ANTHROPIC_RESPONSE),
        "expected live onboarding Anthropic reply marker, got:\n{final_pane}"
    );

    let config_text = std::fs::read_to_string(interpreter_home.join("config.toml"))
        .context("read persisted live onboarding anthropic config")?;
    assert!(
        config_text.contains(&format!("model = \"{selected_model}\"")),
        "expected onboarding config model to update, got:\n{config_text}"
    );
    assert!(
        config_text.contains("model_provider = \"anthropic\""),
        "expected onboarding provider to update, got:\n{config_text}"
    );
    assert!(
        config_text.contains("harness = \"claude-code\""),
        "expected onboarding Anthropic selection to persist the Claude Code harness, got:\n{config_text}"
    );
    assert!(
        !interpreter_home
            .join(".fresh_home_provider_onboarding")
            .exists(),
        "expected fresh-home onboarding marker to be cleared after live selection"
    );

    Ok(())
}

async fn launch_interpreter(home: &Path, repo_root: &Path) -> Result<TmuxSession> {
    launch_interpreter_with_env(
        home,
        repo_root,
        vec![(
            "ANTHROPIC_API_KEY".to_string(),
            ANTHROPIC_API_KEY.to_string(),
        )],
        None,
    )
    .await
}

async fn launch_interpreter_with_env(
    home: &Path,
    repo_root: &Path,
    mut extra_env: Vec<(String, String)>,
    log_dir: Option<&Path>,
) -> Result<TmuxSession> {
    let binary = resolve_interpreter_bin()?;
    let args = vec![
        "--no-alt-screen".to_string(),
        "-C".to_string(),
        repo_root.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
        "-c".to_string(),
        format!("log_dir={}", log_dir.unwrap_or(home).display()),
    ];
    let mut env = vec![
        (
            "OPEN_INTERPRETER_HOME".to_string(),
            home.display().to_string(),
        ),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    env.append(&mut extra_env);

    TmuxSession::start(
        "interpreter-anthropic-tui-flow",
        binary.as_path(),
        &args,
        repo_root,
        &env,
    )
}

fn write_live_openai_and_anthropic_config(home: &Path, repo_root: &Path) -> Result<()> {
    let repo_root_display = repo_root.display();
    let config_contents = format!(
        r#"
model = "{LIVE_CURRENT_MODEL}"
model_provider = "openai_responses_api_key"
approval_policy = "never"
sandbox_mode = "read-only"

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

[model_providers.anthropic]
name = "Anthropic"
base_url = "https://api.anthropic.com"
env_key = "ANTHROPIC_API_KEY"
wire_api = "messages"
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

fn write_config(
    home: &Path,
    repo_root: &Path,
    current_provider_base_url: &str,
    anthropic_provider_base_url: &str,
) -> Result<()> {
    let config_contents = format!(
        r#"
model = "{CURRENT_MODEL}"
model_provider = "current_local_bench"
approval_policy = "never"
sandbox_mode = "read-only"

[projects."{}"]
trust_level = "trusted"

[model_providers.current_local_bench]
name = "Current Local Bench"
base_url = "{current_provider_base_url}/v1"

[model_providers.anthropic]
name = "Anthropic"
base_url = "{anthropic_provider_base_url}"
env_key = "ANTHROPIC_API_KEY"
wire_api = "responses"
"#,
        repo_root.display()
    );
    let config_path = home.join("config.toml");
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

fn write_imported_groq_config(
    home: &Path,
    repo_root: &Path,
    current_provider_base_url: &str,
    anthropic_provider_base_url: &str,
) -> Result<()> {
    let config_contents = format!(
        r#"
model = "{IMPORTED_GROQ_MODEL}"
model_provider = "groq"
approval_policy = "never"
sandbox_mode = "read-only"

[projects."{}"]
trust_level = "trusted"

[model_providers.groq]
name = "Groq"
base_url = "{current_provider_base_url}/v1"
env_key = "GROQ_API_KEY"
wire_api = "chat"

[model_providers.anthropic]
name = "Anthropic"
base_url = "{anthropic_provider_base_url}"
env_key = "ANTHROPIC_API_KEY"
wire_api = "messages"
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

fn write_imported_groq_codex_config(
    codex_home: &Path,
    groq_provider_base_url: &str,
    anthropic_provider_base_url: Option<&str>,
) -> Result<()> {
    let anthropic_provider_body =
        anthropic_provider_base_url.map_or_else(String::new, |base_url| {
            format!(
                r#"

[model_providers.anthropic]
name = "Anthropic"
base_url = "{base_url}"
env_key = "ANTHROPIC_API_KEY"
wire_api = "messages"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#
            )
        });
    let config_contents = format!(
        r#"
model = "{IMPORTED_GROQ_MODEL}"
model_provider = "groq"

[model_providers.groq]
name = "Groq"
base_url = "{groq_provider_base_url}"
env_key = "GROQ_API_KEY"
wire_api = "chat"{anthropic_provider_body}
"#
    );
    let config_path = codex_home.join("config.toml");
    std::fs::write(&config_path, config_contents)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}
