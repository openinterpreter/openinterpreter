//! Route composer input and distinguish admission, rejection, and uncertainty.
//! A duplicate or an unconfirmed response must never trigger an automatic resend.

use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_client::CloudUserMessageReceipt;
use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::ApprovalsReviewer;
use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::CodexErrorInfo as AppServerCodexErrorInfo;
use codex_app_server_protocol::TurnError as AppServerTurnError;
use codex_app_server_protocol::TurnStartParams;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::turn_input::CyberAccessProgram;
use codex_utils_absolute_path::AbsolutePathBuf;
use color_eyre::eyre::Report;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use crate::app_command::AppCommand;
use crate::app_command::CloudSubmissionIntent;

use super::AppServerSession;
use super::JSONRPC_INVALID_PARAMS;
use super::JSONRPC_INVALID_REQUEST;
use super::JSONRPC_METHOD_NOT_FOUND;
use super::ThreadParamsMode;
use super::TurnPermissionsOverride;
use super::UnsupportedLegacyPermissionProfile;

// Both conventional transports return this before the request reaches its handler.
const JSONRPC_SERVER_OVERLOADED: i64 = -32001;

pub(crate) struct TurnStartSettings<'a> {
    pub(crate) approval_policy: Option<AskForApproval>,
    pub(crate) approvals_reviewer: Option<ApprovalsReviewer>,
    pub(crate) permissions: TurnPermissionsOverride,
    pub(crate) workspace_roots: &'a [AbsolutePathBuf],
}

pub(crate) enum UserMessageAdmission {
    Started(String),
    Steered(String),
    /// The turn/start response confirms admission and does not distinguish start from steer.
    StartResponse(String),
    Duplicate,
    Queued(String),
}

#[derive(Debug)]
pub(crate) enum UserMessageSubmissionFailure {
    Preflight(String),
    Rejected(Report),
    Unconfirmed { error: Report, reconnect: bool },
    Unhandled(Report),
}

