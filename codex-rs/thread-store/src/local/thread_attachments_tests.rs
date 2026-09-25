//! Coverage for unloaded attachment operations and thread deletion exclusion.

use super::LocalThreadStore;
use crate::AddThreadAttachmentOutcome;
use crate::AddThreadAttachmentParams;
use crate::DeleteThreadParams;
use crate::DeleteThreadsParams;
use crate::InMemoryThreadStore;
use crate::ListThreadAttachmentsParams;
use crate::RemoveThreadAttachmentOutcome;
use crate::RemoveThreadAttachmentParams;
use crate::ThreadAttachmentPage;
use crate::ThreadStore;
use crate::ThreadStoreError;
use crate::local::test_support::test_config;
use crate::local::test_support::write_session_file;
use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

#[tokio::test]
async fn thread_attachments_require_a_supported_sqlite_store() {
    let thread_id = ThreadId::new();
    let default_store = InMemoryThreadStore::default();
    assert!(!default_store.supports_thread_attachments());
    assert!(matches!(
        default_store
            .add_thread_attachment(AddThreadAttachmentParams {
                thread_id,
                attachment_type: "pull_request".to_string(),
                identity_key: "openai/codex#1".to_string(),
                payload: json!({"url": "https://github.com/openai/codex/pull/1"}),
            })
            .await,
        Err(ThreadStoreError::Unsupported {
            operation: "thread/attachment/add"
        })
    ));

    let home = tempfile::TempDir::new().expect("temp dir");
    let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
    assert!(!store.supports_thread_attachments());
    assert!(matches!(
        store
            .list_thread_attachments(ListThreadAttachmentsParams {
                thread_id,
                cursor: None,
                limit: 10,
            })
            .await,
        Err(ThreadStoreError::Unsupported {
            operation: "thread/attachment/list"
        })
    ));
    assert!(matches!(
        store
            .remove_thread_attachment(RemoveThreadAttachmentParams {
                thread_id,
                attachment_type: "pull_request".to_string(),
                identity_key: "openai/codex#1".to_string(),
            })
            .await,
        Err(ThreadStoreError::Unsupported {
            operation: "thread/attachment/remove"
        })
    ));
}

#[tokio::test]
async fn unloaded_thread_attachments_support_listing_and_removal() {
    let home = tempfile::TempDir::new().expect("temp dir");
    let config = test_config(home.path());
    let runtime = codex_state::StateRuntime::init(
        config.sqlite.clone(),
        config.default_model_provider_id.clone(),
    )
    .await
    .expect("state db should initialize");
    let store = LocalThreadStore::new(config.clone(), Some(runtime.clone()));
    assert!(store.supports_thread_attachments());

    let first_thread_id = ThreadId::new();
    let second_thread_id = ThreadId::new();
    for thread_id in [first_thread_id, second_thread_id] {
        let metadata = codex_state::ThreadMetadataBuilder::new(
            thread_id,
            home.path().join(format!("{thread_id}.jsonl")),
            Utc::now(),
            SessionSource::Cli,
        )
        .build(&config.default_model_provider_id);
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("persist unloaded thread metadata");
    }

    let params = AddThreadAttachmentParams {
        thread_id: first_thread_id,
        attachment_type: "pull_request".to_string(),
        identity_key: "openai/codex#1".to_string(),
        payload: json!({"url": "https://github.com/openai/codex/pull/1"}),
    };
    let AddThreadAttachmentOutcome::Created(first_attachment) = store
        .add_thread_attachment(params.clone())
        .await
        .expect("attach attachment to unloaded persisted thread")
    else {
        panic!("first explicit attachment should create an attachment");
    };
    assert_eq!(
        store
            .add_thread_attachment(params.clone())
            .await
            .expect("repeat attachment attachment"),
        AddThreadAttachmentOutcome::Existing(first_attachment.clone())
    );

    let second_params = AddThreadAttachmentParams {
        thread_id: first_thread_id,
        identity_key: "openai/codex#2".to_string(),
        payload: json!({"url": "https://github.com/openai/codex/pull/2"}),
        ..params.clone()
    };
    let AddThreadAttachmentOutcome::Created(second_attachment) = store
        .add_thread_attachment(second_params)
        .await
        .expect("attach second thread attachment")
    else {
        panic!("second thread attachment should create an attachment");
    };

    let first_page = store
        .list_thread_attachments(ListThreadAttachmentsParams {
            thread_id: first_thread_id,
            cursor: None,
            limit: 1,
        })
        .await
        .expect("list first page");
    assert_eq!(first_page.attachments.len(), 1);
    let second_page = store
        .list_thread_attachments(ListThreadAttachmentsParams {
            thread_id: first_thread_id,
            cursor: first_page.next_cursor,
            limit: 1,
        })
        .await
        .expect("list second page");
    assert_eq!(second_page.attachments.len(), 1);
    assert_eq!(second_page.next_cursor, None);
    let listed_ids = [
        first_page.attachments[0].id.clone(),
        second_page.attachments[0].id.clone(),
    ];
    assert!(listed_ids.contains(&first_attachment.id));
    assert!(listed_ids.contains(&second_attachment.id));

    let removal_params = RemoveThreadAttachmentParams {
        thread_id: first_thread_id,
        attachment_type: params.attachment_type.clone(),
        identity_key: params.identity_key.clone(),
    };
    assert_eq!(
        store
            .remove_thread_attachment(removal_params.clone())
            .await
            .expect("remove existing attachment"),
        RemoveThreadAttachmentOutcome::Removed(first_attachment)
    );
    assert_eq!(
        store
            .remove_thread_attachment(removal_params)
            .await
            .expect("repeat attachment removal"),
        RemoveThreadAttachmentOutcome::NotFound
    );
    assert!(matches!(
        store
            .add_thread_attachment(params)
            .await
            .expect("reattach removed attachment"),
        AddThreadAttachmentOutcome::Created(_)
    ));
}

