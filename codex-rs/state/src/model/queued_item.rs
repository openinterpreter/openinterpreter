use anyhow::Result;
use codex_protocol::ThreadId;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// One durable, ordered user submission for a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedUserSubmissionRecord {
    pub id: String,
    pub thread_id: ThreadId,
    pub payload: String,
}

/// Durable metadata for an agent-contract admission.
///
/// This extends the existing user-message queue rather than introducing a
/// second admission registry. Admission rows are retained after dispatch so a
/// lost response or a history projection gap cannot cause a retry to enqueue
/// the same logical task again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedAdmissionRecord {
    pub id: String,
    pub thread_id: ThreadId,
    pub payload: String,
    pub admission_id: String,
    pub goal_binding_json: String,
    pub state: String,
    pub blocker: Option<String>,
    pub dispatched_at_ms: Option<i64>,
    pub dispatch_attempt_at_ms: Option<i64>,
    pub dispatch_turn_id: Option<String>,
    pub dispatch_mode: String,
    pub expected_turn_id: Option<String>,
    pub terminal_outcome: Option<String>,
    pub terminal_turn_id: Option<String>,
    pub terminal_goal_id: Option<String>,
    pub terminal_mission_revision: Option<i64>,
    pub terminal_lifecycle_revision: Option<i64>,
    pub terminal_objective: Option<String>,
    pub terminal_at_ms: Option<i64>,
}

impl QueuedAdmissionRecord {
    pub(crate) fn try_from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            thread_id: ThreadId::try_from(row.try_get::<String, _>("thread_id")?)?,
            payload: row.try_get("payload_json")?,
            admission_id: row.try_get("admission_id")?,
            goal_binding_json: row.try_get("admission_goal_binding_json")?,
            state: row.try_get("admission_state")?,
            blocker: row.try_get("admission_blocker")?,
            dispatched_at_ms: row.try_get("dispatched_at_ms")?,
            dispatch_attempt_at_ms: row.try_get("dispatch_attempt_at_ms")?,
            dispatch_turn_id: row.try_get("dispatch_turn_id")?,
            dispatch_mode: row.try_get("admission_dispatch_mode")?,
            expected_turn_id: row.try_get("admission_expected_turn_id")?,
            terminal_outcome: row.try_get("admission_terminal_outcome")?,
            terminal_turn_id: row.try_get("admission_terminal_turn_id")?,
            terminal_goal_id: row.try_get("admission_terminal_goal_id")?,
            terminal_mission_revision: row.try_get("admission_terminal_mission_revision")?,
            terminal_lifecycle_revision: row.try_get("admission_terminal_lifecycle_revision")?,
            terminal_objective: row.try_get("admission_terminal_objective")?,
            terminal_at_ms: row.try_get("admission_terminal_at_ms")?,
        })
    }
}

impl QueuedUserSubmissionRecord {
    pub(crate) fn try_from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            thread_id: ThreadId::try_from(row.try_get::<String, _>("thread_id")?)?,
            payload: row.try_get("payload_json")?,
        })
    }
}
