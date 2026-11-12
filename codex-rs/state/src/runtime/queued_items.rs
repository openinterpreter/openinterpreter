use super::*;
use crate::MAX_QUEUE_ITEMS;
use crate::QueuedAdmissionRecord;
use crate::QueuedUserSubmissionRecord;
use crate::sqlite::SqlitePoolWriteExt;
use crate::sqlite::SqliteWriteQueryExt;
use sqlx::Connection;
use tokio::sync::Mutex;
use uuid::Uuid;

/// SQLite-backed persistence for durable, thread-scoped user messages.
#[derive(Clone)]
pub struct SqliteQueueStore {
    pool: Arc<SqlitePool>,
    change_version_connection: Arc<Mutex<Option<SqliteConnection>>>,
}

impl SqliteQueueStore {
    pub(crate) fn new(pool: Arc<SqlitePool>) -> Self {
        Self {
            pool,
            change_version_connection: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) async fn close(&self) {
        let connection = self.change_version_connection.lock().await.take();
        if let Some(connection) = connection
            && let Err(error) = connection.close().await
        {
            tracing::warn!(%error, "failed to close queue change-version connection");
        }
        self.pool.close().await;
    }

    /// Observe queue-database commits through one stable SQLite connection.
    pub async fn change_version(&self) -> anyhow::Result<i64> {
        let mut connection = Arc::clone(&self.change_version_connection)
            .lock_owned()
            .await;
        if connection.is_none() {
            *connection = Some(self.pool.acquire().await?.detach());
        }
        let Some(connection) = connection.as_mut() else {
            unreachable!("queue change-version connection was initialized");
        };
        Ok(sqlx::query_scalar("PRAGMA data_version")
            .fetch_one(connection)
            .await?)
    }

    /// Return changed revisions only for the supplied loaded thread IDs.
    pub async fn changes_since(
        &self,
        revision: i64,
        thread_ids: &[ThreadId],
    ) -> anyhow::Result<Vec<(ThreadId, i64)>> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT thread_id, revision FROM queued_thread_revisions WHERE revision > ",
        );
        query.push_bind(revision).push(" AND thread_id IN (");
        let mut separated = query.separated(", ");
        for thread_id in thread_ids {
            separated.push_bind(thread_id.to_string());
        }
        separated.push_unseparated(") ORDER BY revision");
        let rows = query
            .build_query_as::<(String, i64)>()
            .fetch_all(self.pool.as_ref())
            .await?;
        rows.into_iter()
            .map(|(thread_id, revision)| Ok((ThreadId::try_from(thread_id)?, revision)))
            .collect()
    }

    pub async fn enqueue(
        &self,
        thread_id: ThreadId,
        payload_json: &str,
    ) -> anyhow::Result<QueuedUserSubmissionRecord> {
        let now_ms = datetime_to_epoch_millis(Utc::now());
        let row = sqlx::query(
            "INSERT INTO queued_items (
                id, thread_id, payload_json, queue_order,
                created_at_ms, updated_at_ms
             )
             SELECT ?, ?, ?,
                    COALESCE((SELECT MAX(queue_order) FROM queued_items WHERE thread_id = ?), -1) + 1,
                    ?, ?
             WHERE (SELECT COUNT(*) FROM queued_items
                    WHERE thread_id = ? AND admission_state = 'pending') < ?
             RETURNING id, thread_id, payload_json",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(thread_id.to_string())
        .bind(payload_json)
        .bind(thread_id.to_string())
        .bind(now_ms)
        .bind(now_ms)
        .bind(thread_id.to_string())
        .bind(i64::try_from(MAX_QUEUE_ITEMS)?)
        .fetch_one_write(self.pool.as_ref())
        .await?;
        QueuedUserSubmissionRecord::try_from_row(&row)
    }

    /// Atomically claim a logical agent admission in the existing durable
    /// queue. The partial unique index is intentionally the authority across
    /// processes and restarts; a caller that sees its conflict must reconcile
    /// with [`Self::find_admission`] and must not enqueue a second row.
    pub async fn enqueue_admission(
        &self,
        thread_id: ThreadId,
        payload_json: &str,
        admission_id: &str,
        goal_binding_json: &str,
        dispatch_mode: &str,
        expected_turn_id: Option<&str>,
    ) -> anyhow::Result<QueuedAdmissionRecord> {
        let now_ms = datetime_to_epoch_millis(Utc::now());
        let row = sqlx::query(
            "INSERT INTO queued_items (
                id, thread_id, payload_json, queue_order,
                created_at_ms, updated_at_ms, admission_id,
                admission_goal_binding_json, admission_state,
                admission_dispatch_mode, admission_expected_turn_id
             )
             SELECT ?, ?, ?,
                    COALESCE((SELECT MAX(queue_order) FROM queued_items WHERE thread_id = ?), -1) + 1,
                    ?, ?, ?, ?, 'pending', ?, ?
             WHERE (SELECT COUNT(*) FROM queued_items
                    WHERE thread_id = ? AND admission_state = 'pending') < ?
             RETURNING id, thread_id, payload_json, admission_id,
                       admission_goal_binding_json, admission_state,
                       admission_blocker, dispatched_at_ms,
                       dispatch_attempt_at_ms, dispatch_turn_id,
                       admission_dispatch_mode, admission_expected_turn_id,
                       admission_terminal_outcome, admission_terminal_turn_id,
                       admission_terminal_goal_id, admission_terminal_mission_revision,
                       admission_terminal_lifecycle_revision, admission_terminal_objective,
                       admission_terminal_at_ms",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(thread_id.to_string())
        .bind(payload_json)
        .bind(thread_id.to_string())
        .bind(now_ms)
        .bind(now_ms)
        .bind(admission_id)
        .bind(goal_binding_json)
        .bind(dispatch_mode)
        .bind(expected_turn_id)
        .bind(thread_id.to_string())
        .bind(i64::try_from(MAX_QUEUE_ITEMS)?)
        .fetch_one(self.pool.as_ref())
        .await?;
        QueuedAdmissionRecord::try_from_row(&row)
    }

    pub async fn find_admission(
        &self,
        thread_id: ThreadId,
        admission_id: &str,
    ) -> anyhow::Result<Option<QueuedAdmissionRecord>> {
        let row = sqlx::query(
            "SELECT id, thread_id, payload_json, admission_id,
                    admission_goal_binding_json, admission_state,
                    admission_blocker, dispatched_at_ms,
                    dispatch_attempt_at_ms, dispatch_turn_id,
                    admission_dispatch_mode, admission_expected_turn_id,
                    admission_terminal_outcome, admission_terminal_turn_id,
                    admission_terminal_goal_id, admission_terminal_mission_revision,
                    admission_terminal_lifecycle_revision, admission_terminal_objective,
                    admission_terminal_at_ms
             FROM queued_items
             WHERE thread_id = ? AND admission_id = ?",
        )
        .bind(thread_id.to_string())
        .bind(admission_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        row.as_ref()
            .map(QueuedAdmissionRecord::try_from_row)
            .transpose()
    }

    /// Find the first retained durable admission attached to one exact native
    /// turn. Callers that need to distinguish a task from a control steer
    /// sharing that turn must use [`Self::list_admissions_by_turn`].
    pub async fn find_admission_by_turn(
        &self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> anyhow::Result<Option<QueuedAdmissionRecord>> {
        Ok(self
            .list_admissions_by_turn(thread_id, turn_id)
            .await?
            .into_iter()
            .next())
    }

    /// List every retained admission attached to one exact native turn. A
    /// useful active-turn steer and a Goal task can legitimately share a turn;
    /// completion code must inspect dispatch mode and preserve each admission
    /// identity rather than applying a `LIMIT 1` projection.
    pub async fn list_admissions_by_turn(
        &self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> anyhow::Result<Vec<QueuedAdmissionRecord>> {
        let row = sqlx::query(
            "SELECT id, thread_id, payload_json, admission_id,
                    admission_goal_binding_json, admission_state,
                    admission_blocker, dispatched_at_ms,
                    dispatch_attempt_at_ms, dispatch_turn_id,
                    admission_dispatch_mode, admission_expected_turn_id,
                    admission_terminal_outcome, admission_terminal_turn_id,
                    admission_terminal_goal_id, admission_terminal_mission_revision,
                    admission_terminal_lifecycle_revision, admission_terminal_objective,
                    admission_terminal_at_ms
             FROM queued_items
             WHERE thread_id = ? AND dispatch_turn_id = ?
               AND admission_state = 'dispatched'
             ORDER BY queue_order, created_at_ms, id",
        )
        .bind(thread_id.to_string())
        .bind(turn_id)
        .fetch_all(self.pool.as_ref())
        .await?;
        row.iter()
            .map(QueuedAdmissionRecord::try_from_row)
            .collect()
    }

    /// List unfinished dispatched admissions in durable queue order. This is
    /// used only at the native Goal-continuation completion boundary: the
    /// validator must still prove the current Goal belongs to exactly one
    /// retained admission before any row can be completed.
    pub async fn list_dispatched_admissions(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<Vec<QueuedAdmissionRecord>> {
        let rows = sqlx::query(
            "SELECT id, thread_id, payload_json, admission_id,
                    admission_goal_binding_json, admission_state,
                    admission_blocker, dispatched_at_ms,
                    dispatch_attempt_at_ms, dispatch_turn_id,
                    admission_dispatch_mode, admission_expected_turn_id,
                    admission_terminal_outcome, admission_terminal_turn_id,
                    admission_terminal_goal_id, admission_terminal_mission_revision,
                    admission_terminal_lifecycle_revision, admission_terminal_objective,
                    admission_terminal_at_ms
             FROM queued_items
             WHERE thread_id = ? AND admission_state = 'dispatched'
               AND admission_terminal_outcome IS NULL
             ORDER BY queue_order, created_at_ms, id",
        )
        .bind(thread_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;
        rows.iter()
            .map(QueuedAdmissionRecord::try_from_row)
            .collect()
    }

    pub async fn mark_admission_dispatched(
        &self,
        thread_id: ThreadId,
        item_id: &str,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE queued_items
             SET admission_state = 'dispatched', dispatched_at_ms = ?, updated_at_ms = ?
             WHERE thread_id = ? AND id = ? AND admission_id IS NOT NULL
               AND admission_state = 'starting'",
        )
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(item_id)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn mark_admission_starting(
        &self,
        thread_id: ThreadId,
        item_id: &str,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE queued_items
             SET admission_state = 'starting', dispatch_attempt_at_ms = ?, updated_at_ms = ?
             WHERE thread_id = ? AND id = ? AND admission_id IS NOT NULL
               AND admission_state = 'pending'",
        )
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(item_id)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn mark_admission_started(
        &self,
        thread_id: ThreadId,
        item_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE queued_items
             SET admission_state = 'dispatched', dispatch_turn_id = ?,
                 dispatched_at_ms = ?, updated_at_ms = ?
             WHERE thread_id = ? AND id = ? AND admission_id IS NOT NULL
               AND admission_state = 'starting'",
        )
        .bind(turn_id)
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(item_id)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected()
            > 0)
    }

    /// Persist the exact Goal/turn proof before a successful task is reported
    /// as completed. The update is one-shot; later Goal replacement cannot
    /// erase this historical proof, and a mismatched dispatch turn is never
    /// promoted to completion.
    pub async fn mark_admission_completed(
        &self,
        thread_id: ThreadId,
        admission_id: &str,
        turn_id: &str,
        goal_id: &str,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: &str,
    ) -> anyhow::Result<Option<QueuedAdmissionRecord>> {
        let row = sqlx::query(
            "UPDATE queued_items
             SET admission_terminal_outcome = 'completed',
                 admission_terminal_turn_id = ?,
                 admission_terminal_goal_id = ?,
                 admission_terminal_mission_revision = ?,
                 admission_terminal_lifecycle_revision = ?,
                 admission_terminal_objective = ?,
                 admission_terminal_at_ms = ?,
                 updated_at_ms = ?
             WHERE thread_id = ? AND admission_id = ?
               AND admission_state = 'dispatched'
               AND dispatch_turn_id = ?
               AND admission_terminal_outcome IS NULL
             RETURNING id, thread_id, payload_json, admission_id,
                       admission_goal_binding_json, admission_state,
                       admission_blocker, dispatched_at_ms,
                       dispatch_attempt_at_ms, dispatch_turn_id,
                       admission_dispatch_mode, admission_expected_turn_id,
                       admission_terminal_outcome, admission_terminal_turn_id,
                       admission_terminal_goal_id, admission_terminal_mission_revision,
                       admission_terminal_lifecycle_revision, admission_terminal_objective,
                       admission_terminal_at_ms",
        )
        .bind(turn_id)
        .bind(goal_id)
        .bind(mission_revision)
        .bind(lifecycle_revision)
        .bind(objective)
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(admission_id)
        .bind(turn_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        match row {
            Some(row) => Ok(Some(QueuedAdmissionRecord::try_from_row(&row)?)),
            None => self.find_admission(thread_id, admission_id).await,
        }
    }

    /// Persist a completion reached by a native Goal continuation turn. The
    /// admission was dispatched by an earlier turn, so the completing turn is
    /// recorded as terminal lineage without changing the immutable dispatch
    /// turn. The queue service only calls this after an exact current-Goal
    /// proof selected one retained admission at the native idle boundary.
    pub async fn mark_admission_completed_from_continuation(
        &self,
        thread_id: ThreadId,
        admission_id: &str,
        completion_turn_id: &str,
        goal_id: &str,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: &str,
    ) -> anyhow::Result<Option<QueuedAdmissionRecord>> {
        let row = sqlx::query(
            "UPDATE queued_items
             SET admission_terminal_outcome = 'completed',
                 admission_terminal_turn_id = ?,
                 admission_terminal_goal_id = ?,
                 admission_terminal_mission_revision = ?,
                 admission_terminal_lifecycle_revision = ?,
                 admission_terminal_objective = ?,
                 admission_terminal_at_ms = ?,
                 updated_at_ms = ?
             WHERE thread_id = ? AND admission_id = ?
               AND admission_state = 'dispatched'
               AND admission_terminal_outcome IS NULL
             RETURNING id, thread_id, payload_json, admission_id,
                       admission_goal_binding_json, admission_state,
                       admission_blocker, dispatched_at_ms,
                       dispatch_attempt_at_ms, dispatch_turn_id,
                       admission_dispatch_mode, admission_expected_turn_id,
                       admission_terminal_outcome, admission_terminal_turn_id,
                       admission_terminal_goal_id, admission_terminal_mission_revision,
                       admission_terminal_lifecycle_revision, admission_terminal_objective,
                       admission_terminal_at_ms",
        )
        .bind(completion_turn_id)
        .bind(goal_id)
        .bind(mission_revision)
        .bind(lifecycle_revision)
        .bind(objective)
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(admission_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        match row {
            Some(row) => Ok(Some(QueuedAdmissionRecord::try_from_row(&row)?)),
            None => self.find_admission(thread_id, admission_id).await,
        }
    }

    pub async fn reset_admission_pending(
        &self,
        thread_id: ThreadId,
        item_id: &str,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE queued_items
             SET admission_state = 'pending', dispatch_attempt_at_ms = NULL,
                 updated_at_ms = ?
             WHERE thread_id = ? AND id = ? AND admission_id IS NOT NULL
               AND admission_state = 'starting'",
        )
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(item_id)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn mark_admission_blocked(
        &self,
        thread_id: ThreadId,
        item_id: &str,
        blocker: &str,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query(
            "UPDATE queued_items
             SET admission_state = 'blocked', admission_blocker = ?, updated_at_ms = ?
             WHERE thread_id = ? AND id = ? AND admission_id IS NOT NULL
               AND admission_state IN ('pending', 'starting')",
        )
        .bind(blocker)
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(item_id)
        .execute(self.pool.as_ref())
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn list_page(
        &self,
        thread_id: ThreadId,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<Vec<QueuedUserSubmissionRecord>> {
        let rows = sqlx::query(
            "SELECT id, thread_id, payload_json
             FROM queued_items
             WHERE thread_id = ? AND admission_state = 'pending'
               AND admission_dispatch_mode = 'queue'
             ORDER BY queue_order LIMIT ? OFFSET ?",
        )
        .bind(thread_id.to_string())
        .bind(i64::try_from(limit)?)
        .bind(i64::try_from(offset)?)
        .fetch_all(self.pool.as_ref())
        .await?;
        rows.iter()
            .map(QueuedUserSubmissionRecord::try_from_row)
            .collect()
    }

    pub async fn update(
        &self,
        thread_id: ThreadId,
        item_id: &str,
        payload_json: &str,
    ) -> anyhow::Result<Option<QueuedUserSubmissionRecord>> {
        let row = sqlx::query(
            "UPDATE queued_items
             SET payload_json = ?, updated_at_ms = ?
             WHERE thread_id = ? AND id = ? AND admission_state = 'pending'
             RETURNING id, thread_id, payload_json",
        )
        .bind(payload_json)
        .bind(datetime_to_epoch_millis(Utc::now()))
        .bind(thread_id.to_string())
        .bind(item_id)
        .fetch_optional_write(self.pool.as_ref())
        .await?;
        row.as_ref()
            .map(QueuedUserSubmissionRecord::try_from_row)
            .transpose()
    }

    pub async fn delete(&self, thread_id: ThreadId, item_id: &str) -> anyhow::Result<bool> {
        Ok(
            sqlx::query("DELETE FROM queued_items WHERE thread_id = ? AND id = ?")
                .bind(thread_id.to_string())
                .bind(item_id)
                .execute_with_busy_retry(self.pool.as_ref())
                .await?
                .rows_affected()
                > 0,
        )
    }

    pub async fn reorder(&self, thread_id: ThreadId, ordered_ids: &[String]) -> anyhow::Result<()> {
        let mut transaction = self.pool.begin_immediate().await?;
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT id, queue_order FROM queued_items
             WHERE thread_id = ? AND admission_state = 'pending'
               AND admission_dispatch_mode = 'queue' ORDER BY queue_order",
        )
        .bind(thread_id.to_string())
        .fetch_all(transaction.as_mut())
        .await?;
        let mut expected_ids = rows.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let mut requested_ids = ordered_ids.to_vec();
        expected_ids.sort();
        requested_ids.sort();
        if expected_ids != requested_ids {
            transaction.rollback().await?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "queue reorder must include every queued submission exactly once",
            )
            .into());
        }
        let now_ms = datetime_to_epoch_millis(Utc::now());
        let max_queue_order = rows.last().map_or(-1, |(_, queue_order)| *queue_order);
        for (index, item_id) in ordered_ids.iter().enumerate() {
            sqlx::query(
                "UPDATE queued_items SET queue_order = ?, updated_at_ms = ?
                 WHERE thread_id = ? AND id = ?",
            )
            .bind(max_queue_order + i64::try_from(index)? + 1)
            .bind(now_ms)
            .bind(thread_id.to_string())
            .bind(item_id)
            .execute(transaction.as_mut())
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn delete_thread_queue(&self, thread_id: ThreadId) -> anyhow::Result<bool> {
        Ok(sqlx::query("DELETE FROM queued_items WHERE thread_id = ?")
            .bind(thread_id.to_string())
            .execute_with_busy_retry(self.pool.as_ref())
            .await?
            .rows_affected()
            > 0)
    }
}

#[cfg(test)]
#[path = "queued_items_tests.rs"]
mod tests;
