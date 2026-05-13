use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::KimiAgentHandler;
use crate::tools::handlers::KimiAskUserQuestionHandler;
use crate::tools::handlers::KimiExitPlanModeHandler;
use crate::tools::handlers::KimiGlobHandler;
use crate::tools::handlers::KimiGrepHandler;
use crate::tools::handlers::KimiReadFileHandler;
use crate::tools::handlers::KimiSetTodoListHandler;
use crate::tools::handlers::KimiShellHandler;
use crate::tools::handlers::KimiStrReplaceFileHandler;
use crate::tools::handlers::KimiWriteFileHandler;
use crate::tools::handlers::parse_kimi_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::models::FunctionCallOutputContentItem;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

pub struct QwenAgentHandler;
pub struct QwenAskUserQuestionHandler;
pub struct QwenEditHandler;
pub struct QwenExitPlanModeHandler;
pub struct QwenGlobHandler;
pub struct QwenGrepSearchHandler;
pub struct QwenReadFileHandler;
pub struct QwenShellHandler;
pub struct QwenTodoWriteHandler;
pub struct QwenWriteFileHandler;

#[derive(Deserialize)]
struct QwenReadFileArgs {
    file_path: String,
    offset: Option<isize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct QwenEditArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    replace_all: Option<bool>,
}

#[derive(Deserialize)]
struct QwenShellArgs {
    command: String,
    is_background: Option<bool>,
    timeout: Option<u64>,
    description: Option<String>,
    directory: Option<String>,
}

macro_rules! forward_handler {
    ($handler:ident, $target:ident) => {
        impl ToolHandler for $handler {
            type Output = FunctionToolOutput;

            fn kind(&self) -> ToolKind {
                ToolKind::Function
            }

            async fn is_mutating(&self, invocation: &ToolInvocation) -> bool {
                $target.is_mutating(invocation).await
            }

            async fn handle(
                &self,
                invocation: ToolInvocation,
            ) -> Result<Self::Output, FunctionCallError> {
                $target.handle(invocation).await
            }
        }
    };
}

forward_handler!(QwenAgentHandler, KimiAgentHandler);
forward_handler!(QwenAskUserQuestionHandler, KimiAskUserQuestionHandler);
forward_handler!(QwenExitPlanModeHandler, KimiExitPlanModeHandler);
forward_handler!(QwenGlobHandler, KimiGlobHandler);
forward_handler!(QwenGrepSearchHandler, KimiGrepHandler);
forward_handler!(QwenTodoWriteHandler, KimiSetTodoListHandler);
forward_handler!(QwenWriteFileHandler, KimiWriteFileHandler);

impl ToolHandler for QwenReadFileHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "read_file received unsupported payload".to_string(),
            ));
        };
        let args: QwenReadFileArgs = parse_kimi_arguments(arguments)?;
        let line_offset = args.offset.map(|offset| offset.saturating_add(1));
        KimiReadFileHandler
            .handle(with_arguments(
                invocation,
                json!({
                    "path": args.file_path,
                    "line_offset": line_offset,
                    "n_lines": args.limit,
                }),
            )?)
            .await?;
        let content = tokio::fs::read_to_string(&args.file_path)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("read_file failed: {err}")))?;
        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

impl ToolHandler for QwenEditHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "edit received unsupported payload".to_string(),
            ));
        };
        let args: QwenEditArgs = parse_kimi_arguments(arguments)?;
        KimiStrReplaceFileHandler
            .handle(with_arguments(
                invocation,
                json!({
                    "path": args.file_path,
                    "edit": {
                        "old": args.old_string,
                        "new": args.new_string,
                        "replace_all": args.replace_all,
                    },
                }),
            )?)
            .await?;
        let content = tokio::fs::read_to_string(&args.file_path)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("edit failed: {err}")))?;
        let line_count = qwen_line_count(&content);
        Ok(FunctionToolOutput::from_text(
            format!(
                "The file: {} has been updated. Showing lines 1-{line_count} of {line_count} from the edited file:\n\n---\n\n{content}",
                args.file_path
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for QwenShellHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "run_shell_command received unsupported payload".to_string(),
            ));
        };
        let args: QwenShellArgs = parse_kimi_arguments(arguments)?;
        let command = qwen_command_with_directory(&args.command, args.directory.as_deref())?;
        let timeout_seconds = args
            .timeout
            .map(|timeout_ms| timeout_ms.saturating_add(999) / 1000);
        let output = KimiShellHandler
            .handle(with_arguments(
                invocation,
                json!({
                    "command": command,
                    "timeout": timeout_seconds,
                    "run_in_background": args.is_background,
                    "description": args.description,
                }),
            )?)
            .await?;
        Ok(qwen_shell_output(
            &args.command,
            args.directory.as_deref(),
            output,
        ))
    }
}

fn qwen_command_with_directory(
    command: &str,
    directory: Option<&str>,
) -> Result<String, FunctionCallError> {
    let Some(directory) = directory.filter(|directory| !directory.trim().is_empty()) else {
        return Ok(command.to_string());
    };
    if !Path::new(directory).is_absolute() {
        return Err(FunctionCallError::RespondToModel(
            "directory must be an absolute path".to_string(),
        ));
    }
    let cd_command =
        codex_shell_command::parse_command::shlex_join(&["cd".to_string(), directory.to_string()]);
    Ok(format!("{cd_command} && {command}"))
}

fn qwen_line_count(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }
    let trailing_line = usize::from(content.ends_with('\n'));
    content.lines().count() + trailing_line
}

fn qwen_shell_output(
    command: &str,
    directory: Option<&str>,
    output: FunctionToolOutput,
) -> FunctionToolOutput {
    let output_text = output
        .body
        .iter()
        .filter_map(|item| match item {
            FunctionCallOutputContentItem::InputText { text } if !text.starts_with("<system>") => {
                Some(text.as_str())
            }
            FunctionCallOutputContentItem::InputText { .. }
            | FunctionCallOutputContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("");
    let error_text = if output.success == Some(false) {
        "(see output)"
    } else {
        "(none)"
    };
    let exit_code = if output.success == Some(false) { 1 } else { 0 };
    FunctionToolOutput::from_text(
        format!(
            "Command: {command}\nDirectory: {}\nOutput: {output_text}Error: {error_text}\nExit Code: {exit_code}\nSignal: 0\nProcess Group PGID: {}",
            directory.unwrap_or("(root)"),
            std::process::id()
        ),
        output.success,
    )
}

fn with_arguments(
    mut invocation: ToolInvocation,
    arguments: serde_json::Value,
) -> Result<ToolInvocation, FunctionCallError> {
    invocation.payload = ToolPayload::Function {
        arguments: serde_json::to_string(&arguments).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize Qwen tool arguments: {err}"))
        })?,
    };
    Ok(invocation)
}
