use super::thread_input::DIRECT_INPUT_TO_MULTI_AGENT_V2_SUBAGENT_ERROR;
use super::thread_input::can_accept_direct_input;
use super::thread_input::ensure_direct_input_allowed;
use super::*;
use crate::error_code::goal_conflict;
use codex_app_server_protocol::ThreadGoalMutationOrigin;
use codex_core::context::UserGoalUpdate;
use codex_goal_extension::GoalCas;
use codex_goal_extension::GoalDocumentCreateAdmissionState;
use codex_goal_extension::GoalDocumentCreatePrecondition;
use codex_goal_extension::GoalDocumentCreateRequest;
use codex_goal_extension::GoalDocumentSetRequest;
use codex_goal_extension::GoalObjectiveUpdate;
use codex_goal_extension::GoalService;
use codex_goal_extension::GoalServiceError;
use codex_goal_extension::GoalSetRequest;
use codex_goal_extension::GoalTokenBudgetUpdate;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_rollout::RolloutRecorder;
use codex_rollout::WriterLockCoordinator;

#[path = "thread_goal_user_context.rs"]
mod user_context;

enum GoalAccess {
    Read,
    Mutate,
}

#[derive(Clone)]
pub(crate) struct ThreadGoalRequestProcessor {
    thread_manager: Arc<ThreadManager>,
    outgoing: Arc<OutgoingMessageSender>,
    config: Arc<Config>,
    thread_state_manager: ThreadStateManager,
    state_db: Option<StateDbHandle>,
    goal_service: Arc<GoalService>,
    config_manager: ConfigManager,
    writer_locks: Arc<WriterLockCoordinator>,
}

impl ThreadGoalRequestProcessor {
    pub(crate) fn new(
        thread_manager: Arc<ThreadManager>,
        outgoing: Arc<OutgoingMessageSender>,
        config: Arc<Config>,
        thread_state_manager: ThreadStateManager,
        state_db: Option<StateDbHandle>,
        goal_service: Arc<GoalService>,
        config_manager: ConfigManager,
    ) -> Self {
        let writer_locks = Arc::new(WriterLockCoordinator::new(&config.codex_home));
        Self {
            thread_manager,
            outgoing,
            config,
            thread_state_manager,
            state_db,
            goal_service,
            config_manager,
            writer_locks,
        }
    }

    pub(crate) async fn thread_goal_set(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalSetParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_goal_set_inner(request_id, params)
            .await
            .map(|()| None)
    }

    pub(crate) async fn thread_goal_document_set(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalDocumentSetParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let _goal_resume_guard = self.thread_state_manager.lock_goal_resume(thread_id).await;
        let state_db = self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Mutate)
            .await?;
        self.reconcile_thread_goal_rollout(thread_id, &state_db)
            .await?;
        let existing_goal = self
            .goal_service
            .get_thread_goal(&state_db, thread_id)
            .await
            .map_err(goal_service_error)?;
        if existing_goal.is_none() {
            return Err(invalid_params(
                "thread/goal/document/set cannot create a Goal; use thread/goal/document/create with an explicit absent/noGoal precondition",
            ));
        }
        let expected_goal = goal_document_cas_from_params(
            params.expected_goal_id.clone(),
            params.expected_status.clone(),
            params.expected_mission_revision,
            params.expected_lifecycle_revision,
        )?;
        if expected_goal.is_none() {
            return Err(invalid_params(
                "thread/goal/document/set requires expectedGoalId, expectedStatus, expectedMissionRevision, and expectedLifecycleRevision for an existing Goal",
            ));
        }
        let resulting_status = params
            .status
            .map(ThreadGoalStatus::to_core)
            .or_else(|| existing_goal.as_ref().map(|goal| goal.status))
            .unwrap_or(codex_protocol::protocol::ThreadGoalStatus::Active);
        if resulting_status == codex_protocol::protocol::ThreadGoalStatus::Active
            && let Ok(thread) = self.thread_manager.get_thread(thread_id).await
        {
            self.config_manager
                .check_thread_model_provider(thread.config().await.as_ref())
                .await
                .map_err(|error| config_load_error(&error))?;
        }

        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        let outcome = self
            .goal_service
            .set_thread_goal_document(
                &state_db,
                GoalDocumentSetRequest {
                    thread_id,
                    document: params.document.as_str(),
                    status: params.status.map(ThreadGoalStatus::to_core),
                    expected_goal,
                },
            )
            .await
            .map_err(goal_service_error)?;
        let goal = ThreadGoal::from(outcome.goal());
        let document = api_thread_goal_document_from_state(outcome.document());
        let revision = api_goal_revision(outcome.identity());

