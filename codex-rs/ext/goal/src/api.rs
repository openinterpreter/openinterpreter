use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::Weak;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use codex_protocol::ThreadId;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_protocol::protocol::ThreadGoalUpdatedEvent;
use codex_protocol::protocol::validate_thread_goal_objective;
use codex_rollout::RolloutItem;
use sha2::Digest;
use sha2::Sha256;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::runtime::GoalRuntimeHandle;
use crate::runtime::PreviousGoalSnapshot;
use crate::tool::fill_empty_thread_preview_if_possible;
use crate::tool::protocol_goal_from_state;
use crate::tool::state_status_from_protocol;
use crate::tool::validate_goal_budget;

static MANAGED_GOAL_DOCUMENT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalServiceError {
    InvalidRequest(String),
    Conflict(String),
    Internal(String),
}

impl fmt::Display for GoalServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) | Self::Conflict(message) | Self::Internal(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for GoalServiceError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalObjectiveUpdate<'a> {
    Keep,
    Set(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalTokenBudgetUpdate {
    Keep,
    Set(Option<i64>),
}

#[derive(Clone, Copy, Debug)]
pub struct GoalSetRequest<'a> {
    pub thread_id: ThreadId,
    pub objective: GoalObjectiveUpdate<'a>,
    pub status: Option<ThreadGoalStatus>,
    pub token_budget: GoalTokenBudgetUpdate,
    pub max_goal_token_budget: Option<i64>,
}

/// Complete identity snapshot required for an external lifecycle mutation.
/// Accounting updates intentionally do not advance either revision, but a
/// status mutation must still match both revisions to avoid stale writers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoalCas {
    pub goal_id: String,
    pub status: ThreadGoalStatus,
    pub mission_revision: i64,
    pub lifecycle_revision: i64,
}

/// The single managed-document creation contract.  `Absent` is accepted only
/// with the explicit native `noGoal` admission state; an unavailable, denied,
/// timed-out, or unloaded read is never converted to this variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoalDocumentCreateAdmissionState {
    NoGoal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoalDocumentCreatePrecondition {
    Absent {
        admission_state: GoalDocumentCreateAdmissionState,
    },
    Completed(GoalCas),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoalIdentity {
    pub goal_id: String,
    pub status: ThreadGoalStatus,
    pub mission_revision: i64,
    pub lifecycle_revision: i64,
}

impl From<&codex_state::ThreadGoal> for GoalIdentity {
    fn from(goal: &codex_state::ThreadGoal) -> Self {
        Self {
            goal_id: goal.goal_id.clone(),
            status: match goal.status {
                codex_state::ThreadGoalStatus::Active => ThreadGoalStatus::Active,
                codex_state::ThreadGoalStatus::Paused => ThreadGoalStatus::Paused,
                codex_state::ThreadGoalStatus::Blocked => ThreadGoalStatus::Blocked,
                codex_state::ThreadGoalStatus::UsageLimited => ThreadGoalStatus::UsageLimited,
                codex_state::ThreadGoalStatus::BudgetLimited => ThreadGoalStatus::BudgetLimited,
                codex_state::ThreadGoalStatus::Complete => ThreadGoalStatus::Complete,
            },
            mission_revision: goal.mission_revision,
            lifecycle_revision: goal.lifecycle_revision,
        }
    }
}

pub const MAX_MANAGED_GOAL_DOCUMENT_BYTES: usize = 1024 * 1024;
const MANAGED_GOAL_DOCUMENT_DIRECTORY: &str = "managed-goals";

pub struct GoalDocumentSetRequest<'a> {
    pub thread_id: ThreadId,
    pub document: &'a str,
    pub status: Option<ThreadGoalStatus>,
    pub expected_goal: Option<GoalCas>,
}

/// Starts a fresh file-backed mission after an exact completed predecessor.
/// `creation_id` is stable across response-loss retries; it is not a Goal id
/// and is never reused for a different request.
pub struct GoalDocumentCreateRequest<'a> {
    pub thread_id: ThreadId,
    pub creation_id: &'a str,
    pub document: &'a str,
    pub objective: &'a str,
    pub status: Option<ThreadGoalStatus>,
    pub token_budget: Option<i64>,
    pub max_goal_token_budget: Option<i64>,
    pub precondition: GoalDocumentCreatePrecondition,
}

pub struct GoalDocumentSetOutcome {
    goal: GoalSetOutcome,
    document: codex_state::ThreadGoalDocument,
}

pub struct GoalDocumentCreateOutcome {
    goal: GoalSetOutcome,
    document: codex_state::ThreadGoalDocument,
    previous_identity: Option<GoalIdentity>,
    creation_id: String,
    replayed: bool,
}

impl GoalDocumentCreateOutcome {
    pub fn goal(&self) -> ThreadGoal {
        self.goal.goal.clone()
    }

