use super::ContextualUserFragment;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ContentItemKind;
use codex_protocol::models::input_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UnsupportedMedia {
    text: &'static str,
    content_kind: &'static str,
}

impl UnsupportedMedia {
    pub(crate) const IMAGE_KIND: &str = "images.unsupported";
    pub(crate) const AUDIO_KIND: &str = "audio.unsupported";

    pub(crate) const IMAGE: Self = Self {
        text: "image content omitted because you do not support image input",
        content_kind: Self::IMAGE_KIND,
    };

    pub(crate) const AUDIO: Self = Self {
        text: "audio content omitted because you do not support audio input",
        content_kind: Self::AUDIO_KIND,
    };
}

impl ContextualUserFragment for UnsupportedMedia {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind(self.content_kind.to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("", "")
    }

    fn body(&self) -> ContentItem {
        input_text(self.text.to_string())
    }
}
