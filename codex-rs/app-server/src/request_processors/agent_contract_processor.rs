//! Content-free, caller-bound fleet-agent coordination methods.
//!
//! This module intentionally composes the existing persisted thread store,
//! native goal service, durable user-input queue, and owner-bound background
//! terminal status lookup.  It does not introduce a process broker, a second
//! admission queue, or a shell-based custody probe.

use std::path::PathBuf;
use std::sync::Arc;

use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::outgoing_message::ConnectionId;
use crate::thread_state::ThreadStateManager;
use codex_app_server_protocol::AgentGoalAvailability;
use codex_app_server_protocol::AgentGoalReadParams;
use codex_app_server_protocol::AgentInventoryAvailability;
use codex_app_server_protocol::AgentInventoryEntry;
use codex_app_server_protocol::AgentInventoryListParams;
use codex_app_server_protocol::AgentInventoryListResponse;
use codex_app_server_protocol::AgentInventoryScope;
use codex_app_server_protocol::AgentInventoryVisibility;
use codex_app_server_protocol::AgentRuntimeResidency;
use codex_app_server_protocol::AgentRuntimeStatus;
use codex_app_server_protocol::AgentSessionAvailability;
use codex_app_server_protocol::AgentSessionPhase;
use codex_app_server_protocol::AgentSessionStatusParams;
use codex_app_server_protocol::AgentSessionStatusResponse;
use codex_app_server_protocol::AgentTaskAdmissionState;
use codex_app_server_protocol::AgentTaskAdmitParams;
use codex_app_server_protocol::AgentTaskAdmitResponse;
use codex_app_server_protocol::AgentTaskBlocker;
use codex_app_server_protocol::AgentTaskGoalBinding;
use codex_app_server_protocol::AgentTaskObservation;
use codex_app_server_protocol::AgentTaskOutcome;
use codex_app_server_protocol::AgentTaskReceiptReadParams;
use codex_app_server_protocol::AgentTaskReceiptReadResponse;
use codex_app_server_protocol::AgentTaskTurnState;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ThreadGoalStatus;
use codex_core::BackgroundTerminalStatusPhase;
use codex_core::CodexThread;
use codex_core::ThreadManager;
use codex_core::TurnInput;
use codex_extension_api::ExtensionFuture;
use codex_goal_extension::GoalCompletionObserver;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::SessionSource;
use codex_queue_extension::QueueAdmissionCompletionProof;
use codex_queue_extension::QueueAdmissionState;
use codex_queue_extension::QueueAdmissionValidation;
use codex_queue_extension::QueueAdmissionValidator;
use codex_queue_extension::QueuedAdmission;
use codex_queue_extension::QueuedItem;
use codex_queue_extension::QueuedItemService;
use codex_rollout::RolloutItem;
use codex_thread_store::ListThreadsParams;
use codex_thread_store::ReadThreadParams;
use codex_thread_store::SortDirection;
use codex_thread_store::StoredThread;
use codex_thread_store::ThreadRelationFilter;
use codex_thread_store::ThreadSortKey;
use codex_thread_store::ThreadStore;
use codex_thread_store::ThreadStoreError;

use super::thread_goal_processor::ThreadGoalRequestProcessor;
use super::thread_queue_processor::ensure_direct_input_allowed;
use super::thread_queue_processor::submission_into_turn_input;
use super::turn_processor::validate_user_input_image_urls;

const DEFAULT_LIMIT: usize = 25;
const MAX_LIMIT: usize = 100;

pub(crate) struct AgentContractRequestProcessor {
    thread_manager: Arc<ThreadManager>,
    thread_store: Arc<dyn ThreadStore>,
    queue_service: Option<Arc<QueuedItemService>>,
    goal_processor: ThreadGoalRequestProcessor,
    thread_state_manager: ThreadStateManager,
}

/// Revalidates an agent admission at the durable queue dispatch boundary.
/// Admission is not allowed to rely on the Goal snapshot captured before a
/// queued item waited behind another turn.
pub(crate) struct AgentGoalAdmissionValidator {
    goal_processor: ThreadGoalRequestProcessor,
}

pub(crate) struct AgentGoalCompletionObserver {
    queue_service: Arc<QueuedItemService>,
}

impl AgentGoalCompletionObserver {
    pub(crate) fn new(queue_service: Arc<QueuedItemService>) -> Self {
        Self { queue_service }
    }
}

impl GoalCompletionObserver for AgentGoalCompletionObserver {
    fn on_goal_completed<'a>(
        &'a self,
        goal: &'a codex_state::ThreadGoal,
        completing_turn_id: Option<String>,
    ) -> codex_goal_extension::GoalCompletionObserverFuture<'a> {
        let queue_service = Arc::clone(&self.queue_service);
        let goal_id = goal.goal_id.clone();
        let mission_revision = goal.mission_revision;
        let lifecycle_revision = goal.lifecycle_revision;
        let objective = goal.objective.clone();
        Box::pin(async move {
            queue_service
                .record_completed_admission_for_goal(
                    goal.thread_id,
                    &goal_id,
                    mission_revision,
                    lifecycle_revision,
                    &objective,
                    completing_turn_id.as_deref(),
                )
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }
}

impl AgentGoalAdmissionValidator {
    pub(crate) fn new(goal_processor: ThreadGoalRequestProcessor) -> Self {
        Self { goal_processor }
    }
}

impl QueueAdmissionValidator for AgentGoalAdmissionValidator {
    fn validate<'a>(
        &'a self,
        thread_id: ThreadId,
        _admission_id: &'a str,
        goal_binding_json: &'a str,
    ) -> ExtensionFuture<'a, QueueAdmissionValidation> {
        Box::pin(async move {
            let Ok(binding) = serde_json::from_str::<AgentTaskGoalBinding>(goal_binding_json)
            else {
                return QueueAdmissionValidation {
                    accepted: false,
                    blocker: Some("invalid_goal_binding".to_string()),
                };
            };
            let read = match self
                .goal_processor
                .agent_goal_read(AgentGoalReadParams {
                    thread_id: thread_id.to_string(),
                })
                .await
            {
                Ok(read) => read,
                Err(_) => {
                    return QueueAdmissionValidation {
                        accepted: false,
                        blocker: Some("goal_unavailable".to_string()),
                    };
                }
            };
            let blocker = goal_binding_blocker(&read, &binding).map(|blocker| {
                match blocker {
                    AgentTaskBlocker::GoalUnavailable => "goal_unavailable",
                    AgentTaskBlocker::GoalNotFound => "goal_not_found",
                    AgentTaskBlocker::GoalNotActive => "goal_not_active",
                    AgentTaskBlocker::GoalChanged => "goal_changed",
                    _ => "goal_binding_rejected",
                }
                .to_string()
            });
            QueueAdmissionValidation {
                accepted: blocker.is_none(),
                blocker,
            }
        })
    }

    fn completion_proof<'a>(
        &'a self,
        thread_id: ThreadId,
        _admission_id: &'a str,
        goal_binding_json: &'a str,
        _turn_id: &'a str,
    ) -> ExtensionFuture<'a, Option<QueueAdmissionCompletionProof>> {
        Box::pin(async move {
            let Ok(binding) = serde_json::from_str::<AgentTaskGoalBinding>(goal_binding_json)
            else {
                return None;
            };
            let Ok(read) = self
                .goal_processor
                .agent_goal_read(AgentGoalReadParams {
                    thread_id: thread_id.to_string(),
                })
                .await
            else {
                return None;
            };
            if !goal_identity_matches(&read, &binding)
                || read
                    .goal
                    .as_ref()
                    .is_none_or(|goal| goal.status != ThreadGoalStatus::Complete)
            {
                return None;
            }
            let goal = read.goal?;
            Some(QueueAdmissionCompletionProof {
                goal_id: read.goal_id?,
                mission_revision: read.mission_revision?,
                lifecycle_revision: read.lifecycle_revision?,
                objective: goal.objective,
            })
        })
    }
}

