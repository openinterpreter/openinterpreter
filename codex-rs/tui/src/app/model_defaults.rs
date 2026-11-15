//! Apply session model choices separately from saved model defaults.
//!
//! A successful config write can still be overridden. Report that distinction without
//! replacing the active task's explicit selection with launch-time configuration.

use super::App;
use crate::app_server_session::AppServerSession;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ConfigEdit;
use codex_app_server_protocol::WriteStatus;
use codex_protocol::config_types::ModeKind;
use codex_protocol::openai_models::ReasoningEffort;
use color_eyre::eyre::Result;

impl App {
    pub(super) async fn select_session_model(
        &mut self,
        app_server: &mut AppServerSession,
        model: String,
        effort: Option<ReasoningEffort>,
        service_tier: Option<String>,
    ) {
        if app_server.thread_params_mode() == crate::app_server_session::ThreadParamsMode::Cloud {
            let Some(thread_id) = self.current_displayed_thread_id() else {
                self.chat_widget.add_error_message(
                    "Select a conversation before changing its Cloud model".into(),
                );
                return;
            };
            if !app_server.stage_cloud_model_selection(thread_id, model.clone(), effort.clone()) {
                self.chat_widget.add_error_message(
                    "Cloud model list changed; refresh before selecting a model".into(),
                );
                return;
            }
            if let Some(tier) = service_tier.clone() {
                if !app_server.stage_cloud_service_tier(thread_id, Some(tier)) {
                    self.chat_widget.add_error_message(
                        "Cloud model list changed; refresh before selecting a service tier".into(),
                    );
                    return;
                }
            }
            self.chat_widget.add_info_message(
                format!("Model {model} selected for the next Cloud turn"),
                None,
            );
            self.refresh_status_line();
            return;
        }
        let model_changed = self.chat_widget.current_model() != model
            || self.chat_widget.current_collaboration_mode().model() != model;
        if model_changed
            && self
                .active_thread_model_setting_update_params(model.clone())
                .is_some_and(|params| params.permissions.is_some())
            && self.reject_pending_permission_change()
        {
            return;
        }
        let in_plan_mode = self.chat_widget.effective_collaboration_mode().mode == ModeKind::Plan;
        let ultra = effort == Some(ReasoningEffort::Ultra);
        let clear_default_ultra = self
            .chat_widget
            .current_collaboration_mode()
            .reasoning_effort()
            == Some(ReasoningEffort::Ultra)
            && self.config.model_reasoning_effort != Some(ReasoningEffort::Ultra);
        let clear_plan_ultra = self.chat_widget.config_ref().plan_mode_reasoning_effort
            == Some(ReasoningEffort::Ultra)
            && self.config.plan_mode_reasoning_effort != Some(ReasoningEffort::Ultra);
        self.chat_widget.set_model(&model);
        if let Some(service_tier) = service_tier {
            self.chat_widget.set_service_tier(Some(service_tier));
        }
        if !in_plan_mode || ultra || clear_default_ultra {
            self.chat_widget.set_reasoning_effort(effort.clone());
        }
        if in_plan_mode || ultra || clear_plan_ultra {
            self.chat_widget
                .set_plan_mode_reasoning_effort(effort.clone());
        }
        if model_changed {
            self.sync_active_thread_model_setting(app_server, model.clone(), effort.clone())
                .await;
        } else if let Some(mut params) =
            self.active_thread_reasoning_setting_update_params(effort.clone())
        {
            params.collaboration_mode = Some(self.chat_widget.effective_collaboration_mode());
            self.send_thread_settings_update(app_server, params).await;
        }
        self.sync_active_thread_service_tier_to_cached_session()
            .await;
        let mut message = format!("Model changed to {model}");
        if let Some(label) = Self::reasoning_label_for(&model, effort.as_ref()) {
            message.push(' ');
            message.push_str(&label);
        }
        message.push_str(" for this session only");
        self.chat_widget.add_info_message(message, /*hint*/ None);
    }

    pub(super) async fn persist_default_model_selection(
        &mut self,
        request_handle: AppServerRequestHandle,
        model: String,
        effort: Option<ReasoningEffort>,
    ) {
        self.app_event_tx
            .send(crate::app_event::AppEvent::FollowTranscript);
        match self
            .persist_model_defaults(
                request_handle,
                crate::config_update::build_model_selection_edits(model.as_str(), effort.as_ref()),
                "default model and reasoning effort",
            )
            .await
        {
            Ok(()) => {
                let effort_label = effort
                    .as_ref()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_else(|| "default".to_string());
                tracing::info!("Selected model: {model}, Selected effort: {effort_label}");
                let mut message = format!("Model changed to {model}");
                if let Some(label) = Self::reasoning_label_for(&model, effort.as_ref()) {
                    message.push(' ');
                    message.push_str(&label);
                }
                self.chat_widget.add_info_message(message, /*hint*/ None);
            }
            Err(err) => {
                let error = crate::config_update::format_config_error(&err);
                tracing::error!(error = %error, "failed to persist model selection");
                self.chat_widget
                    .add_error_message(format!("Failed to save default model: {error}"));
            }
        }
    }

    pub(super) async fn persist_model_defaults(
        &mut self,
        request_handle: AppServerRequestHandle,
        edits: Vec<ConfigEdit>,
        setting: &str,
    ) -> Result<()> {
        let response = crate::config_update::write_config_batch(request_handle, edits).await?;
        if response.status == WriteStatus::OkOverridden {
            self.chat_widget.add_warning_message(format!(
                "Saved {setting}, but a higher-priority configuration layer overrides the saved value."
            ));
        }
        Ok(())
    }
}
