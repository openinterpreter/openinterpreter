//! Transitional access to core-only types still used by the TUI.
//!
//! This keeps the thin-client build from depending on
//! `codex-app-server-client::legacy_core`, which is only available on the
//! in-process app-server path.

#[cfg(not(feature = "thin-client"))]
pub use codex_core::Cursor;
pub use codex_core::DEFAULT_AGENTS_MD_FILENAME;
#[cfg(not(feature = "thin-client"))]
pub use codex_core::INTERACTIVE_SESSION_SOURCES;
#[allow(unused_imports)]
pub use codex_core::LOCAL_AGENTS_MD_FILENAME;
#[cfg(not(feature = "cross-session-history"))]
pub use codex_core::MessageHistoryEntry;
#[cfg(not(feature = "thin-client"))]
pub use codex_core::RolloutRecorder;
#[cfg(not(feature = "thin-client"))]
pub use codex_core::SortDirection;
#[cfg(not(feature = "thin-client"))]
pub use codex_core::ThreadItem;
#[cfg(not(feature = "thin-client"))]
pub use codex_core::ThreadsPage;
#[cfg(feature = "cross-session-history")]
pub use codex_core::append_message_history_entry;
pub use codex_core::check_execpolicy_for_warnings;
pub use codex_core::find_thread_meta_by_name_str;
pub use codex_core::find_thread_name_by_id;
pub use codex_core::find_thread_names_by_ids;
pub use codex_core::format_exec_policy_error_with_source;
pub use codex_core::grant_read_root_non_elevated;
#[cfg(feature = "cross-session-history")]
pub use codex_core::lookup_message_history_entry;
pub use codex_core::mention_syntax::PLUGIN_TEXT_MENTION_SIGIL;
pub use codex_core::mention_syntax::TOOL_MENTION_SIGIL;
#[cfg(feature = "cross-session-history")]
pub use codex_core::message_history_metadata;
pub use codex_core::path_utils;
pub use codex_core::read_session_meta_line;
pub use codex_core::web_search_detail;
#[cfg(not(feature = "cross-session-history"))]
use codex_protocol::ThreadId;

#[cfg(not(feature = "cross-session-history"))]
pub async fn append_message_history_entry(
    _text: &str,
    _conversation_id: &ThreadId,
    _config: &config::Config,
) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(feature = "cross-session-history"))]
pub fn lookup_message_history_entry(
    _log_id: u64,
    _offset: usize,
    _config: &config::Config,
) -> Option<MessageHistoryEntry> {
    None
}

#[cfg(not(feature = "cross-session-history"))]
pub async fn message_history_metadata(_config: &config::Config) -> (u64, usize) {
    (0, 0)
}

pub mod config {
    pub use codex_core::config::*;

    pub mod edit {
        pub use codex_core::config::edit::*;
    }
}

pub mod config_loader {
    pub use codex_core::config_loader::*;
}

pub mod connectors {
    #[allow(unused_imports)]
    pub use codex_core::connectors::*;
}

pub mod otel_init {
    pub use codex_core::otel_init::*;
}

pub mod personality_migration {
    pub use codex_core::personality_migration::*;
}

pub mod plugins {
    pub use codex_core::plugins::*;
}

pub mod review_format {
    #[allow(unused_imports)]
    pub use codex_core::review_format::*;
}

pub mod review_prompts {
    #[allow(unused_imports)]
    pub use codex_core::review_prompts::*;
}

pub mod skills {
    pub use codex_core::skills::*;
}

pub mod test_support {
    #[allow(unused_imports)]
    pub use codex_core::test_support::*;
}

pub mod util {
    pub use codex_core::util::*;
}

pub mod windows_sandbox {
    pub use codex_core::windows_sandbox::*;
}
