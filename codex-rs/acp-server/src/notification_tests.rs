use super::*;
use codex_app_server_protocol::AgentMessageDeltaNotification;
use codex_app_server_protocol::ItemCompletedNotification;
use codex_app_server_protocol::ThreadItem;

fn message(id: &str, text: &str) -> ThreadItem {
    ThreadItem::AgentMessage {
        id: id.to_string(),
        text: text.to_string(),
        phase: None,
        memory_citation: None,
        delivery: None,
        questions: None,
    }
}

fn delta(id: &str, text: &str) -> ServerNotification {
    ServerNotification::AgentMessageDelta(AgentMessageDeltaNotification {
        thread_id: "thread".to_string(),
        turn_id: "turn".to_string(),
        item_id: id.to_string(),
        delta: text.to_string(),
    })
}

fn completed(item: ThreadItem) -> ServerNotification {
    ServerNotification::ItemCompleted(ItemCompletedNotification {
        thread_id: "thread".to_string(),
        turn_id: "turn".to_string(),
        item,
        completed_at_ms: 0,
    })
}

fn turn_completed(items: Vec<ThreadItem>) -> ServerNotification {
    serde_json::from_value(serde_json::json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thread",
            "turn": { "id": "turn", "items": items, "status": "completed", "error": null }
        }
    }))
    .expect("valid turn completion")
}

async fn collect_updates(notifications: Vec<ServerNotification>) -> Vec<SessionUpdate> {
    let (tx, rx) = async_channel::unbounded();
    let client = Client.builder().on_receive_notification(
        async move |notification: SessionNotification, _cx: ConnectionTo<Agent>| {
            tx.send(notification.update)
                .await
                .expect("collector is open");
            Ok(())
        },
        acp::on_receive_notification!(),
    );
    Agent
        .builder()
        .connect_with(client, async move |cx| {
            let session_id = SessionId::new("session");
            let mut completed_item_ids = HashSet::new();
            let mut streamed_agent_messages = HashMap::new();
            for notification in notifications {
                handle_notification(
                    &session_id,
                    "thread",
                    "turn",
                    notification,
                    &cx,
                    &mut completed_item_ids,
                    &mut streamed_agent_messages,
                )?;
            }
            // A final notification is a FIFO barrier, so no sleep or timing assumption is needed.
            let barrier = SessionUpdate::UserMessageChunk(ContentChunk::new("barrier".into()));
            send_update(&session_id, &cx, barrier.clone())?;
            let mut updates = Vec::new();
            loop {
                let update = rx.recv().await.expect("notification handler is alive");
                if update == barrier {
                    return Ok(updates);
                }
                updates.push(update);
            }
        })
        .await
        .expect("ACP in-memory connection")
}

fn chunks(texts: &[&str]) -> Vec<SessionUpdate> {
    texts
        .iter()
        .map(|text| SessionUpdate::AgentMessageChunk(ContentChunk::new((*text).into())))
        .collect()
}

#[tokio::test]
async fn streamed_message_is_not_repeated_at_item_or_turn_completion() {
    let item = message("message", "Hello world");
    let updates = collect_updates(vec![
        delta("message", "Hello "),
        delta("message", "world"),
        completed(item.clone()),
        turn_completed(vec![item]),
    ])
    .await;
    assert_eq!(updates, chunks(&["Hello ", "world"]));
}

#[tokio::test]
async fn streamed_message_is_not_repeated_by_turn_fallback() {
    let updates = collect_updates(vec![
        delta("message", "Hello"),
        turn_completed(vec![message("message", "Hello")]),
    ])
    .await;
    assert_eq!(updates, chunks(&["Hello"]));
}

#[tokio::test]
async fn unstreamed_message_is_delivered_once() {
    let item = message("message", "Hello");
    let updates = collect_updates(vec![completed(item.clone()), turn_completed(vec![item])]).await;
    assert_eq!(updates, chunks(&["Hello"]));
}

#[tokio::test]
async fn unstreamed_message_is_delivered_by_turn_fallback() {
    let updates = collect_updates(vec![turn_completed(vec![message("message", "Hello")])]).await;
    assert_eq!(updates, chunks(&["Hello"]));
}

#[tokio::test]
async fn streaming_one_message_does_not_suppress_another() {
    let first = message("first", "First");
    let second = message("second", "Second");
    let updates = collect_updates(vec![
        delta("first", "First"),
        completed(first.clone()),
        completed(second.clone()),
        turn_completed(vec![first, second]),
    ])
    .await;
    assert_eq!(updates, chunks(&["First", "Second"]));
}

#[tokio::test]
async fn foreign_thread_and_turn_deltas_do_not_suppress_completion() {
    let mut other_thread = delta("message", "Wrong thread");
    let ServerNotification::AgentMessageDelta(payload) = &mut other_thread else {
        unreachable!()
    };
    payload.thread_id = "other".to_string();
    let mut other_turn = delta("message", "Wrong turn");
    let ServerNotification::AgentMessageDelta(payload) = &mut other_turn else {
        unreachable!()
    };
    payload.turn_id = "other".to_string();
    let updates = collect_updates(vec![
        other_thread,
        other_turn,
        completed(message("message", "Hello")),
    ])
    .await;
    assert_eq!(updates, chunks(&["Hello"]));
}

#[tokio::test]
async fn empty_delta_does_not_suppress_completed_text() {
    let updates = collect_updates(vec![
        delta("message", ""),
        completed(message("message", "Hello")),
    ])
    .await;
    assert_eq!(updates, chunks(&["", "Hello"]));
}

#[tokio::test]
async fn completion_delivers_only_the_unstreamed_suffix() {
    let item = message("message", "你好，world");
    let updates = collect_updates(vec![
        delta("message", "你好"),
        completed(item.clone()),
        turn_completed(vec![item]),
    ])
    .await;
    assert_eq!(updates, chunks(&["你好", "，world"]));
}

#[tokio::test]
async fn mismatched_stream_keeps_complete_text_fallback() {
    let item = message("message", "Final answer");
    let updates = collect_updates(vec![
        delta("message", "Draft answer"),
        completed(item.clone()),
        turn_completed(vec![item]),
    ])
    .await;
    assert_eq!(updates, chunks(&["Draft answer", "Final answer"]));
}