    pub fn document(&self) -> codex_state::ThreadGoalDocument {
        self.document.clone()
    }

    pub fn identity(&self) -> GoalIdentity {
        self.goal.identity()
    }

    pub fn previous_identity(&self) -> Option<GoalIdentity> {
        self.previous_identity.clone()
    }

    pub fn creation_id(&self) -> &str {
        self.creation_id.as_str()
    }

    pub fn replayed(&self) -> bool {
        self.replayed
    }

    pub fn thread_goal_updated_item(&self) -> RolloutItem {
        self.goal.thread_goal_updated_item()
    }

    pub async fn apply_runtime_effects(&self, goal_service: &GoalService) {
        self.goal.apply_runtime_effects(goal_service).await;
    }
}

impl GoalDocumentSetOutcome {
    pub fn goal(&self) -> ThreadGoal {
        self.goal.goal.clone()
    }

    pub fn document(&self) -> codex_state::ThreadGoalDocument {
        self.document.clone()
    }

    pub fn identity(&self) -> GoalIdentity {
        self.goal.identity()
    }

    pub fn thread_goal_updated_item(&self) -> RolloutItem {
        self.goal.thread_goal_updated_item()
    }

    pub async fn apply_runtime_effects(&self, goal_service: &GoalService) {
        self.goal.apply_runtime_effects(goal_service).await;
    }
}

#[derive(Clone, Debug)]
pub struct GoalSetOutcome {
    pub goal: ThreadGoal,
    state_goal: codex_state::ThreadGoal,
    previous_goal: Option<PreviousGoalSnapshot>,
}

/// Receives the authoritative Goal transition before a later Goal replacement
/// can hide a completed mission from a durable task receipt. Implementations
/// must persist only exact task/mission matches and may fail closed when the
/// completing native turn is unavailable.
pub type GoalCompletionObserverFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait GoalCompletionObserver: Send + Sync {
    fn on_goal_completed<'a>(
        &'a self,
        goal: &'a codex_state::ThreadGoal,
        completing_turn_id: Option<String>,
    ) -> GoalCompletionObserverFuture<'a>;
}

impl GoalSetOutcome {
    pub fn identity(&self) -> GoalIdentity {
        GoalIdentity::from(&self.state_goal)
    }

    pub fn thread_goal_updated_item(&self) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::ThreadGoalUpdated(ThreadGoalUpdatedEvent {
            thread_id: self.goal.thread_id,
            turn_id: None,
            goal: self.goal.clone(),
        }))
    }

    pub async fn apply_runtime_effects(&self, goal_service: &GoalService) {
        if let Some(runtime) = goal_service.runtime_for_thread(self.goal.thread_id)
            && let Err(err) = runtime
                .apply_external_goal_set(self.state_goal.clone(), self.previous_goal.clone())
                .await
        {
            tracing::warn!("failed to apply external goal status runtime effects: {err}");
        }
    }
}

#[derive(Default)]
pub struct GoalService {
    runtimes: Mutex<HashMap<String, Weak<GoalRuntimeHandle>>>,
    completion_observer: Mutex<Option<Arc<dyn GoalCompletionObserver>>>,
}

impl fmt::Debug for GoalService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoalService")
            .field("runtimes", &self.runtimes)
            .field(
                "completion_observer",
                &self
                    .completion_observer
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_some(),
            )
            .finish()
    }
}