impl AgentContractRequestProcessor {
    pub(crate) fn new(
        thread_manager: Arc<ThreadManager>,
        thread_store: Arc<dyn ThreadStore>,
        queue_service: Option<Arc<QueuedItemService>>,
        goal_processor: ThreadGoalRequestProcessor,
        thread_state_manager: ThreadStateManager,
    ) -> Self {
        Self {
            thread_manager,
            thread_store,
            queue_service,
            goal_processor,
            thread_state_manager,
        }
    }

    pub(crate) async fn inventory_list(
        &self,
        connection_id: ConnectionId,
        params: AgentInventoryListParams,
    ) -> Result<AgentInventoryListResponse, JSONRPCErrorError> {
        let caller_id = parse_thread_id(&params.thread_id)?;
        if !self.is_owner(connection_id, caller_id).await {
            return Ok(AgentInventoryListResponse {
                availability: AgentInventoryAvailability::Unavailable,
                data: vec![],
                next_cursor: None,
            });
        }
        let caller = match self
            .thread_store
            .read_thread(ReadThreadParams {
                thread_id: caller_id,
                include_archived: true,
                include_history: false,
            })
            .await
        {
            Ok(thread) => thread,
            Err(ThreadStoreError::ThreadNotFound { .. }) => {
                return Ok(AgentInventoryListResponse {
                    availability: AgentInventoryAvailability::NotFound,
                    data: vec![],
                    next_cursor: None,
                });
            }
            Err(_) => {
                return Ok(AgentInventoryListResponse {
                    availability: AgentInventoryAvailability::Unavailable,
                    data: vec![],
                    next_cursor: None,
                });
            }
        };

        let limit = params
            .limit
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, MAX_LIMIT);
        let scope = params.scope.unwrap_or(AgentInventoryScope::Subtree);
        let visibility = params.visibility.unwrap_or(AgentInventoryVisibility::Known);
        let include_caller = params.cursor.is_none()
            && matches!(
                scope,
                AgentInventoryScope::Tree | AgentInventoryScope::Subtree
            );

        // The caller is a fixed context row. `limit` bounds the persisted
        // descendants so the opaque store cursor always resumes after exactly
        // the last returned descendant, never after a hidden root row.
        let mut data = Vec::new();
        if include_caller
            && (visibility == AgentInventoryVisibility::Known || self.is_loaded(caller_id).await)
        {
            data.push(self.inventory_entry(&caller).await);
        }

        let relation_filter = match scope {
            AgentInventoryScope::Children => ThreadRelationFilter::DirectChildrenOf(caller_id),
            AgentInventoryScope::Tree | AgentInventoryScope::Subtree => {
                ThreadRelationFilter::DescendantsOf(caller_id)
            }
        };
        let page = match self
            .thread_store
            .list_threads(ListThreadsParams {
                page_size: limit,
                cursor: params.cursor,
                sort_key: ThreadSortKey::CreatedAt,
                sort_direction: SortDirection::Asc,
                allowed_sources: vec![],
                model_providers: Some(vec![]),
                cwd_filters: None,
                section: None,
                project_id: None,
                archived: false,
                search_term: None,
                relation_filter: Some(relation_filter),
                included_thread_ids: None,
                use_state_db_only: false,
            })
            .await
        {
            Ok(page) => page,
            Err(_) => {
                return Ok(AgentInventoryListResponse {
                    availability: AgentInventoryAvailability::Unavailable,
                    data: vec![],
                    next_cursor: None,
                });
            }
        };

        for thread in page.items {
            if visibility == AgentInventoryVisibility::Loaded
                && !self.is_loaded(thread.thread_id).await
            {
                continue;
            }
            data.push(self.inventory_entry(&thread).await);
        }