impl UserMessageSubmissionFailure {
    fn conventional(error: Report, intent: CloudSubmissionIntent) -> Self {
        if intent == CloudSubmissionIntent::UseKnownTurn
            || super::model_provider_error(&error).is_some()
        {
            return Self::Unhandled(error);
        }
        if error
            .downcast_ref::<UnsupportedLegacyPermissionProfile>()
            .is_some()
            || matches!(error.downcast_ref::<TypedRequestError>(),
                Some(TypedRequestError::Server { source, .. })
                    if matches!(source.code, JSONRPC_INVALID_REQUEST | JSONRPC_INVALID_PARAMS | JSONRPC_METHOD_NOT_FOUND | JSONRPC_SERVER_OVERLOADED))
        {
            Self::Rejected(error)
        } else {
            Self::Unconfirmed {
                error,
                reconnect: true,
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct CloudModelSelection {
    pub(crate) model: String,
    pub(crate) effort: Option<ReasoningEffort>,
}

#[derive(Clone, Default)]
pub(crate) struct CloudTurnSettings {
    pub(crate) model: Option<CloudModelSelection>,
    pub(crate) service_tier: Option<String>,
}

/// Pending selections belong to one Cloud connection, keyed by conversation.
#[derive(Clone, Default)]
pub(crate) struct PendingCloudTurnSettings(Arc<Mutex<HashMap<ThreadId, CloudTurnSettings>>>);

impl PendingCloudTurnSettings {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ThreadId, CloudTurnSettings>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn model(&self, thread_id: ThreadId) -> Option<String> {
        self.lock()
            .get(&thread_id)?
            .model
            .as_ref()
            .map(|selection| selection.model.clone())
    }

    pub(crate) fn service_tier(&self, thread_id: ThreadId) -> Option<String> {
        self.lock().get(&thread_id)?.service_tier.clone()
    }
}

impl AppServerSession {
    pub(crate) fn pending_cloud_turn_settings(&self) -> Option<PendingCloudTurnSettings> {
        (self.thread_params_mode() == ThreadParamsMode::Cloud)
            .then(|| self.cloud_turn_settings.clone())
    }

    fn cloud_model_catalog_is_current(&self) -> bool {
        matches!(&self.request_handle,
            AppServerRequestHandle::Cloud(handle) if handle.model_catalog_is_current()
        )
    }

    pub(crate) fn stage_cloud_model_selection(
        &mut self,
        thread_id: ThreadId,
        model: String,
        effort: Option<ReasoningEffort>,
    ) -> bool {
        if !self.cloud_model_catalog_is_current() {
            return false;
        }
        let mut pending = self.cloud_turn_settings.lock();
        let settings = pending.entry(thread_id).or_default();
        if let Some(tier) = &settings.service_tier
            && tier != codex_protocol::config_types::SERVICE_TIER_DEFAULT_REQUEST_VALUE
            && !self.available_models.iter().any(|preset| {
                preset.model == model
                    && preset.service_tiers.iter().any(|available| {
                        available.id == *tier
                            || matches!(
                                (available.id.as_str(), tier.as_str()),
                                ("fast", "priority") | ("priority", "fast")
                            )
                    })
            })
        {
            settings.service_tier = None;
        }
        settings.model = Some(CloudModelSelection { model, effort });
        true
    }

    pub(crate) fn stage_cloud_service_tier(
        &mut self,
        thread_id: ThreadId,
        service_tier: Option<String>,
    ) -> bool {
        if !self.cloud_model_catalog_is_current() {
            return false;
        }
        self.cloud_turn_settings
            .lock()
            .entry(thread_id)
            .or_default()
            .service_tier = service_tier;
        true
    }

    pub(crate) async fn submit_user_message<'a>(
        &mut self,
        thread_id: ThreadId,
        op: &AppCommand,
        intent: CloudSubmissionIntent,
        active_turn: &mut Option<String>,
        preflight: impl FnOnce(&str) -> Result<Option<CyberAccessProgram>, String>,
        start_settings: impl FnOnce() -> TurnStartSettings<'a>,
    ) -> Result<(UserMessageAdmission, Option<CloudModelSelection>), UserMessageSubmissionFailure>
    {
        let AppCommand::UserTurn {
            turn_trigger,
            client_user_message_id,
            items,
            final_output_json_schema,
            cwd,
            model,
            effort,
            summary,
            service_tier,
            collaboration_mode,
            ..
        } = op
        else {
            unreachable!("user-message submission requires a user turn")
        };
        let cloud = self.thread_params_mode() == ThreadParamsMode::Cloud;
        if (!cloud || intent == CloudSubmissionIntent::UseKnownTurn) && active_turn.is_some() {
            if *turn_trigger != "user" {
                return Err(UserMessageSubmissionFailure::Unhandled(
                    color_eyre::eyre::eyre!("the fork already has an active turn"),
                ));
            }
            let mut retried = false;
            while let Some(turn_id) = active_turn.clone() {
                let error = match self
                    .turn_steer(
                        thread_id,
                        turn_id.clone(),
                        client_user_message_id.clone(),
                        items.clone(),
                    )
                    .await
                {
                    Ok(response) => {
                        return Ok((UserMessageAdmission::Steered(response.turn_id), None));
                    }
                    Err(error) => error,
                };
                if let Some(error) = active_turn_not_steerable_turn_error(&error) {
                    return Ok((UserMessageAdmission::Queued(error.message), None));
                }
                match active_turn_steer_race(&error) {
                    Some(ActiveTurnSteerRace::Missing) => {
                        *active_turn = None;
                        break;
                    }
                    Some(ActiveTurnSteerRace::ExpectedTurnMismatch { actual_turn_id }) => {
                        let retry = !retried && actual_turn_id != turn_id;
                        *active_turn = Some(actual_turn_id);
                        if retry {
                            retried = true;
                            continue;
                        }
                    }
                    None => {}
                }
                return Err(UserMessageSubmissionFailure::conventional(
                    error.into(),
                    intent,
                ));
            }
        }

        let selection =
            if cloud && *turn_trigger == "user" && intent != CloudSubmissionIntent::UseKnownTurn {
                self.cloud_turn_settings.lock().get(&thread_id).cloned()
            } else {
                None
            };
        if selection.is_some() && !self.cloud_model_catalog_is_current() {
            self.cloud_turn_settings.lock().remove(&thread_id);
            return Err(UserMessageSubmissionFailure::Preflight(
                "The Cloud account changed. Reopen the conversation before sending.".into(),
            ));
        }
        let selected_model = selection
            .as_ref()
            .and_then(|selection| selection.model.as_ref())
            .map_or(model.as_str(), |selection| selection.model.as_str());
        let program = preflight(selected_model).map_err(UserMessageSubmissionFailure::Preflight)?;
        let take_selection = |session: &mut Self| {
            selection.as_ref()?;
            session
                .cloud_turn_settings
                .lock()
                .remove(&thread_id)
                .filter(|_| session.cloud_model_catalog_is_current())
                .and_then(|settings| settings.model)
        };
        if cloud
            && intent == CloudSubmissionIntent::StartOrSteer
            && let AppServerRequestHandle::Cloud(handle) = &self.request_handle
        {
            let (model, effort) = selection
                .as_ref()
                .and_then(|settings| settings.model.as_ref())
                .map_or((None, None), |model| {
                    (Some(model.model.clone()), model.effort.clone())
                });
            let result = handle
                .submit_user_message(TurnStartParams {
                    thread_id: thread_id.to_string(),
                    turn_trigger: selection.as_ref().map(|_| turn_trigger.to_string()),
                    client_user_message_id: Some(client_user_message_id.clone()),
                    input: items.clone(),
                    output_schema: final_output_json_schema.clone(),
                    model,
                    effort,
                    service_tier: selection
                        .as_ref()
                        .and_then(|settings| settings.service_tier.clone())
                        .map(Some),
                    ..Default::default()
                })
                .await;
            let admission = match result {
                Ok(CloudUserMessageReceipt::Started(turn)) => {
                    Ok(UserMessageAdmission::Started(turn.id))
                }
                // A successful steer saves these settings for subsequent starts.
                Ok(CloudUserMessageReceipt::Steered(turn)) => {
                    Ok(UserMessageAdmission::Steered(turn.id))
                }
                Ok(CloudUserMessageReceipt::Duplicate { .. }) => {
                    return Ok((UserMessageAdmission::Duplicate, None));
                }
                // The adapter returns invalid params only for failures before admission.
                Err(TypedRequestError::Server { source, .. })
                    if source.code == JSONRPC_INVALID_PARAMS =>
                {
                    Err(UserMessageSubmissionFailure::Rejected(Report::msg(
                        source.message,
                    )))
                }
                Err(error) => Err(UserMessageSubmissionFailure::Unconfirmed {
                    error: error.into(),
                    reconnect: false,
                }),
            }?;
            return Ok((admission, take_selection(self)));
        }
        let settings = start_settings();
        self.turn_start(
            thread_id,
            turn_trigger,
            client_user_message_id.clone(),
            items.clone(),
            cwd.clone(),
            settings.approval_policy,
            settings.approvals_reviewer,
            settings.permissions,
            settings.workspace_roots,
            model.to_string(),
            effort.clone(),
            *summary,
            service_tier.clone(),
            collaboration_mode.clone(),
            final_output_json_schema.clone(),
            program.map(Into::into),
            selection.as_ref(),
        )
        .await
        .map(|response| {
            let admission = UserMessageAdmission::StartResponse(response.turn.id);
            (admission, take_selection(self))
        })
        .map_err(
            |error| match UserMessageSubmissionFailure::conventional(error, intent) {
                UserMessageSubmissionFailure::Rejected(error) => {
                    UserMessageSubmissionFailure::Rejected(error.wrap_err("Failed to start turn"))
                }
                failure => failure,
            },
        )
    }
}

pub(crate) fn active_turn_not_steerable_turn_error(
    error: &TypedRequestError,
) -> Option<AppServerTurnError> {
    let TypedRequestError::Server { source, .. } = error else {
        return None;
    };
    let turn_error: AppServerTurnError = serde_json::from_value(source.data.clone()?).ok()?;
    matches!(
        turn_error.codex_error_info,
        Some(AppServerCodexErrorInfo::ActiveTurnNotSteerable { .. })
    )
    .then_some(turn_error)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActiveTurnSteerRace {
    Missing,
    ExpectedTurnMismatch { actual_turn_id: String },
}

pub(crate) fn active_turn_steer_race(error: &TypedRequestError) -> Option<ActiveTurnSteerRace> {
    let TypedRequestError::Server { method, source } = error else {
        return None;
    };
    if method != "turn/steer" {
        return None;
    }
    if source.message == "no active turn to steer" {
        return Some(ActiveTurnSteerRace::Missing);
    }

    // App-server steer mismatches mean our cached active turn id is stale, but the response
    // includes the server's current active turn so we can resynchronize and retry once.
    let mismatch_prefix = "expected active turn id `";
    let mismatch_separator = "` but found `";
    let actual_turn_id = source
        .message
        .strip_prefix(mismatch_prefix)?
        .split_once(mismatch_separator)?
        .1
        .strip_suffix('`')?
        .to_string();
    Some(ActiveTurnSteerRace::ExpectedTurnMismatch { actual_turn_id })
}

#[cfg(test)]
mod cloud_pending_tests {
    use super::*;

    #[test]
    fn pending_cloud_settings_are_shared_but_scoped_and_consumed_per_thread() {
        let first = ThreadId::new();
        let second = ThreadId::new();
        let settings = PendingCloudTurnSettings::default();
        let widget_view = settings.clone();
        settings.lock().insert(
            first,
            CloudTurnSettings {
                model: Some(CloudModelSelection {
                    model: "model-a".into(),
                    effort: None,
                }),
                service_tier: Some("priority".into()),
            },
        );
        assert_eq!(widget_view.model(first).as_deref(), Some("model-a"));
        assert_eq!(widget_view.service_tier(first).as_deref(), Some("priority"));
        assert_eq!(widget_view.model(second), None);
        assert_eq!(widget_view.service_tier(second), None);

        settings.lock().remove(&first);
        assert_eq!(widget_view.model(first), None);
        assert_eq!(widget_view.service_tier(first), None);
    }
}
