//! Failure accounting distinguishes internal retry cleanup from a requested stop.

use super::super::StartupRetry;
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn terminal_error_during_retry_cleanup_records_one_failure() {
    for (retry, expected_failure) in [
        (StartupRetry::WaitingForStop, true),
        (StartupRetry::Used, false),
    ] {
        let (mut chat, _sender, _events, _ops) = make_chatwidget_manual_with_sender().await;
        activate_voice(&mut chat);
        chat.realtime_conversation.phase = RealtimeConversationPhase::Stopping;
        chat.realtime_conversation.startup_retry = retry;

        chat.on_realtime_error("connection lost".into());

        assert_eq!(
            chat.realtime_conversation.failure_recorded,
            expected_failure
        );
        chat.on_realtime_error("duplicate error".into());
        assert_eq!(
            chat.realtime_conversation.failure_recorded,
            expected_failure
        );
    }
}
