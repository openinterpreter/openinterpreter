use super::*;
use crate::session::tests::make_session_and_context;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::turn_diff_tracker::TurnDiffTracker;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

fn invocation(
    session: Session,
    turn: TurnContext,
    tool_name: &str,
    arguments: serde_json::Value,
) -> ToolInvocation {
    ToolInvocation {
        session: Arc::new(session),
        turn: Arc::new(turn),
        tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
        call_id: "call_1".to_string(),
        tool_name: codex_tools::ToolName::plain(tool_name),
        payload: ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    }
}

fn set_danger_full_access(turn: &mut TurnContext) {
    turn.sandbox_policy
        .set(SandboxPolicy::DangerFullAccess)
        .expect("test setup should allow updating sandbox policy");
    turn.file_system_sandbox_policy = FileSystemSandboxPolicy::from(turn.sandbox_policy.get());
    turn.network_sandbox_policy = NetworkSandboxPolicy::from(turn.sandbox_policy.get());
}

#[tokio::test]
async fn read_formats_numbered_lines() {
    let (session, turn) = make_session_and_context().await;
    let path = turn.cwd.join("read-target.txt");
    tokio::fs::write(path.as_path(), "READ_OK\nsecond\n")
        .await
        .expect("write file");

    let output = ClaudeReadHandler
        .handle(invocation(
            session,
            turn,
            "Read",
            json!({
                "file_path": path.to_string_lossy(),
                "limit": 1
            }),
        ))
        .await
        .expect("read succeeds")
        .into_text();

    assert_eq!(output, "1\tREAD_OK");
}

#[tokio::test]
async fn read_treats_offset_as_one_based() {
    let (session, turn) = make_session_and_context().await;
    let path = turn.cwd.join("read-offset-target.txt");
    tokio::fs::write(path.as_path(), "zero\nONE\nTWO\n")
        .await
        .expect("write file");

    let output = ClaudeReadHandler
        .handle(invocation(
            session,
            turn,
            "Read",
            json!({
                "file_path": path.to_string_lossy(),
                "offset": 2,
                "limit": 1
            }),
        ))
        .await
        .expect("read succeeds")
        .into_text();

    assert_eq!(output, "2\tONE");
}

#[test]
fn ask_user_question_normalizes_to_request_user_input_args() {
    let args = normalize_claude_ask_user_question_args(ClaudeAskUserQuestionArgs {
        questions: vec![
            ClaudeAskUserQuestionItem {
                question: "Which provider should we use?".to_string(),
                header: "Provider".to_string(),
                options: vec![
                    ClaudeAskUserQuestionOption {
                        label: "Anthropic".to_string(),
                        description: "Use Claude".to_string(),
                        preview: Some("Claude preview".to_string()),
                    },
                    ClaudeAskUserQuestionOption {
                        label: "OpenAI".to_string(),
                        description: "Use GPT".to_string(),
                        preview: None,
                    },
                ],
                multi_select: false,
            },
            ClaudeAskUserQuestionItem {
                question: "Which provider should we avoid?".to_string(),
                header: "Provider".to_string(),
                options: vec![
                    ClaudeAskUserQuestionOption {
                        label: "None".to_string(),
                        description: "Avoid none".to_string(),
                        preview: None,
                    },
                    ClaudeAskUserQuestionOption {
                        label: "Groq".to_string(),
                        description: "Avoid Groq".to_string(),
                        preview: None,
                    },
                ],
                multi_select: false,
            },
        ],
    })
    .expect("normalization succeeds");

    assert_eq!(
        args,
        RequestUserInputArgs {
            questions: vec![
                RequestUserInputQuestion {
                    id: "provider".to_string(),
                    header: "Provider".to_string(),
                    question: "Which provider should we use?".to_string(),
                    is_other: true,
                    is_secret: false,
                    options: Some(vec![
                        RequestUserInputQuestionOption {
                            label: "Anthropic".to_string(),
                            description: "Use Claude".to_string(),
                        },
                        RequestUserInputQuestionOption {
                            label: "OpenAI".to_string(),
                            description: "Use GPT".to_string(),
                        },
                    ]),
                },
                RequestUserInputQuestion {
                    id: "provider_2".to_string(),
                    header: "Provider".to_string(),
                    question: "Which provider should we avoid?".to_string(),
                    is_other: true,
                    is_secret: false,
                    options: Some(vec![
                        RequestUserInputQuestionOption {
                            label: "None".to_string(),
                            description: "Avoid none".to_string(),
                        },
                        RequestUserInputQuestionOption {
                            label: "Groq".to_string(),
                            description: "Avoid Groq".to_string(),
                        },
                    ]),
                },
            ],
        }
    );
}

