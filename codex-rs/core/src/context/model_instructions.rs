//! A single model-context fragment for independent model and instruction changes.

use super::ContextualUserFragment;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ContentItemKind;
use codex_protocol::models::input_text;

const REPLACEMENT_NOTICE: &str =
    "The following instructions replace the previous base instructions:";
const REMOVAL_NOTICE: &str =
    "The previous base instructions no longer apply. No replacement base instructions are active.";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelInstructions {
    pub(crate) model: Option<String>,
    /// None leaves instructions unchanged; Some replaces them, including an empty value.
    pub(crate) instructions: Option<String>,
}

impl ContextualUserFragment for ModelInstructions {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("model.instructions".to_string())
    }

    fn role(&self) -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<model_instructions>", "</model_instructions>")
    }

    fn body(&self) -> ContentItem {
        let mut body = String::from("\n");
        if let Some(model) = &self.model {
            body.push_str(&format!("The current model is {model}.\n"));
        }
        match &self.instructions {
            Some(instructions) if instructions.is_empty() => {
                body.push_str(REMOVAL_NOTICE);
                body.push('\n');
            }
            Some(instructions) => {
                body.push_str(&format!("{REPLACEMENT_NOTICE}\n\n{instructions}\n"));
            }
            None => {}
        }
        input_text(body)
    }
}
