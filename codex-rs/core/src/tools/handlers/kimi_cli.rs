use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::claude_code::effective_turn_file_system_policy;
use crate::tools::handlers::claude_code::ensure_readable_path;
use crate::tools::handlers::claude_code::ensure_writable_path;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::request_user_input::RequestUserInputAnswer;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use codex_tools::request_user_input_unavailable_message;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use tokio::fs::OpenOptions;

use super::plan::handle_update_plan;

pub struct KimiAskUserQuestionHandler;
pub struct KimiReadFileHandler;
pub struct KimiSetTodoListHandler;
pub struct KimiStrReplaceFileHandler;
pub struct KimiWriteFileHandler;

#[derive(Deserialize)]
struct KimiTodoListArgs {
    todos: Option<Vec<KimiTodoItem>>,
}

#[derive(Deserialize)]
struct KimiTodoItem {
    title: String,
    status: KimiTodoStatus,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KimiTodoStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Deserialize)]
struct KimiReadFileArgs {
    path: String,
    line_offset: Option<isize>,
    n_lines: Option<usize>,
}

#[derive(Deserialize)]
struct KimiWriteFileArgs {
    path: String,
    content: String,
    mode: Option<KimiWriteMode>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KimiWriteMode {
    Overwrite,
    Append,
}

#[derive(Deserialize)]
struct KimiStrReplaceFileArgs {
    path: String,
    edit: KimiEditArg,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum KimiEditArg {
    Single(KimiEdit),
    Multiple(Vec<KimiEdit>),
}

#[derive(Deserialize)]
struct KimiEdit {
    old: String,
    new: String,
    replace_all: Option<bool>,
}

#[derive(Deserialize)]
struct KimiAskUserQuestionArgs {
    questions: Vec<KimiAskUserQuestionItem>,
}

#[derive(Deserialize)]
struct KimiAskUserQuestionItem {
    question: String,
    header: String,
    options: Vec<KimiAskUserQuestionOption>,
    #[serde(default)]
    multi_select: bool,
}

#[derive(Deserialize)]
struct KimiAskUserQuestionOption {
    label: String,
    description: String,
}

#[derive(Serialize)]
struct KimiAskUserQuestionOutput {
    answers: std::collections::HashMap<String, RequestUserInputAnswer>,
}

impl ToolHandler for KimiSetTodoListHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "SetTodoList received unsupported payload".to_string(),
            ));
        };
        let args: KimiTodoListArgs = parse_arguments(&arguments)?;
        let Some(todos) = args.todos else {
            return Ok(FunctionToolOutput::from_text(
                "Todo list is empty.".to_string(),
                Some(true),
            ));
        };
        let plan = UpdatePlanArgs {
            explanation: None,
            plan: todos
                .into_iter()
                .map(|todo| PlanItemArg {
                    step: todo.title,
                    status: match todo.status {
                        KimiTodoStatus::Pending => StepStatus::Pending,
                        KimiTodoStatus::InProgress => StepStatus::InProgress,
                        KimiTodoStatus::Done => StepStatus::Completed,
                    },
                })
                .collect(),
        };
        let arguments = serde_json::to_string(&plan).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize SetTodoList update: {err}"))
        })?;
        handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;
        Ok(FunctionToolOutput::from_content(
            vec![
                FunctionCallOutputContentItem::InputText {
                    text: "<system>Todo list updated</system>".to_string(),
                },
                FunctionCallOutputContentItem::InputText {
                    text: "Todo list updated".to_string(),
                },
            ],
            Some(true),
        ))
    }
}

impl ToolHandler for KimiReadFileHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "ReadFile received unsupported payload".to_string(),
            ));
        };
        let args: KimiReadFileArgs = parse_arguments(&arguments)?;
        let path = resolve_workspace_path(turn.as_ref(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("ReadFile failed: {err}")))?;
        let output = format_kimi_read_output(
            &content,
            args.line_offset.unwrap_or(1),
            args.n_lines.unwrap_or(1000),
        );
        Ok(FunctionToolOutput::from_content(
            vec![
                FunctionCallOutputContentItem::InputText {
                    text: output.system_message,
                },
                FunctionCallOutputContentItem::InputText { text: output.body },
            ],
            Some(true),
        ))
    }
}

