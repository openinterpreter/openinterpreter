use super::*;
use crate::session::tests::make_session_and_context;
use crate::tools::context::ToolCallSource;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::turn_diff_tracker::TurnDiffTracker;
use codex_protocol::openai_models::InputModality;
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
use tokio_util::sync::CancellationToken;

fn invocation(
    session: Session,
    turn: TurnContext,
    tool_name: &str,
    arguments: serde_json::Value,
) -> ToolInvocation {
    ToolInvocation {
        session: Arc::new(session),
        turn: Arc::new(turn),
        cancellation_token: CancellationToken::new(),
        tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
        call_id: "call_1".to_string(),
        tool_name: codex_tools::ToolName::plain(tool_name),
        source: ToolCallSource::Direct,
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

/// A minimal valid 1x1 PNG, used to exercise the image-reading path.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xFF, 0xFF, 0x3F,
    0x00, 0x05, 0xFE, 0x02, 0xFE, 0x33, 0x12, 0x95, 0x14, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn set_image_input(turn: &mut TurnContext, enabled: bool) {
    if enabled {
        if !turn
            .model_info
            .input_modalities
            .contains(&InputModality::Image)
        {
            turn.model_info.input_modalities.push(InputModality::Image);
        }
    } else {
        turn.model_info
            .input_modalities
            .retain(|modality| *modality != InputModality::Image);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_returns_image_content_for_png_when_model_supports_images() {
    let (session, mut turn) = make_session_and_context().await;
    set_image_input(&mut turn, true);
    let path = turn.cwd.join("shot.png");
    tokio::fs::write(path.as_path(), PNG_1X1)
        .await
        .expect("write png");

    let output = ClaudeReadHandler
        .handle(invocation(
            session,
            turn,
            "Read",
            json!({ "file_path": path.to_string_lossy() }),
        ))
        .await
        .expect("reading a png should succeed, not fail on UTF-8");

    // Image content carries no text body; it is returned as an image block.
    assert_eq!(output.into_text(), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_rejects_image_when_model_lacks_image_input() {
    let (session, mut turn) = make_session_and_context().await;
    set_image_input(&mut turn, false);
    let path = turn.cwd.join("shot.png");
    tokio::fs::write(path.as_path(), PNG_1X1)
        .await
        .expect("write png");

    let result = ClaudeReadHandler
        .handle(invocation(
            session,
            turn,
            "Read",
            json!({ "file_path": path.to_string_lossy() }),
        ))
        .await;

    let err = match result {
        Ok(_) => panic!("image read must be rejected without image input"),
        Err(err) => err,
    };
    assert!(
        format!("{err:?}").contains("does not support image input"),
        "unexpected error: {err:?}"
    );
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

#[tokio::test]
async fn read_large_file_without_range_returns_captured_size_message() {
    let (session, turn) = make_session_and_context().await;
    let path = turn.cwd.join("read-large-target.txt");
    tokio::fs::write(
        path.as_path(),
        "x".repeat(CLAUDE_READ_MAX_WHOLE_FILE_BYTES + 1),
    )
    .await
    .expect("write file");

    let output = ClaudeReadHandler
        .handle(invocation(
            session,
            turn,
            "Read",
            json!({
                "file_path": path.to_string_lossy()
            }),
        ))
        .await
        .expect("read succeeds")
        .into_text();

    assert_eq!(
        output,
        "File content (0.3MB) exceeds maximum allowed size (256KB). Use offset and limit parameters to read specific portions of the file, or search for specific content instead of reading the whole file."
    );
}

#[tokio::test]
async fn bash_empty_failure_returns_claude_exit_code_message() {
    let (session, mut turn) = make_session_and_context().await;
    set_danger_full_access(&mut turn);

    let result = ClaudeBashHandler
        .handle(invocation(
            session,
            turn,
            "Bash",
            json!({
                "command": "false",
                "description": "Run false command"
            }),
        ))
        .await;

    assert_eq!(
        result.err(),
        Some(FunctionCallError::RespondToModel("Exit code 1".to_string()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bash_run_in_background_returns_captured_started_message_and_output_file() {
    let (session, mut turn) = make_session_and_context().await;
    set_danger_full_access(&mut turn);
    let cwd = turn.cwd.clone();

    let output = ClaudeBashHandler
        .handle(invocation(
            session,
            turn,
            "Bash",
            json!({
                "command": "echo HELLO_BG; sleep 2",
                "description": "background sleep",
                "run_in_background": true
            }),
        ))
        .await
        .expect("background bash should start, not return the old stub error")
        .into_text();

    // Matches the captured real Claude Code shape.
    assert!(
        output.starts_with("Command running in background with ID: "),
        "unexpected start message: {output}"
    );
    assert!(
        output.contains("Output is being written to: "),
        "expected an output-file path: {output}"
    );
    // The output file the model is told about exists and collects output.
    let path = output
        .split("Output is being written to: ")
        .nth(1)
        .expect("output path")
        .trim()
        .to_string();
    assert!(
        path.starts_with(cwd.as_path().to_string_lossy().as_ref()),
        "output file should be under the workspace: {path}"
    );
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let written = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    assert!(
        written.contains("HELLO_BG"),
        "background output file should collect stdout, got: {written:?}"
    );
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
        format!(
            "File created successfully at: {} (file state is current in your context — no need to Read it back)",
            path.display()
        )
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
    session
        .record_claude_code_current_file(path.as_path())
        .await;

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
        format!(
            "The file {} has been updated successfully. (file state is current in your context — no need to Read it back)",
            path.display()
        )
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
    session
        .record_claude_code_current_file(path.as_path())
        .await;

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
async fn edit_trims_new_string_trailing_horizontal_whitespace() {
    let (session, mut turn) = make_session_and_context().await;
    set_danger_full_access(&mut turn);
    let path = turn.cwd.join("edit-trims-new-string-target.txt");
    tokio::fs::write(path.as_path(), "before\nOLD_VALUE\nafter\n")
        .await
        .expect("write file");
    session
        .record_claude_code_current_file(path.as_path())
        .await;

    ClaudeEditHandler
        .handle(invocation(
            session,
            turn,
            "Edit",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "OLD_VALUE",
                "new_string": "NEW_VALUE  \nNEXT\t",
                "replace_all": false
            }),
        ))
        .await
        .expect("edit succeeds");

    assert_eq!(
        tokio::fs::read_to_string(path.as_path())
            .await
            .expect("read edited file"),
        "before\nNEW_VALUE\nNEXT\nafter\n"
    );
}

#[tokio::test]
async fn edit_requires_target_file_to_be_current() {
    let (session, mut turn) = make_session_and_context().await;
    set_danger_full_access(&mut turn);
    let path = turn.cwd.join("edit-unread-target.txt");
    tokio::fs::write(path.as_path(), "before\nOLD_VALUE\nafter\n")
        .await
        .expect("write file");

    let result = ClaudeEditHandler
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
        .await;
    let Err(err) = result else {
        panic!("edit should require a prior read");
    };

    assert_eq!(
        err.to_string(),
        "File has not been read yet. Read it first before writing to it."
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
