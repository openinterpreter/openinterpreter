use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_kimi_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_protocol::models::DEFAULT_IMAGE_DETAIL;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::openai_models::InputModality;
use serde::Deserialize;

pub struct KimiReadMediaFileHandler;

#[derive(Deserialize)]
struct KimiReadMediaFileArgs {
    path: String,
}

impl ToolHandler for KimiReadMediaFileHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        if !invocation
            .turn
            .model_info
            .input_modalities
            .contains(&InputModality::Image)
        {
            return Err(FunctionCallError::RespondToModel(
                "ReadMediaFile is not allowed because this model does not support image inputs"
                    .to_string(),
            ));
        }

        let ToolInvocation { turn, payload, .. } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "ReadMediaFile received unsupported payload".to_string(),
            ));
        };

        let args: KimiReadMediaFileArgs = parse_kimi_arguments(&arguments)?;
        let Some(environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "ReadMediaFile is unavailable in this session".to_string(),
            ));
        };
        let path = turn.resolve_path(Some(args.path));
        let sandbox = environment
            .is_remote()
            .then(|| turn.file_system_sandbox_context(/*additional_permissions*/ None));
        let metadata = environment
            .get_filesystem()
            .get_metadata(&path, sandbox.as_ref())
            .await
            .map_err(|error| {
                FunctionCallError::RespondToModel(format!(
                    "unable to locate media at `{}`: {error}",
                    path.display()
                ))
            })?;
        if !metadata.is_file {
            return Err(FunctionCallError::RespondToModel(format!(
                "media path `{}` is not a file",
                path.display()
            )));
        }

        let bytes = environment
            .get_filesystem()
            .read_file(&path, sandbox.as_ref())
            .await
            .map_err(|error| {
                FunctionCallError::RespondToModel(format!(
                    "unable to read media at `{}`: {error}",
                    path.display()
                ))
            })?;
        let mime = mime_from_path(path.as_path()).ok_or_else(|| {
            FunctionCallError::RespondToModel(format!(
                "ReadMediaFile only supports common image formats right now: `{}`",
                path.display()
            ))
        })?;
        let byte_len = bytes.len();
        let dimensions = image::load_from_memory(&bytes)
            .map(|image| (image.width(), image.height()))
            .map_err(|error| {
                FunctionCallError::RespondToModel(format!(
                    "unable to decode image at `{}`: {error}",
                    path.display()
                ))
            })?;
        let image_url = format!("data:{mime};base64,{}", BASE64_STANDARD.encode(bytes));
        let display_path = path.display();

        Ok(FunctionToolOutput::from_content(
            vec![
                FunctionCallOutputContentItem::InputText {
                    text: format!(
                        "<system>Loaded image file `{display_path}` ({mime}, {byte_len} bytes, original size {}x{}px). If you need to output coordinates, output relative coordinates first and compute absolute coordinates using the original image size; if you generate or edit images/videos via commands or scripts, read the result back immediately before continuing.</system>",
                        dimensions.0, dimensions.1
                    ),
                },
                FunctionCallOutputContentItem::InputText {
                    text: format!("<image path=\"{display_path}\">"),
                },
                FunctionCallOutputContentItem::InputImage {
                    image_url,
                    detail: Some(DEFAULT_IMAGE_DETAIL),
                },
                FunctionCallOutputContentItem::InputText {
                    text: "</image>".to_string(),
                },
            ],
            Some(true),
        ))
    }
}

fn mime_from_path(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        Some("svg") => Some("image/svg+xml"),
        _ => None,
    }
}
