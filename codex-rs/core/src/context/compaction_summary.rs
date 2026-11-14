use super::ContextualUserFragment;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ContentItemKind;
use codex_protocol::models::input_text;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactionSummary {
    summary: String,
}

impl CompactionSummary {
    pub(crate) const KIND: &str = codex_context_fragments::COMPACTION_SUMMARY_KIND;

    pub(crate) fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
        }
    }
}

impl ContextualUserFragment for CompactionSummary {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind(Self::KIND.to_string())
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
        input_text(self.summary.clone())
    }
}
