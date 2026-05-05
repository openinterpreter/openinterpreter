use super::KimiEdit;
use super::KimiSetTodoListHandler;
use super::apply_kimi_edit;
use super::format_kimi_read_output;
use crate::session::tests::make_session_and_context;
use crate::session::turn_context::TurnContext;
use crate::tools::context::ToolCallSource;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::turn_diff_tracker::TurnDiffTracker;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

fn invocation(
    session: Arc<crate::session::session::Session>,
    turn: Arc<TurnContext>,
    arguments: serde_json::Value,
) -> ToolInvocation {
    ToolInvocation {
        session,
        turn,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
        call_id: "call_1".to_string(),
        tool_name: codex_tools::ToolName::plain("SetTodoList"),
        source: ToolCallSource::Direct,
        payload: ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    }
}

#[test]
fn format_kimi_read_output_numbers_lines() {
    let output = format_kimi_read_output("alpha\nbeta\ngamma\n", 2, 2);
    assert_eq!(output.body, "     2\tbeta\n     3\tgamma\n");
    assert_eq!(
        output.system_message,
        "<system>2 lines read from file starting from line 2. Total lines in file: 3.</system>"
    );
}

#[test]
fn format_kimi_read_output_supports_negative_offsets() {
    let output = format_kimi_read_output("alpha\nbeta\ngamma\n", -2, 2);
    assert_eq!(output.body, "     2\tbeta\n     3\tgamma\n");
    assert_eq!(
        output.system_message,
        "<system>2 lines read from file starting from line 2. Total lines in file: 3.</system>"
    );
}

#[test]
fn format_kimi_read_output_reports_eof_when_less_than_requested() {
    let output = format_kimi_read_output("alpha\nbeta\n", 1, 1000);
    assert_eq!(output.body, "     1\talpha\n     2\tbeta\n");
    assert_eq!(
        output.system_message,
        "<system>2 lines read from file starting from line 1. Total lines in file: 2. End of file reached.</system>"
    );
}

#[test]
fn apply_kimi_edit_replaces_first_match_by_default() {
    let (output, replacement_count) = apply_kimi_edit(
        "one two one",
        &KimiEdit {
            old: "one".to_string(),
            new: "ONE".to_string(),
            replace_all: None,
        },
    );
    assert_eq!(output, "ONE two one");
    assert_eq!(replacement_count, 1);
}

#[test]
fn apply_kimi_edit_replaces_all_matches_when_requested() {
    let (output, replacement_count) = apply_kimi_edit(
        "one two one",
        &KimiEdit {
            old: "one".to_string(),
            new: "ONE".to_string(),
            replace_all: Some(true),
        },
    );
    assert_eq!(output, "ONE two ONE");
    assert_eq!(replacement_count, 2);
}

#[tokio::test]
async fn set_todo_list_query_returns_current_todos() {
    let (session, turn) = make_session_and_context().await;
    let session = Arc::new(session);
    let turn = Arc::new(turn);

    KimiSetTodoListHandler
        .handle(invocation(
            Arc::clone(&session),
            Arc::clone(&turn),
            json!({
                "todos": [
                    {"title": "Read source", "status": "done"},
                    {"title": "Write fix", "status": "in_progress"}
                ]
            }),
        ))
        .await
        .expect("todo update succeeds");

    let output = KimiSetTodoListHandler
        .handle(invocation(session, turn, json!({})))
        .await
        .expect("todo query succeeds")
        .into_text();

    assert_eq!(
        output,
        "Current todo list:\n- [done] Read source\n- [in_progress] Write fix"
    );
}