        Ok(AgentInventoryListResponse {
            availability: AgentInventoryAvailability::Available,
            data,
            next_cursor: page.next_cursor,
        })
    }

    pub(crate) async fn goal_read(
        &self,
        connection_id: ConnectionId,
        params: AgentGoalReadParams,
    ) -> Result<codex_app_server_protocol::AgentGoalReadResponse, JSONRPCErrorError> {
        let thread_id = parse_thread_id(&params.thread_id)?;
        if !self.is_owner(connection_id, thread_id).await {
            return Ok(codex_app_server_protocol::AgentGoalReadResponse {
                availability: AgentGoalAvailability::Unavailable,
                goal: None,
                goal_id: None,
                mission_revision: None,
                lifecycle_revision: None,
                explicit_resume_required: false,
                unavailable_reason: Some(
                    codex_app_server_protocol::AgentGoalUnavailableReason::CallerNotOwner,
                ),
            });
        }
        self.goal_processor.agent_goal_read(params).await
    }

    pub(crate) async fn task_admit(
        &self,
        connection_id: ConnectionId,
        params: AgentTaskAdmitParams,
    ) -> Result<AgentTaskAdmitResponse, JSONRPCErrorError> {
        validate_user_input_image_urls(&params.input)?;
        if params.admission_id.trim().is_empty() {
            return Err(invalid_request("admission_id must not be empty"));
        }
        let thread_id = parse_thread_id(&params.thread_id)?;
        if !self.is_owner(connection_id, thread_id).await {
            return Ok(task_admit_blocked(
                &params.admission_id,
                AgentTaskBlocker::ThreadUnavailable,
            ));
        }

        // A retry first reads the durable receipt.  If a prior submission is
        // already visible, returning it is the only effect; no second queue
        // record is created.
        let receipt = self
            .read_receipt(
                connection_id,
                &AgentTaskReceiptReadParams {
                    thread_id: params.thread_id.clone(),
                    admission_id: params.admission_id.clone(),
                },
            )
            .await?;
        if let Some(expected_turn_id) = params.expected_turn_id.as_deref()
            && let Some(service) = self.queue_service.as_ref()
            && let Some(admission) = service
                .find_admission(thread_id, params.admission_id.as_str())
                .await
                .map_err(|_| internal_error("durable task receipt is temporarily unavailable"))?
            && admission.dispatch_mode == "steer"
            && admission.admission_id == params.admission_id
            && admission.state == QueueAdmissionState::Pending
        {
            let _ = service
                .resume_pending_steer(thread_id, &params.admission_id, expected_turn_id)
                .await
                .map_err(|_| internal_error("active steer admission is temporarily unavailable"))?;
            let receipt = self
                .read_receipt(
                    connection_id,
                    &AgentTaskReceiptReadParams {
                        thread_id: params.thread_id.clone(),
                        admission_id: params.admission_id.clone(),
                    },
                )
                .await?;
            return Ok(admit_response_from_receipt(receipt));
        }
        if should_reuse_receipt(&receipt) {
            return Ok(admit_response_from_receipt(receipt));
        }

        if let Some(blocker) = self.goal_blocker(thread_id, &params.goal).await? {
            return Ok(task_admit_blocked(&params.admission_id, blocker));
        }

        let (loaded_thread, source, cwd) = self.require_thread(thread_id).await?;
        ensure_direct_input_allowed(loaded_thread.as_deref(), &source)?;
        let Some(service) = self.queue_service.as_ref() else {
            return Ok(task_admit_blocked(
                &params.admission_id,
                AgentTaskBlocker::QueueUnavailable,
            ));
        };

        let goal_binding_json = serde_json::to_string(&params.goal)
            .map_err(|_| internal_error("failed to persist Goal admission binding"))?;
        match service
            .enqueue_admission(
                thread_id,
                submission_into_turn_input(params.input, Some(params.admission_id.clone()), &cwd)?,
                params.admission_id.clone(),
                goal_binding_json,
                params.expected_turn_id.clone(),
            )
            .await
        {
            Ok(admission) => Ok(admit_response_from_admission(admission)),
            Err(error) => {
                // A storage failure can occur after the durable write.  Read
                // the same receipt before surfacing an unknown outcome, and
                // never blindly retry or start another turn.
                let receipt = self
                    .read_receipt(
                        connection_id,
                        &AgentTaskReceiptReadParams {
                            thread_id: params.thread_id,
                            admission_id: params.admission_id.clone(),
                        },
                    )
                    .await?;
                if receipt.admission_state != AgentTaskAdmissionState::NeverSeen {
                    Ok(admit_response_from_receipt(receipt))
                } else if receipt.observation == AgentTaskObservation::Unavailable {
                    Ok(task_admit_unavailable(&params.admission_id))
                } else {
                    tracing::warn!(
                        "agent task admission outcome is unknown after queue error: {error}"
                    );
                    Ok(task_admit_unknown(&params.admission_id))
                }
            }
        }
    }

    pub(crate) async fn task_receipt_read(
        &self,
        connection_id: ConnectionId,
        params: AgentTaskReceiptReadParams,
    ) -> Result<AgentTaskReceiptReadResponse, JSONRPCErrorError> {
        self.read_receipt(connection_id, &params).await
    }

    pub(crate) async fn session_status(
        &self,
        connection_id: ConnectionId,
        params: AgentSessionStatusParams,
    ) -> Result<AgentSessionStatusResponse, JSONRPCErrorError> {
        let thread_id = parse_thread_id(&params.thread_id)?;
        if !self.is_owner(connection_id, thread_id).await {
            return Ok(unavailable_session_status());
        }
        let Ok(thread) = self.thread_manager.get_thread(thread_id).await else {
            return Ok(unavailable_session_status());
        };
        let status = thread
            .read_background_terminal_status(
                &params.turn_id,
                &params.item_id,
                &params.native_process_id,
                params.max_wait_ms,
            )
            .await;
        Ok(AgentSessionStatusResponse {
            availability: AgentSessionAvailability::Available,
            phase: match status.phase {
                BackgroundTerminalStatusPhase::Running => AgentSessionPhase::Running,
                BackgroundTerminalStatusPhase::Exited => AgentSessionPhase::Exited,
                BackgroundTerminalStatusPhase::Unknown => AgentSessionPhase::Unknown,
            },
            output_bytes: status.output_bytes,
            output_sha256: status.output_sha256,
            output_cap_reached: status.output_cap_reached,
            exit_code: status.exit_code,
            terminal_generation: status.terminal_generation,
            observation_timed_out: status.observation_timed_out,
        })
    }

    async fn goal_blocker(
        &self,
        thread_id: ThreadId,
        binding: &AgentTaskGoalBinding,
    ) -> Result<Option<AgentTaskBlocker>, JSONRPCErrorError> {
        let read = self
            .goal_processor
            .agent_goal_read(AgentGoalReadParams {
                thread_id: thread_id.to_string(),
            })
            .await?;
        Ok(goal_binding_blocker(&read, binding))
    }

    async fn require_thread(
        &self,
        thread_id: ThreadId,
    ) -> Result<(Option<Arc<CodexThread>>, SessionSource, PathBuf), JSONRPCErrorError> {
        if let Ok(thread) = self.thread_manager.get_thread(thread_id).await {
            let snapshot = thread.config_snapshot().await;
            if snapshot.ephemeral {
                return Err(invalid_request(
                    "ephemeral thread cannot admit durable tasks",
                ));
            }
            let cwd = snapshot.cwd().to_path_buf();
            return Ok((Some(thread), snapshot.session_source, cwd));
        }
        let stored = self
            .thread_store
            .read_thread(ReadThreadParams {
                thread_id,
                include_archived: true,
                include_history: false,
            })
            .await
            .map_err(|error| match error {
                ThreadStoreError::ThreadNotFound { .. } => invalid_request("thread not found"),
                _ => internal_error("failed to read thread"),
            })?;
        if stored.archived_at.is_some() {
            return Err(invalid_request(
                "archived thread cannot admit durable tasks",
            ));
        }
        Ok((None, stored.source, stored.cwd))
    }

    async fn read_receipt(
        &self,
        connection_id: ConnectionId,
        params: &AgentTaskReceiptReadParams,
    ) -> Result<AgentTaskReceiptReadResponse, JSONRPCErrorError> {
        let thread_id = parse_thread_id(&params.thread_id)?;
        if !self.is_owner(connection_id, thread_id).await {
            return Ok(receipt_unavailable(&params.admission_id));
        }
        let Some(service) = self.queue_service.as_ref() else {
            return Ok(receipt_unavailable(&params.admission_id));
        };
        let durable_admission = service
            .find_admission(thread_id, &params.admission_id)
            .await
            .map_err(|_| internal_error("durable task receipt is temporarily unavailable"))?;
        if let Some(admission) = durable_admission.as_ref()
            && admission.terminal_outcome.as_deref() == Some("completed")
        {
            let goal_status = self.current_goal_status(thread_id).await;
            return Ok(receipt_from_admission(
                admission.clone(),
                goal_status,
                /*history_confirmed*/ true,
            ));
        }
        if let Some(admission) = durable_admission.as_ref()
            && matches!(
                admission.state,
                QueueAdmissionState::Pending | QueueAdmissionState::Blocked
            )
        {
            let goal_status = self.current_goal_status(thread_id).await;
            return Ok(receipt_from_admission(
                admission.clone(),
                goal_status,
                /*history_confirmed*/ true,
            ));
        }

        let stored = match self
            .thread_store
            .read_thread(ReadThreadParams {
                thread_id,
                include_archived: true,
                include_history: true,
            })
            .await
        {
            Ok(stored) => stored,
            Err(ThreadStoreError::ThreadNotFound { .. }) => {
                return Ok(receipt_unavailable(&params.admission_id));
            }
            Err(_) => return Ok(receipt_unavailable(&params.admission_id)),
        };

        let Some(history) = stored.history else {
            return Ok(durable_admission.map_or_else(
                || receipt_unknown(&params.admission_id),
                |admission| {
                    receipt_from_admission(admission, None, /*history_confirmed*/ false)
                },
            ));
        };
        let mut current_turn_id: Option<String> = None;
        let mut matched_turn_id: Option<String> = None;
        let mut terminal: Option<bool> = None;
        let mut interrupted = false;
        for item in history.items {
            let RolloutItem::EventMsg(event) = item else {
                continue;
            };
            match event {
                EventMsg::TurnStarted(event) => current_turn_id = Some(event.turn_id),
                EventMsg::UserMessage(event)
                    if event.client_id.as_deref() == Some(params.admission_id.as_str()) =>
                {
                    matched_turn_id = current_turn_id.clone();
                }
                EventMsg::TurnComplete(event)
                    if matched_turn_id.as_deref() == Some(event.turn_id.as_str()) =>
                {
                    terminal = Some(event.error.is_none());
                }
                EventMsg::TurnAborted(event)
                    if event
                        .turn_id
                        .as_deref()
                        .is_some_and(|turn_id| matched_turn_id.as_deref() == Some(turn_id)) =>
                {
                    interrupted = true;
                    terminal = Some(false);
                }
                _ => {}
            }
        }

        if matched_turn_id.is_none() {
            return Ok(durable_admission.map_or_else(
                || receipt_never_seen(&params.admission_id),
                |admission| {
                    receipt_from_admission(admission, None, /*history_confirmed*/ false)
                },
            ));
        }
        let (runtime_status, active_turn_id) = match self.thread_manager.get_thread(thread_id).await
        {
            Ok(thread) => (
                Some(thread.agent_status().await),
                thread.active_turn_id().await,
            ),
            Err(_) => (None, None),
        };
        let goal_read = self.current_goal_read(thread_id).await;
        let goal_status = goal_read
            .as_ref()
            .and_then(|read| read.goal.as_ref().map(|goal| goal.status));
        let is_goal_task = durable_admission
            .as_ref()
            .is_some_and(|admission| admission.dispatch_mode == "queue");
        let binding = durable_admission.as_ref().and_then(|admission| {
            serde_json::from_str::<AgentTaskGoalBinding>(&admission.goal_binding_json).ok()
        });
        // Control steers share the native turn but are not Goal tasks. Their
        // receipt is determined by their own client/admission history and
        // must not borrow the current Goal B after a task A continuation.
        let goal_identity_matches = !is_goal_task
            || binding.as_ref().is_some_and(|binding| {
                goal_read
                    .as_ref()
                    .is_some_and(|read| goal_identity_matches(read, binding))
            });
        let bound_goal_status = if is_goal_task && goal_identity_matches {
            goal_status
        } else {
            None
        };
        let active_turn_matches = active_turn_id.as_deref() == matched_turn_id.as_deref();

        // A successful native turn plus a matching completed Goal is only a
        // candidate until its exact historical mission proof is durable. Once
        // recorded, later Goal replacement and reconnects read the retained
        // proof instead of borrowing the current Goal B.
        let terminal_goal_match = is_goal_task
            && terminal == Some(true)
            && binding.is_some()
            && goal_identity_matches
            && bound_goal_status == Some(ThreadGoalStatus::Complete);
        if terminal_goal_match {
            let Some(admission) = durable_admission.as_ref() else {
                return Ok(receipt_unknown(&params.admission_id));
            };
            let Some(binding) = binding.as_ref() else {
                return Ok(receipt_unknown(&params.admission_id));
            };
            let Some(turn_id) = matched_turn_id.as_deref() else {
                return Ok(receipt_unknown(&params.admission_id));
            };
            let Some(goal_read) = goal_read.as_ref() else {
                return Ok(receipt_from_admission(
                    admission.clone(),
                    goal_status,
                    /*history_confirmed*/ false,
                ));
            };
            let Some(goal) = goal_read.goal.as_ref() else {
                return Ok(receipt_from_admission(
                    admission.clone(),
                    goal_status,
                    /*history_confirmed*/ false,
                ));
            };
            let Some(persisted) = service
                .record_admission_completed(
                    thread_id,
                    &params.admission_id,
                    turn_id,
                    &binding.goal_id,
                    binding.mission_revision,
                    goal_read
                        .lifecycle_revision
                        .unwrap_or(binding.lifecycle_revision),
                    &goal.objective,
                )
                .await
                .map_err(|_| internal_error("terminal task proof is temporarily unavailable"))?
            else {
                return Ok(receipt_unknown(&params.admission_id));
            };
            return Ok(
                if persisted.terminal_outcome.as_deref() == Some("completed") {
                    receipt_from_admission(persisted, goal_status, /*history_confirmed*/ true)
                } else {
                    receipt_from_admission(persisted, goal_status, /*history_confirmed*/ false)
                },
            );
        }
        let (outcome, turn_state, requires_reconciliation) = receipt_state(
            matched_turn_id.clone(),
            terminal,
            interrupted,
            runtime_status,
            active_turn_matches,
            bound_goal_status,
            goal_identity_matches,
        );
        Ok(AgentTaskReceiptReadResponse {
            admission_id: params.admission_id.clone(),
            outcome,
            observation: AgentTaskObservation::Durable,
            queue_id: durable_admission
                .as_ref()
                .map(|admission| admission.item.id.clone()),
            turn_id: matched_turn_id,
            acknowledged: true,
            requires_reconciliation,
            blocker: if is_goal_task && !goal_identity_matches {
                Some(AgentTaskBlocker::GoalChanged)
            } else {
                None
            },
            admission_state: AgentTaskAdmissionState::Dispatched,
            turn_state,
            goal_status: if is_goal_task { goal_status } else { None },
            ..empty_supervisor_receipt()
        })
    }

    async fn current_goal_read(
        &self,
        thread_id: ThreadId,
    ) -> Option<codex_app_server_protocol::AgentGoalReadResponse> {
        self.goal_processor
            .agent_goal_read(AgentGoalReadParams {
                thread_id: thread_id.to_string(),
            })
            .await
            .ok()
    }

    async fn current_goal_status(&self, thread_id: ThreadId) -> Option<ThreadGoalStatus> {
        self.current_goal_read(thread_id)
            .await
            .and_then(|read| read.goal.map(|goal| goal.status))
    }

    async fn is_loaded(&self, thread_id: ThreadId) -> bool {
        self.thread_manager.get_thread(thread_id).await.is_ok()
    }

    async fn is_owner(&self, connection_id: ConnectionId, thread_id: ThreadId) -> bool {
        self.thread_state_manager
            .subscribed_connection_ids(thread_id)
            .await
            .contains(&connection_id)
    }

    async fn inventory_entry(&self, thread: &StoredThread) -> AgentInventoryEntry {
        let loaded = self.thread_manager.get_thread(thread.thread_id).await.ok();
        let is_loaded = loaded.is_some();
        let status = match loaded {
            Some(thread) => runtime_status(thread.agent_status().await),
            None => AgentRuntimeStatus::Unknown,
        };
        AgentInventoryEntry {
            agent_id: thread.thread_id.to_string(),
            parent_thread_id: thread.parent_thread_id.map(|id| id.to_string()),
            agent_path: thread.agent_path.clone(),
            agent_nickname: thread.agent_nickname.clone(),
            agent_role: thread.agent_role.clone(),
            residency: if is_loaded {
                AgentRuntimeResidency::Loaded
            } else {
                AgentRuntimeResidency::Known
            },
            status,
        }
    }
}

