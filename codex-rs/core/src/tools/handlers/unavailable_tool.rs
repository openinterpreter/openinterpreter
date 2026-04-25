use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_tools::Harness;

pub struct UnavailableToolHandler;

pub(crate) fn unavailable_tool_message(
    tool_name: impl std::fmt::Display,
    next_step: &str,
) -> String {
    format!(
        "Tool `{tool_name}` is not currently available. It appeared in earlier tool calls in this conversation, but its implementation is not available in the current request. {next_step}"
    )
}

pub(crate) fn hidden_tool_call_message(
    tool_name: impl std::fmt::Display,
    harness: &Harness,
) -> String {
    let tool_name = tool_name.to_string();
    if harness.is_claude_code() {
        return format!(
            "<tool_use_error>Error: No such tool available: {tool_name}. {tool_name} exists but is not enabled in this context. Use one of the available tools instead.</tool_use_error>"
        );
    }

    unavailable_tool_message(
        tool_name,
        "Retry after the tool becomes available or ask the user to re-enable it.",
    )
}

impl ToolHandler for UnavailableToolHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            tool_name, payload, ..
        } = invocation;

        match payload {
            ToolPayload::Function { .. } => Ok(FunctionToolOutput::from_text(
                hidden_tool_call_message(
                    tool_name.display(),
                    &invocation.turn.tools_config.harness,
                ),
                Some(false),
            )),
            _ => Err(FunctionCallError::RespondToModel(
                "unavailable tool handler received unsupported payload".to_string(),
            )),
        }
    }
}
