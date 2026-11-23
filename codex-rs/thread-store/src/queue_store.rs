use std::fmt::Display;
use std::future::Future;

use codex_protocol::ThreadId;
use codex_rollout::StateDbHandle;
use codex_state::QueuedAdmissionRecord;
use codex_state::QueuedUserSubmissionRecord;
use codex_state::SqliteQueueStore;

use crate::MAX_QUEUE_ITEMS;
use crate::ThreadStoreError;
use crate::ThreadStoreFuture;

/// Storage-neutral persistence for ordered, thread-scoped user messages.
pub trait QueueStore: Send + Sync {
    /// Return a stable revision that changes when another connection updates the queue.
    fn change_version(&self) -> ThreadStoreFuture<'_, i64>;

    /// Return changed, loaded thread IDs and their durable revisions after `revision`.
    fn changes_since<'a>(
        &'a self,
        revision: i64,
        thread_ids: &'a [ThreadId],
    ) -> ThreadStoreFuture<'a, Vec<(ThreadId, i64)>>;

    fn enqueue(
        &self,
        thread_id: ThreadId,
        payload: String,
    ) -> ThreadStoreFuture<'_, QueuedUserSubmissionRecord>;

    /// Persist one agent-contract admission with its idempotency key and
    /// caller-supplied goal precondition. The unique (thread, admission_id)
    /// index is the cross-process claim; callers must reconcile a conflict
    /// through [`Self::find_admission`] rather than enqueue again.
    fn enqueue_admission(
        &self,
        thread_id: ThreadId,
        payload: String,
        admission_id: String,
        goal_binding_json: String,
        dispatch_mode: String,
        expected_turn_id: Option<String>,
    ) -> ThreadStoreFuture<'_, QueuedAdmissionRecord>;

    fn find_admission(
        &self,
        thread_id: ThreadId,
        admission_id: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>>;

    /// Find the durable admission retained for one exact native turn.
    fn find_admission_by_turn(
        &self,
        thread_id: ThreadId,
        turn_id: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>>;

    /// List all admissions attached to one native turn so task and control
    /// steer identities are not collapsed by a first-row lookup.
    fn list_admissions_by_turn(
        &self,
        thread_id: ThreadId,
        turn_id: String,
    ) -> ThreadStoreFuture<'_, Vec<QueuedAdmissionRecord>>;

    /// List unfinished dispatched admissions for a native Goal-continuation
    /// completion lookup. Callers must prove one current Goal binding before
    /// marking any row terminal.
    fn list_dispatched_admissions(
        &self,
        thread_id: ThreadId,
    ) -> ThreadStoreFuture<'_, Vec<QueuedAdmissionRecord>>;

    /// Mark an admission as dispatched, retaining the row as the durable
    /// receipt. Returns false when it was already terminally marked.
    fn mark_admission_dispatched(
        &self,
        thread_id: ThreadId,
        item_id: String,
    ) -> ThreadStoreFuture<'_, bool>;

    /// Claim the durable dispatch transition before invoking Core. A
    /// `starting` row is recoverable evidence and is never returned to the
    /// pending dispatcher after a crash.
    fn mark_admission_starting(
        &self,
        thread_id: ThreadId,
        item_id: String,
    ) -> ThreadStoreFuture<'_, bool>;

    /// Attach the native turn identity after Core accepts a claimed dispatch.
    fn mark_admission_started(
        &self,
        thread_id: ThreadId,
        item_id: String,
        turn_id: String,
    ) -> ThreadStoreFuture<'_, bool>;

    /// Persist a one-shot terminal completion proof bound to the exact native
    /// turn and mission identity. A conflict returns the retained admission
    /// row for reconciliation rather than creating another effect.
    fn mark_admission_completed(
        &self,
        thread_id: ThreadId,
        admission_id: String,
        turn_id: String,
        goal_id: String,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>>;

    /// Persist terminal lineage when a later native Goal continuation turn
    /// completes an admission dispatched by an earlier turn.
    fn mark_admission_completed_from_continuation(
        &self,
        thread_id: ThreadId,
        admission_id: String,
        completion_turn_id: String,
        goal_id: String,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>>;

    /// Return a claim to pending only when Core explicitly reports that no
    /// submission was made. Unknown storage/core errors keep `starting`.
    fn reset_admission_pending(
        &self,
        thread_id: ThreadId,
        item_id: String,
    ) -> ThreadStoreFuture<'_, bool>;

    /// Fail closed at queue dispatch when the persisted Goal precondition no
    /// longer matches. The row remains as a receipt and is removed from the
    /// pending dispatch view.
    fn mark_admission_blocked(
        &self,
        thread_id: ThreadId,
        item_id: String,
        blocker: String,
    ) -> ThreadStoreFuture<'_, bool>;

    fn list_page(
        &self,
        thread_id: ThreadId,
        offset: usize,
        limit: usize,
    ) -> ThreadStoreFuture<'_, Vec<QueuedUserSubmissionRecord>>;

    fn update(
        &self,
        thread_id: ThreadId,
        item_id: String,
        payload: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedUserSubmissionRecord>>;

    fn delete(&self, thread_id: ThreadId, item_id: String) -> ThreadStoreFuture<'_, bool>;

    /// Atomically replace queue order with every current item ID exactly once.
    ///
    /// Returns [`ThreadStoreError::InvalidRequest`] when `item_ids` is not a
    /// permutation of the complete queue.
    fn reorder(&self, thread_id: ThreadId, item_ids: Vec<String>) -> ThreadStoreFuture<'_, ()>;
}

/// Adapts the local state runtime to the shared queue-storage interface.
#[derive(Clone)]
pub struct LocalQueueStore {
    state_db: StateDbHandle,
}

impl LocalQueueStore {
    pub fn new(state_db: StateDbHandle) -> Self {
        Self { state_db }
    }

    fn queue(&self) -> &SqliteQueueStore {
        self.state_db.thread_queue()
    }
}

fn queue_future<'a, T, E>(
    future: impl Future<Output = Result<T, E>> + Send + 'a,
) -> ThreadStoreFuture<'a, T>
where
    T: Send + 'a,
    E: Display + Send + 'a,
{
    Box::pin(async move {
        future.await.map_err(|error| ThreadStoreError::Internal {
            message: format!("queue storage failed: {error}"),
        })
    })
}

impl QueueStore for LocalQueueStore {
    fn change_version(&self) -> ThreadStoreFuture<'_, i64> {
        queue_future(self.queue().change_version())
    }

    fn changes_since<'a>(
        &'a self,
        revision: i64,
        thread_ids: &'a [ThreadId],
    ) -> ThreadStoreFuture<'a, Vec<(ThreadId, i64)>> {
        queue_future(self.queue().changes_since(revision, thread_ids))
    }

    fn enqueue(
        &self,
        thread_id: ThreadId,
        payload: String,
    ) -> ThreadStoreFuture<'_, QueuedUserSubmissionRecord> {
        Box::pin(async move {
            self.queue()
                .enqueue(thread_id, &payload)
                .await
                .map_err(|error| match error.downcast_ref::<sqlx::Error>() {
                    Some(sqlx::Error::RowNotFound) => ThreadStoreError::InvalidRequest {
                        message: format!(
                            "queue cannot contain more than {MAX_QUEUE_ITEMS} submissions"
                        ),
                    },
                    _ => ThreadStoreError::Internal {
                        message: format!("queue storage failed: {error}"),
                    },
                })
        })
    }

    fn enqueue_admission(
        &self,
        thread_id: ThreadId,
        payload: String,
        admission_id: String,
        goal_binding_json: String,
        dispatch_mode: String,
        expected_turn_id: Option<String>,
    ) -> ThreadStoreFuture<'_, QueuedAdmissionRecord> {
        Box::pin(async move {
            self.queue()
                .enqueue_admission(
                    thread_id,
                    &payload,
                    &admission_id,
                    &goal_binding_json,
                    &dispatch_mode,
                    expected_turn_id.as_deref(),
                )
                .await
                .map_err(|error| {
                    if matches!(error.downcast_ref::<sqlx::Error>(), Some(sqlx::Error::Database(db)) if db.message().contains("queued_items_thread_admission_idx")) {
                        ThreadStoreError::Conflict {
                            message: "agent admission already exists".to_string(),
                        }
                    } else {
                        ThreadStoreError::Internal {
                            message: format!("queue storage failed: {error}"),
                        }
                    }
                })
        })
    }

    fn find_admission(
        &self,
        thread_id: ThreadId,
        admission_id: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>> {
        Box::pin(async move {
            self.queue()
                .find_admission(thread_id, &admission_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn find_admission_by_turn(
        &self,
        thread_id: ThreadId,
        turn_id: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>> {
        Box::pin(async move {
            self.queue()
                .find_admission_by_turn(thread_id, &turn_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn list_admissions_by_turn(
        &self,
        thread_id: ThreadId,
        turn_id: String,
    ) -> ThreadStoreFuture<'_, Vec<QueuedAdmissionRecord>> {
        Box::pin(async move {
            self.queue()
                .list_admissions_by_turn(thread_id, &turn_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn list_dispatched_admissions(
        &self,
        thread_id: ThreadId,
    ) -> ThreadStoreFuture<'_, Vec<QueuedAdmissionRecord>> {
        Box::pin(async move {
            self.queue()
                .list_dispatched_admissions(thread_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn mark_admission_dispatched(
        &self,
        thread_id: ThreadId,
        item_id: String,
    ) -> ThreadStoreFuture<'_, bool> {
        Box::pin(async move {
            self.queue()
                .mark_admission_dispatched(thread_id, &item_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn mark_admission_starting(
        &self,
        thread_id: ThreadId,
        item_id: String,
    ) -> ThreadStoreFuture<'_, bool> {
        Box::pin(async move {
            self.queue()
                .mark_admission_starting(thread_id, &item_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn mark_admission_started(
        &self,
        thread_id: ThreadId,
        item_id: String,
        turn_id: String,
    ) -> ThreadStoreFuture<'_, bool> {
        Box::pin(async move {
            self.queue()
                .mark_admission_started(thread_id, &item_id, &turn_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn mark_admission_completed(
        &self,
        thread_id: ThreadId,
        admission_id: String,
        turn_id: String,
        goal_id: String,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>> {
        Box::pin(async move {
            self.queue()
                .mark_admission_completed(
                    thread_id,
                    &admission_id,
                    &turn_id,
                    &goal_id,
                    mission_revision,
                    lifecycle_revision,
                    &objective,
                )
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn mark_admission_completed_from_continuation(
        &self,
        thread_id: ThreadId,
        admission_id: String,
        completion_turn_id: String,
        goal_id: String,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedAdmissionRecord>> {
        Box::pin(async move {
            self.queue()
                .mark_admission_completed_from_continuation(
                    thread_id,
                    &admission_id,
                    &completion_turn_id,
                    &goal_id,
                    mission_revision,
                    lifecycle_revision,
                    &objective,
                )
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn reset_admission_pending(
        &self,
        thread_id: ThreadId,
        item_id: String,
    ) -> ThreadStoreFuture<'_, bool> {
        Box::pin(async move {
            self.queue()
                .reset_admission_pending(thread_id, &item_id)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn mark_admission_blocked(
        &self,
        thread_id: ThreadId,
        item_id: String,
        blocker: String,
    ) -> ThreadStoreFuture<'_, bool> {
        Box::pin(async move {
            self.queue()
                .mark_admission_blocked(thread_id, &item_id, &blocker)
                .await
                .map_err(|error| ThreadStoreError::Internal {
                    message: format!("queue storage failed: {error}"),
                })
        })
    }

    fn list_page(
        &self,
        thread_id: ThreadId,
        offset: usize,
        limit: usize,
    ) -> ThreadStoreFuture<'_, Vec<QueuedUserSubmissionRecord>> {
        queue_future(self.queue().list_page(thread_id, offset, limit))
    }

    fn update(
        &self,
        thread_id: ThreadId,
        item_id: String,
        payload: String,
    ) -> ThreadStoreFuture<'_, Option<QueuedUserSubmissionRecord>> {
        queue_future(async move { self.queue().update(thread_id, &item_id, &payload).await })
    }

    fn delete(&self, thread_id: ThreadId, item_id: String) -> ThreadStoreFuture<'_, bool> {
        queue_future(async move { self.queue().delete(thread_id, &item_id).await })
    }

    fn reorder(&self, thread_id: ThreadId, item_ids: Vec<String>) -> ThreadStoreFuture<'_, ()> {
        Box::pin(async move {
            self.queue()
                .reorder(thread_id, &item_ids)
                .await
                .map_err(|error| match error.downcast_ref::<std::io::Error>() {
                    Some(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                        ThreadStoreError::InvalidRequest {
                            message: error.to_string(),
                        }
                    }
                    _ => ThreadStoreError::Internal {
                        message: format!("queue storage failed: {error}"),
                    },
                })
        })
    }
}