fn goal_binding_blocker(
    read: &codex_app_server_protocol::AgentGoalReadResponse,
    binding: &AgentTaskGoalBinding,
) -> Option<AgentTaskBlocker> {
    match read.availability {
        AgentGoalAvailability::Unavailable => Some(AgentTaskBlocker::GoalUnavailable),
        AgentGoalAvailability::NotFound => Some(AgentTaskBlocker::GoalNotFound),
        AgentGoalAvailability::Available => match read.goal.as_ref() {
            None => Some(AgentTaskBlocker::GoalNotFound),
            Some(goal)
                if binding.status != ThreadGoalStatus::Active
                    || goal.status != ThreadGoalStatus::Active =>
            {
                Some(AgentTaskBlocker::GoalNotActive)
            }
            Some(goal)
                if read.goal_id.as_deref() != Some(binding.goal_id.as_str())
                    || read.mission_revision != Some(binding.mission_revision)
                    || goal.objective != binding.objective =>
            {
                Some(AgentTaskBlocker::GoalChanged)
            }
            Some(_) => None,
        },
    }
}

fn goal_identity_matches(
    read: &codex_app_server_protocol::AgentGoalReadResponse,
    binding: &AgentTaskGoalBinding,
) -> bool {
    read.availability == AgentGoalAvailability::Available
        && read.goal_id.as_deref() == Some(binding.goal_id.as_str())
        && read.mission_revision == Some(binding.mission_revision)
        && read
            .goal
            .as_ref()
            .is_some_and(|goal| goal.objective == binding.objective)
}