impl GoalService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_completion_observer(&self, observer: Arc<dyn GoalCompletionObserver>) {
        *self
            .completion_observer
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(observer);
    }

    /// Restores persisted goal state into the registered runtime for `thread_id`.
    pub async fn restore_thread_runtime_after_resume(
        &self,
        thread_id: ThreadId,
    ) -> Result<(), GoalServiceError> {
        let runtime = self.runtime_for_thread(thread_id).ok_or_else(|| {
            GoalServiceError::Internal(format!(
                "goal runtime is unavailable for thread {thread_id}"
            ))
        })?;
        runtime
            .restore_after_resume()
            .await
            .map_err(GoalServiceError::Internal)
    }

    /// Flushes any in-flight goal accounting before a fork copies the source goal snapshot.
    pub async fn flush_thread_goal_progress_for_fork(
        &self,
        thread_id: ThreadId,
    ) -> Result<(), GoalServiceError> {
        let Some(runtime) = self.runtime_for_thread(thread_id) else {
            return Ok(());
        };
        let _goal_state_permit = runtime
            .goal_state_permit()
            .await
            .map_err(GoalServiceError::Internal)?;
        runtime
            .prepare_external_goal_mutation()
            .await
            .map_err(GoalServiceError::Internal)
    }

    pub async fn get_thread_goal(
        &self,
        state_db: &codex_state::StateRuntime,
        thread_id: ThreadId,
    ) -> Result<Option<ThreadGoal>, GoalServiceError> {
        state_db
            .thread_goals()
            .get_thread_goal(thread_id)
            .await
            .map(|goal| goal.map(protocol_goal_from_state))
            .map_err(|err| GoalServiceError::Internal(format!("failed to read thread goal: {err}")))
    }

    /// Records the user's instruction after validation and before changing goal state.
    /// The recording future runs under the runtime's goal mutation permit; failure leaves
    /// the goal unchanged. Recorded user intent remains valid if the database write fails.
    pub async fn set_thread_goal(
        &self,
        state_db: &codex_state::StateRuntime,
        request: GoalSetRequest<'_>,
        record_user_instruction: impl Future<Output = Result<(), GoalServiceError>> + Send,
    ) -> Result<GoalSetOutcome, GoalServiceError> {
        self.set_thread_goal_inner(state_db, request, None, record_user_instruction)
            .await
    }

    pub async fn set_thread_goal_cas(
        &self,
        state_db: &codex_state::StateRuntime,
        request: GoalSetRequest<'_>,
        expected: GoalCas,
    ) -> Result<GoalSetOutcome, GoalServiceError> {
        self.set_thread_goal_inner(state_db, request, Some(expected), async { Ok(()) })
            .await
    }

    /// Records user-origin instructions only after a conditional Goal identity matches.
    /// A stale precondition never appends an instruction to retained history.
    pub async fn set_thread_goal_cas_with_user_instruction(
        &self,
        state_db: &codex_state::StateRuntime,
        request: GoalSetRequest<'_>,
        expected: GoalCas,
        record_user_instruction: impl Future<Output = Result<(), GoalServiceError>> + Send,
    ) -> Result<GoalSetOutcome, GoalServiceError> {
        self.set_thread_goal_inner(state_db, request, Some(expected), record_user_instruction)
            .await
    }

    async fn set_thread_goal_inner(
        &self,
        state_db: &codex_state::StateRuntime,
        request: GoalSetRequest<'_>,
        expected: Option<GoalCas>,
        record_user_instruction: impl Future<Output = Result<(), GoalServiceError>> + Send,
    ) -> Result<GoalSetOutcome, GoalServiceError> {
        let GoalSetRequest {
            thread_id,
            objective,
            status,
            token_budget,
            max_goal_token_budget,
        } = request;
        let status = status.map(state_status_from_protocol);
        let objective = match objective {
            GoalObjectiveUpdate::Keep => None,
            GoalObjectiveUpdate::Set(objective) => Some(objective.trim()),
        };
        let token_budget = match token_budget {
            GoalTokenBudgetUpdate::Keep => None,
            GoalTokenBudgetUpdate::Set(token_budget) => {
                Some(token_budget.or(max_goal_token_budget))
            }
        };

        if let Some(objective) = objective {
            validate_thread_goal_objective(objective).map_err(GoalServiceError::InvalidRequest)?;
        }
        if objective.is_some() || token_budget.is_some() {
            validate_goal_budget(token_budget.flatten(), max_goal_token_budget)
                .map_err(GoalServiceError::InvalidRequest)?;
        }
        let runtime = self.runtime_for_thread(thread_id);
        // Hold this through the prepare/write window so idle continuation cannot
        // launch from goal state that this external mutation is about to change.
        let _goal_state_permit = match runtime.as_ref() {
            Some(runtime) => Some(
                runtime
                    .goal_state_permit()
                    .await
                    .map_err(GoalServiceError::Internal)?,
            ),
            None => None,
        };
        if let Some(runtime) = runtime.as_ref()
            && let Err(err) = runtime.prepare_external_goal_mutation().await
        {
            tracing::warn!("failed to prepare external goal mutation: {err}");
        }

        let existing_goal = state_db
            .thread_goals()
            .get_thread_goal(thread_id)
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to read thread goal: {err}"))
            })?;

        if let Some(expected) = expected.as_ref() {
            let matches = existing_goal.as_ref().is_some_and(|goal| {
                goal.goal_id == expected.goal_id
                    && goal.mission_revision == expected.mission_revision
                    && goal.lifecycle_revision == expected.lifecycle_revision
                    && state_status_from_protocol(expected.status) == goal.status
            });
            if !matches {
                return Err(GoalServiceError::Conflict(
                    "goal_cas_conflict: goalId/status/missionRevision/lifecycleRevision changed"
                        .to_string(),
                ));
            }
        }

        if let Some(existing_goal) = existing_goal.as_ref()
            && objective.is_some()
            && state_db
                .thread_goals()
                .get_thread_goal_document(thread_id)
                .await
                .map_err(|err| {
                    GoalServiceError::Internal(format!(
                        "failed to read managed Goal document metadata: {err}"
                    ))
                })?
                .is_some()
        {
            return Err(GoalServiceError::InvalidRequest(
                "managed Goal objectives must be edited with thread/goal/document/set".to_string(),
            ));
        }
        if objective.is_none() && existing_goal.is_none() {
            return Err(GoalServiceError::InvalidRequest(format!(
                "cannot update goal for thread {thread_id}: no goal exists"
            )));
        }

        // Record only a validated, non-stale user request. A failed recording must not
        // change Goal state; a later database failure leaves the instruction durable.
        record_user_instruction.await?;

        let (goal, previous_goal) = if let Some(objective) = objective {
            if let Some(existing_goal) = existing_goal.as_ref() {
                let previous_goal = PreviousGoalSnapshot::from(existing_goal);
                let update = codex_state::GoalUpdate {
                    objective: Some(objective.to_string()),
                    status,
                    token_budget,
                    expected_goal_id: Some(existing_goal.goal_id.clone()),
                };
                let updated = if let Some(expected) = expected.as_ref() {
                    state_db
                        .thread_goals()
                        .update_thread_goal_cas(
                            thread_id,
                            update,
                            &expected.goal_id,
                            state_status_from_protocol(expected.status),
                            expected.mission_revision,
                            expected.lifecycle_revision,
                        )
                        .await
                } else {
                    state_db
                        .thread_goals()
                        .update_thread_goal(thread_id, update)
                        .await
                };
                updated
                    .map_err(|err| {
                        GoalServiceError::Internal(format!("failed to update thread goal: {err}"))
                    })?
                    .ok_or_else(|| {
                        if expected.is_some() {
                            GoalServiceError::Conflict(
                                "goal_cas_conflict: goal changed during conditional update"
                                    .to_string(),
                            )
                        } else {
                            GoalServiceError::InvalidRequest(format!(
                                "cannot update goal for thread {thread_id}: no goal exists"
                            ))
                        }
                    })
                    .map(|goal| (goal, Some(previous_goal)))?
            } else {
                state_db
                    .thread_goals()
                    .replace_thread_goal(
                        thread_id,
                        objective,
                        status.unwrap_or(codex_state::ThreadGoalStatus::Active),
                        token_budget.flatten().or(max_goal_token_budget),
                    )
                    .await
                    .map_err(|err| {
                        GoalServiceError::Internal(format!("failed to replace thread goal: {err}"))
                    })
                    .map(|goal| (goal, None))?
            }
        } else {
            let existing_goal = existing_goal.ok_or_else(|| {
                GoalServiceError::InvalidRequest(format!(
                    "cannot update goal for thread {thread_id}: no goal exists"
                ))
            })?;
            let previous_goal = PreviousGoalSnapshot::from(&existing_goal);
            let expected_goal_id = existing_goal.goal_id.clone();
            let updated = if let Some(expected) = expected.as_ref() {
                state_db
                    .thread_goals()
                    .update_thread_goal_cas(
                        thread_id,
                        codex_state::GoalUpdate {
                            objective: None,
                            status,
                            token_budget,
                            expected_goal_id: Some(expected_goal_id.clone()),
                        },
                        &expected.goal_id,
                        state_status_from_protocol(expected.status),
                        expected.mission_revision,
                        expected.lifecycle_revision,
                    )
                    .await
            } else {
                state_db
                    .thread_goals()
                    .update_thread_goal(
                        thread_id,
                        codex_state::GoalUpdate {
                            objective: None,
                            status,
                            token_budget,
                            expected_goal_id: Some(expected_goal_id),
                        },
                    )
                    .await
            };
            updated
                .map_err(|err| {
                    GoalServiceError::Internal(format!("failed to update thread goal: {err}"))
                })?
                .ok_or_else(|| {
                    if expected.is_some() {
                        GoalServiceError::Conflict(
                            "goal_cas_conflict: goal changed during conditional update".to_string(),
                        )
                    } else {
                        GoalServiceError::InvalidRequest(format!(
                            "cannot update goal for thread {thread_id}: no goal exists"
                        ))
                    }
                })
                .map(|goal| (goal, Some(previous_goal)))?
        };

        if let Some(runtime) = runtime.as_ref() {
            runtime.clear_pending_turn_start_options().await;
        }

        if objective.is_some() {
            fill_empty_thread_preview_if_possible(state_db, thread_id, &goal).await;
        }
        self.observe_goal_completion(&goal).await;
        Ok(GoalSetOutcome {
            goal: protocol_goal_from_state(goal.clone()),
            state_goal: goal,
            previous_goal,
        })
    }

    /// Atomically installs a file-backed mission document and its concise
    /// native Goal pointer. The document is written and synced first; the
    /// SQLite Goal pointer plus its digest metadata are then committed with a
    /// mission-revision CAS. A failed CAS leaves the prior Goal and document
    /// untouched and never creates a second active mission.
    pub async fn set_thread_goal_document(
        &self,
        state_db: &codex_state::StateRuntime,
        request: GoalDocumentSetRequest<'_>,
    ) -> Result<GoalDocumentSetOutcome, GoalServiceError> {
        if request.document.is_empty() {
            return Err(GoalServiceError::InvalidRequest(
                "managed Goal document must not be empty".to_string(),
            ));
        }
        if request.document.len() > MAX_MANAGED_GOAL_DOCUMENT_BYTES {
            return Err(GoalServiceError::InvalidRequest(format!(
                "managed Goal document must be at most {MAX_MANAGED_GOAL_DOCUMENT_BYTES} bytes"
            )));
        }

        let thread_id = request.thread_id;
        let runtime = self.runtime_for_thread(thread_id);
        let _goal_state_permit = match runtime.as_ref() {
            Some(runtime) => Some(
                runtime
                    .goal_state_permit()
                    .await
                    .map_err(GoalServiceError::Internal)?,
            ),
            None => None,
        };

        let existing_goal = state_db
            .thread_goals()
            .get_thread_goal(thread_id)
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to read thread goal: {err}"))
            })?;
        let Some(existing_goal) = existing_goal else {
            return Err(GoalServiceError::InvalidRequest(
                "managed Goal document creation requires thread/goal/document/create with an explicit absent/noGoal precondition"
                    .to_string(),
            ));
        };
        let Some(expected_goal) = request.expected_goal.as_ref() else {
            return Err(GoalServiceError::InvalidRequest(
                "managed Goal document updates require expected goalId/status/missionRevision/lifecycleRevision"
                    .to_string(),
            ));
        };

        let mission_revision = existing_goal.mission_revision + 1;
        let digest = format!("{:x}", Sha256::digest(request.document.as_bytes()));
        let relative_path = PathBuf::from(MANAGED_GOAL_DOCUMENT_DIRECTORY)
            .join(thread_id.to_string())
            .join(format!("mission-{mission_revision}-{digest}.md"));
        let absolute_path = state_db.sqlite().home().join(&relative_path);
        let created_file = write_managed_goal_document(&absolute_path, request.document.as_bytes())
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to write Goal document: {err}"))
            })?;
        let pointer = format!(
            "managed Goal document: {} (sha256={digest}, bytes={}, mission_revision={mission_revision})",
            relative_path.display(),
            request.document.len()
        );

        let status = request
            .status
            .unwrap_or(codex_protocol::protocol::ThreadGoalStatus::Active);
        let result = state_db
            .thread_goals()
            .update_thread_goal_document_cas(
                thread_id,
                &pointer,
                Some(state_goal_status(status)),
                &expected_goal.goal_id,
                state_goal_status(expected_goal.status),
                expected_goal.mission_revision,
                expected_goal.lifecycle_revision,
                &relative_path.to_string_lossy(),
                &digest,
                request.document.len() as i64,
            )
            .await;
        let goal = match result {
            Ok(Some(goal)) => goal,
            Ok(None) => {
                if created_file {
                    let _ = fs::remove_file(&absolute_path).await;
                }
                return Err(if request.expected_goal.is_some() {
                    GoalServiceError::Conflict(
                        "goal_document_cas_conflict: Goal identity or revision changed".to_string(),
                    )
                } else {
                    GoalServiceError::InvalidRequest(
                        "managed Goal changed during document update; retry from a fresh read"
                            .to_string(),
                    )
                });
            }
            Err(err) => {
                if created_file {
                    let _ = fs::remove_file(&absolute_path).await;
                }
                return Err(GoalServiceError::Internal(format!(
                    "failed to commit Goal document pointer: {err}"
                )));
            }
        };

        if let Some(runtime) = runtime.as_ref() {
            runtime.clear_pending_turn_start_options().await;
        }
        fill_empty_thread_preview_if_possible(state_db, thread_id, &goal).await;
        self.observe_goal_completion(&goal).await;
        let document = state_db
            .thread_goals()
            .get_thread_goal_document(thread_id)
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to read committed Goal document: {err}"))
            })?
            .ok_or_else(|| {
                GoalServiceError::Internal(
                    "committed Goal document metadata is missing".to_string(),
                )
            })?;
        Ok(GoalDocumentSetOutcome {
            goal: GoalSetOutcome {
                goal: protocol_goal_from_state(goal.clone()),
                state_goal: goal,
                previous_goal: Some(PreviousGoalSnapshot::from(&existing_goal)),
            },
            document,
        })
    }

    /// Starts a new managed mission from an exact completed predecessor.  The
    /// state transaction owns the four-field predecessor CAS and durable
    /// creation-id receipt; this layer owns the fail-closed document write and
    /// runtime serialization.
    pub async fn create_thread_goal_document(
        &self,
        state_db: &codex_state::StateRuntime,
        request: GoalDocumentCreateRequest<'_>,
    ) -> Result<GoalDocumentCreateOutcome, GoalServiceError> {
        if request.document.is_empty() {
            return Err(GoalServiceError::InvalidRequest(
                "managed Goal document must not be empty".to_string(),
            ));
        }
        if request.document.len() > MAX_MANAGED_GOAL_DOCUMENT_BYTES {
            return Err(GoalServiceError::InvalidRequest(format!(
                "managed Goal document must be at most {MAX_MANAGED_GOAL_DOCUMENT_BYTES} bytes"
            )));
        }
        if request.creation_id.trim().is_empty() || request.creation_id.len() > 256 {
            return Err(GoalServiceError::InvalidRequest(
                "creationId must be non-empty and at most 256 bytes".to_string(),
            ));
        }
        let expected_goal = match &request.precondition {
            GoalDocumentCreatePrecondition::Absent { admission_state } => match admission_state {
                GoalDocumentCreateAdmissionState::NoGoal => None,
            },
            GoalDocumentCreatePrecondition::Completed(expected_goal) => {
                if expected_goal.goal_id.trim().is_empty() || expected_goal.goal_id.len() > 256 {
                    return Err(GoalServiceError::InvalidRequest(
                        "expected Goal id must be non-empty and at most 256 bytes".to_string(),
                    ));
                }
                if expected_goal.status != ThreadGoalStatus::Complete {
                    return Err(GoalServiceError::InvalidRequest(
                        "completed predecessor requires expectedStatus=complete".to_string(),
                    ));
                }
                Some(expected_goal)
            }
        };
        let objective = request.objective.trim();
        validate_thread_goal_objective(objective).map_err(GoalServiceError::InvalidRequest)?;
        validate_goal_budget(request.token_budget, request.max_goal_token_budget)
            .map_err(GoalServiceError::InvalidRequest)?;
        let status = request.status.unwrap_or(ThreadGoalStatus::Active);
        let state_status = state_status_from_protocol(status);
        let request_digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&serde_json::json!({
                    "document": request.document,
                    "objective": objective,
                    "status": state_status.as_str(),
                    "token_budget": request.token_budget,
                    "precondition": match &request.precondition {
                        GoalDocumentCreatePrecondition::Absent { .. } => serde_json::json!({
                            "kind": "absent",
                            "admission_state": "no_goal",
                        }),
                        GoalDocumentCreatePrecondition::Completed(expected_goal) => serde_json::json!({
                            "kind": "completed",
                            "expected_goal_id": expected_goal.goal_id,
                            "expected_status": state_status_from_protocol(expected_goal.status).as_str(),
                            "expected_mission_revision": expected_goal.mission_revision,
                            "expected_lifecycle_revision": expected_goal.lifecycle_revision,
                        }),
                    },
                }))
                .map_err(|err| GoalServiceError::Internal(format!(
                    "failed to hash Goal creation request: {err}"
                )))?,
            )
        );
        let digest = format!("{:x}", Sha256::digest(request.document.as_bytes()));
        let relative_path = PathBuf::from(MANAGED_GOAL_DOCUMENT_DIRECTORY)
            .join(request.thread_id.to_string())
            .join(format!("mission-1-{digest}.md"));
        let absolute_path = state_db.sqlite().home().join(&relative_path);
        // The path is content-addressed.  Leave it in place if the state CAS
        // later loses a race: another creator may have committed the same
        // immutable bytes, and deleting the path here could remove its live
        // document after a concurrent winner has committed.
        write_managed_goal_document(&absolute_path, request.document.as_bytes())
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to write Goal document: {err}"))
            })?;

        let runtime = self.runtime_for_thread(request.thread_id);
        let _goal_state_permit = match runtime.as_ref() {
            Some(runtime) => Some(
                runtime
                    .goal_state_permit()
                    .await
                    .map_err(GoalServiceError::Internal)?,
            ),
            None => None,
        };
        if let Some(runtime) = runtime.as_ref()
            && let Err(err) = runtime.prepare_external_goal_mutation().await
        {
            tracing::warn!("failed to prepare external Goal creation: {err}");
        }

        let result = match expected_goal {
            None => {
                state_db
                    .thread_goals()
                    .create_initial_thread_goal_document(
                        request.thread_id,
                        request.creation_id,
                        &request_digest,
                        objective,
                        state_status,
                        request.token_budget,
                        &relative_path.to_string_lossy(),
                        &digest,
                        request.document.len() as i64,
                    )
                    .await
            }
            Some(expected_goal) => {
                state_db
                    .thread_goals()
                    .create_thread_goal_document(
                        request.thread_id,
                        request.creation_id,
                        &request_digest,
                        &expected_goal.goal_id,
                        state_status_from_protocol(expected_goal.status),
                        expected_goal.mission_revision,
                        expected_goal.lifecycle_revision,
                        objective,
                        state_status,
                        request.token_budget,
                        &relative_path.to_string_lossy(),
                        &digest,
                        request.document.len() as i64,
                    )
                    .await
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(err) => {
                return Err(GoalServiceError::Internal(format!(
                    "failed to commit new Goal identity: {err}"
                )));
            }
        };
        let receipt = match result {
            codex_state::ThreadGoalDocumentCreateOutcome::Created(receipt) => (receipt, false),
            codex_state::ThreadGoalDocumentCreateOutcome::Replayed(receipt) => (receipt, true),
            codex_state::ThreadGoalDocumentCreateOutcome::Conflict(message) => {
                return Err(GoalServiceError::Conflict(message));
            }
        };
        let (receipt, replayed) = receipt;
        let previous_identity = receipt.previous_goal.as_ref().map(GoalIdentity::from);
        let outcome = GoalSetOutcome {
            goal: protocol_goal_from_state(receipt.goal.clone()),
            state_goal: receipt.goal,
            previous_goal: receipt
                .previous_goal
                .as_ref()
                .map(PreviousGoalSnapshot::from),
        };
        Ok(GoalDocumentCreateOutcome {
            goal: outcome,
            document: receipt.document,
            previous_identity,
            creation_id: receipt.creation_id,
            replayed,
        })
    }

    pub async fn read_thread_goal_document(
        &self,
        state_db: &codex_state::StateRuntime,
        thread_id: ThreadId,
    ) -> Result<
        Option<(
            codex_state::ThreadGoal,
            codex_state::ThreadGoalDocument,
            String,
        )>,
        GoalServiceError,
    > {
        let Some(goal) = state_db
            .thread_goals()
            .get_thread_goal(thread_id)
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to read thread goal: {err}"))
            })?
        else {
            return Ok(None);
        };
        let Some(document) = state_db
            .thread_goals()
            .get_thread_goal_document(thread_id)
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to read Goal document metadata: {err}"))
            })?
        else {
            return Ok(None);
        };
        if document.mission_revision != goal.mission_revision {
            return Err(GoalServiceError::Internal(
                "Goal document mission revision does not match the current Goal".to_string(),
            ));
        }
        let relative_path = Path::new(document.path.as_str());
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(GoalServiceError::Internal(
                "Goal document path is outside the managed state directory".to_string(),
            ));
        }
        let path = state_db.sqlite().home().join(relative_path);
        let bytes = fs::read(&path).await.map_err(|err| {
            GoalServiceError::Internal(format!("failed to read Goal document: {err}"))
        })?;
        if bytes.len() as i64 != document.byte_length
            || format!("{:x}", Sha256::digest(&bytes)) != document.sha256
        {
            return Err(GoalServiceError::Internal(
                "Goal document digest or byte length does not match durable metadata".to_string(),
            ));
        }
        let content = String::from_utf8(bytes).map_err(|_| {
            GoalServiceError::Internal("Goal document is not valid UTF-8".to_string())
        })?;
        Ok(Some((goal, document, content)))
    }

    /// Persists any durable task completion that is proven by this exact Goal
    /// transition. Tool-backed Goal mutations use this same hook so a
    /// replacement Goal cannot hide a completion before the next receipt read.
    pub(crate) async fn observe_goal_completion(&self, goal: &codex_state::ThreadGoal) {
        if goal.status != codex_state::ThreadGoalStatus::Complete {
            return;
        }
        let observer = self
            .completion_observer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            let completing_turn_id = self
                .runtime_for_thread(goal.thread_id)
                .and_then(|runtime| runtime.current_turn_id());
            if let Err(err) = observer.on_goal_completed(goal, completing_turn_id).await {
                tracing::warn!(
                    thread_id = %goal.thread_id,
                    %err,
                    "failed to persist durable task completion at Goal transition"
                );
            }
        }
    }

    pub async fn clear_thread_goal(
        &self,
        state_db: &codex_state::StateRuntime,
        thread_id: ThreadId,
        record_user_instruction: impl Future<Output = Result<(), GoalServiceError>> + Send,
    ) -> Result<bool, GoalServiceError> {
        let runtime = self.runtime_for_thread(thread_id);
        // Hold this through the prepare/write window so idle continuation cannot
        // launch from goal state that this external mutation is about to change.
        let goal_state_permit = match runtime.as_ref() {
            Some(runtime) => Some(
                runtime
                    .goal_state_permit()
                    .await
                    .map_err(GoalServiceError::Internal)?,
            ),
            None => None,
        };
        if let Some(runtime) = runtime.as_ref()
            && let Err(err) = runtime.prepare_external_goal_mutation().await
        {
            tracing::warn!("failed to prepare external goal mutation: {err}");
        }

        // A failed set can leave authorization in history without a goal row.
        record_user_instruction.await?;

        let cleared_goal = state_db
            .thread_goals()
            .delete_thread_goal(thread_id)
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to clear thread goal: {err}"))
            })?;
        let cleared = cleared_goal.is_some();
        if cleared && let Some(runtime) = runtime.as_ref() {
            runtime.clear_pending_turn_start_options().await;
        }
        drop(goal_state_permit);
        drop(runtime);

        if let (Some(runtime), Some(goal)) = (self.runtime_for_thread(thread_id), cleared_goal)
            && let Err(err) = runtime.apply_external_goal_clear(goal).await
        {
            tracing::warn!("failed to apply external goal clear runtime effects: {err}");
        }

        Ok(cleared)
    }

    pub async fn clear_thread_goal_cas(
        &self,
        state_db: &codex_state::StateRuntime,
        thread_id: ThreadId,
        expected: GoalCas,
    ) -> Result<codex_state::ThreadGoal, GoalServiceError> {
        let runtime = self.runtime_for_thread(thread_id);
        let goal_state_permit = match runtime.as_ref() {
            Some(runtime) => Some(
                runtime
                    .goal_state_permit()
                    .await
                    .map_err(GoalServiceError::Internal)?,
            ),
            None => None,
        };
        if let Some(runtime) = runtime.as_ref()
            && let Err(err) = runtime.prepare_external_goal_mutation().await
        {
            tracing::warn!("failed to prepare external goal CAS clear: {err}");
        }

        let cleared = state_db
            .thread_goals()
            .delete_thread_goal_cas(
                thread_id,
                &expected.goal_id,
                state_status_from_protocol(expected.status),
                expected.mission_revision,
                expected.lifecycle_revision,
            )
            .await
            .map_err(|err| {
                GoalServiceError::Internal(format!("failed to clear thread Goal: {err}"))
            })?;
        let Some(goal) = cleared else {
            return Err(GoalServiceError::Conflict(
                "goal_cas_conflict: goalId/status/missionRevision/lifecycleRevision changed"
                    .to_string(),
            ));
        };
        if let Some(runtime) = runtime.as_ref() {
            runtime.clear_pending_turn_start_options().await;
        }
        drop(goal_state_permit);
        drop(runtime);
        if let Some(runtime) = self.runtime_for_thread(thread_id)
            && let Err(err) = runtime.apply_external_goal_clear(goal.clone()).await
        {
            tracing::warn!("failed to apply external Goal CAS clear runtime effects: {err}");
        }
        Ok(goal)
    }

    pub(crate) fn register_runtime(&self, runtime: &Arc<GoalRuntimeHandle>) {
        self.runtimes()
            .insert(runtime.thread_id().to_string(), Arc::downgrade(runtime));
    }

    pub(crate) fn unregister_runtime(&self, runtime: &Arc<GoalRuntimeHandle>) {
        let key = runtime.thread_id().to_string();
        let runtime = Arc::downgrade(runtime);
        let mut runtimes = self.runtimes();
        if runtimes
            .get(&key)
            .is_some_and(|registered| registered.ptr_eq(&runtime))
        {
            runtimes.remove(&key);
        }
    }

    pub(crate) fn runtime_for_thread(&self, thread_id: ThreadId) -> Option<Arc<GoalRuntimeHandle>> {
        let key = thread_id.to_string();
        let mut runtimes = self.runtimes();
        let runtime = runtimes.get(&key).and_then(Weak::upgrade);
        if runtime.is_none() {
            runtimes.remove(&key);
        }
        runtime
    }

    fn runtimes(&self) -> std::sync::MutexGuard<'_, HashMap<String, Weak<GoalRuntimeHandle>>> {
        self.runtimes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn state_goal_status(status: ThreadGoalStatus) -> codex_state::ThreadGoalStatus {
    match status {
        ThreadGoalStatus::Active => codex_state::ThreadGoalStatus::Active,
        ThreadGoalStatus::Paused => codex_state::ThreadGoalStatus::Paused,
        ThreadGoalStatus::Blocked => codex_state::ThreadGoalStatus::Blocked,
        ThreadGoalStatus::UsageLimited => codex_state::ThreadGoalStatus::UsageLimited,
        ThreadGoalStatus::BudgetLimited => codex_state::ThreadGoalStatus::BudgetLimited,
        ThreadGoalStatus::Complete => codex_state::ThreadGoalStatus::Complete,
    }
}

async fn write_managed_goal_document(path: &Path, bytes: &[u8]) -> Result<bool, std::io::Error> {
    if let Ok(existing) = fs::read(path).await {
        if existing == bytes {
            return Ok(false);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "managed Goal document path already contains different bytes",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        MANAGED_GOAL_DOCUMENT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    drop(file);
    fs::rename(&temporary, path).await?;
    Ok(true)
}
