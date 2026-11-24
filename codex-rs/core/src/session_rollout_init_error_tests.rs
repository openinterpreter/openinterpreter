//! Checks that storage classifications retain the existing session startup failure policy.

use super::map_session_init_error;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErrorDetails;
use codex_thread_store::ThreadStoreError;
use pretty_assertions::assert_eq;
use std::path::Path;

#[test]
fn storage_failures_remain_fatal_during_session_initialization() {
    for error in [
        ThreadStoreError::ThreadNotFound {
            thread_id: ThreadId::new(),
        },
        ThreadStoreError::InvalidRequest {
            message: "invalid request".to_owned(),
        },
        ThreadStoreError::Internal {
            message: "retryable conflict; outcome unknown".to_owned(),
        },
        ThreadStoreError::Retryable {
            message: "store temporarily unavailable".to_owned(),
        },
        ThreadStoreError::OutcomeUnknown {
            message: "connection lost after write".to_owned(),
        },
    ] {
        let wrapped = anyhow::Error::new(error.clone()).context("opening persistence");
        let mapped = map_session_init_error(&wrapped, Path::new("codex-home"));
        let CodexErrorDetails::Fatal(message) = mapped.details() else {
            panic!("storage failure must remain fatal during startup: {mapped}");
        };
        assert_eq!(
            message,
            &format!("Failed to initialize session: opening persistence: {error}")
        );
    }
}

#[test]
fn conflict_and_unsupported_keep_their_session_initialization_categories() {
    let conflict = anyhow::Error::new(ThreadStoreError::Conflict {
        message: "writer already active".to_owned(),
    })
    .context("opening persistence");
    let mapped = map_session_init_error(&conflict, Path::new("codex-home"));
    assert!(
        matches!(mapped.details(), CodexErrorDetails::InvalidRequest(message)
        if message == "writer already active")
    );

    let unsupported = anyhow::Error::new(ThreadStoreError::Unsupported {
        operation: "create_thread",
    })
    .context("opening persistence");
    let mapped = map_session_init_error(&unsupported, Path::new("codex-home"));
    assert!(
        matches!(mapped.details(), CodexErrorDetails::UnsupportedOperation(message)
        if message == "create_thread is not supported yet")
    );
}