        let persist_result = match self.thread_manager.get_thread(thread_id).await {
            Ok(thread) => match thread.rollout_path() {
                Some(path) if codex_rollout::existing_rollout_path(&path).await.is_none() => {
                    let persisted_settings = thread.thread_settings_snapshot().await;
                    let items = [
                        thread_settings_applied_item(thread_id, persisted_settings.clone()),
                        outcome.thread_goal_updated_item(),
                    ];
                    match thread.append_rollout_items(&items).await {
                        Err(err) => Err(err),
                        Ok(()) => {
                            let current_settings = thread.thread_settings_snapshot().await;
                            if current_settings == persisted_settings {
                                Ok(())
                            } else {
                                thread
                                    .append_rollout_items(&[thread_settings_applied_item(
                                        thread_id,
                                        current_settings,
                                    )])
                                    .await
                            }
                        }
                    }
                }
                Some(_) | None => {
                    thread
                        .append_rollout_items(&[outcome.thread_goal_updated_item()])
                        .await
                }
            },
            Err(_) => Ok(()),
        };
        if let Err(err) = persist_result {
            warn!(
                "failed to persist managed Goal document update for live thread {thread_id}: {err}"
            );
        }

        self.outgoing
            .send_response(
                request_id.clone(),
                ThreadGoalDocumentSetResponse {
                    goal: goal.clone(),
                    document,
                    revision,
                },
            )
            .await;
        self.emit_thread_goal_updated_ordered(thread_id, goal, listener_command_tx)
            .await;
        outcome.apply_runtime_effects(&self.goal_service).await;
        Ok(None)
    }

    pub(crate) async fn thread_goal_document_create(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalDocumentCreateParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let _goal_resume_guard = self.thread_state_manager.lock_goal_resume(thread_id).await;
        let precondition = match &params.precondition {
            ThreadGoalDocumentCreatePrecondition::Absent { admission_state } => {
                match admission_state {
                    ThreadGoalDocumentCreateAdmissionState::NoGoal => {
                        GoalDocumentCreatePrecondition::Absent {
                            admission_state: GoalDocumentCreateAdmissionState::NoGoal,
                        }
                    }
                }
            }
            ThreadGoalDocumentCreatePrecondition::Completed {
                expected_goal_id,
                expected_status,
                expected_mission_revision,
                expected_lifecycle_revision,
            } => {
                if expected_goal_id.trim().is_empty() || expected_goal_id.len() > 256 {
                    return Err(invalid_params("expectedGoalId is invalid"));
                }
                if *expected_status != ThreadGoalStatus::Complete {
                    return Err(invalid_params(
                        "thread/goal/document/create completed precondition requires expectedStatus=complete",
                    ));
                }
                GoalDocumentCreatePrecondition::Completed(GoalCas {
                    goal_id: expected_goal_id.clone(),
                    status: expected_status.to_core(),
                    mission_revision: *expected_mission_revision,
                    lifecycle_revision: *expected_lifecycle_revision,
                })
            }
        };
        let state_db = self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Mutate)
            .await?;
        self.reconcile_thread_goal_rollout(thread_id, &state_db)
            .await?;

        let resulting_status = params
            .status
            .map(ThreadGoalStatus::to_core)
            .unwrap_or(codex_protocol::protocol::ThreadGoalStatus::Active);
        if resulting_status == codex_protocol::protocol::ThreadGoalStatus::Active
            && let Ok(thread) = self.thread_manager.get_thread(thread_id).await
        {
            self.config_manager
                .check_thread_model_provider(thread.config().await.as_ref())
                .await
                .map_err(|error| config_load_error(&error))?;
        }

        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        let max_goal_token_budget = match self.thread_manager.get_thread(thread_id).await {
            Ok(thread) => thread.config().await.max_goal_token_budget,
            Err(_) => self.config.max_goal_token_budget,
        };
        let outcome = self
            .goal_service
            .create_thread_goal_document(
                &state_db,
                GoalDocumentCreateRequest {
                    thread_id,
                    creation_id: params.creation_id.as_str(),
                    document: params.document.as_str(),
                    objective: params.objective.as_str(),
                    status: params.status.map(ThreadGoalStatus::to_core),
                    token_budget: params.token_budget,
                    max_goal_token_budget,
                    precondition,
                },
            )
            .await
            .map_err(goal_service_error)?;
        let goal = ThreadGoal::from(outcome.goal());
        let document = api_thread_goal_document_from_state(outcome.document());
        let previous_revision = outcome.previous_identity().map(api_goal_revision);
        let revision = api_goal_revision(outcome.identity());
        let replayed = outcome.replayed();

        if !replayed {
            let persist_result = match self.thread_manager.get_thread(thread_id).await {
                Ok(thread) => match thread.rollout_path() {
                    Some(path) if codex_rollout::existing_rollout_path(&path).await.is_none() => {
                        let persisted_settings = thread.thread_settings_snapshot().await;
                        let items = [
                            thread_settings_applied_item(thread_id, persisted_settings.clone()),
                            outcome.thread_goal_updated_item(),
                        ];
                        match thread.append_rollout_items(&items).await {
                            Err(err) => Err(err),
                            Ok(()) => {
                                let current_settings = thread.thread_settings_snapshot().await;
                                if current_settings == persisted_settings {
                                    Ok(())
                                } else {
                                    thread
                                        .append_rollout_items(&[thread_settings_applied_item(
                                            thread_id,
                                            current_settings,
                                        )])
                                        .await
                                }
                            }
                        }
                    }
                    Some(_) | None => {
                        thread
                            .append_rollout_items(&[outcome.thread_goal_updated_item()])
                            .await
                    }
                },
                Err(_) => Ok(()),
            };
            if let Err(err) = persist_result {
                warn!(
                    "failed to persist new managed Goal document for live thread {thread_id}: {err}"
                );
            }
        }

        self.outgoing
            .send_response(
                request_id.clone(),
                ThreadGoalDocumentCreateResponse {
                    creation_id: outcome.creation_id().to_string(),
                    goal: goal.clone(),
                    document,
                    previous_revision,
                    revision,
                    replayed,
                },
            )
            .await;
        if !replayed {
            self.emit_thread_goal_updated_ordered(thread_id, goal, listener_command_tx)
                .await;
            outcome.apply_runtime_effects(&self.goal_service).await;
        }
        Ok(None)
    }

    pub(crate) async fn thread_goal_get(
        &self,
        params: ThreadGoalGetParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_goal_get_inner(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn thread_goal_document_get(
        &self,
        params: ThreadGoalDocumentGetParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }
        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let state_db = self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Read)
            .await?;
        let goal = self
            .goal_service
            .get_thread_goal(&state_db, thread_id)
            .await
            .map_err(goal_service_error)?
            .map(ThreadGoal::from);
        let document = self
            .goal_service
            .read_thread_goal_document(&state_db, thread_id)
            .await
            .map_err(goal_service_error)?
            .map(|(_, document, content)| {
                (api_thread_goal_document_from_state(document), Some(content))
            });
        let (document, content) = document
            .map(|(document, content)| (Some(document), content))
            .unwrap_or((None, None));
        Ok(Some(
            ThreadGoalDocumentGetResponse {
                goal,
                document,
                content,
            }
            .into(),
        ))
    }

    /// Read a goal for the content-free agent contract without applying the
    /// legacy `Option<ThreadGoal>` projection.  In particular, an unavailable
    /// state database and an absent goal are different results, and this read
    /// never resumes or otherwise changes the goal runtime.
    pub(crate) async fn agent_goal_read(
        &self,
        params: AgentGoalReadParams,
    ) -> Result<AgentGoalReadResponse, JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Ok(AgentGoalReadResponse {
                availability: AgentGoalAvailability::Unavailable,
                goal: None,
                goal_id: None,
                mission_revision: None,
                lifecycle_revision: None,
                explicit_resume_required: false,
                unavailable_reason: Some(AgentGoalUnavailableReason::GoalsDisabled),
            });
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let state_db = match self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Read)
            .await
        {
            Ok(state_db) => state_db,
            Err(_) => {
                if self
                    .thread_manager
                    .get_thread(thread_id)
                    .await
                    .is_ok_and(|thread| thread.rollout_path().is_none())
                {
                    return Ok(AgentGoalReadResponse {
                        availability: AgentGoalAvailability::Unavailable,
                        goal: None,
                        goal_id: None,
                        mission_revision: None,
                        lifecycle_revision: None,
                        explicit_resume_required: false,
                        unavailable_reason: Some(AgentGoalUnavailableReason::EphemeralThread),
                    });
                }

                let persisted = codex_rollout::find_thread_path_by_id_str(
                    &self.config.codex_home,
                    &thread_id.to_string(),
                    self.state_db.as_deref(),
                )
                .await
                .ok()
                .flatten();
                return Ok(match persisted {
                    Some(_) => AgentGoalReadResponse {
                        availability: AgentGoalAvailability::Unavailable,
                        goal: None,
                        goal_id: None,
                        mission_revision: None,
                        lifecycle_revision: None,
                        explicit_resume_required: false,
                        unavailable_reason: Some(AgentGoalUnavailableReason::StateUnavailable),
                    },
                    None => AgentGoalReadResponse {
                        availability: AgentGoalAvailability::NotFound,
                        goal: None,
                        goal_id: None,
                        mission_revision: None,
                        lifecycle_revision: None,
                        explicit_resume_required: false,
                        unavailable_reason: None,
                    },
                });
            }
        };

        let goal = match state_db.thread_goals().get_thread_goal(thread_id).await {
            Ok(goal) => goal,
            Err(_) => {
                return Ok(AgentGoalReadResponse {
                    availability: AgentGoalAvailability::Unavailable,
                    goal: None,
                    goal_id: None,
                    mission_revision: None,
                    lifecycle_revision: None,
                    explicit_resume_required: false,
                    unavailable_reason: Some(AgentGoalUnavailableReason::StateUnavailable),
                });
            }
        };
        let goal_id = goal.as_ref().map(|goal| goal.goal_id.clone());
        let mission_revision = goal.as_ref().map(|goal| goal.mission_revision);
        let lifecycle_revision = goal.as_ref().map(|goal| goal.lifecycle_revision);
        let goal = goal.map(api_thread_goal_from_state);
        let explicit_resume_required = goal.as_ref().is_some_and(|goal| {
            matches!(
                goal.status,
                ThreadGoalStatus::Paused | ThreadGoalStatus::Blocked
            )
        });
        Ok(AgentGoalReadResponse {
            availability: AgentGoalAvailability::Available,
            goal,
            goal_id,
            mission_revision,
            lifecycle_revision,
            explicit_resume_required,
            unavailable_reason: None,
        })
    }

    pub(crate) async fn thread_goal_clear(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalClearParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.thread_goal_clear_inner(request_id, params)
            .await
            .map(|()| None)
    }

    pub(crate) async fn emit_resume_goal_snapshot(&self, thread_id: ThreadId) {
        if !self.config.features.enabled(Feature::Goals) {
            return;
        }
        self.emit_thread_goal_snapshot(thread_id).await;
    }

    pub(crate) async fn pending_resume_goal_state(
        &self,
        thread: &CodexThread,
    ) -> (bool, Option<StateDbHandle>) {
        let emit_thread_goal_update = self.config.features.enabled(Feature::Goals);
        let thread_goal_state_db = if emit_thread_goal_update {
            if let Some(state_db) = thread.state_db() {
                Some(state_db)
            } else {
                self.state_db.clone()
            }
        } else {
            None
        };
        (emit_thread_goal_update, thread_goal_state_db)
    }

    pub(crate) async fn restore_inherited_goal_runtime(&self, thread_id: ThreadId) {
        if let Err(err) = self
            .goal_service
            .restore_thread_runtime_after_resume(thread_id)
            .await
        {
            warn!("failed to restore inherited goal runtime for {thread_id}: {err}");
        }
    }

    pub(crate) async fn flush_goal_progress_for_fork(
        &self,
        thread_id: ThreadId,
    ) -> Result<(), String> {
        self.goal_service
            .flush_thread_goal_progress_for_fork(thread_id)
            .await
            .map_err(|err| err.to_string())
    }

    async fn thread_goal_set_inner(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalSetParams,
    ) -> Result<(), JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let _goal_resume_guard = self.thread_state_manager.lock_goal_resume(thread_id).await;
        let expected = goal_cas_from_params(
            params.expected_goal_id.clone(),
            params.expected_status.clone(),
            params.expected_mission_revision,
            params.expected_lifecycle_revision,
        )?;
        let state_db = self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Mutate)
            .await?;
        self.reconcile_thread_goal_rollout(thread_id, &state_db)
            .await?;
        // Active goals can immediately inject an objective or start an idle turn.
        // Stopping a goal must remain possible after managed policy changes.
        let existing_goal = self
            .goal_service
            .get_thread_goal(&state_db, thread_id)
            .await
            .map_err(goal_service_error)?;
        if existing_goal.is_some() && expected.is_none() {
            return Err(invalid_params(
                "thread/goal/set requires expectedGoalId, expectedStatus, expectedMissionRevision, and expectedLifecycleRevision for an existing Goal",
            ));
        }
        let resulting_status = params
            .status
            .map(ThreadGoalStatus::to_core)
            .or_else(|| existing_goal.as_ref().map(|goal| goal.status))
            .unwrap_or(codex_protocol::protocol::ThreadGoalStatus::Active);
        if resulting_status == codex_protocol::protocol::ThreadGoalStatus::Active
            && let Ok(thread) = self.thread_manager.get_thread(thread_id).await
        {
            self.config_manager
                .check_thread_model_provider(thread.config().await.as_ref())
                .await
                .map_err(|error| config_load_error(&error))?;
        }
        let max_goal_token_budget = match self.thread_manager.get_thread(thread_id).await {
            Ok(thread) => thread.config().await.max_goal_token_budget,
            Err(_) => self.config.max_goal_token_budget,
        };

        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        let status = params.status.map(ThreadGoalStatus::to_core);
        let objective = params.objective.as_deref().map(str::trim);

        let goal_request = GoalSetRequest {
            thread_id,
            objective: objective
                .map(GoalObjectiveUpdate::Set)
                .unwrap_or(GoalObjectiveUpdate::Keep),
            status,
            token_budget: match params.token_budget {
                Some(token_budget) => GoalTokenBudgetUpdate::Set(token_budget),
                None => GoalTokenBudgetUpdate::Keep,
            },
            max_goal_token_budget,
        };
        let record_user_instruction = async {
            if params.origin == Some(ThreadGoalMutationOrigin::User)
                && (objective.is_some() || status.is_some())
            {
                self.record_user_goal_update(
                    thread_id,
                    UserGoalUpdate::Set {
                        objective: objective.map(str::to_owned),
                        status,
                    },
                )
                .await?;
            }
            Ok(())
        };
        let outcome = match expected {
            Some(expected) => {
                self.goal_service
                    .set_thread_goal_cas_with_user_instruction(
                        &state_db,
                        goal_request,
                        expected,
                        record_user_instruction,
                    )
                    .await
            }
            None => {
                self.goal_service
                    .set_thread_goal(&state_db, goal_request, record_user_instruction)
                    .await
            }
        }
        .map_err(goal_service_error)?;
        let goal = ThreadGoal::from(outcome.goal.clone());
        let revision = api_goal_revision(outcome.identity());

        let persist_result = match self.thread_manager.get_thread(thread_id).await {
            Ok(thread) => match thread.rollout_path() {
                Some(path) if codex_rollout::existing_rollout_path(&path).await.is_none() => {
                    // The settings and Goal must be the first atomic append for Goal-first threads.
                    let persisted_settings = thread.thread_settings_snapshot().await;
                    let items = [
                        thread_settings_applied_item(thread_id, persisted_settings.clone()),
                        outcome.thread_goal_updated_item(),
                    ];
                    match thread.append_rollout_items(&items).await {
                        Err(err) => Err(err),
                        Ok(()) => {
                            // Catch up a concurrent settings update after materialization.
                            let current_settings = thread.thread_settings_snapshot().await;
                            if current_settings == persisted_settings {
                                Ok(())
                            } else {
                                thread
                                    .append_rollout_items(&[thread_settings_applied_item(
                                        thread_id,
                                        current_settings,
                                    )])
                                    .await
                            }
                        }
                    }
                }
                Some(_) | None => {
                    thread
                        .append_rollout_items(&[outcome.thread_goal_updated_item()])
                        .await
                }
            },
            Err(_) => Ok(()),
        };
        if let Err(err) = persist_result {
            warn!("failed to persist goal update for live thread {thread_id}: {err}");
        }

        self.outgoing
            .send_response(
                request_id.clone(),
                ThreadGoalSetResponse {
                    goal: goal.clone(),
                    revision,
                },
            )
            .await;
        self.emit_thread_goal_updated_ordered(thread_id, goal, listener_command_tx)
            .await;
        outcome.apply_runtime_effects(&self.goal_service).await;
        Ok(())
    }

    async fn thread_goal_get_inner(
        &self,
        params: ThreadGoalGetParams,
    ) -> Result<ThreadGoalGetResponse, JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }

        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let state_db = self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Read)
            .await?;
        let state_goal = state_db
            .thread_goals()
            .get_thread_goal(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to read thread goal: {err}")))?;
        let revision = state_goal
            .as_ref()
            .map(|goal| api_goal_revision(goal.into()));
        let goal = state_goal.map(api_thread_goal_from_state);
        Ok(ThreadGoalGetResponse { goal, revision })
    }

    async fn thread_goal_clear_inner(
        &self,
        request_id: ConnectionRequestId,
        params: ThreadGoalClearParams,
    ) -> Result<(), JSONRPCErrorError> {
        if !self.config.features.enabled(Feature::Goals) {
            return Err(invalid_request("goals feature is disabled"));
        }
        let thread_id = parse_thread_id_for_request(params.thread_id.as_str())?;
        let _goal_resume_guard = self.thread_state_manager.lock_goal_resume(thread_id).await;
        let expected = goal_cas_from_params(
            params.expected_goal_id.clone(),
            params.expected_status.clone(),
            params.expected_mission_revision,
            params.expected_lifecycle_revision,
        )?;
        let state_db = self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Mutate)
            .await?;
        self.reconcile_thread_goal_rollout(thread_id, &state_db)
            .await?;

        let prior = state_db
            .thread_goals()
            .get_thread_goal(thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to read thread goal: {err}")))?;
        if prior.is_some() && expected.is_none() {
            return Err(invalid_params(
                "thread/goal/clear requires expectedGoalId, expectedStatus, expectedMissionRevision, and expectedLifecycleRevision for an existing Goal",
            ));
        }

        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        let (cleared, revision) = if let Some(expected) = expected {
            let goal = self
                .goal_service
                .clear_thread_goal_cas(&state_db, thread_id, expected)
                .await
                .map_err(goal_service_error)?;
            (true, Some(api_goal_revision((&goal).into())))
        } else {
            let cleared = self
                .goal_service
                .clear_thread_goal(&state_db, thread_id, async { Ok(()) })
                .await
                .map_err(goal_service_error)?;
            (
                cleared,
                prior.as_ref().map(|goal| api_goal_revision(goal.into())),
            )
        };

        self.outgoing
            .send_response(request_id, ThreadGoalClearResponse { cleared, revision })
            .await;
        if cleared {
            self.emit_thread_goal_cleared_ordered(thread_id, listener_command_tx)
                .await;
        }
        Ok(())
    }

    async fn state_db_for_materialized_thread(
        &self,
        thread_id: ThreadId,
        access: GoalAccess,
    ) -> Result<StateDbHandle, JSONRPCErrorError> {
        if let Ok(thread) = self.thread_manager.get_thread(thread_id).await {
            if matches!(access, GoalAccess::Mutate) {
                ensure_direct_input_allowed(thread.as_ref()).await?;
            }
            if thread.rollout_path().is_none() {
                return Err(invalid_request(format!(
                    "ephemeral thread does not support goals: {thread_id}"
                )));
            }
            if let Some(state_db) = thread.state_db() {
                return Ok(state_db);
            }
        } else {
            let rollout_path = codex_rollout::find_thread_path_by_id_str(
                &self.config.codex_home,
                &thread_id.to_string(),
                self.state_db.as_deref(),
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to locate thread id {thread_id}: {err}"))
            })?
            .ok_or_else(|| invalid_request(format!("thread not found: {thread_id}")))?;
            if matches!(access, GoalAccess::Mutate) {
                let session_meta = codex_rollout::read_session_meta_line(&rollout_path)
                    .await
                    .map_err(|err| {
                        internal_error(format!("failed to read thread ownership: {err}"))
                    })?;
                if session_meta.meta.id != thread_id {
                    return Err(invalid_request(
                        "thread metadata does not match requested id",
                    ));
                }
                if matches!(
                    session_meta.meta.source,
                    SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })
                ) {
                    // Match resume's latest version metadata, including legacy TurnContext
                    // fallback, rather than trusting only the initial session header.
                    let history = RolloutRecorder::get_rollout_history(&rollout_path)
                        .await
                        .map_err(|err| {
                            internal_error(format!("failed to read thread ownership: {err}"))
                        })?;
                    if !can_accept_direct_input(
                        history.get_multi_agent_version(),
                        &session_meta.meta.source,
                    ) {
                        return Err(invalid_request(
                            DIRECT_INPUT_TO_MULTI_AGENT_V2_SUBAGENT_ERROR,
                        ));
                    }
                }
            }
        }

        self.state_db
            .clone()
            .ok_or_else(|| internal_error("sqlite state db unavailable for thread goals"))
    }

    async fn reconcile_thread_goal_rollout(
        &self,
        thread_id: ThreadId,
        state_db: &StateDbHandle,
    ) -> Result<(), JSONRPCErrorError> {
        let running_thread = self.thread_manager.get_thread(thread_id).await.ok();
        let rollout_path = match running_thread.as_ref() {
            Some(thread) => thread.rollout_path().ok_or_else(|| {
                invalid_request(format!(
                    "ephemeral thread does not support goals: {thread_id}"
                ))
            })?,
            None => codex_rollout::find_thread_path_by_id_str(
                &self.config.codex_home,
                &thread_id.to_string(),
                self.state_db.as_deref(),
            )
            .await
            .map_err(|err| {
                internal_error(format!("failed to locate thread id {thread_id}: {err}"))
            })?
            .ok_or_else(|| invalid_request(format!("thread not found: {thread_id}")))?,
        };

        if let Ok(Some(metadata)) = state_db.get_thread(thread_id).await
            && codex_rollout::plain_rollout_path(metadata.rollout_path.as_path())
                == codex_rollout::plain_rollout_path(rollout_path.as_path())
            && let Some(existing_path) =
                codex_rollout::existing_rollout_path(metadata.rollout_path.as_path()).await
            && codex_rollout::read_session_meta_line(existing_path.as_path())
                .await
                .is_ok_and(|session_meta| session_meta.meta.id == thread_id)
        {
            return Ok(());
        }

        reconcile_rollout(
            Some(state_db),
            rollout_path.as_path(),
            self.config.model_provider_id.as_str(),
            /*builder*/ None,
            &[],
            /*archived_only*/ None,
            /*new_thread_memory_mode*/ None,
        )
        .await;
        Ok(())
    }

    pub(crate) async fn emit_thread_goal_snapshot(&self, thread_id: ThreadId) {
        let state_db = match self
            .state_db_for_materialized_thread(thread_id, GoalAccess::Read)
            .await
        {
            Ok(state_db) => state_db,
            Err(err) => {
                warn!(
                    "failed to open state db before emitting thread goal resume snapshot for {thread_id}: {}",
                    err.message
                );
                return;
            }
        };
        let listener_command_tx = {
            let thread_state = self.thread_state_manager.thread_state(thread_id).await;
            let thread_state = thread_state.lock().await;
            thread_state.listener_command_tx()
        };
        if let Some(listener_command_tx) = listener_command_tx {
            let command = crate::thread_state::ThreadListenerCommand::EmitThreadGoalSnapshot {
                state_db: state_db.clone(),
            };
            if listener_command_tx.send(command).is_ok() {
                return;
            }
            warn!(
                "failed to enqueue thread goal snapshot for {thread_id}: listener command channel is closed"
            );
        }
        send_thread_goal_snapshot_notification(&self.outgoing, thread_id, &state_db).await;
    }

    async fn emit_thread_goal_updated_ordered(
        &self,
        thread_id: ThreadId,
        goal: ThreadGoal,
        listener_command_tx: Option<tokio::sync::mpsc::UnboundedSender<ThreadListenerCommand>>,
    ) {
        if let Some(listener_command_tx) = listener_command_tx {
            let command = crate::thread_state::ThreadListenerCommand::EmitThreadGoalUpdated {
                turn_id: None,
                goal: goal.clone(),
            };
            if listener_command_tx.send(command).is_ok() {
                return;
            }
            warn!(
                "failed to enqueue thread goal update for {thread_id}: listener command channel is closed"
            );
        }
        self.outgoing
            .send_server_notification(ServerNotification::ThreadGoalUpdated(
                ThreadGoalUpdatedNotification {
                    thread_id: thread_id.to_string(),
                    turn_id: None,
                    goal,
                },
            ))
            .await;
    }

    async fn emit_thread_goal_cleared_ordered(
        &self,
        thread_id: ThreadId,
        listener_command_tx: Option<tokio::sync::mpsc::UnboundedSender<ThreadListenerCommand>>,
    ) {
        if let Some(listener_command_tx) = listener_command_tx {
            let command = crate::thread_state::ThreadListenerCommand::EmitThreadGoalCleared;
            if listener_command_tx.send(command).is_ok() {
                return;
            }
            warn!(
                "failed to enqueue thread goal clear for {thread_id}: listener command channel is closed"
            );
        }
        self.outgoing
            .send_server_notification(ServerNotification::ThreadGoalCleared(
                ThreadGoalClearedNotification {
                    thread_id: thread_id.to_string(),
                },
            ))
            .await;
    }
}

