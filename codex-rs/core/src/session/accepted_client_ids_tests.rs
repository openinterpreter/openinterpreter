use super::*;
use codex_protocol::config_types::ModeKind;
use codex_protocol::protocol::TurnStartedEvent;
use codex_protocol::protocol::UserMessageEvent;
use pretty_assertions::assert_eq;

fn turn_started(turn_id: &str) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
        turn_id: turn_id.to_string(),
        root_turn_id: None,
        trace_id: None,
        started_at: None,
        model_context_window: None,
        collaboration_mode_kind: ModeKind::default(),
    }))
}

fn user_message(client_id: Option<&str>) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
        client_id: client_id.map(str::to_string),
        message: "hello".to_string(),
        ..Default::default()
    }))
}

#[test]
fn evicts_oldest_id_beyond_bound() {
    let mut accepted = AcceptedClientIds::default();
    for index in 0..=MAX_ACCEPTED_CLIENT_IDS {
        accepted.insert(format!("id-{index}"), "turn".to_string());
    }

    assert_eq!(accepted.turn_id("id-0"), None);
    assert_eq!(accepted.turn_id("id-1"), Some("turn"));
    assert_eq!(accepted.order.len(), MAX_ACCEPTED_CLIENT_IDS);
    assert_eq!(accepted.turn_ids.len(), MAX_ACCEPTED_CLIENT_IDS);
}

#[test]
fn reinserting_known_id_moves_it_to_new_turn_without_reordering() {
    let mut accepted = AcceptedClientIds::default();
    accepted.insert("a".to_string(), "turn-1".to_string());
    accepted.insert("b".to_string(), "turn-1".to_string());
    accepted.insert("a".to_string(), "turn-2".to_string());

    assert_eq!(
        (accepted.turn_id("a"), accepted.turn_id("b")),
        (Some("turn-2"), Some("turn-1"))
    );
    assert_eq!(
        accepted.order,
        VecDeque::from(vec!["a".to_string(), "b".to_string()])
    );
}

#[test]
fn removed_id_is_forgotten_and_frees_its_slot() {
    let mut accepted = AcceptedClientIds::default();
    accepted.insert("a".to_string(), "turn-1".to_string());
    accepted.insert("b".to_string(), "turn-1".to_string());
    accepted.remove("a");
    accepted.remove("missing");

    assert_eq!(accepted.turn_id("a"), None);
    assert_eq!(accepted.order, VecDeque::from(vec!["b".to_string()]));
}

#[test]
fn rebuilds_from_recorded_user_messages() {
    let accepted = AcceptedClientIds::from_rollout(&[
        // A message before any turn boundary has no turn to map to.
        user_message(Some("orphan")),
        turn_started("turn-1"),
        user_message(Some("first")),
        user_message(/*client_id*/ None),
        turn_started("turn-2"),
        user_message(Some("second")),
    ]);

    assert_eq!(
        (
            accepted.turn_id("orphan"),
            accepted.turn_id("first"),
            accepted.turn_id("second"),
        ),
        (None, Some("turn-1"), Some("turn-2"))
    );
    assert_eq!(
        accepted.order,
        VecDeque::from(vec!["first".to_string(), "second".to_string()])
    );
}
