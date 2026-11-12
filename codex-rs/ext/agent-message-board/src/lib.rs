//! Shared agent discussions with interchangeable local and remote backends.
//!
//! Board identity and caller identity come from the host. Implementations own
//! storage and notification fanout; tools and feature registration are separate.

mod api;
mod backend;
mod extension;
mod extension_v2;
mod host;
mod local;
mod mcp;
mod tools;
mod types;

pub use api::AgentMessageBoard;
pub use api::ChannelQuery;
pub use api::CreateChannelRequest;
pub use api::PageRequest;
pub use api::PostDestination;
pub use api::PostQuery;
pub use api::PostRequest;
pub use api::ReadPostRequest;
pub use api::ReadThreadRequest;
pub use api::SortDirection;
pub use api::SubscriptionChange;
pub use api::SubscriptionRequest;
pub use api::SubscriptionTarget;
pub use api::ThreadQuery;
pub use api::ThreadSort;
pub use backend::BoardBackend;
pub use backend::install as install_backends;
pub use extension::install;
pub use host::MessageBoardHost;
pub use host::NotificationDelivery;
pub use local::LocalAgentMessageBoard;
pub use tools::message_board_tools;
pub use types::ChannelSummary;
pub use types::Page;
pub use types::PostContent;
pub use types::PostMetadata;
pub use types::PostPreview;
pub use types::SubscriptionState;
pub use types::ThreadPage;
pub use types::ThreadSummary;

/// Storage selected by the host for newly configured local boards. The
/// existing OIX local-board API remains available for current MAv2 callers.
pub fn local_board_options() -> codex_message_board::LocalBoardOptions {
    codex_message_board::LocalBoardOptions {
        storage_namespace: "agent_message_board".into(),
    }
}