pub(super) fn api_thread_goal_from_state(goal: codex_state::ThreadGoal) -> ThreadGoal {
    ThreadGoal {
        thread_id: goal.thread_id.to_string(),
        objective: goal.objective,
        status: api_thread_goal_status_from_state(goal.status),
        token_budget: goal.token_budget,
        tokens_used: goal.tokens_used,
        time_used_seconds: goal.time_used_seconds,
        created_at: goal.created_at.timestamp(),
        updated_at: goal.updated_at.timestamp(),
    }
}

fn api_thread_goal_document_from_state(
    document: codex_state::ThreadGoalDocument,
) -> ThreadGoalDocument {
    ThreadGoalDocument {
        path: document.path,
        sha256: document.sha256,
        byte_length: document.byte_length,
        mission_revision: document.mission_revision,
    }
}

fn api_thread_goal_status_from_state(status: codex_state::ThreadGoalStatus) -> ThreadGoalStatus {
    match status {
        codex_state::ThreadGoalStatus::Active => ThreadGoalStatus::Active,
        codex_state::ThreadGoalStatus::Paused => ThreadGoalStatus::Paused,
        codex_state::ThreadGoalStatus::Blocked => ThreadGoalStatus::Blocked,
        codex_state::ThreadGoalStatus::UsageLimited => ThreadGoalStatus::UsageLimited,
        codex_state::ThreadGoalStatus::BudgetLimited => ThreadGoalStatus::BudgetLimited,
        codex_state::ThreadGoalStatus::Complete => ThreadGoalStatus::Complete,
    }
}