fn receipt_state(
    matched_turn_id: Option<String>,
    terminal: Option<bool>,
    interrupted: bool,
    runtime_status: Option<AgentStatus>,
    active_turn_matches: bool,
    bound_goal_status: Option<ThreadGoalStatus>,
    goal_identity_matches: bool,
) -> (AgentTaskOutcome, AgentTaskTurnState, bool) {
    let Some(_) = matched_turn_id else {
        return (AgentTaskOutcome::Unknown, AgentTaskTurnState::Unknown, true);
    };
    match terminal {
        Some(true) => match (goal_identity_matches, bound_goal_status) {
            // A replacement Goal cannot complete the old task. Preserve the
            // historical turn but require reconciliation against the bound
            // Goal identity rather than the current Goal row.
            (false, _) => (
                AgentTaskOutcome::Accepted,
                AgentTaskTurnState::Completed,
                true,
            ),
            (true, Some(ThreadGoalStatus::Complete)) => (
                AgentTaskOutcome::Completed,
                AgentTaskTurnState::Completed,
                false,
            ),
            (
                true,
                Some(
                    ThreadGoalStatus::Blocked
                    | ThreadGoalStatus::Paused
                    | ThreadGoalStatus::UsageLimited
                    | ThreadGoalStatus::BudgetLimited,
                ),
            ) => (AgentTaskOutcome::Failed, AgentTaskTurnState::Failed, true),
            // A successful turn is not the same as completion of an active
            // Goal; the Goal may require later turns.
            (true, _) => (
                AgentTaskOutcome::Accepted,
                AgentTaskTurnState::Completed,
                false,
            ),
        },
        Some(false) => (
            AgentTaskOutcome::Failed,
            if interrupted {
                AgentTaskTurnState::Interrupted
            } else {
                AgentTaskTurnState::Failed
            },
            true,
        ),
        None => match (active_turn_matches, runtime_status) {
            (false, Some(AgentStatus::Running | AgentStatus::PendingInit)) => {
                (AgentTaskOutcome::Unknown, AgentTaskTurnState::Unknown, true)
            }
            (true, Some(AgentStatus::Running | AgentStatus::PendingInit)) => (
                AgentTaskOutcome::Running,
                AgentTaskTurnState::Running,
                false,
            ),
            (_, Some(AgentStatus::Interrupted)) => (
                AgentTaskOutcome::Failed,
                AgentTaskTurnState::Interrupted,
                true,
            ),
            (_, Some(AgentStatus::Errored(_) | AgentStatus::Shutdown | AgentStatus::NotFound))
            | (_, None) => (AgentTaskOutcome::Unknown, AgentTaskTurnState::Unknown, true),
            (_, Some(AgentStatus::Completed(_))) => {
                (AgentTaskOutcome::Unknown, AgentTaskTurnState::Unknown, true)
            }
        },
    }
}

fn should_reuse_receipt(receipt: &AgentTaskReceiptReadResponse) -> bool {
    receipt.observation == AgentTaskObservation::Unavailable
        || (receipt.acknowledged
            && receipt.outcome != AgentTaskOutcome::Unknown
            && !receipt.requires_reconciliation)
}

fn parse_thread_id(raw: &str) -> Result<ThreadId, JSONRPCErrorError> {
    ThreadId::from_string(raw)
        .map_err(|error| invalid_request(format!("invalid thread id: {error}")))
}

fn runtime_status(status: AgentStatus) -> AgentRuntimeStatus {
    match status {
        AgentStatus::PendingInit => AgentRuntimeStatus::PendingInit,
        AgentStatus::Running => AgentRuntimeStatus::Running,
        AgentStatus::Interrupted => AgentRuntimeStatus::Interrupted,
        AgentStatus::Completed(_) => AgentRuntimeStatus::Completed,
        AgentStatus::Errored(_) => AgentRuntimeStatus::Errored,
        AgentStatus::Shutdown => AgentRuntimeStatus::Shutdown,
        AgentStatus::NotFound => AgentRuntimeStatus::Unknown,
    }
}

fn task_admit_blocked(admission_id: &str, blocker: AgentTaskBlocker) -> AgentTaskAdmitResponse {
    AgentTaskAdmitResponse {
        admission_id: admission_id.to_string(),
        outcome: AgentTaskOutcome::Unavailable,
        observation: AgentTaskObservation::Durable,
        queue_id: None,
        turn_id: None,
        acknowledged: false,
        requires_reconciliation: false,
        blocker: Some(blocker),
        admission_state: AgentTaskAdmissionState::NeverSeen,
        turn_state: AgentTaskTurnState::NotStarted,
        goal_status: None,
        ..empty_supervisor_admit()
    }
}