#[tokio::test]
async fn write_creates_file_and_returns_claude_message() {
    let (session, mut turn) = make_session_and_context().await;
    set_danger_full_access(&mut turn);
    let path = turn.cwd.join("write-target.txt");

    let output = ClaudeWriteHandler
        .handle(invocation(
            session,
            turn,
            "Write",
            json!({
                "file_path": path.to_string_lossy(),
                "content": "WRITE_OK\n"
            }),
        ))
        .await
        .expect("write succeeds")
        .into_text();

    assert_eq!(
        output,
        format!("File created successfully at: {}", path.display())
    );
    assert_eq!(
        tokio::fs::read_to_string(path.as_path())
            .await
            .expect("read written file"),
        "WRITE_OK\n"
    );
}

#[tokio::test]
async fn todo_write_returns_claude_success_message() {
    let (session, turn) = make_session_and_context().await;

    let output = ClaudeTodoWriteHandler
        .handle(invocation(
            session,
            turn,
            "TodoWrite",
            json!({
                "todos": [
                    {
                        "content": "Read the file",
                        "status": "in_progress",
                        "activeForm": "Reading the file"
                    },
                    {
                        "content": "Update the file",
                        "status": "pending",
                        "activeForm": "Updating the file"
                    }
                ]
            }),
        ))
        .await
        .expect("todo write succeeds");

    assert_eq!(
        output.log_preview(),
        "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable"
    );
}

#[tokio::test]
async fn edit_updates_matching_text() {
    let (session, mut turn) = make_session_and_context().await;
    set_danger_full_access(&mut turn);
    let path = turn.cwd.join("edit-target.txt");
    tokio::fs::write(path.as_path(), "before\nOLD_VALUE\nafter\n")
        .await
        .expect("write file");

    let output = ClaudeEditHandler
        .handle(invocation(
            session,
            turn,
            "Edit",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "OLD_VALUE",
                "new_string": "NEW_VALUE",
                "replace_all": false
            }),
        ))
        .await
        .expect("edit succeeds")
        .into_text();

    assert_eq!(
        output,
        format!("The file {} has been updated.", path.display())
    );
    assert_eq!(
        tokio::fs::read_to_string(path.as_path())
            .await
            .expect("read edited file"),
        "before\nNEW_VALUE\nafter\n"
    );
}

#[tokio::test]
async fn edit_replace_all_uses_claude_message() {
    let (session, mut turn) = make_session_and_context().await;
    set_danger_full_access(&mut turn);
    let path = turn.cwd.join("edit-replace-all-target.txt");
    tokio::fs::write(path.as_path(), "TOKEN_OLD\nTOKEN_OLD\n")
        .await
        .expect("write file");

    let output = ClaudeEditHandler
        .handle(invocation(
            session,
            turn,
            "Edit",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "TOKEN_OLD",
                "new_string": "TOKEN_NEW",
                "replace_all": true
            }),
        ))
        .await
        .expect("edit succeeds")
        .into_text();

    assert_eq!(
        output,
        format!(
            "The file {} has been updated. All occurrences were successfully replaced.",
            path.display()
        )
    );
    assert_eq!(
        tokio::fs::read_to_string(path.as_path())
            .await
            .expect("read edited file"),
        "TOKEN_NEW\nTOKEN_NEW\n"
    );
}

#[tokio::test]
async fn bash_returns_empty_output_marker() {
    let (session, turn) = make_session_and_context().await;
    let output = ClaudeBashHandler
        .handle(invocation(
            session,
            turn,
            "Bash",
            json!({
                "command": "true",
                "description": "No-op"
            }),
        ))
        .await
        .expect("bash succeeds")
        .into_text();

    assert_eq!(output, CLAUDE_BASH_EMPTY_OUTPUT);
}
