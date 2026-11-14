use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::Weak;
use std::time::Duration;

use codex_core::CodexThread;
use codex_core::NotSubmittedReason;
use codex_core::StartIfIdleSubmission;
use codex_core::SteerSubmission;
use codex_core::ThreadManager;
use codex_core::TurnInput;
use codex_core::TurnInputRequest;
use codex_core::TurnStartOptions;
use codex_extension_api::ExtensionEventSink;
use codex_extension_api::ExtensionFuture;
use codex_extension_api::ThreadIdleCause;
use codex_extension_api::ThreadIdleInput;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadResumeInput;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::models::snapshot_local_user_input;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadQueueChangedEvent;
use codex_protocol::protocol::W3cTraceContext;
use codex_protocol::user_input::MAX_USER_INPUT_TEXT_CHARS;
use codex_protocol::user_input::UserInput;
use codex_thread_store::MAX_QUEUE_ITEMS;
use codex_thread_store::QueueStore;
use codex_thread_store::QueuedAdmissionRecord;
use codex_thread_store::QueuedUserSubmissionRecord;
use codex_thread_store::ThreadStoreError;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::sync::OwnedMutexGuard;
use tokio::sync::broadcast::error::TryRecvError;
use uuid::Uuid;

/// One user message waiting to start on its thread.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedItem {
    pub id: String,
    pub input: TurnInput,
    pub responsesapi_client_metadata: Option<HashMap<String, String>>,
}

/// Content-free dispatch decision for a persisted agent admission. The
/// validator runs at the durable queue dispatch boundary, not only at request
/// admission time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueAdmissionValidation {
    pub accepted: bool,
    pub blocker: Option<String>,
}

/// Content-free proof returned by the owning Goal layer when a native turn
/// completed the mission captured by a durable admission. The turn may be the
/// admission's dispatch turn or a later Goal-continuation turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueAdmissionCompletionProof {
    pub goal_id: String,
    pub mission_revision: i64,
    pub lifecycle_revision: i64,
    pub objective: String,
}

