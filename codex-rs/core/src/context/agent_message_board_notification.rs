//! Fixed-size discussion notices. Post text is fetched through bounded tools.

use super::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;
use uuid::Uuid;

pub struct AgentMessageBoardNotification {
    pub(crate) message_id: Uuid,
    pub(crate) thread_id: Uuid,
}

impl AgentMessageBoardNotification {
    /// Construct a bounded, typed notice without copying untrusted post text.
    pub fn new(message_id: Uuid, thread_id: Uuid) -> Self {
        Self {
            message_id,
            thread_id,
        }
    }
}

impl ContextualUserFragment for AgentMessageBoardNotification {
    fn role(&self) -> &'static str {
        "user"
    }
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("agent_message_board.notification".into())
    }
    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }
    fn type_markers() -> (&'static str, &'static str) {
        (
            "<agent_message_board_notification>",
            "</agent_message_board_notification>",
        )
    }
    fn body(&self) -> String {
        format!(
            "\nNew post {} in discussion {}. Use read_post or read_thread to read it.\n",
            self.message_id, self.thread_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_notice_contains_only_fixed_identifiers_and_read_instructions() {
        let message_id = Uuid::from_u128(1);
        let thread_id = Uuid::from_u128(2);
        let notice = AgentMessageBoardNotification::new(message_id, thread_id);
        let body = notice.body();
        assert!(body.contains(&message_id.to_string()));
        assert!(body.contains(&thread_id.to_string()));
        assert!(body.contains("Use read_post or read_thread"));
        assert!(body.len() < 160);
        assert_eq!(
            AgentMessageBoardNotification::type_markers(),
            (
                "<agent_message_board_notification>",
                "</agent_message_board_notification>",
            )
        );
    }
}
