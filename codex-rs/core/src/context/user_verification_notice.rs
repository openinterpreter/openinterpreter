//! A bounded realtime notice that never includes verification challenge or display text.

use super::ContextualUserFragment;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ContentItemKind;
use codex_protocol::models::input_text;

pub(crate) struct UserVerificationNotice;

impl ContextualUserFragment for UserVerificationNotice {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("user_verification.notice".to_string())
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<user_verification_notice>", "</user_verification_notice>")
    }

    fn body(&self) -> ContentItem {
        input_text("User verification is required. Please respond in the app.".to_string())
    }
}
