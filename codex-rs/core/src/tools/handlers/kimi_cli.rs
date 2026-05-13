use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::claude_code::effective_turn_file_system_policy;
use crate::tools::handlers::claude_code::ensure_readable_path;
use crate::tools::handlers::claude_code::ensure_writable_path;
use crate::tools::handlers::parse_kimi_arguments;
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
pub(super) struct KimiTodoItem {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(alias = "content")]
    title: String,
    status: KimiTodoStatus,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum KimiTodoStatus {
    Pending,
    InProgress,
    Completed,
    Done,
}

#[derive(Deserialize)]
pub(super) struct KimiReadFileArgs {
    #[serde(alias = "file_path")]
    path: String,
    line_offset: Option<isize>,
    n_lines: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct KimiWriteFileArgs {
    #[serde(alias = "file_path")]
    path: String,
    content: String,
    mode: Option<KimiWriteMode>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum KimiWriteMode {
    Overwrite,
    Append,
}

#[derive(Deserialize)]
pub(super) struct KimiStrReplaceFileArgs {
    #[serde(alias = "file_path")]
    path: String,
    edit: KimiEditArg,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum KimiEditArg {
    Single(KimiEdit),
    Multiple(Vec<KimiEdit>),
}

#[derive(Deserialize)]
pub(super) struct KimiEdit {
    #[serde(alias = "old_string")]
    old: String,
    #[serde(alias = "new_string")]
    new: String,
    replace_all: Option<bool>,
}

#[derive(Deserialize)]
struct KimiAskUserQuestionArgs {
    questions: Vec<KimiAskUserQuestionItem>,
}

#[derive(Deserialize)]
pub(super) struct KimiAskUserQuestionItem {
    question: String,
    header: String,
    options: Vec<KimiAskUserQuestionOption>,
    #[serde(default, alias = "multiSelect")]
    multi_select: bool,
}

#[derive(Deserialize)]
pub(super) struct KimiAskUserQuestionOption {
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
        let args: KimiTodoListArgs = parse_kimi_arguments(&arguments)?;
        let Some(todos) = args.todos else {
            let todos = session.kimi_todos().await;
            let output = if todos.is_empty() {
                "Todo list is empty.".to_string()
            } else {
                let mut lines = vec!["Current todo list:".to_string()];
                lines.extend(todos.into_iter().map(|todo| {
                    let status = match todo.status {
                        StepStatus::Pending => "pending",
                        StepStatus::InProgress => "in_progress",
                        StepStatus::Completed => "done",
                    };
                    format!("- [{status}] {}", todo.step)
                }));
                lines.join("\n")
            };
            return Ok(FunctionToolOutput::from_text(output, Some(true)));
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
                        KimiTodoStatus::Completed | KimiTodoStatus::Done => StepStatus::Completed,
                    },
                })
                .collect(),
        };
        session.set_kimi_todos(plan.plan.clone()).await;
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
        let args: KimiReadFileArgs = parse_kimi_arguments(&arguments)?;
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
        let args: KimiWriteFileArgs = parse_kimi_arguments(&arguments)?;
        let path = resolve_workspace_path(turn.as_ref(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        let mode = args.mode.unwrap_or(KimiWriteMode::Overwrite);
        match mode {
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
        let action = match mode {
            KimiWriteMode::Overwrite => "overwritten",
            KimiWriteMode::Append => "appended to",
        };
        Ok(FunctionToolOutput::from_text(
            format!(
                "<system>File successfully {action}. Current size: {size_bytes} bytes.</system>"
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
        let args: KimiStrReplaceFileArgs = parse_kimi_arguments(&arguments)?;
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
        let args: KimiAskUserQuestionArgs = parse_kimi_arguments(&arguments)?;
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

pub(super) fn resolve_workspace_path(
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

pub(super) struct KimiReadOutput {
    system_message: String,
    body: String,
}

const KIMI_READ_MAX_LINES: usize = 1000;
const KIMI_READ_MAX_LINE_LENGTH: usize = 2000;
const KIMI_READ_MAX_BYTES: usize = 100 << 10;

pub(super) fn format_kimi_read_output(
    content: &str,
    line_offset: isize,
    n_lines: usize,
) -> KimiReadOutput {
    let all_lines = content.split_inclusive('\n').collect::<Vec<_>>();
    let total = all_lines.len();
    let start = if line_offset < 0 {
        total.saturating_sub(line_offset.unsigned_abs())
    } else {
        usize::try_from(line_offset.saturating_sub(1)).unwrap_or(0)
    };

    let mut lines = Vec::new();
    let mut n_bytes = 0usize;
    let mut truncated_line_numbers = Vec::new();
    let mut max_lines_reached = false;
    let mut max_bytes_reached = false;
    let mut collecting = true;
    let n_lines = n_lines.max(1);
    for (index, line) in all_lines.into_iter().enumerate().skip(start) {
        if !collecting {
            continue;
        }
        let truncated = truncate_kimi_read_line(line, KIMI_READ_MAX_LINE_LENGTH);
        if truncated != line {
            truncated_line_numbers.push(index + 1);
        }
        n_bytes += truncated.len();
        lines.push((index + 1, truncated));
        if lines.len() >= n_lines {
            collecting = false;
        } else if lines.len() >= KIMI_READ_MAX_LINES {
            max_lines_reached = true;
            collecting = false;
        } else if n_bytes >= KIMI_READ_MAX_BYTES {
            max_bytes_reached = true;
            collecting = false;
        }
    }

    let mut numbered_lines = String::new();
    for (line_number, line) in &lines {
        numbered_lines.push_str(&format!("{line_number:6}\t{line}"));
    }
    let lines_read = lines.len();
    let start_line = start.saturating_add(1);
    let mut message = if lines_read > 0 {
        format!("{lines_read} lines read from file starting from line {start_line}.")
    } else {
        "No lines read from file.".to_string()
    };
    message.push_str(&format!(" Total lines in file: {total}."));
    if max_lines_reached {
        message.push_str(&format!(" Max {KIMI_READ_MAX_LINES} lines reached."));
    } else if max_bytes_reached {
        message.push_str(&format!(" Max {KIMI_READ_MAX_BYTES} bytes reached."));
    } else if lines_read < n_lines {
        message.push_str(" End of file reached.");
    }
    if !truncated_line_numbers.is_empty() {
        message.push_str(&format!(
            " Lines {truncated_line_numbers:?} were truncated."
        ));
    }

    KimiReadOutput {
        system_message: format!("<system>{message}</system>"),
        body: numbered_lines,
    }
}

fn truncate_kimi_read_line(line: &str, max_length: usize) -> String {
    if line.chars().count() <= max_length {
        return line.to_string();
    }
    let linebreak_start = line
        .char_indices()
        .rev()
        .find(|(_, ch)| !matches!(ch, '\r' | '\n'))
        .map_or(0, |(idx, ch)| idx + ch.len_utf8());
    let linebreak = &line[linebreak_start..];
    let marker = "...";
    let suffix = format!("{marker}{linebreak}");
    let suffix_chars = suffix.chars().count();
    let prefix_chars = max_length.max(suffix_chars).saturating_sub(suffix_chars);
    let prefix = line.chars().take(prefix_chars).collect::<String>();
    format!("{prefix}{suffix}")
}

pub(super) fn apply_kimi_edit(content: &str, edit: &KimiEdit) -> (String, usize) {
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