#[tokio::test]
async fn attachment_state_errors_preserve_missing_thread_and_invalid_request_categories() {
    let home = tempfile::TempDir::new().expect("temp dir");
    let config = test_config(home.path());
    let runtime = codex_state::StateRuntime::init(
        config.sqlite.clone(),
        config.default_model_provider_id.clone(),
    )
    .await
    .expect("state db should initialize");
    let store = LocalThreadStore::new(config, Some(runtime));
    let thread_id = ThreadId::new();

    assert!(matches!(
        store
            .add_thread_attachment(AddThreadAttachmentParams {
                thread_id,
                attachment_type: "pull_request".to_string(),
                identity_key: "openai/codex#1".to_string(),
                payload: json!({}),
            })
            .await,
        Err(ThreadStoreError::ThreadNotFound { thread_id: missing }) if missing == thread_id
    ));
    assert!(matches!(
        store
            .add_thread_attachment(AddThreadAttachmentParams {
                thread_id,
                attachment_type: " ".to_string(),
                identity_key: "openai/codex#1".to_string(),
                payload: json!({}),
            })
            .await,
        Err(ThreadStoreError::InvalidRequest { message })
            if message == "attachment type must not be empty"
    ));
    assert!(matches!(
        store
            .list_thread_attachments(ListThreadAttachmentsParams {
                thread_id,
                cursor: None,
                limit: 0,
            })
            .await,
        Err(ThreadStoreError::InvalidRequest { message })
            if message == "page limit must be between 1 and 100"
    ));
}