fn task_admit_unavailable(admission_id: &str) -> AgentTaskAdmitResponse {
    task_admit_blocked(admission_id, AgentTaskBlocker::QueueUnavailable)
}

fn task_admit_unknown(admission_id: &str) -> AgentTaskAdmitResponse {
    AgentTaskAdmitResponse {
        admission_id: admission_id.to_string(),
        outcome: AgentTaskOutcome::Unknown,
        observation: AgentTaskObservation::Durable,
        queue_id: None,
        turn_id: None,
        acknowledged: false,
        requires_reconciliation: true,
        blocker: None,
        admission_state: AgentTaskAdmissionState::Unknown,
        turn_state: AgentTaskTurnState::Unknown,
        goal_status: None,
        ..empty_supervisor_admit()
    }
}

fn receipt_unavailable(admission_id: &str) -> AgentTaskReceiptReadResponse {
    AgentTaskReceiptReadResponse {
        admission_id: admission_id.to_string(),
        outcome: AgentTaskOutcome::Unavailable,
        observation: AgentTaskObservation::Unavailable,
        queue_id: None,
        turn_id: None,
        acknowledged: false,
        requires_reconciliation: true,
        blocker: Some(AgentTaskBlocker::QueueUnavailable),
        admission_state: AgentTaskAdmissionState::Unavailable,
        turn_state: AgentTaskTurnState::Unknown,
        goal_status: None,
        ..empty_supervisor_receipt()
    }
}

fn receipt_unknown(admission_id: &str) -> AgentTaskReceiptReadResponse {
    AgentTaskReceiptReadResponse {
        admission_id: admission_id.to_string(),
        outcome: AgentTaskOutcome::Unknown,
        observation: AgentTaskObservation::Durable,
        queue_id: None,
        turn_id: None,
        acknowledged: false,
        requires_reconciliation: true,
        blocker: None,
        admission_state: AgentTaskAdmissionState::Unknown,
        turn_state: AgentTaskTurnState::Unknown,
        goal_status: None,
        ..empty_supervisor_receipt()
    }
}

fn receipt_never_seen(admission_id: &str) -> AgentTaskReceiptReadResponse {
    AgentTaskReceiptReadResponse {
        admission_id: admission_id.to_string(),
        outcome: AgentTaskOutcome::Unknown,
        observation: AgentTaskObservation::Durable,
        queue_id: None,
        turn_id: None,
        acknowledged: false,
        requires_reconciliation: false,
        blocker: None,
        admission_state: AgentTaskAdmissionState::NeverSeen,
        turn_state: AgentTaskTurnState::NotStarted,
        goal_status: None,
        ..empty_supervisor_receipt()
    }
}

fn receipt_from_admission(
    admission: QueuedAdmission,
    goal_status: Option<ThreadGoalStatus>,
    history_confirmed: bool,
) -> AgentTaskReceiptReadResponse {
    let admission_state_value = admission.state;
    let dispatch_turn_id = admission.dispatch_turn_id.clone();
    let terminal_completed = admission.terminal_outcome.as_deref() == Some("completed");
    let (admission_state, outcome, blocker, observation, requires_reconciliation) =
        if terminal_completed {
            (
                AgentTaskAdmissionState::Dispatched,
                AgentTaskOutcome::Completed,
                None,
                AgentTaskObservation::Durable,
                false,
            )
        } else {
            match admission_state_value {
                QueueAdmissionState::Pending => (
                    AgentTaskAdmissionState::Queued,
                    AgentTaskOutcome::Queued,
                    None,
                    AgentTaskObservation::Queue,
                    false,
                ),
                QueueAdmissionState::Starting => (
                    AgentTaskAdmissionState::Unknown,
                    AgentTaskOutcome::Unknown,
                    None,
                    AgentTaskObservation::Durable,
                    true,
                ),
                QueueAdmissionState::Dispatched => {
                    if !history_confirmed {
                        (
                            AgentTaskAdmissionState::Dispatched,
                            AgentTaskOutcome::Unknown,
                            None,
                            AgentTaskObservation::Durable,
                            true,
                        )
                    } else {
                        (
                            AgentTaskAdmissionState::Dispatched,
                            if dispatch_turn_id.is_some() {
                                AgentTaskOutcome::Accepted
                            } else {
                                AgentTaskOutcome::Unknown
                            },
                            None,
                            AgentTaskObservation::Durable,
                            dispatch_turn_id.is_none(),
                        )
                    }
                }
                QueueAdmissionState::Blocked => (
                    AgentTaskAdmissionState::Blocked,
                    AgentTaskOutcome::Unavailable,
                    admission.blocker.as_deref().and_then(parse_task_blocker),
                    AgentTaskObservation::Durable,
                    false,
                ),
            }
        };
    let turn_state = if terminal_completed {
        AgentTaskTurnState::Completed
    } else {
        match admission_state_value {
            QueueAdmissionState::Pending | QueueAdmissionState::Blocked => {
                AgentTaskTurnState::NotStarted
            }
            QueueAdmissionState::Dispatched if history_confirmed => AgentTaskTurnState::Running,
            QueueAdmissionState::Starting | QueueAdmissionState::Dispatched => {
                AgentTaskTurnState::Unknown
            }
        }
    };
    // A task receipt is historical evidence for its bound mission, not a
    // view of whichever Goal happens to be current after reconnect.  Once the
    // terminal proof is durable, expose the proven terminal state here too;
    // otherwise Goal B could make completed task A appear active again.
    let receipt_goal_status = if terminal_completed {
        Some(ThreadGoalStatus::Complete)
    } else {
        goal_status
    };
    AgentTaskReceiptReadResponse {
        admission_id: admission.admission_id,
        outcome,
        observation,
        queue_id: Some(admission.item.id),
        turn_id: admission.terminal_turn_id.or(dispatch_turn_id),
        acknowledged: true,
        requires_reconciliation,
        blocker,
        admission_state,
        turn_state,
        goal_status: receipt_goal_status,
        ..empty_supervisor_receipt()
    }
}

fn parse_task_blocker(value: &str) -> Option<AgentTaskBlocker> {
    Some(match value {
        "goal_unavailable" => AgentTaskBlocker::GoalUnavailable,
        "goal_not_found" => AgentTaskBlocker::GoalNotFound,
        "goal_not_active" => AgentTaskBlocker::GoalNotActive,
        "goal_changed" => AgentTaskBlocker::GoalChanged,
        _ => return None,
    })
}

fn empty_supervisor_admit() -> AgentTaskAdmitResponse {
    AgentTaskAdmitResponse {
        admission_id: String::new(),
        outcome: AgentTaskOutcome::Unknown,
        observation: AgentTaskObservation::Durable,
        queue_id: None,
        turn_id: None,
        acknowledged: false,
        requires_reconciliation: false,
        blocker: None,
        admission_state: AgentTaskAdmissionState::Unknown,
        turn_state: AgentTaskTurnState::Unknown,
        goal_status: None,
        supervisor_acknowledged: false,
        supervisor_ack_id: None,
        supervisor_next_actor: None,
        supervisor_wake_id: None,
    }
}

