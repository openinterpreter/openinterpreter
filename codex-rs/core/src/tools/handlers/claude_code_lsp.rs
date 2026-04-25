use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use serde::Deserialize;

pub struct ClaudeLspHandler;

#[derive(Deserialize)]
struct ClaudeLspArgs {
    operation: String,
    #[serde(rename = "filePath")]
    file_path: String,
    line: usize,
    character: usize,
}

impl ToolHandler for ClaudeLspHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "LSP received unsupported payload".to_string(),
            ));
        };

        let args: ClaudeLspArgs = parse_arguments(&arguments)?;
        Err(FunctionCallError::RespondToModel(format!(
            "LSP operation {} at {}:{}:{} is not configured in this Open Interpreter build yet.",
            args.operation, args.file_path, args.line, args.character
        )))
    }
}