impl ToolHandler for KimiWriteFileHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "WriteFile received unsupported payload".to_string(),
            ));
        };
        let args: KimiWriteFileArgs = parse_arguments(&arguments)?;
        let path = resolve_workspace_path(turn.as_ref(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        match args.mode.unwrap_or(KimiWriteMode::Overwrite) {
            KimiWriteMode::Overwrite => {
                tokio::fs::write(path.as_path(), args.content)
                    .await
                    .map_err(|err| {
                        FunctionCallError::RespondToModel(format!("WriteFile failed: {err}"))
                    })?;
            }
            KimiWriteMode::Append => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path.as_path())
                    .await
                    .map_err(|err| {
                        FunctionCallError::RespondToModel(format!("WriteFile failed: {err}"))
                    })?;
                use tokio::io::AsyncWriteExt;
                file.write_all(args.content.as_bytes())
                    .await
                    .map_err(|err| {
                        FunctionCallError::RespondToModel(format!("WriteFile failed: {err}"))
                    })?;
            }
        }
        let size_bytes = tokio::fs::metadata(path.as_path())
            .await
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        Ok(FunctionToolOutput::from_text(
            format!(
                "<system>File successfully overwritten. Current size: {size_bytes} bytes.</system>"
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for KimiStrReplaceFileHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "StrReplaceFile received unsupported payload".to_string(),
            ));
        };
        let args: KimiStrReplaceFileArgs = parse_arguments(&arguments)?;
        let path = resolve_workspace_path(turn.as_ref(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("StrReplaceFile failed: {err}"))
            })?;
        let edits = match args.edit {
            KimiEditArg::Single(edit) => vec![edit],
            KimiEditArg::Multiple(edits) => edits,
        };
        let mut updated = content.clone();
        let mut total_replacements = 0usize;
        for edit in &edits {
            let (next_content, replacement_count) = apply_kimi_edit(&updated, edit);
            updated = next_content;
            total_replacements += replacement_count;
        }
        if updated == content {
            return Err(FunctionCallError::RespondToModel(
                "No replacements were made. The old string was not found in the file.".to_string(),
            ));
        }
        tokio::fs::write(path.as_path(), updated)
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("StrReplaceFile failed: {err}"))
            })?;
        Ok(FunctionToolOutput::from_text(
            format!(
                "<system>File successfully edited. Applied {} edit(s) with {total_replacements} total replacement(s).</system>",
                edits.len()
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for KimiAskUserQuestionHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "AskUserQuestion received unsupported payload".to_string(),
            ));
        };
        let mode = session.collaboration_mode().await.mode;
        if let Some(message) = request_user_input_unavailable_message(
            mode,
            turn.tools_config.default_mode_request_user_input,
        ) {
            return Err(FunctionCallError::RespondToModel(message));
        }
        let args: KimiAskUserQuestionArgs = parse_arguments(&arguments)?;
        let request = RequestUserInputArgs {
            questions: args
                .questions
                .into_iter()
                .enumerate()
                .map(|(index, question)| {
                    let _multi_select = question.multi_select;
                    RequestUserInputQuestion {
                        id: format!("kimi-question-{index}"),
                        header: question.header,
                        question: question.question,
                        is_other: false,
                        is_secret: false,
                        options: Some(
                            question
                                .options
                                .into_iter()
                                .map(|option| RequestUserInputQuestionOption {
                                    label: option.label,
                                    description: option.description,
                                })
                                .collect(),
                        ),
                    }
                })
                .collect(),
        };
        let response = session
            .request_user_input(turn.as_ref(), call_id, request)
            .await
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "AskUserQuestion was cancelled before receiving a response".to_string(),
                )
            })?;
        let content = serde_json::to_string(&KimiAskUserQuestionOutput {
            answers: response.answers,
        })
        .map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize AskUserQuestion response: {err}"
            ))
        })?;
        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

fn resolve_workspace_path(
    turn: &TurnContext,
    raw_path: &str,
) -> Result<AbsolutePathBuf, FunctionCallError> {
    let path = Path::new(raw_path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        turn.cwd.as_path().join(path)
    };
    AbsolutePathBuf::try_from(joined).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid path `{raw_path}`: {err}"))
    })
}

struct KimiReadOutput {
    system_message: String,
    body: String,
}

fn format_kimi_read_output(content: &str, line_offset: isize, n_lines: usize) -> KimiReadOutput {
    let lines = content.lines().collect::<Vec<_>>();
    let total = lines.len();
    let n_lines = n_lines.max(1);
    let start = if line_offset < 0 {
        total.saturating_sub(line_offset.unsigned_abs())
    } else {
        usize::try_from(line_offset.saturating_sub(1)).unwrap_or(0)
    };
    let mut numbered_lines = lines
        .into_iter()
        .enumerate()
        .skip(start)
        .take(n_lines)
        .map(|(index, line)| format!("{:6}\t{line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let lines_read = numbered_lines.lines().count();
    if lines_read > 0 {
        numbered_lines.push('\n');
    }
    let start_line = start.saturating_add(1);
    let end_of_file = start.saturating_add(lines_read) >= total;
    KimiReadOutput {
        system_message: format!(
            "<system>{lines_read} lines read from file starting from line {start_line}. Total lines in file: {total}. {}</system>",
            if end_of_file {
                "End of file reached."
            } else {
                "File has more lines."
            }
        ),
        body: numbered_lines,
    }
}

fn apply_kimi_edit(content: &str, edit: &KimiEdit) -> (String, usize) {
    if edit.replace_all.unwrap_or(false) {
        let replacement_count = content.matches(&edit.old).count();
        (content.replace(&edit.old, &edit.new), replacement_count)
    } else {
        let replacement_count = usize::from(content.contains(&edit.old));
        (content.replacen(&edit.old, &edit.new, 1), replacement_count)
    }
}

#[cfg(test)]
#[path = "kimi_cli_tests.rs"]
mod tests;