fn api_goal_revision(identity: codex_goal_extension::GoalIdentity) -> ThreadGoalRevision {
    ThreadGoalRevision {
        goal_id: identity.goal_id,
        status: identity.status.into(),
        mission_revision: identity.mission_revision,
        lifecycle_revision: identity.lifecycle_revision,
    }
}

fn goal_cas_from_params(
    expected_goal_id: Option<String>,
    expected_status: Option<ThreadGoalStatus>,
    expected_mission_revision: Option<i64>,
    expected_lifecycle_revision: Option<i64>,
) -> Result<Option<GoalCas>, JSONRPCErrorError> {
    let present = [
        expected_goal_id.is_some(),
        expected_status.is_some(),
        expected_mission_revision.is_some(),
        expected_lifecycle_revision.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if present == 0 {
        return Ok(None);
    }
    if present != 4 {
        return Err(invalid_params(
            "expectedGoalId, expectedStatus, expectedMissionRevision, and expectedLifecycleRevision must be supplied together",
        ));
    }
    let goal_id = expected_goal_id.expect("Goal CAS field count checked");
    if goal_id.trim().is_empty() || goal_id.len() > 256 {
        return Err(invalid_params("expectedGoalId is invalid"));
    }
    Ok(Some(GoalCas {
        goal_id,
        status: expected_status
            .expect("Goal CAS field count checked")
            .to_core(),
        mission_revision: expected_mission_revision.expect("Goal CAS field count checked"),
        lifecycle_revision: expected_lifecycle_revision.expect("Goal CAS field count checked"),
    }))
}

fn goal_document_cas_from_params(
    expected_goal_id: Option<String>,
    expected_status: Option<ThreadGoalStatus>,
    expected_mission_revision: Option<i64>,
    expected_lifecycle_revision: Option<i64>,
) -> Result<Option<GoalCas>, JSONRPCErrorError> {
    let present = [
        expected_goal_id.is_some(),
        expected_status.is_some(),
        expected_mission_revision.is_some(),
        expected_lifecycle_revision.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if present == 0 {
        return Ok(None);
    }
    if present != 4 {
        return Err(invalid_params(
            "document Goal CAS requires expectedGoalId, expectedStatus, expectedMissionRevision, and expectedLifecycleRevision together",
        ));
    }
    let goal_id = expected_goal_id.expect("document Goal CAS field count checked");
    if goal_id.trim().is_empty() || goal_id.len() > 256 {
        return Err(invalid_params("expectedGoalId is invalid"));
    }
    Ok(Some(GoalCas {
        goal_id,
        status: expected_status
            .expect("document Goal CAS field count checked")
            .to_core(),
        mission_revision: expected_mission_revision.expect("document Goal CAS field checked"),
        lifecycle_revision: expected_lifecycle_revision
            .expect("document Goal CAS field count checked"),
    }))
}

fn goal_service_error(err: GoalServiceError) -> JSONRPCErrorError {
    match err {
        GoalServiceError::InvalidRequest(message) => invalid_request(message),
        GoalServiceError::Conflict(message) => goal_conflict(message),
        GoalServiceError::Internal(message) => internal_error(message),
    }
}

fn parse_thread_id_for_request(thread_id: &str) -> Result<ThreadId, JSONRPCErrorError> {
    ThreadId::from_string(thread_id)
        .map_err(|err| invalid_request(format!("invalid thread id: {err}")))
}
