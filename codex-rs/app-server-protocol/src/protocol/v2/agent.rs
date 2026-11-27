//! Content-free, caller-bound fleet-agent coordination primitives.
//!
//! These methods deliberately build on the app-server's existing durable thread,
//! goal, queue, and native-terminal records.  They are not a second process
//! broker or a replacement for `thread/goal/set` and `thread/queue/add`.

use super::ThreadGoal;
use super::ThreadGoalStatus;
use super::UserInput;
use crate::JsonSchema;
use crate::TS;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;

/// Version of the content-free native caller-policy contract.
pub const AGENT_CALLER_POLICY_CONTRACT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum AgentInventoryScope {
    /// The caller and every persisted descendant in its agent tree.
    Tree,
    /// Only direct persisted children of the caller.
    Children,
    /// The caller and every persisted descendant in its subtree.
    Subtree,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum AgentInventoryVisibility {
    /// Include known persisted identities even when their runtime is unloaded.
    Known,
    /// Include only identities with a currently loaded runtime.
    Loaded,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum AgentInventoryAvailability {
    Available,
    Unavailable,
    NotFound,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentRuntimeResidency {
    Loaded,
    Known,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentRuntimeStatus {
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentInventoryListParams {
    /// The caller/owner thread.  Scope is resolved relative to this identity.
    pub thread_id: String,
    #[ts(optional = nullable)]
    pub scope: Option<AgentInventoryScope>,
    #[ts(optional = nullable)]
    pub visibility: Option<AgentInventoryVisibility>,
    /// Opaque cursor returned by a previous call.
    #[ts(optional = nullable)]
    pub cursor: Option<String>,
    #[ts(optional = nullable)]
    pub limit: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentInventoryEntry {
    pub agent_id: String,
    pub parent_thread_id: Option<String>,
    pub agent_path: Option<String>,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub residency: AgentRuntimeResidency,
    /// Runtime status is intentionally content-free.  Completed/errored text is
    /// never included in this inventory.
    pub status: AgentRuntimeStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentInventoryListResponse {
    pub availability: AgentInventoryAvailability,
    pub data: Vec<AgentInventoryEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum AgentGoalAvailability {
    Available,
    Unavailable,
    NotFound,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentGoalUnavailableReason {
    GoalsDisabled,
    StateUnavailable,
    EphemeralThread,
    CallerNotOwner,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentGoalReadParams {
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentGoalReadResponse {
    pub availability: AgentGoalAvailability,
    pub goal: Option<ThreadGoal>,
    /// Stable identity of the native Goal row.
    pub goal_id: Option<String>,
    /// Stable mission identity revision; objective and budget edits advance it,
    /// while status-only lifecycle progress does not.
    pub mission_revision: Option<i64>,
    /// Lifecycle revision; usage accounting does not advance it.
    pub lifecycle_revision: Option<i64>,
    /// A paused or blocked goal must be explicitly resumed through
    /// `thread/goal/set`; this endpoint never reactivates it.
    pub explicit_resume_required: bool,
    pub unavailable_reason: Option<AgentGoalUnavailableReason>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentTaskOutcome {
    Queued,
    Accepted,
    Running,
    Completed,
    Failed,
    Unknown,
    Unavailable,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentTaskObservation {
    Durable,
    Queue,
    Unavailable,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentTaskAdmissionState {
    NeverSeen,
    Queued,
    Dispatched,
    Blocked,
    Unknown,
    Unavailable,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentTaskTurnState {
    NotStarted,
    Running,
    Completed,
    Failed,
    Interrupted,
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentTaskBlocker {
    GoalUnavailable,
    GoalNotFound,
    GoalNotActive,
    GoalChanged,
    ThreadUnavailable,
    QueueUnavailable,
}

/// The caller's fresh, full native-goal readback that authorizes one new
/// substantive admission.  The server never fills this from an older goal and
/// never resumes a paused, blocked, or complete goal as a side effect.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentTaskGoalBinding {
    pub goal_id: String,
    pub mission_revision: i64,
    pub lifecycle_revision: i64,
    pub objective: String,
    pub status: ThreadGoalStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentTaskAdmitParams {
    pub thread_id: String,
    /// Caller-owned idempotency key.  It is persisted as the user-message
    /// client id and is the only retry identity for this admission.
    pub admission_id: String,
    /// Fresh full-goal precondition.  A caller must read the exact goal and
    /// explicitly activate it through `thread/goal/set` before admission.
    pub goal: AgentTaskGoalBinding,
    /// One durable active-turn steer, never also queued for a later turn.
    #[ts(optional = nullable)]
    pub expected_turn_id: Option<String>,
    pub input: Vec<UserInput>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentTaskAdmitResponse {
    pub admission_id: String,
    pub outcome: AgentTaskOutcome,
    pub observation: AgentTaskObservation,
    pub queue_id: Option<String>,
    pub turn_id: Option<String>,
    /// True only when the exact admission id was found in the durable queue or
    /// persisted thread history.  A lost submission remains `unknown`.
    pub acknowledged: bool,
    pub requires_reconciliation: bool,
    pub blocker: Option<AgentTaskBlocker>,
    /// Admission, turn, and Goal-task lifecycle are reported separately.
    pub admission_state: AgentTaskAdmissionState,
    pub turn_state: AgentTaskTurnState,
    pub goal_status: Option<ThreadGoalStatus>,
    /// Queue custody is not supervisor acceptance. These fields remain empty
    /// until the existing supervisor/wake layer returns a correlated receipt.
    pub supervisor_acknowledged: bool,
    pub supervisor_ack_id: Option<String>,
    pub supervisor_next_actor: Option<String>,
    pub supervisor_wake_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentTaskReceiptReadParams {
    pub thread_id: String,
    pub admission_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentTaskReceiptReadResponse {
    pub admission_id: String,
    pub outcome: AgentTaskOutcome,
    pub observation: AgentTaskObservation,
    pub queue_id: Option<String>,
    pub turn_id: Option<String>,
    pub acknowledged: bool,
    pub requires_reconciliation: bool,
    pub blocker: Option<AgentTaskBlocker>,
    pub admission_state: AgentTaskAdmissionState,
    pub turn_state: AgentTaskTurnState,
    pub goal_status: Option<ThreadGoalStatus>,
    pub supervisor_acknowledged: bool,
    pub supervisor_ack_id: Option<String>,
    pub supervisor_next_actor: Option<String>,
    pub supervisor_wake_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum AgentSessionAvailability {
    Available,
    Unavailable,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum AgentSessionPhase {
    Running,
    Exited,
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentSessionStatusParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    /// Opaque server-issued native identity.  It is not the client command or
    /// exec `processId` namespace.
    pub native_process_id: String,
    #[ts(optional = nullable)]
    pub max_wait_ms: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentSessionStatusResponse {
    pub availability: AgentSessionAvailability,
    pub phase: AgentSessionPhase,
    pub output_bytes: u64,
    pub output_sha256: Option<String>,
    pub output_cap_reached: bool,
    pub exit_code: Option<i32>,
    pub terminal_generation: Option<u64>,
    pub observation_timed_out: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum AgentCallerPolicyAvailability {
    Available,
    Unavailable,
    NotFound,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum AgentCallerPolicyUnavailableReason {
    CallerNotOwner,
    ThreadNotLoaded,
    ThreadActive,
    PendingIncompatibleWork,
    StaleGeneration,
    RuntimeUnsupported,
    StateUnavailable,
}

/// Presence-only projection for the native caller carrier. Values are never
/// returned, hashed individually, or included in this response.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentCallerPolicyKeyPresence {
    pub interpreter_cli_path: bool,
    pub interpreter_cli_server_connection: bool,
    pub interpreter_caller_token: bool,
    pub path: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentCallerPolicyReadParams {
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentCallerPolicyReadResponse {
    pub contract_version: u32,
    pub thread_id: String,
    pub availability: AgentCallerPolicyAvailability,
    pub key_presence: Option<AgentCallerPolicyKeyPresence>,
    /// Opaque digest used only for a subsequent CAS repair.
    pub policy_generation: Option<String>,
    /// Stable opaque owner identity for this exact thread. The caller must
    /// still be subscribed to the thread; this is not a bearer credential.
    pub owner_identity: Option<String>,
    pub unavailable_reason: Option<AgentCallerPolicyUnavailableReason>,
}

/// Values are accepted only in a repair request and are never returned. The
/// custom Debug implementation prevents credentials from entering diagnostics.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentCallerPolicyRepairValues {
    #[ts(optional = nullable)]
    pub interpreter_cli_path: Option<String>,
    #[ts(optional = nullable)]
    pub interpreter_cli_server_connection: Option<String>,
    #[ts(optional = nullable)]
    pub interpreter_caller_token: Option<String>,
    #[ts(optional = nullable)]
    pub path: Option<String>,
}

impl fmt::Debug for AgentCallerPolicyRepairValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentCallerPolicyRepairValues")
            .field(
                "interpreter_cli_path",
                &self.interpreter_cli_path.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "interpreter_cli_server_connection",
                &self
                    .interpreter_cli_server_connection
                    .as_ref()
                    .map(|_| "<redacted>"),
            )
            .field(
                "interpreter_caller_token",
                &self.interpreter_caller_token.as_ref().map(|_| "<redacted>"),
            )
            .field("path", &self.path.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentCallerPolicyRepairParams {
    pub thread_id: String,
    pub expected_policy_generation: String,
    pub expected_owner_identity: String,
    pub values: AgentCallerPolicyRepairValues,
}

impl fmt::Debug for AgentCallerPolicyRepairParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentCallerPolicyRepairParams")
            .field("thread_id", &self.thread_id)
            .field(
                "expected_policy_generation",
                &self.expected_policy_generation,
            )
            .field("expected_owner_identity", &self.expected_owner_identity)
            .field("values", &"<redacted>")
            .finish()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentCallerPolicyRepairResponse {
    pub contract_version: u32,
    pub thread_id: String,
    pub availability: AgentCallerPolicyAvailability,
    pub repaired: bool,
    pub key_presence: Option<AgentCallerPolicyKeyPresence>,
    pub policy_generation: Option<String>,
    pub owner_identity: Option<String>,
    pub unavailable_reason: Option<AgentCallerPolicyUnavailableReason>,
}

#[cfg(test)]
mod caller_policy_contract_tests {
    use super::*;

    #[test]
    fn repair_debug_redacts_values_but_wire_request_can_carry_them() {
        let params = AgentCallerPolicyRepairParams {
            thread_id: "thread-1".to_string(),
            expected_policy_generation: "sha256:old".to_string(),
            expected_owner_identity: "thread-1".to_string(),
            values: AgentCallerPolicyRepairValues {
                interpreter_cli_path: Some("/private/interpreter".to_string()),
                interpreter_cli_server_connection: Some("private-connection".to_string()),
                interpreter_caller_token: Some("private-token".to_string()),
                path: Some("/private/path".to_string()),
            },
        };

        let debug = format!("{params:?}");
        assert!(!debug.contains("/private/interpreter"));
        assert!(!debug.contains("private-connection"));
        assert!(!debug.contains("private-token"));
        assert!(!debug.contains("/private/path"));

        let wire = serde_json::to_value(&params).expect("repair request serializes");
        assert_eq!(
            wire["values"]["interpreterCliServerConnection"],
            "private-connection"
        );
        assert_eq!(wire["values"]["interpreterCallerToken"], "private-token");

        let response = AgentCallerPolicyRepairResponse {
            contract_version: AGENT_CALLER_POLICY_CONTRACT_VERSION,
            thread_id: "thread-1".to_string(),
            availability: AgentCallerPolicyAvailability::Available,
            repaired: true,
            key_presence: Some(AgentCallerPolicyKeyPresence {
                interpreter_cli_path: true,
                interpreter_cli_server_connection: true,
                interpreter_caller_token: true,
                path: true,
            }),
            policy_generation: Some("sha256:new".to_string()),
            owner_identity: Some("thread-1".to_string()),
            unavailable_reason: None,
        };
        let response_wire = serde_json::to_string(&response).expect("repair response serializes");
        assert!(!response_wire.contains("private-connection"));
        assert!(!response_wire.contains("private-token"));
        assert!(!response_wire.contains("/private/interpreter"));
    }
}