fn empty_supervisor_receipt() -> AgentTaskReceiptReadResponse {
    AgentTaskReceiptReadResponse {
        admission_id: String::new(),
        outcome: AgentTaskOutcome::Unknown,
        observation: AgentTaskObservation::Durable,
        queue_id: None,
        turn_id: None,
        acknowledged: false,
        requires_reconciliation: false,
        blocker: None,
        admission_state: AgentTaskAdmissionState::Unknown,
        turn_state: AgentTaskTurnState::Unknown,
        goal_status: None,
        supervisor_acknowledged: false,
        supervisor_ack_id: None,
        supervisor_next_actor: None,
        supervisor_wake_id: None,
    }
}

fn unavailable_session_status() -> AgentSessionStatusResponse {
    AgentSessionStatusResponse {
        availability: AgentSessionAvailability::Unavailable,
        phase: AgentSessionPhase::Unknown,
        output_bytes: 0,
        output_sha256: None,
        output_cap_reached: false,
        exit_code: None,
        terminal_generation: None,
        observation_timed_out: false,
    }
}

fn admit_response_from_receipt(value: AgentTaskReceiptReadResponse) -> AgentTaskAdmitResponse {
    AgentTaskAdmitResponse {
        admission_id: value.admission_id,
        outcome: value.outcome,
        observation: value.observation,
        queue_id: value.queue_id,
        turn_id: value.turn_id,
        acknowledged: value.acknowledged,
        requires_reconciliation: value.requires_reconciliation,
        blocker: value.blocker,
        admission_state: value.admission_state,
        turn_state: value.turn_state,
        goal_status: value.goal_status,
        supervisor_acknowledged: value.supervisor_acknowledged,
        supervisor_ack_id: value.supervisor_ack_id,
        supervisor_next_actor: value.supervisor_next_actor,
        supervisor_wake_id: value.supervisor_wake_id,
    }
}

