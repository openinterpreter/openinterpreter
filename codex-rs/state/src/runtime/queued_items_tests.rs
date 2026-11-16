use super::*;
use crate::migrations::QUEUE_MIGRATOR;
use crate::runtime::test_support::test_thread_metadata;
use crate::runtime::test_support::unique_temp_dir;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;
use sqlx::migrate::Migrator;
use std::borrow::Cow;

async fn runtime_with_thread() -> (Arc<StateRuntime>, ThreadId) {
    let home = unique_temp_dir();
    let runtime = StateRuntime::init(
        crate::SqliteConfig::new_for_testing(home.as_path().abs()),
        "test-provider".to_string(),
    )
    .await
    .expect("state runtime");
    let thread_id = ThreadId::new();
    let metadata = test_thread_metadata(home.as_path(), thread_id, home.clone());
    runtime.upsert_thread(&metadata).await.unwrap();
    (runtime, thread_id)
}

#[tokio::test]
async fn competing_runtimes_preserve_fifo_queue_order() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let other = StateRuntime::init(runtime.sqlite().clone(), "test-provider".to_string())
        .await
        .unwrap();
    let queue = runtime.thread_queue();
    let other_queue = other.thread_queue();
    let (first, second) = tokio::join!(
        queue.enqueue(thread_id, r#"{"first":true}"#),
        other_queue.enqueue(thread_id, r#"{"second":true}"#),
    );
    let mut expected = vec![first.unwrap(), second.unwrap()];
    expected.sort_by(|first, second| first.id.cmp(&second.id));
    let mut actual = queue
        .list_page(thread_id, /*offset*/ 0, /*limit*/ 2)
        .await
        .unwrap();
    actual.sort_by(|first, second| first.id.cmp(&second.id));
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn admission_claim_is_unique_across_runtimes_and_survives_dispatch() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let other = StateRuntime::init(runtime.sqlite().clone(), "test-provider".to_string())
        .await
        .unwrap();
    let admission_id = "same-admission";
    let (first, second) = tokio::join!(
        runtime.thread_queue().enqueue_admission(
            thread_id,
            r#"{"first":true}"#,
            admission_id,
            r#"{"objective":"job","status":"active","updated_at":1}"#,
            "queue",
            None,
        ),
        other.thread_queue().enqueue_admission(
            thread_id,
            r#"{"second":true}"#,
            admission_id,
            r#"{"objective":"job","status":"active","updated_at":1}"#,
            "queue",
            None,
        ),
    );
    assert_eq!(1, usize::from(first.is_ok()) + usize::from(second.is_ok()));

    let receipt = runtime
        .thread_queue()
        .find_admission(thread_id, admission_id)
        .await
        .unwrap()
        .expect("the unique admission must remain readable");
    assert_eq!(admission_id, receipt.admission_id);
    assert_eq!("pending", receipt.state);

    // The dispatch claim is persisted before Core is called. Simulate a
    // process crash in that exact window and reopen through the other runtime:
    // the row is retained as recoverable evidence and is not visible to the
    // pending dispatcher.
    assert!(
        runtime
            .thread_queue()
            .mark_admission_starting(thread_id, &receipt.id)
            .await
            .unwrap()
    );
    let starting = other
        .thread_queue()
        .find_admission(thread_id, admission_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!("starting", starting.state);
    assert!(
        runtime
            .thread_queue()
            .list_page(thread_id, 0, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let dispatched = runtime
        .thread_queue()
        .find_admission(thread_id, admission_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!("starting", dispatched.state);
    assert!(
        runtime
            .thread_queue()
            .mark_admission_started(thread_id, &receipt.id, "native-turn")
            .await
            .unwrap()
    );
    assert_eq!(
        Some("native-turn".to_string()),
        runtime
            .thread_queue()
            .find_admission(thread_id, admission_id)
            .await
            .unwrap()
            .unwrap()
            .dispatch_turn_id
    );
    assert!(
        !runtime
            .thread_queue()
            .mark_admission_starting(thread_id, &receipt.id)
            .await
            .unwrap()
    );
    assert!(
        runtime
            .thread_queue()
            .enqueue_admission(
                thread_id,
                r#"{"retry":true}"#,
                admission_id,
                r#"{"objective":"other","status":"active","updated_at":2}"#,
                "queue",
                None,
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn terminal_completion_proof_survives_reopen_and_cannot_be_borrowed_by_goal_b() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let other = StateRuntime::init(runtime.sqlite().clone(), "test-provider".to_string())
        .await
        .unwrap();
    let admission = runtime
        .thread_queue()
        .enqueue_admission(
            thread_id,
            r#"{"task":"A"}"#,
            "task-a",
            r#"{"goal_id":"goal-a","mission_revision":1,"lifecycle_revision":1,"objective":"mission A","status":"active"}"#,
            "queue",
            None,
        )
        .await
        .unwrap();
    assert!(
        runtime
            .thread_queue()
            .mark_admission_starting(thread_id, &admission.id)
            .await
            .unwrap()
    );
    assert!(
        runtime
            .thread_queue()
            .mark_admission_started(thread_id, &admission.id, "turn-a")
            .await
            .unwrap()
    );

    let completed = runtime
        .thread_queue()
        .mark_admission_completed(thread_id, "task-a", "turn-a", "goal-a", 1, 2, "mission A")
        .await
        .unwrap()
        .expect("completion proof should be persisted");
    assert_eq!(Some("completed".to_string()), completed.terminal_outcome);
    assert_eq!(Some("turn-a".to_string()), completed.terminal_turn_id);
    assert_eq!(Some("goal-a".to_string()), completed.terminal_goal_id);
    assert_eq!(Some(1), completed.terminal_mission_revision);
    assert_eq!(Some(2), completed.terminal_lifecycle_revision);

    // A reconnect/restart reads the same row, and a later Goal B cannot
    // overwrite the historical proof or make it look like B completed A.
    let reopened = other
        .thread_queue()
        .find_admission(thread_id, "task-a")
        .await
        .unwrap()
        .expect("terminal proof should survive another runtime");
    assert_eq!(completed, reopened);
    let second = other
        .thread_queue()
        .mark_admission_completed(thread_id, "task-a", "turn-b", "goal-b", 2, 1, "mission B")
        .await
        .unwrap()
        .expect("existing proof should remain readable");
    assert_eq!(Some("turn-a".to_string()), second.terminal_turn_id);
    assert_eq!(Some("goal-a".to_string()), second.terminal_goal_id);
    assert_eq!(Some(1), second.terminal_mission_revision);
}

#[tokio::test]
async fn continuation_completion_retains_dispatch_and_completion_turn_lineage() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let admission = runtime
        .thread_queue()
        .enqueue_admission(
            thread_id,
            r#"{"task":"A"}"#,
            "task-continuation",
            r#"{"goal_id":"goal-a","mission_revision":1,"objective":"mission A","status":"active"}"#,
            "queue",
            None,
        )
        .await
        .unwrap();
    assert!(
        runtime
            .thread_queue()
            .mark_admission_starting(thread_id, &admission.id)
            .await
            .unwrap()
    );
    assert!(
        runtime
            .thread_queue()
            .mark_admission_started(thread_id, &admission.id, "turn-a")
            .await
            .unwrap()
    );

    let completed = runtime
        .thread_queue()
        .mark_admission_completed_from_continuation(
            thread_id,
            "task-continuation",
            "turn-a-continuation",
            "goal-a",
            1,
            2,
            "mission A",
        )
        .await
        .unwrap()
        .expect("continuation completion should be persisted");
    assert_eq!(Some("turn-a".to_string()), completed.dispatch_turn_id);
    assert_eq!(
        Some("turn-a-continuation".to_string()),
        completed.terminal_turn_id
    );

    let later_goal = runtime
        .thread_queue()
        .mark_admission_completed_from_continuation(
            thread_id,
            "task-continuation",
            "turn-b-continuation",
            "goal-b",
            2,
            1,
            "mission B",
        )
        .await
        .unwrap()
        .expect("existing terminal row should remain readable");
    assert_eq!(completed, later_goal);
}

#[tokio::test]
async fn migrating_existing_queue_backfills_thread_revisions() {
    let home = unique_temp_dir();
    tokio::fs::create_dir_all(&home).await.unwrap();
    let sqlite = crate::SqliteConfig::new_for_testing(home.as_path().abs());
    let queue_path = sqlite.queue_db_path();
    let old_queue_migrator = Migrator {
        migrations: Cow::Owned(vec![QUEUE_MIGRATOR.migrations[0].clone()]),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: QUEUE_MIGRATOR.table_name.clone(),
        create_schemas: QUEUE_MIGRATOR.create_schemas.clone(),
    };
    let pool = sqlite.open_read_write_pool(&queue_path).await.unwrap();
    old_queue_migrator.run(&pool).await.unwrap();

    let thread_id = ThreadId::new();
    let queued = QueuedUserSubmissionRecord {
        id: Uuid::now_v7().to_string(),
        thread_id,
        payload: r#"{"existing":true}"#.to_string(),
    };
    sqlx::query(
        "INSERT INTO queued_items
         (id, thread_id, payload_json, queue_order, created_at_ms, updated_at_ms)
         VALUES (?, ?, ?, 0, 0, 0)",
    )
    .bind(&queued.id)
    .bind(thread_id.to_string())
    .bind(&queued.payload)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let runtime = StateRuntime::init(sqlite, "test-provider".to_string())
        .await
        .unwrap();
    let queue = runtime.thread_queue();
    assert_eq!(
        vec![(thread_id, 1)],
        queue
            .changes_since(/*revision*/ 0, &[thread_id])
            .await
            .unwrap()
    );
    assert_eq!(
        vec![queued],
        queue
            .list_page(thread_id, /*offset*/ 0, /*limit*/ 1)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn queue_revisions_identify_changed_threads_after_updates_and_deletions() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let queue = runtime.thread_queue();
    let first = queue.enqueue(thread_id, r#"{"first":true}"#).await.unwrap();
    let first_revision = queue
        .changes_since(/*revision*/ 0, &[thread_id])
        .await
        .unwrap()[0]
        .1;
    queue
        .update(thread_id, &first.id, r#"{"updated":true}"#)
        .await
        .unwrap();
    let updated_revision = queue
        .changes_since(first_revision, &[thread_id])
        .await
        .unwrap()[0]
        .1;
    let other_thread_id = ThreadId::new();
    queue
        .enqueue(other_thread_id, r#"{"other":true}"#)
        .await
        .unwrap();
    let newly_loaded_changes = queue
        .changes_since(/*revision*/ 0, &[other_thread_id])
        .await
        .unwrap();
    assert_eq!(
        vec![(thread_id, updated_revision), newly_loaded_changes[0]],
        queue
            .changes_since(first_revision, &[thread_id, other_thread_id])
            .await
            .unwrap()
    );
    assert!(queue.delete(thread_id, &first.id).await.unwrap());
    assert!(
        queue
            .changes_since(updated_revision, &[thread_id])
            .await
            .unwrap()
            .iter()
            .any(|(changed_thread, _)| *changed_thread == thread_id)
    );
}

#[tokio::test]
async fn fifo_dispatch_preserves_edits_reordering_and_pagination() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let queue = runtime.thread_queue();
    let first = queue.enqueue(thread_id, r#"{"n":1}"#).await.unwrap();
    let second = queue.enqueue(thread_id, r#"{"n":2}"#).await.unwrap();
    let third = queue.enqueue(thread_id, r#"{"n":3}"#).await.unwrap();

    let updated = queue
        .update(thread_id, &first.id, r#"{"n":"edited"}"#)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.id, updated.id);
    let error = queue
        .reorder(thread_id, std::slice::from_ref(&first.id))
        .await
        .unwrap_err();
    assert_eq!(
        std::io::ErrorKind::InvalidInput,
        error.downcast_ref::<std::io::Error>().unwrap().kind()
    );

    let ordered_ids = vec![third.id, first.id, second.id];
    queue.reorder(thread_id, &ordered_ids).await.unwrap();

    let items = queue
        .list_page(thread_id, /*offset*/ 0, /*limit*/ 3)
        .await
        .unwrap();
    let page = queue
        .list_page(thread_id, /*offset*/ 1, /*limit*/ 1)
        .await
        .unwrap();
    assert_eq!(vec![items[1].clone()], page);
    assert_eq!(r#"{"n":"edited"}"#, items[1].payload);

    for item in items {
        assert!(queue.delete(thread_id, &item.id).await.unwrap());
    }
    assert!(
        queue
            .list_page(thread_id, /*offset*/ 0, /*limit*/ 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn queue_operations_cannot_mutate_another_threads_messages() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let queue = runtime.thread_queue();
    let first = queue.enqueue(thread_id, r#"{"n":1}"#).await.unwrap();
    let other_thread_id = ThreadId::new();
    let other = queue.enqueue(other_thread_id, r#"{"n":2}"#).await.unwrap();
    let other_id = &other.id;

    assert_eq!(
        None,
        queue
            .update(thread_id, other_id, r#"{"n":3}"#)
            .await
            .unwrap()
    );
    assert!(!queue.delete(thread_id, other_id).await.unwrap());
    assert!(
        queue
            .reorder(thread_id, std::slice::from_ref(other_id))
            .await
            .is_err()
    );
    let (items, other_items) = tokio::join!(
        queue.list_page(thread_id, /*offset*/ 0, /*limit*/ 1),
        queue.list_page(other_thread_id, /*offset*/ 0, /*limit*/ 1),
    );
    assert_eq!(
        (vec![first], vec![other]),
        (items.unwrap(), other_items.unwrap())
    );
}

#[tokio::test]
async fn deleting_a_thread_removes_its_queue() {
    let (runtime, thread_id) = runtime_with_thread().await;
    runtime
        .thread_queue()
        .enqueue(thread_id, r#"{"n":1}"#)
        .await
        .unwrap();

    assert_eq!(1, runtime.delete_thread(thread_id).await.unwrap());
    assert!(
        runtime
            .thread_queue()
            .list_page(thread_id, /*offset*/ 0, /*limit*/ 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn concurrent_inserts_enforce_the_queue_limit() {
    let (runtime, thread_id) = runtime_with_thread().await;
    let other = StateRuntime::init(runtime.sqlite().clone(), "test-provider".to_string())
        .await
        .unwrap();

    for _ in 0..MAX_QUEUE_ITEMS - 1 {
        runtime
            .thread_queue()
            .enqueue(thread_id, r#"{"n":1}"#)
            .await
            .unwrap();
    }
    let (first, second) = tokio::join!(
        runtime.thread_queue().enqueue(thread_id, r#"{"n":2}"#),
        other.thread_queue().enqueue(thread_id, r#"{"n":3}"#),
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        MAX_QUEUE_ITEMS,
        runtime
            .thread_queue()
            .list_page(thread_id, /*offset*/ 0, /*limit*/ MAX_QUEUE_ITEMS)
            .await
            .unwrap()
            .len()
    );
}
