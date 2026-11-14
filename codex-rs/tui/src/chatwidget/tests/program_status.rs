//! Exercises terminal status across live, interrupted, and replayed turns.
//! Replayed results and work already underway must never appear as new completions.

use super::*;
use crate::terminal_program_status::ProgramStatus;
use crate::terminal_program_status::ProgramStatusKind;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn program_status_tracks_live_results_and_resolved_approvals() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    let mut statuses = vec![chat.desired_program_status()];
    handle_turn_started(&mut chat, "status-turn");
    statuses.push(chat.desired_program_status());
    handle_exec_approval_request(
        &mut chat,
        "status-approval",
        ExecApprovalRequestEvent {
            kind: Default::default(),
            call_id: "status-call".to_string(),
            approval_id: Some("status-call".to_string()),
            turn_id: "status-turn".to_string(),
            environment_id: None,
            command: vec!["echo".to_string(), "hello".to_string()],
            cwd: AbsolutePathBuf::current_dir().expect("current dir"),
            reason: Some("need confirmation".to_string()),
            network_approval_context: None,
            proposed_execpolicy_amendment: None,
            proposed_network_policy_amendments: None,
            additional_permissions: None,
            available_decisions: None,
        },
    );
    statuses.push(chat.desired_program_status());
    chat.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    statuses.push(chat.desired_program_status());
    handle_turn_completed(&mut chat, "status-turn", /*duration_ms*/ None);
    statuses.push(chat.desired_program_status());
    chat.pre_draw_tick();
    statuses.push(chat.desired_program_status());
    chat.acknowledge_program_status();
    statuses.push(chat.desired_program_status());
    handle_turn_started(&mut chat, "cancelled-turn");
    handle_turn_interrupted(&mut chat, "cancelled-turn");
    statuses.push(chat.desired_program_status());
    handle_turn_started(&mut chat, "failed-turn");
    chat.handle_server_notification(
        ServerNotification::TurnCompleted(TurnCompletedNotification {
            thread_id: String::new(),
            turn: app_server_turn(
                "failed-turn",
                AppServerTurnStatus::Failed,
                /*duration_ms*/ None,
                /*error*/ None,
            ),
        }),
        /*replay_kind*/ None,
    );
    statuses.push(chat.desired_program_status());
    chat.acknowledge_program_status();
    statuses.push(chat.desired_program_status());
    assert_eq!(
        statuses,
        [
            ProgramStatus::Idle,
            ProgramStatus::Working,
            ProgramStatus::Blocked(ProgramStatusKind::Permission),
            ProgramStatus::Working,
            ProgramStatus::Done,
            ProgramStatus::Done,
            ProgramStatus::Idle,
            ProgramStatus::Idle,
            ProgramStatus::Error,
            ProgramStatus::Idle,
        ]
    );
}

#[tokio::test]
async fn program_status_does_not_treat_replayed_results_as_new_or_hide_follow_up_work() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    handle_turn_started(&mut chat, "old-turn");
    chat.on_task_complete(
        /*last_agent_message*/ None, /*completion*/ None, /*from_replay*/ true,
    );
    let replayed = chat.desired_program_status();
    handle_turn_started(&mut chat, "live-turn");
    handle_turn_completed(&mut chat, "live-turn", /*duration_ms*/ None);
    let completed = chat.desired_program_status();
    // A queued follow-up is already submitted, before its TurnStarted notification arrives.
    chat.input_queue.user_turn_pending_start = true;
    let pending = chat.desired_program_status();
    assert_eq!(
        [replayed, completed, pending],
        [
            ProgramStatus::Idle,
            ProgramStatus::Done,
            ProgramStatus::Working
        ]
    );
}