fn admit_response_from_admission(admission: QueuedAdmission) -> AgentTaskAdmitResponse {
    let receipt = receipt_from_admission(admission, None, /*history_confirmed*/ true);
    admit_response_from_receipt(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::AgentGoalUnavailableReason;
    use codex_app_server_protocol::ThreadGoal;
    use codex_app_server_protocol::ThreadGoalStatus;

    fn goal(status: ThreadGoalStatus, objective: &str, updated_at: i64) -> ThreadGoal {
        ThreadGoal {
            thread_id: "00000000-0000-7000-8000-000000000001".to_string(),
            objective: objective.to_string(),
            status,
            token_budget: None,
            tokens_used: 0,
            time_used_seconds: 0,
            created_at: 1,
            updated_at,
        }
    }

    fn binding(
        status: ThreadGoalStatus,
        objective: &str,
        mission_revision: i64,
    ) -> AgentTaskGoalBinding {
        AgentTaskGoalBinding {
            goal_id: "goal-1".to_string(),
            mission_revision,
            lifecycle_revision: 7,
            objective: objective.to_string(),
            status,
        }
    }

    #[test]
    fn unrelated_complete_goal_is_not_reused_for_admission() {
        let read = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            goal: Some(goal(ThreadGoalStatus::Complete, "old job", 7)),
            goal_id: Some("goal-1".to_string()),
            mission_revision: Some(7),
            lifecycle_revision: Some(7),
            explicit_resume_required: false,
            unavailable_reason: None,
        };
        assert_eq!(
            goal_binding_blocker(&read, &binding(ThreadGoalStatus::Active, "new job", 8)),
            Some(AgentTaskBlocker::GoalNotActive)
        );
    }

    #[test]
    fn stale_blocked_goal_requires_explicit_resume_and_never_auto_activates() {
        let read = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            goal: Some(goal(ThreadGoalStatus::Blocked, "old maintenance", 7)),
            goal_id: Some("goal-1".to_string()),
            mission_revision: Some(7),
            lifecycle_revision: Some(7),
            explicit_resume_required: true,
            unavailable_reason: None,
        };
        assert_eq!(
            goal_binding_blocker(
                &read,
                &binding(ThreadGoalStatus::Blocked, "old maintenance", 7)
            ),
            Some(AgentTaskBlocker::GoalNotActive)
        );
        assert!(read.explicit_resume_required);
    }

    #[test]
    fn changed_goal_and_unavailable_state_fail_closed() {
        let changed = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            goal: Some(goal(ThreadGoalStatus::Active, "current", 9)),
            goal_id: Some("goal-1".to_string()),
            mission_revision: Some(9),
            lifecycle_revision: Some(9),
            explicit_resume_required: false,
            unavailable_reason: None,
        };
        assert_eq!(
            goal_binding_blocker(&changed, &binding(ThreadGoalStatus::Active, "current", 8)),
            Some(AgentTaskBlocker::GoalChanged)
        );

        let unavailable = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Unavailable,
            goal: None,
            goal_id: None,
            mission_revision: None,
            lifecycle_revision: None,
            explicit_resume_required: false,
            unavailable_reason: Some(AgentGoalUnavailableReason::StateUnavailable),
        };
        assert_eq!(
            goal_binding_blocker(
                &unavailable,
                &binding(ThreadGoalStatus::Active, "current", 9)
            ),
            Some(AgentTaskBlocker::GoalUnavailable)
        );
    }

    #[test]
    fn accounting_timestamp_does_not_change_goal_binding() {
        let accounted = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            // Usage accounting may advance this public timestamp without
            // changing the mission identity or lifecycle revision.
            goal: Some(goal(ThreadGoalStatus::Active, "same mission", 99)),
            goal_id: Some("goal-1".to_string()),
            mission_revision: Some(7),
            lifecycle_revision: Some(7),
            explicit_resume_required: false,
            unavailable_reason: None,
        };
        assert_eq!(
            goal_binding_blocker(
                &accounted,
                &binding(ThreadGoalStatus::Active, "same mission", 7)
            ),
            None
        );
    }

    #[test]
    fn same_mission_completion_allows_lifecycle_progress_but_edit_reconciles() {
        let completed = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            goal: Some(goal(ThreadGoalStatus::Complete, "same mission", 101)),
            goal_id: Some("goal-1".to_string()),
            mission_revision: Some(7),
            lifecycle_revision: Some(8),
            explicit_resume_required: false,
            unavailable_reason: None,
        };
        let admitted = binding(ThreadGoalStatus::Active, "same mission", 7);
        assert!(goal_identity_matches(&completed, &admitted));
        assert_eq!(
            receipt_state(
                Some("turn-1".to_string()),
                Some(true),
                false,
                Some(AgentStatus::Completed(None)),
                false,
                Some(ThreadGoalStatus::Complete),
                true,
            ),
            (
                AgentTaskOutcome::Completed,
                AgentTaskTurnState::Completed,
                false
            )
        );

        let accounted = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            goal: Some(goal(ThreadGoalStatus::Active, "same mission", 999)),
            goal_id: Some("goal-1".to_string()),
            mission_revision: Some(7),
            lifecycle_revision: Some(99),
            explicit_resume_required: false,
            unavailable_reason: None,
        };
        assert!(goal_identity_matches(&accounted, &admitted));

        let edited = codex_app_server_protocol::AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            goal: Some(goal(ThreadGoalStatus::Complete, "new mission", 102)),
            goal_id: Some("goal-1".to_string()),
            mission_revision: Some(8),
            lifecycle_revision: Some(9),
            explicit_resume_required: false,
            unavailable_reason: None,
        };
        assert!(!goal_identity_matches(&edited, &admitted));
        assert_eq!(
            receipt_state(
                Some("turn-1".to_string()),
                Some(true),
                false,
                Some(AgentStatus::Completed(None)),
                false,
                None,
                false,
            ),
            (
                AgentTaskOutcome::Accepted,
                AgentTaskTurnState::Completed,
                true
            )
        );
    }

    #[test]
    fn receipt_state_distinguishes_late_completion_from_unknown_submission() {
        assert_eq!(
            receipt_state(
                Some("turn".to_string()),
                Some(true),
                false,
                Some(AgentStatus::Completed(None)),
                true,
                Some(ThreadGoalStatus::Complete),
                true,
            ),
            (
                AgentTaskOutcome::Completed,
                AgentTaskTurnState::Completed,
                false
            )
        );
        assert_eq!(
            receipt_state(
                Some("turn".to_string()),
                Some(false),
                false,
                Some(AgentStatus::Completed(None)),
                true,
                Some(ThreadGoalStatus::Active),
                true,
            ),
            (AgentTaskOutcome::Failed, AgentTaskTurnState::Failed, true)
        );
        assert_eq!(
            receipt_state(
                Some("turn".to_string()),
                None,
                false,
                Some(AgentStatus::Running),
                true,
                Some(ThreadGoalStatus::Active),
                true,
            ),
            (
                AgentTaskOutcome::Running,
                AgentTaskTurnState::Running,
                false
            )
        );
        assert_eq!(
            receipt_state(
                None,
                None,
                false,
                Some(AgentStatus::Running),
                false,
                Some(ThreadGoalStatus::Active),
                true,
            ),
            (AgentTaskOutcome::Unknown, AgentTaskTurnState::Unknown, true)
        );
        // A same-row objective edit advances the lifecycle revision. A late
        // completion must not be attributed to the edited mission.
        assert_eq!(
            receipt_state(
                Some("turn".to_string()),
                Some(true),
                false,
                Some(AgentStatus::Completed(None)),
                false,
                None,
                false,
            ),
            (
                AgentTaskOutcome::Accepted,
                AgentTaskTurnState::Completed,
                true
            )
        );
    }

    #[test]
    fn unknown_receipt_is_not_replayed_but_acknowledged_receipt_is_reused() {
        let unknown = receipt_unavailable("admission");
        assert!(should_reuse_receipt(&unknown));

        let durable_unknown = AgentTaskReceiptReadResponse {
            admission_id: "admission".to_string(),
            outcome: AgentTaskOutcome::Unknown,
            observation: AgentTaskObservation::Durable,
            queue_id: None,
            turn_id: None,
            acknowledged: false,
            requires_reconciliation: true,
            blocker: None,
            admission_state: AgentTaskAdmissionState::Unknown,
            turn_state: AgentTaskTurnState::Unknown,
            goal_status: None,
            ..empty_supervisor_receipt()
        };
        assert!(!should_reuse_receipt(&durable_unknown));

        let queued = AgentTaskReceiptReadResponse {
            admission_id: "admission".to_string(),
            outcome: AgentTaskOutcome::Queued,
            observation: AgentTaskObservation::Queue,
            queue_id: Some("queue".to_string()),
            turn_id: None,
            acknowledged: true,
            requires_reconciliation: false,
            blocker: None,
            admission_state: AgentTaskAdmissionState::Queued,
            turn_state: AgentTaskTurnState::NotStarted,
            goal_status: None,
            ..empty_supervisor_receipt()
        };
        assert!(should_reuse_receipt(&queued));
    }

    #[test]
    fn dispatched_without_history_is_unknown_not_running() {
        let admission = QueuedAdmission {
            item: QueuedItem {
                id: "queue".to_string(),
                input: TurnInput::UserInput {
                    content: Vec::new(),
                    client_id: Some("admission".to_string()),
                },
            },
            admission_id: "admission".to_string(),
            goal_binding_json: "{}".to_string(),
            dispatch_mode: "queue".to_string(),
            expected_turn_id: None,
            state: QueueAdmissionState::Dispatched,
            blocker: None,
            dispatched_at_ms: Some(1),
            dispatch_attempt_at_ms: Some(1),
            dispatch_turn_id: Some("turn".to_string()),
            terminal_outcome: None,
            terminal_turn_id: None,
            terminal_goal_id: None,
            terminal_mission_revision: None,
            terminal_lifecycle_revision: None,
            terminal_objective: None,
            terminal_at_ms: None,
        };
        let receipt = receipt_from_admission(admission, None, /*history_confirmed*/ false);
        assert_eq!(AgentTaskOutcome::Unknown, receipt.outcome);
        assert_eq!(AgentTaskTurnState::Unknown, receipt.turn_state);
        assert!(receipt.requires_reconciliation);
    }

    #[test]
    fn retained_terminal_proof_wins_after_goal_replacement() {
        let admission = QueuedAdmission {
            item: QueuedItem {
                id: "queue".to_string(),
                input: TurnInput::UserInput {
                    content: Vec::new(),
                    client_id: Some("admission".to_string()),
                },
            },
            admission_id: "admission".to_string(),
            goal_binding_json: r#"{"goal_id":"goal-a","mission_revision":1,"objective":"mission A","status":"active"}"#.to_string(),
            dispatch_mode: "queue".to_string(),
            expected_turn_id: None,
            state: QueueAdmissionState::Dispatched,
            blocker: None,
            dispatched_at_ms: Some(1),
            dispatch_attempt_at_ms: Some(1),
            dispatch_turn_id: Some("turn-a".to_string()),
            terminal_outcome: Some("completed".to_string()),
            terminal_turn_id: Some("turn-a".to_string()),
            terminal_goal_id: Some("goal-a".to_string()),
            terminal_mission_revision: Some(1),
            terminal_lifecycle_revision: Some(2),
            terminal_objective: Some("mission A".to_string()),
            terminal_at_ms: Some(3),
        };
        let receipt = receipt_from_admission(
            admission,
            Some(ThreadGoalStatus::Active),
            /*history_confirmed*/ false,
        );
        assert_eq!(AgentTaskOutcome::Completed, receipt.outcome);
        assert_eq!(Some("turn-a".to_string()), receipt.turn_id);
        assert_eq!(AgentTaskTurnState::Completed, receipt.turn_state);
        assert_eq!(Some(ThreadGoalStatus::Complete), receipt.goal_status);
        assert!(!receipt.requires_reconciliation);
    }
}