pub trait QueueAdmissionValidator: Send + Sync {
    fn validate<'a>(
        &'a self,
        thread_id: ThreadId,
        admission_id: &'a str,
        goal_binding_json: &'a str,
    ) -> ExtensionFuture<'a, QueueAdmissionValidation>;

    /// Resolve a terminal proof at the native completion boundary. Returning
    /// `None` is fail-closed: the current Goal is unavailable, changed, or is
    /// not a completed match for this exact admission and completion turn.
    fn completion_proof<'a>(
        &'a self,
        _thread_id: ThreadId,
        _admission_id: &'a str,
        _goal_binding_json: &'a str,
        _turn_id: &'a str,
    ) -> ExtensionFuture<'a, Option<QueueAdmissionCompletionProof>> {
        Box::pin(std::future::ready(None))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueAdmissionState {
    Pending,
    Starting,
    Dispatched,
    Blocked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueuedAdmission {
    pub item: QueuedItem,
    pub admission_id: String,
    pub goal_binding_json: String,
    pub dispatch_mode: String,
    pub expected_turn_id: Option<String>,
    pub state: QueueAdmissionState,
    pub blocker: Option<String>,
    pub dispatched_at_ms: Option<i64>,
    pub dispatch_attempt_at_ms: Option<i64>,
    pub dispatch_turn_id: Option<String>,
    pub terminal_outcome: Option<String>,
    pub terminal_turn_id: Option<String>,
    pub terminal_goal_id: Option<String>,
    pub terminal_mission_revision: Option<i64>,
    pub terminal_lifecycle_revision: Option<i64>,
    pub terminal_objective: Option<String>,
    pub terminal_at_ms: Option<i64>,
}

#[derive(Debug, Error)]
pub enum QueueServiceError {
    #[error("queue storage failed: {0}")]
    Storage(#[from] ThreadStoreError),
    #[error("queued submission payload is invalid: {0}")]
    InvalidPayload(#[from] serde_json::Error),
    #[error("local queued attachment is invalid: {0}")]
    InvalidAttachment(#[from] std::io::Error),
    #[error("Core failed to submit queued user message: {0}")]
    CoreSubmissionError(#[from] CodexErr),
    #[error("only user input can be added to the user-message queue")]
    InvalidInput,
    #[error(
        "queued user input exceeds the maximum length of {MAX_USER_INPUT_TEXT_CHARS} characters ({actual_chars} provided)"
    )]
    InputTooLarge { actual_chars: usize },
}

#[derive(Clone)]
pub struct QueuedItemService {
    queue: Arc<dyn QueueStore>,
    thread_manager: Weak<ThreadManager>,
    event_sink: Arc<dyn ExtensionEventSink>,
    dispatch_locks: Arc<StdMutex<HashMap<ThreadId, Weak<Mutex<()>>>>>,
    resumed_threads: Arc<StdMutex<HashSet<ThreadId>>>,
    admission_validator: Arc<StdMutex<Option<Arc<dyn QueueAdmissionValidator>>>>,
}

impl QueuedItemService {
    pub fn new(
        queue: Arc<dyn QueueStore>,
        thread_manager: Weak<ThreadManager>,
        event_sink: Arc<dyn ExtensionEventSink>,
    ) -> Self {
        Self {
            queue,
            thread_manager,
            event_sink,
            dispatch_locks: Arc::new(StdMutex::new(HashMap::new())),
            resumed_threads: Arc::new(StdMutex::new(HashSet::new())),
            admission_validator: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn set_admission_validator(&self, validator: Arc<dyn QueueAdmissionValidator>) {
        *self
            .admission_validator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(validator);
    }

    // Check SQLite's inexpensive data version every 10 seconds, then use the
    // durable revision index to discover only changed threads. Independent
    // dispatch tasks keep a blocked or failed thread from starving other queues.
    pub(crate) async fn watch_external_messages(service: Weak<Self>) {
        let mut last_version = None;
        let mut last_revision = 0;
        let mut dispatches: HashMap<ThreadId, tokio::task::JoinHandle<()>> = HashMap::new();
        let mut interval = tokio::time::interval(Duration::from_secs(/*secs*/ 10));
        let mut manager_initialized = false;
        let mut thread_created = None;
        let mut newly_loaded_threads = HashSet::new();
        loop {
            interval.tick().await;
            let Some(service) = service.upgrade() else {
                return;
            };
            let Some(manager) = service.thread_manager.upgrade() else {
                if manager_initialized {
                    return;
                }
                drop(service);
                tokio::time::sleep(Duration::from_millis(/*millis*/ 1)).await;
                interval.reset_immediately();
                continue;
            };
            manager_initialized = true;
            let thread_created =
                thread_created.get_or_insert_with(|| manager.subscribe_thread_created());
            loop {
                match thread_created.try_recv() {
                    Ok(thread_id) => {
                        newly_loaded_threads.insert(thread_id);
                    }
                    Err(TryRecvError::Lagged(_)) => {
                        newly_loaded_threads.extend(manager.list_thread_ids().await);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Closed) => return,
                }
            }
            newly_loaded_threads.extend(
                service
                    .resumed_threads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .drain(),
            );

            let version = match service.queue.change_version().await {
                Ok(version) => version,
                Err(error) => {
                    tracing::warn!(%error, "failed to check queue change version");
                    continue;
                }
            };
            let version_changed = last_version != Some(version);
            if !version_changed && newly_loaded_threads.is_empty() {
                continue;
            }

            let thread_ids = manager.list_thread_ids().await;
            let mut changes = Vec::new();
            let mut observed_revision = last_revision;
            if version_changed {
                match service
                    .queue
                    .changes_since(last_revision, &thread_ids)
                    .await
                {
                    Ok(changed_threads) => {
                        if let Some((_, revision)) = changed_threads.last() {
                            observed_revision = *revision;
                        }
                        changes.extend(changed_threads);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to discover changed thread queues");
                        continue;
                    }
                }
            }
            if !newly_loaded_threads.is_empty() {
                let created_threads = thread_ids
                    .iter()
                    .copied()
                    .filter(|thread_id| newly_loaded_threads.contains(thread_id))
                    .collect::<Vec<_>>();
                match service
                    .queue
                    .changes_since(/*revision*/ 0, &created_threads)
                    .await
                {
                    Ok(changed_threads) => changes.extend(changed_threads),
                    Err(error) => {
                        tracing::warn!(%error, "failed to discover newly loaded thread queues");
                        continue;
                    }
                }
            }
            last_version = Some(version);
            last_revision = observed_revision;
            newly_loaded_threads.clear();
            dispatches.retain(|_, dispatch| !dispatch.is_finished());

            let mut changed_threads = HashSet::new();
            for (thread_id, _) in changes {
                if !changed_threads.insert(thread_id) {
                    continue;
                }
                service.emit_changed(thread_id);
                if dispatches
                    .get(&thread_id)
                    .is_some_and(|dispatch| !dispatch.is_finished())
                {
                    continue;
                }
                let service = Arc::downgrade(&service);
                let dispatch = tokio::spawn(async move {
                    loop {
                        {
                            let Some(service) = service.upgrade() else {
                                return;
                            };
                            let Some(manager) = service.thread_manager.upgrade() else {
                                return;
                            };
                            let Ok(thread) = manager.get_thread(thread_id).await else {
                                return;
                            };
                            if matches!(
                                thread.agent_status().await,
                                AgentStatus::Running
                                    | AgentStatus::Interrupted
                                    | AgentStatus::Shutdown
                                    | AgentStatus::NotFound
                            ) {
                                return;
                            }
                            match service
                                .queue
                                .list_page(thread_id, /*offset*/ 0, /*limit*/ 1)
                                .await
                            {
                                Ok(items) if items.is_empty() => return,
                                Ok(_) => service.wake_if_loaded(thread_id).await,
                                Err(error) => {
                                    tracing::warn!(%thread_id, %error, "failed to check queued user input");
                                }
                            }
                        }
                        tokio::time::sleep(Duration::from_secs(/*secs*/ 10)).await;
                    }
                });
                dispatches.insert(thread_id, dispatch);
            }
        }
    }

    fn dispatch_lock(&self, thread_id: ThreadId) -> Arc<Mutex<()>> {
        let mut locks = self
            .dispatch_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|_, lock| lock.strong_count() != 0);
        if let Some(lock) = locks.get(&thread_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(thread_id, Arc::downgrade(&lock));
        lock
    }

    async fn dispatch_guard(&self, thread_id: ThreadId) -> OwnedMutexGuard<()> {
        self.dispatch_lock(thread_id).lock_owned().await
    }

    pub async fn enqueue(
        &self,
        thread_id: ThreadId,
        input: TurnInput,
    ) -> Result<QueuedItem, QueueServiceError> {
        self.enqueue_with_metadata(thread_id, input, /*responsesapi_client_metadata*/ None)
            .await
    }

    pub async fn enqueue_with_metadata(
        &self,
        thread_id: ThreadId,
        input: TurnInput,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<QueuedItem, QueueServiceError> {
        let input = prepare_queued_user_input(input).await?;
        let payload = queued_input_payload(&input, responsesapi_client_metadata.as_ref())?;
        let item = {
            let _dispatch_guard = self.dispatch_guard(thread_id).await;
            let item = QueuedItem::try_from(self.queue.enqueue(thread_id, payload).await?)?;
            self.emit_changed(thread_id);
            item
        };
        self.wake_if_loaded(thread_id).await;
        Ok(item)
    }

    /// Claim or reconcile one logical admission in the existing durable queue.
    /// A unique-index conflict is resolved by reading the retained row; it is
    /// never treated as permission to enqueue a second effect.
    pub async fn enqueue_admission(
        &self,
        thread_id: ThreadId,
        input: TurnInput,
        admission_id: String,
        goal_binding_json: String,
        expected_turn_id: Option<String>,
    ) -> Result<QueuedAdmission, QueueServiceError> {
        let input = prepare_queued_user_input(input).await?;
        let payload = serde_json::to_string(&input)?;
        let admission = {
            let _dispatch_guard = self.dispatch_guard(thread_id).await;
            let record = match self
                .queue
                .enqueue_admission(
                    thread_id,
                    payload,
                    admission_id.clone(),
                    goal_binding_json,
                    if expected_turn_id.is_some() {
                        "steer".to_string()
                    } else {
                        "queue".to_string()
                    },
                    expected_turn_id.clone(),
                )
                .await
            {
                Ok(record) => record,
                Err(ThreadStoreError::Conflict { .. }) => self
                    .queue
                    .find_admission(thread_id, admission_id.clone())
                    .await?
                    .ok_or_else(|| ThreadStoreError::Conflict {
                        message: "admission claim conflicted without a readable receipt"
                            .to_string(),
                    })?,
                Err(error) => return Err(error.into()),
            };
            let admission = queued_admission_from_record(record)?;
            if admission.state == QueueAdmissionState::Pending && admission.dispatch_mode == "queue"
            {
                self.emit_changed(thread_id);
            }
            if admission.state == QueueAdmissionState::Pending && admission.dispatch_mode == "steer"
            {
                let Some(expected_turn_id) = admission.expected_turn_id.clone() else {
                    return Err(QueueServiceError::InvalidInput);
                };
                let _ = self
                    .steer_admission_locked(
                        thread_id,
                        &admission.item.id,
                        &expected_turn_id,
                        admission.item.input.clone(),
                    )
                    .await?;
                let receipt = self
                    .find_admission(thread_id, admission.admission_id.as_str())
                    .await?
                    .ok_or_else(|| {
                        QueueServiceError::Storage(ThreadStoreError::Internal {
                            message: "steer admission receipt disappeared".to_string(),
                        })
                    })?;
                return Ok(receipt);
            }
            admission
        };
        if admission.state == QueueAdmissionState::Pending {
            self.wake_if_loaded(thread_id).await;
        }
        Ok(admission)
    }

    pub async fn find_admission(
        &self,
        thread_id: ThreadId,
        admission_id: &str,
    ) -> Result<Option<QueuedAdmission>, QueueServiceError> {
        self.queue
            .find_admission(thread_id, admission_id.to_string())
            .await?
            .map(queued_admission_from_record)
            .transpose()
    }

    /// Persist and return a terminal proof for an exact dispatched turn. The
    /// queue-store transition is one-shot and the dispatch guard prevents a
    /// concurrent queue transition from being mistaken for this proof.
    pub async fn record_admission_completed(
        &self,
        thread_id: ThreadId,
        admission_id: &str,
        turn_id: &str,
        goal_id: &str,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: &str,
    ) -> Result<Option<QueuedAdmission>, QueueServiceError> {
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        self.queue
            .mark_admission_completed(
                thread_id,
                admission_id.to_string(),
                turn_id.to_string(),
                goal_id.to_string(),
                mission_revision,
                lifecycle_revision,
                objective.to_string(),
            )
            .await?
            .map(queued_admission_from_record)
            .transpose()
    }

    /// Capture a completion at the authoritative Goal transition, before a
    /// later create/replace operation can make another Goal current. The
    /// transition itself supplies the exact mission identity; the native
    /// runtime supplies the completing turn when the transition occurred in a
    /// live task. Control-input admissions are deliberately excluded.
    pub async fn record_completed_admission_for_goal(
        &self,
        thread_id: ThreadId,
        goal_id: &str,
        mission_revision: i64,
        lifecycle_revision: i64,
        objective: &str,
        completion_turn_id: Option<&str>,
    ) -> Result<Option<QueuedAdmission>, QueueServiceError> {
        let Some(completion_turn_id) = completion_turn_id else {
            return Ok(None);
        };
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        let mut matched = None;
        for record in self.queue.list_dispatched_admissions(thread_id).await? {
            if record.dispatch_mode != "queue"
                || !goal_binding_matches(
                    &record.goal_binding_json,
                    goal_id,
                    mission_revision,
                    objective,
                )
            {
                continue;
            }
            let admission = queued_admission_from_record(record)?;
            if matched.is_some() {
                tracing::warn!(
                    %thread_id,
                    %goal_id,
                    "multiple task admissions matched one Goal transition; retaining reconciliation state"
                );
                return Ok(None);
            }
            matched = Some(admission);
        }
        let Some(admission) = matched else {
            return Ok(None);
        };
        self.queue
            .mark_admission_completed_from_continuation(
                thread_id,
                admission.admission_id,
                completion_turn_id.to_string(),
                goal_id.to_string(),
                mission_revision,
                lifecycle_revision,
                objective.to_string(),
            )
            .await?
            .map(queued_admission_from_record)
            .transpose()
    }

    /// Capture a terminal proof from the native idle boundary, before a
    /// disconnected coordinator can observe the receipt or a later Goal can
    /// replace the current one. A normal admission completes on its dispatch
    /// turn. A native Goal continuation completes outside the admission queue,
    /// so that path selects exactly one unfinished admission by its current
    /// Goal proof and records the continuation turn as terminal lineage.
    pub async fn record_completed_admission_for_turn(
        &self,
        thread_id: ThreadId,
        turn_id: &str,
    ) -> Result<Option<QueuedAdmission>, QueueServiceError> {
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        let exact_admissions = self
            .queue
            .list_admissions_by_turn(thread_id, turn_id.to_string())
            .await?
            .into_iter()
            .map(queued_admission_from_record)
            .collect::<Result<Vec<_>, _>>()?;
        let validator = self
            .admission_validator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(validator) = validator else {
            return Ok(None);
        };

        // A dispatch turn may contain both a Goal task and one or more
        // user/peer/timer control steers. Only queue-mode admissions are
        // eligible for Goal completion proof; every steer keeps its own
        // receipt and must never be selected by a first-row lookup.
        let exact_task_admissions = exact_admissions
            .iter()
            .filter(|admission| admission.dispatch_mode == "queue")
            .collect::<Vec<_>>();
        if exact_task_admissions.len() > 1 {
            tracing::warn!(
                %thread_id,
                %turn_id,
                "multiple task admissions share one native turn; retaining reconciliation state"
            );
            return Ok(None);
        }
        if let Some(admission) = exact_task_admissions.into_iter().next() {
            if admission.terminal_outcome.is_some() {
                return Ok(Some(admission.clone()));
            }
            let Some(proof) = validator
                .completion_proof(
                    thread_id,
                    admission.admission_id.as_str(),
                    admission.goal_binding_json.as_str(),
                    turn_id,
                )
                .await
            else {
                return Ok(None);
            };
            return self
                .queue
                .mark_admission_completed(
                    thread_id,
                    admission.admission_id.clone(),
                    turn_id.to_string(),
                    proof.goal_id,
                    proof.mission_revision,
                    proof.lifecycle_revision,
                    proof.objective,
                )
                .await?
                .map(queued_admission_from_record)
                .transpose();
        }
        if !exact_admissions.is_empty() {
            // This was a control-input completion, not an unassociated Goal
            // continuation. Do not let it borrow a separate task's Goal proof.
            return Ok(None);
        }

        // A Goal continuation gets a fresh native turn and therefore has no
        // dispatch row of its own. Prove the current completed Goal against
        // every unfinished dispatched admission, but fail closed if more than
        // one row could claim that Goal rather than guessing task identity.
        let candidates = self
            .queue
            .list_dispatched_admissions(thread_id)
            .await?
            .into_iter()
            .filter(|record| record.dispatch_mode == "queue")
            .collect::<Vec<_>>();
        let mut matched = None;
        for record in candidates {
            let admission = queued_admission_from_record(record)?;
            let Some(proof) = validator
                .completion_proof(
                    thread_id,
                    admission.admission_id.as_str(),
                    admission.goal_binding_json.as_str(),
                    turn_id,
                )
                .await
            else {
                continue;
            };
            if matched.is_some() {
                tracing::warn!(
                    %thread_id,
                    %turn_id,
                    "multiple admissions matched one native Goal continuation; retaining reconciliation state"
                );
                return Ok(None);
            }
            matched = Some((admission, proof));
        }
        let Some((admission, proof)) = matched else {
            return Ok(None);
        };
        self.queue
            .mark_admission_completed_from_continuation(
                thread_id,
                admission.admission_id,
                turn_id.to_string(),
                proof.goal_id,
                proof.mission_revision,
                proof.lifecycle_revision,
                proof.objective,
            )
            .await?
            .map(queued_admission_from_record)
            .transpose()
    }

    /// Finish a previously persisted active-steer admission after a caller
    /// crash before the native call. Only a still-pending steer row may enter
    /// Core; `starting` is treated as unknown and is never retried here.
    pub async fn resume_pending_steer(
        &self,
        thread_id: ThreadId,
        admission_id: &str,
        expected_turn_id: &str,
    ) -> Result<Option<QueuedAdmission>, QueueServiceError> {
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        let Some(admission) = self.find_admission(thread_id, admission_id).await? else {
            return Ok(None);
        };
        if admission.dispatch_mode != "steer"
            || admission.state != QueueAdmissionState::Pending
            || admission.expected_turn_id.as_deref() != Some(expected_turn_id)
        {
            return Ok(Some(admission));
        }
        let _ = self
            .steer_admission_locked(
                thread_id,
                &admission.item.id,
                expected_turn_id,
                admission.item.input,
            )
            .await?;
        self.find_admission(thread_id, admission_id).await
    }

    async fn mark_admission_starting(
        &self,
        thread_id: ThreadId,
        item_id: &str,
    ) -> Result<bool, QueueServiceError> {
        Ok(self
            .queue
            .mark_admission_starting(thread_id, item_id.to_string())
            .await?)
    }

    async fn mark_admission_started(
        &self,
        thread_id: ThreadId,
        item_id: &str,
        turn_id: &str,
    ) -> Result<bool, QueueServiceError> {
        Ok(self
            .queue
            .mark_admission_started(thread_id, item_id.to_string(), turn_id.to_string())
            .await?)
    }

    async fn reset_admission_pending(
        &self,
        thread_id: ThreadId,
        item_id: &str,
    ) -> Result<bool, QueueServiceError> {
        Ok(self
            .queue
            .reset_admission_pending(thread_id, item_id.to_string())
            .await?)
    }

    async fn steer_admission_locked(
        &self,
        thread_id: ThreadId,
        item_id: &str,
        expected_turn_id: &str,
        input: TurnInput,
    ) -> Result<SteerSubmission, QueueServiceError> {
        // Active steering is a durable admission too. Revalidate the captured
        // mission/lifecycle binding before claiming the row, including the
        // crash-recovery path, so an objective edit cannot steer the old task
        // into the new mission. A `starting` row is deliberately not retried:
        // its native outcome is still uncertain and must be reconciled.
        if !self
            .validate_admission_before_dispatch(thread_id, &input, item_id)
            .await?
        {
            return Ok(SteerSubmission::NotSubmitted {
                reason: NotSubmittedReason::NoActiveTurn,
            });
        }
        if !self.mark_admission_starting(thread_id, item_id).await? {
            return Ok(SteerSubmission::NotSubmitted {
                reason: NotSubmittedReason::NoActiveTurn,
            });
        }
        let Some(manager) = self.thread_manager.upgrade() else {
            return Err(QueueServiceError::Storage(ThreadStoreError::Internal {
                message: "thread manager unavailable during steer admission".to_string(),
            }));
        };
        let thread = manager.get_thread(thread_id).await.map_err(|error| {
            QueueServiceError::Storage(ThreadStoreError::Internal {
                message: format!("thread unavailable during steer admission: {error}"),
            })
        })?;
        let submission = thread
            .steer_turn(TurnInputRequest::new(input), expected_turn_id.to_string())
            .await?;
        match &submission {
            SteerSubmission::Steered { turn_id } => {
                self.mark_admission_started(thread_id, item_id, turn_id)
                    .await?;
            }
            SteerSubmission::NotSubmitted { reason } => {
                self.queue
                    .mark_admission_blocked(
                        thread_id,
                        item_id.to_string(),
                        format!("steer_not_submitted:{reason:?}"),
                    )
                    .await?;
            }
        }
        Ok(submission)
    }

    pub async fn list(&self, thread_id: ThreadId) -> Result<Vec<QueuedItem>, QueueServiceError> {
        self.list_page(thread_id, /*offset*/ 0, MAX_QUEUE_ITEMS)
            .await
    }

    pub async fn list_page(
        &self,
        thread_id: ThreadId,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<QueuedItem>, QueueServiceError> {
        self.queue
            .list_page(thread_id, offset, limit)
            .await?
            .into_iter()
            .map(QueuedItem::try_from)
            .collect()
    }

    pub async fn update(
        &self,
        thread_id: ThreadId,
        queued_item_id: String,
        input: TurnInput,
    ) -> Result<Option<QueuedItem>, QueueServiceError> {
        self.update_with_metadata(
            thread_id,
            queued_item_id,
            input,
            /*responsesapi_client_metadata*/ None,
        )
        .await
    }

    /// Omitted or empty metadata preserves the input's attribution, not the current controller's.
    pub async fn update_with_metadata(
        &self,
        thread_id: ThreadId,
        queued_item_id: String,
        mut input: TurnInput,
        mut responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<Option<QueuedItem>, QueueServiceError> {
        responsesapi_client_metadata =
            responsesapi_client_metadata.filter(|metadata| !metadata.is_empty());
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        let previous = self
            .list(thread_id)
            .await?
            .into_iter()
            .find(|item| item.id == queued_item_id);
        if let TurnInput::UserInput { client_id, .. } = &mut input {
            *client_id = previous.as_ref().and_then(|item| match &item.input {
                TurnInput::UserInput { client_id, .. } => client_id.clone(),
                _ => None,
            });
        }
        if responsesapi_client_metadata.is_none() {
            responsesapi_client_metadata =
                previous.and_then(|item| item.responsesapi_client_metadata);
        }
        let input = prepare_queued_user_input(input).await?;
        let payload = queued_input_payload(&input, responsesapi_client_metadata.as_ref())?;
        let item = self
            .queue
            .update(thread_id, queued_item_id, payload)
            .await?
            .map(QueuedItem::try_from)
            .transpose()?;
        if item.is_some() {
            self.emit_changed(thread_id);
        }
        Ok(item)
    }

    pub async fn delete(
        &self,
        thread_id: ThreadId,
        queued_item_id: String,
    ) -> Result<bool, QueueServiceError> {
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        self.delete_locked(thread_id, queued_item_id).await
    }

    async fn delete_locked(
        &self,
        thread_id: ThreadId,
        queued_item_id: String,
    ) -> Result<bool, QueueServiceError> {
        let deleted = self.queue.delete(thread_id, queued_item_id).await?;
        if deleted {
            self.emit_changed(thread_id);
        }
        Ok(deleted)
    }

    pub async fn reorder(
        &self,
        thread_id: ThreadId,
        ordered_ids: Vec<String>,
    ) -> Result<(), QueueServiceError> {
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        self.queue.reorder(thread_id, ordered_ids).await?;
        self.emit_changed(thread_id);
        Ok(())
    }

    /// Starts the selected queued message only when its thread is idle.
    pub async fn start(
        &self,
        thread: &CodexThread,
        queued_item_id: Option<String>,
        trace: Option<W3cTraceContext>,
    ) -> Result<StartIfIdleSubmission, QueueServiceError> {
        let thread_id = thread.startup_metadata().thread_id;
        let _dispatch_guard = self.dispatch_guard(thread_id).await;
        let item = self
            .list(thread_id)
            .await?
            .into_iter()
            .find(|item| queued_item_id.as_ref().is_none_or(|id| item.id == *id))
            .ok_or_else(|| ThreadStoreError::InvalidRequest {
                message: queued_item_id.as_ref().map_or_else(
                    || "queue is empty".to_string(),
                    |id| format!("queued submission not found: {id}"),
                ),
            })?;
        let queued_item_id = item.id.clone();
        let input @ TurnInput::UserInput { .. } = item.input else {
            return Err(QueueServiceError::InvalidInput);
        };
        if !self
            .validate_admission_before_dispatch(thread_id, &input, &queued_item_id)
            .await?
        {
            return Ok(StartIfIdleSubmission::NotSubmitted {
                reason: NotSubmittedReason::NotIdle,
            });
        }
        let admission_claimed = self.is_durable_admission(thread_id, &input).await?;
        if admission_claimed
            && !self
                .mark_admission_starting(thread_id, &queued_item_id)
                .await?
        {
            return Ok(StartIfIdleSubmission::NotSubmitted {
                reason: NotSubmittedReason::NotIdle,
            });
        }
        let submission = thread
            .start_turn_if_idle(
                TurnInputRequest::new(input.clone())
                    .with_trace(trace)
                    .on_start(TurnStartOptions {
                        turn_trigger: Some("queue".to_string()),
                        ..Default::default()
                    }),
            )
            .await?;
        match submission {
            StartIfIdleSubmission::Started {
                turn_id,
                root_turn_id,
            } if admission_claimed => {
                self.mark_admission_started(thread_id, &queued_item_id, &turn_id)
                    .await?;
                return Ok(StartIfIdleSubmission::Started {
                    turn_id,
                    root_turn_id,
                });
            }
            StartIfIdleSubmission::Started {
                turn_id,
                root_turn_id,
            } => {
                self.delete_locked(thread_id, queued_item_id).await?;
                return Ok(StartIfIdleSubmission::Started {
                    turn_id,
                    root_turn_id,
                });
            }
            StartIfIdleSubmission::NotSubmitted { reason } if admission_claimed => {
                self.reset_admission_pending(thread_id, &queued_item_id)
                    .await?;
                return Ok(StartIfIdleSubmission::NotSubmitted { reason });
            }
            StartIfIdleSubmission::NotSubmitted { reason } => {
                return Ok(StartIfIdleSubmission::NotSubmitted { reason });
            }
        }
    }

    async fn dispatch_if_idle(&self, thread_id: ThreadId) -> Result<(), QueueServiceError> {
        let Some(manager) = self.thread_manager.upgrade() else {
            return Ok(());
        };
        let Ok(thread) = manager.get_thread(thread_id).await else {
            return Ok(());
        };

        loop {
            let Some(record) = self
                .queue
                .list_page(thread_id, /*offset*/ 0, /*limit*/ 1)
                .await?
                .into_iter()
                .next()
            else {
                return Ok(());
            };
            let queued_item_id = record.id.clone();

            let item = match QueuedItem::try_from(record) {
                Ok(item) => item,
                Err(error) => {
                    tracing::warn!(%queued_item_id, %error, "discarding invalid queued item");
                    self.delete_locked(thread_id, queued_item_id).await?;
                    continue;
                }
            };
            let input @ TurnInput::UserInput { .. } = item.input else {
                tracing::warn!(%queued_item_id, "discarding non-user queued input");
                self.delete_locked(thread_id, queued_item_id).await?;
                continue;
            };

            if !self
                .validate_admission_before_dispatch(thread_id, &input, &queued_item_id)
                .await?
            {
                return Ok(());
            }

            let admission_claimed = self.is_durable_admission(thread_id, &input).await?;
            if admission_claimed
                && !self
                    .mark_admission_starting(thread_id, &queued_item_id)
                    .await?
            {
                return Ok(());
            }

            match thread
                .start_turn_if_idle(TurnInputRequest::new(input.clone()).on_start(
                    TurnStartOptions {
                        turn_trigger: Some("queue".to_string()),
                        ..Default::default()
                    },
                ))
                .await
            {
                Ok(StartIfIdleSubmission::Started { turn_id, .. }) if admission_claimed => {
                    self.mark_admission_started(thread_id, &queued_item_id, &turn_id)
                        .await?;
                    return Ok(());
                }
                Ok(StartIfIdleSubmission::Started { .. }) => {
                    self.delete_locked(thread_id, queued_item_id).await?;
                    return Ok(());
                }
                Ok(StartIfIdleSubmission::NotSubmitted { reason }) => {
                    if admission_claimed {
                        self.reset_admission_pending(thread_id, &queued_item_id)
                            .await?;
                    }
                    tracing::warn!(
                        %thread_id,
                        %queued_item_id,
                        ?reason,
                        "core could not start queued user input"
                    );
                    return Ok(());
                }
                Err(error) => {
                    tracing::warn!(
                        %thread_id,
                        %queued_item_id,
                        %error,
                        "core could not start queued user input"
                    );
                    return Ok(());
                }
            }
        }
    }

    async fn validate_admission_before_dispatch(
        &self,
        thread_id: ThreadId,
        input: &TurnInput,
        queued_item_id: &str,
    ) -> Result<bool, QueueServiceError> {
        let Some(admission_id) = input_client_id(input) else {
            return Ok(true);
        };
        let Some(record) = self
            .queue
            .find_admission(thread_id, admission_id.to_string())
            .await?
        else {
            return Ok(true);
        };
        if record.state != "pending" {
            return Ok(false);
        }
        let validator = self
            .admission_validator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(validator) = validator else {
            return Ok(true);
        };
        let decision = validator
            .validate(thread_id, admission_id, &record.goal_binding_json)
            .await;
        if decision.accepted {
            return Ok(true);
        }
        self.queue
            .mark_admission_blocked(
                thread_id,
                queued_item_id.to_string(),
                decision
                    .blocker
                    .unwrap_or_else(|| "goal_binding_rejected".to_string()),
            )
            .await?;
        self.emit_changed(thread_id);
        Ok(false)
    }

    async fn is_durable_admission(
        &self,
        thread_id: ThreadId,
        input: &TurnInput,
    ) -> Result<bool, QueueServiceError> {
        let Some(admission_id) = input_client_id(input) else {
            return Ok(false);
        };
        Ok(self
            .queue
            .find_admission(thread_id, admission_id.to_string())
            .await?
            .is_some())
    }

    async fn wake_if_loaded(&self, thread_id: ThreadId) {
        let Some(manager) = self.thread_manager.upgrade() else {
            return;
        };
        if let Ok(thread) = manager.get_thread(thread_id).await
            && !matches!(thread.agent_status().await, AgentStatus::Interrupted)
        {
            thread
                .emit_thread_idle_lifecycle_if_idle(ThreadIdleCause::Completed)
                .await;
        }
    }

    fn emit_changed(&self, thread_id: ThreadId) {
        self.event_sink.emit(Event {
            id: Uuid::now_v7().to_string(),
            msg: EventMsg::ThreadQueueChanged(ThreadQueueChangedEvent { thread_id }),
        });
    }
}

async fn prepare_queued_user_input(mut input: TurnInput) -> Result<TurnInput, QueueServiceError> {
    let TurnInput::UserInput { content, client_id } = &mut input else {
        return Err(QueueServiceError::InvalidInput);
    };
    if content.is_empty() {
        return Err(QueueServiceError::InvalidInput);
    }
    let actual_chars: usize = content
        .iter()
        .filter_map(|item| match item {
            UserInput::Text { text, .. } => Some(text.chars().count()),
            _ => None,
        })
        .sum();
    if actual_chars > MAX_USER_INPUT_TEXT_CHARS {
        return Err(QueueServiceError::InputTooLarge { actual_chars });
    }
    client_id.get_or_insert_with(|| Uuid::now_v7().to_string());
    if !content.iter().any(|item| {
        matches!(
            item,
            UserInput::LocalImage { .. } | UserInput::LocalAudio { .. }
        )
    }) {
        return Ok(input);
    }

    tokio::task::spawn_blocking(move || {
        let mut input = input;
        if let TurnInput::UserInput { content, .. } = &mut input {
            for item in content {
                snapshot_local_user_input(item)?;
            }
        }
        Ok::<TurnInput, std::io::Error>(input)
    })
    .await
    .map_err(|error| QueueServiceError::InvalidAttachment(std::io::Error::other(error)))?
    .map_err(QueueServiceError::InvalidAttachment)
}

impl<C> ThreadLifecycleContributor<C> for QueuedItemService
where
    C: Send + Sync + 'static,
{
    fn on_thread_resume<'a>(&'a self, input: ThreadResumeInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if let Ok(thread_id) = ThreadId::from_string(input.thread_store.level_id()) {
                self.resumed_threads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(thread_id);
            }
        })
    }

    fn on_thread_idle<'a>(&'a self, input: ThreadIdleInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if input.cause == ThreadIdleCause::Interrupted {
                return;
            }
            let Ok(thread_id) = ThreadId::from_string(input.thread_store.level_id()) else {
                tracing::warn!(
                    level_id = input.thread_store.level_id(),
                    "queue extension received an invalid thread id"
                );
                return;
            };
            let _guard = self.dispatch_guard(thread_id).await;
            if input.cause == ThreadIdleCause::Completed
                && let Some(turn_id) = input.turn_id
            {
                drop(_guard);
                if let Err(error) = self
                    .record_completed_admission_for_turn(thread_id, turn_id)
                    .await
                {
                    tracing::warn!(%thread_id, %turn_id, %error, "failed to persist terminal agent admission proof");
                }
                let _guard = self.dispatch_guard(thread_id).await;
                if let Err(error) = self.dispatch_if_idle(thread_id).await {
                    tracing::warn!(%thread_id, %error, "failed to dispatch queued user input");
                }
                return;
            }
            if let Err(error) = self.dispatch_if_idle(thread_id).await {
                tracing::warn!(%thread_id, %error, "failed to dispatch queued user input");
            }
        })
    }
}

fn queued_input_payload(
    input: &TurnInput,
    metadata: Option<&HashMap<String, String>>,
) -> Result<String, QueueServiceError> {
    let Some(metadata) = metadata else {
        return Ok(serde_json::to_string(input)?);
    };
    let mut payload = serde_json::to_value(input)?;
    // Keep TurnInput's wire shape so older readers can still execute the input.
    payload["UserInput"]["responsesapi_client_metadata"] = serde_json::to_value(metadata)?;
    Ok(serde_json::to_string(&payload)?)
}

impl TryFrom<QueuedUserSubmissionRecord> for QueuedItem {
    type Error = QueueServiceError;

    fn try_from(record: QueuedUserSubmissionRecord) -> Result<Self, Self::Error> {
        let mut payload: serde_json::Value = serde_json::from_str(&record.payload)?;
        let responsesapi_client_metadata = payload
            .get_mut("UserInput")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|input| input.remove("responsesapi_client_metadata"))
            .map(serde_json::from_value)
            .transpose()?;
        Ok(QueuedItem {
            id: record.id,
            input: serde_json::from_value(payload)?,
            responsesapi_client_metadata,
        })
    }
}

fn queued_admission_from_record(
    record: QueuedAdmissionRecord,
) -> Result<QueuedAdmission, QueueServiceError> {
    let state = match record.state.as_str() {
        "pending" => QueueAdmissionState::Pending,
        "starting" => QueueAdmissionState::Starting,
        "dispatched" => QueueAdmissionState::Dispatched,
        "blocked" => QueueAdmissionState::Blocked,
        _ => {
            return Err(QueueServiceError::InvalidAttachment(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid admission state",
            )));
        }
    };
    Ok(QueuedAdmission {
        item: QueuedItem::try_from(QueuedUserSubmissionRecord {
            id: record.id,
            thread_id: record.thread_id,
            payload: record.payload,
        })?,
        admission_id: record.admission_id,
        goal_binding_json: record.goal_binding_json,
        state,
        blocker: record.blocker,
        dispatched_at_ms: record.dispatched_at_ms,
        dispatch_attempt_at_ms: record.dispatch_attempt_at_ms,
        dispatch_turn_id: record.dispatch_turn_id,
        dispatch_mode: record.dispatch_mode,
        expected_turn_id: record.expected_turn_id,
        terminal_outcome: record.terminal_outcome,
        terminal_turn_id: record.terminal_turn_id,
        terminal_goal_id: record.terminal_goal_id,
        terminal_mission_revision: record.terminal_mission_revision,
        terminal_lifecycle_revision: record.terminal_lifecycle_revision,
        terminal_objective: record.terminal_objective,
        terminal_at_ms: record.terminal_at_ms,
    })
}

fn goal_binding_matches(
    goal_binding_json: &str,
    goal_id: &str,
    mission_revision: i64,
    objective: &str,
) -> bool {
    let Ok(binding) = serde_json::from_str::<serde_json::Value>(goal_binding_json) else {
        return false;
    };
    binding.get("goal_id").and_then(serde_json::Value::as_str) == Some(goal_id)
        && binding
            .get("mission_revision")
            .and_then(serde_json::Value::as_i64)
            == Some(mission_revision)
        && binding.get("objective").and_then(serde_json::Value::as_str) == Some(objective)
}

fn input_client_id(input: &TurnInput) -> Option<&str> {
    match input {
        TurnInput::UserInput { client_id, .. } => client_id.as_deref(),
        _ => None,
    }
}