#[tokio::test]
async fn attachment_mutations_wait_for_exclusive_thread_lifecycle_operations() {
    let home = tempfile::TempDir::new().expect("temp dir");
    let config = test_config(home.path());
    let runtime = codex_state::StateRuntime::init(
        config.sqlite.clone(),
        config.default_model_provider_id.clone(),
    )
    .await
    .expect("state db should initialize");
    let store = LocalThreadStore::new(config.clone(), Some(runtime.clone()));
    let thread_id = ThreadId::new();
    let metadata = codex_state::ThreadMetadataBuilder::new(
        thread_id,
        home.path().join(format!("{thread_id}.jsonl")),
        Utc::now(),
        SessionSource::Cli,
    )
    .build(&config.default_model_provider_id);
    runtime
        .upsert_thread(&metadata)
        .await
        .expect("persist attachment owner");

    let lifecycle_guard = store.live_writer_locks.lock_lifecycle(thread_id).await;
    let mut attachment = Box::pin(store.add_thread_attachment(AddThreadAttachmentParams {
        thread_id,
        attachment_type: "pull_request".to_string(),
        identity_key: "openai/codex#1".to_string(),
        payload: json!({"url": "https://github.com/openai/codex/pull/1"}),
    }));
    tokio::select! {
        biased;
        result = &mut attachment => panic!("attachment attached during lifecycle mutation: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    drop(lifecycle_guard);
    assert!(matches!(
        attachment.await,
        Ok(AddThreadAttachmentOutcome::Created(_))
    ));

    let lifecycle_guard = store.live_writer_locks.lock_lifecycle(thread_id).await;
    let mut removal = Box::pin(
        store.remove_thread_attachment(RemoveThreadAttachmentParams {
            thread_id,
            attachment_type: "pull_request".to_string(),
            identity_key: "openai/codex#1".to_string(),
        }),
    );
    tokio::select! {
        biased;
        result = &mut removal => panic!("attachment removed during lifecycle mutation: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    drop(lifecycle_guard);
    assert!(matches!(
        removal.await,
        Ok(RemoveThreadAttachmentOutcome::Removed(_))
    ));
}

#[tokio::test]
async fn attachment_mutations_queued_behind_deletion_reject_deleted_threads() {
    enum DeleteMode {
        Single,
        Batch,
    }

    for mode in [DeleteMode::Single, DeleteMode::Batch] {
        let home = tempfile::TempDir::new().expect("temp dir");
        let thread_id = ThreadId::new();
        let thread_ids = match mode {
            DeleteMode::Single => vec![thread_id],
            DeleteMode::Batch => vec![thread_id, ThreadId::new()],
        };
        let (store, _) = store_with_attachment_owners(home.path(), &thread_ids).await;
        let runtime = store.state_db().await.expect("state db");

        let reservation = store.live_writer_locks.reserve_lifecycle(thread_id).await;
        let mut deletion = match mode {
            DeleteMode::Single => store.delete_thread(DeleteThreadParams { thread_id }),
            DeleteMode::Batch => store.delete_threads(DeleteThreadsParams {
                thread_ids: thread_ids.clone(),
            }),
        };
        assert!(futures::poll!(&mut deletion).is_pending());

        // The queued exclusive delete must precede subsequent shared attachment reservations.
        let mut attachment = store.add_thread_attachment(AddThreadAttachmentParams {
            thread_id,
            attachment_type: "pull_request".to_string(),
            identity_key: "openai/codex#2".to_string(),
            payload: json!({"url": "https://github.com/openai/codex/pull/2"}),
        });
        let mut removal = store.remove_thread_attachment(RemoveThreadAttachmentParams {
            thread_id,
            attachment_type: "pull_request".to_string(),
            identity_key: "openai/codex#1".to_string(),
        });
        assert!(futures::poll!(&mut attachment).is_pending());
        assert!(futures::poll!(&mut removal).is_pending());
        drop(reservation);

        let (deletion, attachment, removal) = tokio::join!(deletion, attachment, removal);
        deletion.expect("delete attachment owners");
        assert!(
            matches!(attachment, Err(ThreadStoreError::ThreadNotFound { thread_id: missing }) if missing == thread_id),
            "attachment resumed after deletion: {attachment:?}"
        );
        assert!(
            matches!(removal, Err(ThreadStoreError::ThreadNotFound { thread_id: missing }) if missing == thread_id),
            "removal resumed after deletion: {removal:?}"
        );
        for &owner_id in &thread_ids {
            assert_eq!(
                runtime
                    .get_thread(owner_id)
                    .await
                    .expect("read deleted owner"),
                None
            );
        }
        for &owner_id in &thread_ids {
            assert_eq!(
                store
                    .list_thread_attachments(ListThreadAttachmentsParams {
                        thread_id: owner_id,
                        cursor: None,
                        limit: 10,
                    })
                    .await
                    .expect("list attachments after deletion"),
                ThreadAttachmentPage {
                    attachments: Vec::new(),
                    next_cursor: None,
                }
            );
        }
    }
}

#[tokio::test]
async fn attachment_owner_deletion_can_retry_after_state_cleanup_fails() {
    let home = tempfile::TempDir::new().expect("temp dir");
    let parent_id = ThreadId::new();
    let child_id = ThreadId::new();
    let (store, rollout_paths) =
        store_with_attachment_owners(home.path(), &[parent_id, child_id]).await;
    let runtime = store.state_db().await.expect("state db");
    runtime
        .upsert_thread_spawn_edge(
            parent_id,
            child_id,
            codex_state::DirectionalThreadSpawnEdgeStatus::Closed,
        )
        .await
        .expect("persist child relation for deletion retry");
    let pool = runtime
        .sqlite()
        .open_read_write_pool(&runtime.sqlite().state_db_path())
        .await
        .expect("open state db for failure injection");
    sqlx::query(
        "CREATE TRIGGER fail_thread_delete BEFORE DELETE ON threads BEGIN SELECT RAISE(ABORT, 'state cleanup failed'); END",
    )
    .execute(&pool)
    .await
    .expect("inject state cleanup failure");
    let params = DeleteThreadsParams {
        thread_ids: vec![child_id, parent_id],
    };
    let error = store
        .delete_threads(params.clone())
        .await
        .expect_err("state cleanup failure must fail the delete");
    assert!(matches!(error, ThreadStoreError::Internal { .. }));
    assert!(rollout_paths.iter().all(|path| !path.exists()));
    assert_eq!(
        runtime
            .list_thread_spawn_descendants(parent_id)
            .await
            .expect("read child relation after failed deletion"),
        vec![child_id]
    );

    sqlx::query("DROP TRIGGER fail_thread_delete")
        .execute(&pool)
        .await
        .expect("restore state cleanup");
    store
        .delete_thread(DeleteThreadParams {
            thread_id: child_id,
        })
        .await
        .expect("single-thread retry must remove the owner even without a rollout");
    store
        .delete_threads(params.clone())
        .await
        .expect("batch retry must tolerate missing rollouts and already-deleted members");
    for &owner_id in &params.thread_ids {
        assert_eq!(
            store
                .list_thread_attachments(ListThreadAttachmentsParams {
                    thread_id: owner_id,
                    cursor: None,
                    limit: 10,
                })
                .await
                .expect("list attachments after deletion retry"),
            ThreadAttachmentPage {
                attachments: Vec::new(),
                next_cursor: None,
            }
        );
    }
    assert_eq!(
        runtime
            .delete_threads_strict(&params.thread_ids)
            .await
            .expect("repeated app-server state cleanup is idempotent"),
        0
    );
}

async fn store_with_attachment_owners(
    home: &Path,
    thread_ids: &[ThreadId],
) -> (LocalThreadStore, Vec<PathBuf>) {
    let config = test_config(home);
    let runtime = codex_state::StateRuntime::init(
        config.sqlite.clone(),
        config.default_model_provider_id.clone(),
    )
    .await
    .expect("state db should initialize");
    let store = LocalThreadStore::new(config.clone(), Some(runtime.clone()));
    let mut rollout_paths = Vec::new();
    for &thread_id in thread_ids {
        let rollout_path = write_session_file(
            home,
            "2025-01-03T12-00-00",
            Uuid::parse_str(&thread_id.to_string()).expect("thread UUID"),
        )
        .expect("session file");
        let metadata = codex_state::ThreadMetadataBuilder::new(
            thread_id,
            rollout_path.clone(),
            Utc::now(),
            SessionSource::Cli,
        )
        .build(&config.default_model_provider_id);
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("persist attachment owner");
        store
            .add_thread_attachment(AddThreadAttachmentParams {
                thread_id,
                attachment_type: "pull_request".to_string(),
                identity_key: "openai/codex#1".to_string(),
                payload: json!({"url": "https://github.com/openai/codex/pull/1"}),
            })
            .await
            .expect("attach attachment before deleting its owner");
        rollout_paths.push(rollout_path);
    }
    (store, rollout_paths)
}
