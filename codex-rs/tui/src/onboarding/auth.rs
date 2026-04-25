#![allow(clippy::unwrap_used)]

use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::AccountLoginCompletedNotification;
use codex_app_server_protocol::AccountUpdatedNotification;
use codex_app_server_protocol::AuthMode as AppServerAuthMode;
use codex_app_server_protocol::CancelLoginAccountParams;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigBatchWriteParams;
use codex_app_server_protocol::ConfigWriteResponse;
use codex_app_server_protocol::LoginAccountParams;
use codex_app_server_protocol::LoginAccountResponse;
use codex_app_server_protocol::ModelListParams;
use codex_app_server_protocol::ModelListResponse;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Styled as _;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use codex_protocol::config_types::ForcedLoginMethod;
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use tokio::task::AbortHandle;
use uuid::Uuid;

use crate::LoginStatus;
use crate::config_write_edits::provider_model_selection_edits;
use crate::onboarding::local_provider::start_hint as local_provider_start_hint;
use crate::onboarding::local_provider::start_local_provider;
use crate::onboarding::local_provider::wait_for_local_provider_running;
use crate::onboarding::model_selection::LoadingProviderModelsState;
use crate::onboarding::model_selection::LocalProviderUnavailableState;
use crate::onboarding::model_selection::ManualModelEntryState;
use crate::onboarding::model_selection::ProviderModelLoadResolution;
use crate::onboarding::model_selection::ProviderModelSelectionState;
use crate::onboarding::model_selection::ProviderReasoningSelectionState;
use crate::onboarding::model_selection::resolve_provider_model_load_resolution;
use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::onboarding::provider_setup::KIMI_FOR_CODING_PROVIDER_ID;
use crate::onboarding::provider_setup::ProviderPreset;
use crate::onboarding::provider_setup::ProviderSetupField;
use crate::onboarding::provider_setup::ProviderSetupState;
use crate::onboarding::provider_setup::browser_auth_provider_definition_edits;
use crate::onboarding::provider_setup::provider_presets;
use crate::product_branding::ProductBranding;
use crate::provider_model_flow::provider_choice_description;
use crate::provider_model_flow::provider_preset_choice_description;
use crate::provider_preset_repair::configured_provider_repair_edits;
use crate::provider_readiness::ProviderReadinessSnapshot;
use crate::provider_readiness::readiness_for_configured_provider;
use crate::provider_readiness::readiness_for_provider_preset;
use crate::shimmer::shimmer_spans;
use crate::style::app_accent_color;
use crate::style::app_accent_style;
use crate::style::app_accent_underlined_style;
use crate::style::selected_option_style;
use crate::style::unselected_option_style;
use crate::tui::FrameRequester;
#[cfg(test)]
use codex_feedback::CodexFeedback;
#[cfg(feature = "direct-login")]
use codex_login::kimi_code;
use codex_model_provider_info::ModelProviderInfo;

/// Marks buffer cells that have the shared accent+underlined style as an OSC 8 hyperlink.
///
/// Terminal emulators recognise the OSC 8 escape sequence and treat the entire
/// marked region as a single clickable link, regardless of row wrapping.  This
/// is necessary because ratatui's cell-based rendering emits `MoveTo` at every
/// row boundary, which breaks normal terminal URL detection for long URLs that
/// wrap across multiple rows.
pub(crate) fn mark_url_hyperlink(buf: &mut Buffer, area: Rect, url: &str) {
    // Sanitize: strip any characters that could break out of the OSC 8
    // sequence (ESC or BEL) to prevent terminal escape injection from a
    // malformed or compromised upstream URL.
    let safe_url: String = url
        .chars()
        .filter(|&c| c != '\x1B' && c != '\x07')
        .collect();
    if safe_url.is_empty() {
        return;
    }

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            // Only mark cells that carry the URL's distinctive style.
            if cell.fg != app_accent_color() || !cell.modifier.contains(Modifier::UNDERLINED) {
                continue;
            }
            let sym = cell.symbol().to_string();
            if sym.trim().is_empty() {
                continue;
            }
            cell.set_symbol(&format!("\x1B]8;;{safe_url}\x07{sym}\x1B]8;;\x07"));
        }
    }
}
use super::onboarding_screen::StepState;

mod headless_chatgpt_login;

#[derive(Clone)]
pub(crate) enum SignInState {
    ProviderPicker,
    PickMode,
    ConnectingAppServer(ConnectingAppServerState),
    ProviderSetup(ProviderSetupState),
    LoadingProviderModels(LoadingProviderModelsState),
    LocalProviderUnavailable(LocalProviderUnavailableState),
    ProviderModelSelection(ProviderModelSelectionState),
    ProviderReasoningSelection(ProviderReasoningSelectionState),
    ManualModelEntry(ManualModelEntryState),
    ProviderConfigured(String),
    ContinueInBrowser(ContinueInBrowserState),
    #[allow(dead_code)]
    ChatGptDeviceCode(ContinueWithDeviceCodeState),
    ChatGptSuccessMessage,
    ApiKeyEntry(ApiKeyInputState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignInOption {
    Browser,
    ChatGpt,
    DeviceCode,
    ApiKey,
}

const API_KEY_DISABLED_MESSAGE: &str = "API key login is disabled.";
fn onboarding_request_id() -> codex_app_server_protocol::RequestId {
    codex_app_server_protocol::RequestId::String(Uuid::new_v4().to_string())
}

pub(super) async fn cancel_login_attempt(
    request_handle: &AppServerRequestHandle,
    login_id: String,
) {
    let _ = request_handle
        .request_typed::<codex_app_server_protocol::CancelLoginAccountResponse>(
            ClientRequest::CancelLoginAccount {
                request_id: onboarding_request_id(),
                params: CancelLoginAccountParams { login_id },
            },
        )
        .await;
}

pub(crate) fn initial_sign_in_state(
    branding: ProductBranding,
    forced_login_method: Option<ForcedLoginMethod>,
) -> SignInState {
    if branding.is_open_interpreter && forced_login_method.is_none() {
        SignInState::ProviderPicker
    } else {
        SignInState::PickMode
    }
}

#[derive(Clone, Default)]
pub(crate) struct ApiKeyInputState {
    value: String,
    prepopulated_from_env: bool,
}

#[derive(Clone)]
/// Used to manage the lifecycle of SpawnedLogin and ensure it gets cleaned up.
pub(crate) struct ContinueInBrowserState {
    login_id: Option<String>,
    auth_url: String,
    title: String,
    remote_hint: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ContinueWithDeviceCodeState {
    login_id: Option<String>,
    verification_url: Option<String>,
    user_code: Option<String>,
}

impl ContinueWithDeviceCodeState {
    pub(crate) fn pending(_request_id: String) -> Self {
        Self {
            login_id: None,
            verification_url: None,
            user_code: None,
        }
    }

    pub(crate) fn ready(
        _request_id: String,
        login_id: String,
        verification_url: String,
        user_code: String,
    ) -> Self {
        Self {
            login_id: Some(login_id),
            verification_url: Some(verification_url),
            user_code: Some(user_code),
        }
    }
}

impl KeyboardHandler for AuthModeWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if self.handle_loading_provider_models_key_event(&key_event) {
            return;
        }
        if self.handle_local_provider_unavailable_key_event(&key_event) {
            return;
        }
        if self.handle_provider_setup_key_event(&key_event) {
            return;
        }
        if self.handle_provider_model_selection_key_event(&key_event) {
            return;
        }
        if self.handle_provider_reasoning_selection_key_event(&key_event) {
            return;
        }
        if self.handle_manual_model_entry_key_event(&key_event) {
            return;
        }
        if self.handle_api_key_entry_key_event(&key_event) {
            return;
        }

        let sign_in_state = { (*self.sign_in_state.read().unwrap()).clone() };
        match sign_in_state {
            SignInState::ProviderPicker => match key_event.code {
                KeyCode::Up => {
                    self.move_provider_highlight(/*delta*/ -1);
                }
                KeyCode::Down => {
                    self.move_provider_highlight(/*delta*/ 1);
                }
                KeyCode::Char(c) if self.provider_filter_query.is_empty() && c.is_ascii_digit() => {
                    if let Some(index) = c.to_digit(10).and_then(|digit| digit.checked_sub(1)) {
                        self.select_provider_by_index(index as usize);
                    }
                }
                KeyCode::Backspace => {
                    self.pop_provider_filter_char();
                    self.set_error(/*message*/ None);
                    self.request_frame.schedule_frame();
                }
                KeyCode::Enter => {
                    let selected_choice = self
                        .filtered_provider_choices()
                        .into_iter()
                        .find(|choice| choice.preset.provider_id == self.highlighted_provider_id)
                        .or_else(|| self.filtered_provider_choices().into_iter().next());
                    if let Some(choice) = selected_choice {
                        self.handle_provider_option(choice.preset);
                    }
                }
                KeyCode::Esc => {
                    if self.has_provider_filter() {
                        self.clear_provider_filter();
                        self.set_error(/*message*/ None);
                        self.request_frame.schedule_frame();
                    }
                }
                KeyCode::Char(c)
                    if key_event.kind == KeyEventKind::Press
                        && !key_event.modifiers.contains(KeyModifiers::SUPER)
                        && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.push_provider_filter_char(c);
                    self.set_error(/*message*/ None);
                    self.request_frame.schedule_frame();
                }
                _ => {}
            },
            _ => match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_highlight(/*delta*/ -1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_highlight(/*delta*/ 1);
                }
                KeyCode::Char('1') => {
                    self.select_option_by_index(/*index*/ 0);
                }
                KeyCode::Char('2') => {
                    self.select_option_by_index(/*index*/ 1);
                }
                KeyCode::Char('3') => {
                    self.select_option_by_index(/*index*/ 2);
                }
                KeyCode::Enter => match sign_in_state {
                    SignInState::PickMode => {
                        self.handle_sign_in_option(self.highlighted_mode);
                    }
                    SignInState::ChatGptSuccessMessage => {
                        self.start_provider_model_selection(
                            provider_presets()
                                .into_iter()
                                .find(|preset| preset.provider_id == "openai")
                                .expect("openai preset should exist"),
                        );
                    }
                    _ => {}
                },
                KeyCode::Esc => {
                    if matches!(sign_in_state, SignInState::ConnectingAppServer(_)) {
                        self.pending_app_server_request.write().unwrap().take();
                        *self.sign_in_state.write().unwrap() = self.initial_provider_auth_state();
                        self.set_error(/*message*/ None);
                        self.request_frame.schedule_frame();
                    } else if self.branding.is_open_interpreter
                        && matches!(sign_in_state, SignInState::PickMode)
                    {
                        *self.sign_in_state.write().unwrap() = SignInState::ProviderPicker;
                        self.set_error(/*message*/ None);
                        self.request_frame.schedule_frame();
                    } else {
                        tracing::info!("Esc pressed");
                        self.cancel_active_attempt();
                    }
                }
                _ => {}
            },
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        if self.handle_provider_picker_paste(pasted.clone()) {
            return;
        }
        if self.handle_provider_setup_paste(pasted.clone()) {
            return;
        }
        if self.handle_provider_model_selection_paste(pasted.clone()) {
            return;
        }
        if self.handle_manual_model_entry_paste(pasted.clone()) {
            return;
        }
        let _ = self.handle_api_key_entry_paste(pasted);
    }
}

#[derive(Clone)]
pub(crate) struct ConnectingAppServerState {
    message: String,
}

#[derive(Clone)]
pub(crate) enum PendingAppServerAction {
    LoadProviderModels(LoadingProviderModelsState),
    SaveProviderSetup(ProviderSetupState),
    SaveProviderModelSelection {
        provider_id: String,
        provider_name: String,
        model: String,
        effort: Option<codex_protocol::openai_models::ReasoningEffort>,
    },
    SaveApiKey(String),
    StartKimiCodeLogin,
    StartChatGptLogin,
    StartDeviceCodeLogin,
}

#[derive(Clone)]
pub(crate) struct PendingAppServerRequest {
    action: PendingAppServerAction,
    fallback_state: SignInState,
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct AuthModeWidget {
    pub request_frame: FrameRequester,
    pub interpreter_home: PathBuf,
    pub highlighted_mode: SignInOption,
    pub highlighted_provider_id: String,
    pub provider_filter_query: String,
    pub configured_model_providers: std::collections::HashMap<String, ModelProviderInfo>,
    pub current_model_provider_id: String,
    pub current_model: Option<String>,
    pub imported_model_provider_id: Option<String>,
    pub imported_model: Option<String>,
    pub suppress_current_provider: bool,
    pub provider_readiness_snapshot: ProviderReadinessSnapshot,
    pub error: Arc<RwLock<Option<String>>>,
    pub sign_in_state: Arc<RwLock<SignInState>>,
    pub branding: ProductBranding,
    pub login_status: LoginStatus,
    pub app_server_request_handle: Option<AppServerRequestHandle>,
    pub(crate) pending_app_server_request: Arc<RwLock<Option<PendingAppServerRequest>>>,
    pub forced_chatgpt_workspace_id: Option<String>,
    pub forced_login_method: Option<ForcedLoginMethod>,
    pub animations_enabled: bool,
    pub animations_suppressed: Cell<bool>,
    pub provider_login_abort: Arc<Mutex<Option<AbortHandle>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProviderPickerChoice {
    preset: ProviderPreset,
    description: String,
    is_current: bool,
    model_hint: Option<ProviderPickerModelHint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProviderPickerModelHint {
    label: &'static str,
    model: String,
}

impl AuthModeWidget {
    pub(crate) fn set_animations_suppressed(&self, suppressed: bool) {
        self.animations_suppressed.set(suppressed);
    }

    pub(crate) fn should_suppress_animations(&self) -> bool {
        matches!(
            &*self.sign_in_state.read().unwrap(),
            SignInState::ContinueInBrowser(_) | SignInState::ChatGptDeviceCode(_)
        )
    }

    fn selected_provider_preset(&self) -> Option<ProviderPreset> {
        provider_presets()
            .into_iter()
            .find(|preset| preset.provider_id == self.highlighted_provider_id)
    }

    fn selected_provider_uses_browser_auth(&self) -> bool {
        self.selected_provider_preset()
            .is_some_and(|preset| preset.uses_browser_auth())
    }

    pub(crate) fn needs_app_server_connection(&self) -> bool {
        self.app_server_request_handle.is_none()
            && self
                .pending_app_server_request
                .read()
                .is_ok_and(|guard| guard.is_some())
    }

    pub(crate) fn attach_app_server_request_handle(&mut self, handle: AppServerRequestHandle) {
        self.app_server_request_handle = Some(handle);
        let pending_request = self
            .pending_app_server_request
            .write()
            .ok()
            .and_then(|mut guard| guard.take());
        let Some(pending_request) = pending_request else {
            self.request_frame.schedule_frame();
            return;
        };

        match pending_request.action {
            PendingAppServerAction::LoadProviderModels(loading_state) => {
                *self.sign_in_state.write().unwrap() =
                    SignInState::LoadingProviderModels(loading_state.clone());
                self.spawn_provider_model_load(loading_state);
            }
            PendingAppServerAction::SaveProviderSetup(state) => {
                self.save_provider_setup(state);
            }
            PendingAppServerAction::SaveProviderModelSelection {
                provider_id,
                provider_name,
                model,
                effort,
            } => {
                self.persist_provider_model_selection(
                    provider_id,
                    provider_name,
                    model,
                    effort,
                    pending_request.fallback_state,
                );
            }
            PendingAppServerAction::SaveApiKey(api_key) => {
                self.save_api_key(api_key);
            }
            PendingAppServerAction::StartKimiCodeLogin => {
                self.start_kimi_code_login();
            }
            PendingAppServerAction::StartChatGptLogin => {
                self.start_chatgpt_login();
            }
            PendingAppServerAction::StartDeviceCodeLogin => {
                self.start_device_code_login();
            }
        }
    }

    pub(crate) fn on_app_server_connection_failed(&mut self, error_message: String) {
        let fallback_state = self
            .pending_app_server_request
            .write()
            .ok()
            .and_then(|mut guard| guard.take())
            .map(|pending_request| pending_request.fallback_state);
        if let Some(fallback_state) = fallback_state {
            *self.sign_in_state.write().unwrap() = fallback_state;
        }
        self.set_error(Some(error_message));
        self.request_frame.schedule_frame();
    }

    fn defer_app_server_request(
        &mut self,
        action: PendingAppServerAction,
        fallback_state: SignInState,
        loading_state: SignInState,
    ) {
        *self.pending_app_server_request.write().unwrap() = Some(PendingAppServerRequest {
            action,
            fallback_state,
        });
        *self.sign_in_state.write().unwrap() = loading_state;
        self.request_frame.schedule_frame();
    }

    pub(crate) fn cancel_active_attempt(&self) {
        let mut sign_in_state = self.sign_in_state.write().unwrap();
        match &*sign_in_state {
            SignInState::ContinueInBrowser(state) => {
                if let Some(login_id) = state.login_id.clone() {
                    let Some(request_handle) = self.app_server_request_handle.clone() else {
                        *sign_in_state = SignInState::PickMode;
                        drop(sign_in_state);
                        self.set_error(/*message*/ None);
                        self.request_frame.schedule_frame();
                        return;
                    };
                    tokio::spawn(async move {
                        cancel_login_attempt(&request_handle, login_id).await;
                    });
                }
                if let Some(abort_handle) = self.provider_login_abort.lock().unwrap().take() {
                    abort_handle.abort();
                }
            }
            SignInState::ChatGptDeviceCode(state) => {
                if let Some(login_id) = state.login_id.clone() {
                    let Some(request_handle) = self.app_server_request_handle.clone() else {
                        *sign_in_state = SignInState::PickMode;
                        drop(sign_in_state);
                        self.set_error(/*message*/ None);
                        self.request_frame.schedule_frame();
                        return;
                    };
                    tokio::spawn(async move {
                        let _ = request_handle
                            .request_typed::<codex_app_server_protocol::CancelLoginAccountResponse>(
                                ClientRequest::CancelLoginAccount {
                                    request_id: onboarding_request_id(),
                                    params: CancelLoginAccountParams { login_id },
                                },
                            )
                            .await;
                    });
                }
            }
            _ => return,
        }
        *sign_in_state = SignInState::PickMode;
        drop(sign_in_state);
        self.set_error(/*message*/ None);
        self.request_frame.schedule_frame();
    }

    fn set_error(&self, message: Option<String>) {
        *self.error.write().unwrap() = message;
    }

    fn error_message(&self) -> Option<String> {
        self.error.read().unwrap().clone()
    }

    fn available_provider_choices(&self) -> Vec<ProviderPickerChoice> {
        provider_presets()
            .into_iter()
            .map(|preset| {
                let is_current = !self.suppress_current_provider
                    && preset.provider_id == self.current_model_provider_id;
                let model_hint = if is_current {
                    self.current_model
                        .clone()
                        .map(|model| ProviderPickerModelHint {
                            label: "Current model",
                            model,
                        })
                } else if self.imported_model_provider_id.as_deref()
                    == Some(preset.provider_id.as_str())
                {
                    self.imported_model
                        .clone()
                        .map(|model| ProviderPickerModelHint {
                            label: "Imported model",
                            model,
                        })
                } else {
                    None
                };
                let description = if let Some(provider) =
                    self.configured_model_providers.get(&preset.provider_id)
                {
                    let readiness = readiness_for_configured_provider(
                        preset.provider_id.as_str(),
                        provider,
                        &self.provider_readiness_snapshot,
                    );
                    readiness.decorate_description(provider_choice_description(
                        preset.provider_id.as_str(),
                        provider,
                    ))
                } else {
                    let readiness =
                        readiness_for_provider_preset(&preset, &self.provider_readiness_snapshot);
                    readiness.decorate_description(provider_preset_choice_description(&preset))
                };

                ProviderPickerChoice {
                    preset,
                    description,
                    is_current,
                    model_hint,
                }
            })
            .collect()
    }

    fn filtered_provider_choices(&self) -> Vec<ProviderPickerChoice> {
        let query = self.provider_filter_query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return self.available_provider_choices();
        }

        self.available_provider_choices()
            .into_iter()
            .filter(|choice| {
                let model_hint = choice
                    .model_hint
                    .as_ref()
                    .map(|hint| format!(" {} {}", hint.label, hint.model))
                    .unwrap_or_default();
                let haystack = format!(
                    "{} {} {}{}",
                    choice.preset.provider_id, choice.preset.title, choice.description, model_hint
                );
                haystack.to_ascii_lowercase().contains(query.as_str())
            })
            .collect()
    }

    fn has_provider_filter(&self) -> bool {
        !self.provider_filter_query.is_empty()
    }

    fn push_provider_filter_char(&mut self, c: char) {
        self.provider_filter_query.push(c);
        self.sync_provider_highlight_to_filter();
    }

    fn pop_provider_filter_char(&mut self) {
        self.provider_filter_query.pop();
        self.sync_provider_highlight_to_filter();
    }

    fn clear_provider_filter(&mut self) {
        self.provider_filter_query.clear();
        self.sync_provider_highlight_to_filter();
    }

    fn sync_provider_highlight_to_filter(&mut self) {
        let choices = self.filtered_provider_choices();
        if choices.is_empty() {
            return;
        }
        if choices
            .iter()
            .any(|choice| choice.preset.provider_id == self.highlighted_provider_id)
        {
            return;
        }

        self.highlighted_provider_id = choices[0].preset.provider_id.clone();
    }

    fn move_provider_highlight(&mut self, delta: isize) {
        let choices = self.filtered_provider_choices();
        if choices.is_empty() {
            return;
        }

        let current_index = choices
            .iter()
            .position(|choice| choice.preset.provider_id == self.highlighted_provider_id)
            .unwrap_or(0);
        let next_index =
            (current_index as isize + delta).rem_euclid(choices.len() as isize) as usize;
        self.highlighted_provider_id = choices[next_index].preset.provider_id.clone();
    }

    fn select_provider_by_index(&mut self, index: usize) {
        if let Some(choice) = self.filtered_provider_choices().get(index).cloned() {
            self.handle_provider_option(choice.preset);
        }
    }

    fn handle_provider_option(&mut self, preset: ProviderPreset) {
        self.set_error(/*message*/ None);
        self.clear_provider_filter();
        self.highlighted_provider_id = preset.provider_id.clone();
        let has_existing_provider_config = self
            .configured_model_providers
            .contains_key(&preset.provider_id);
        let can_reuse_current_openai_login = preset.uses_openai_auth()
            && self.current_model_provider_id == preset.provider_id
            && self.login_status != LoginStatus::NotAuthenticated;
        if has_existing_provider_config || can_reuse_current_openai_login {
            self.start_provider_model_selection(preset);
            return;
        }
        if preset.uses_openai_auth() {
            self.highlighted_mode = match self.forced_login_method {
                Some(ForcedLoginMethod::Api) => SignInOption::ApiKey,
                _ => SignInOption::ChatGpt,
            };
            *self.sign_in_state.write().unwrap() = SignInState::PickMode;
        } else if preset.uses_browser_auth() {
            self.highlighted_mode = SignInOption::Browser;
            *self.sign_in_state.write().unwrap() = SignInState::PickMode;
        } else if let Some(state) = ProviderSetupState::new(preset.clone()) {
            *self.sign_in_state.write().unwrap() = SignInState::ProviderSetup(state);
        } else {
            self.start_provider_model_selection(preset);
            return;
        }
        self.request_frame.schedule_frame();
    }

    fn start_provider_model_selection(&mut self, preset: ProviderPreset) {
        self.set_error(/*message*/ None);
        let loading_state = LoadingProviderModelsState {
            provider_id: preset.provider_id.to_string(),
            provider_name: preset.title.to_string(),
            manual_model_placeholder: preset.model_placeholder.to_string(),
            default_manual_model: preset.default_model.unwrap_or_default(),
        };
        if self.app_server_request_handle.is_none() {
            self.defer_app_server_request(
                PendingAppServerAction::LoadProviderModels(loading_state.clone()),
                self.initial_provider_auth_state(),
                SignInState::LoadingProviderModels(loading_state),
            );
        } else {
            *self.sign_in_state.write().unwrap() =
                SignInState::LoadingProviderModels(loading_state.clone());

            self.spawn_provider_model_load(loading_state);
            self.request_frame.schedule_frame();
        }
    }

    fn spawn_provider_model_load(&self, loading_state: LoadingProviderModelsState) {
        let Some(request_handle) = self.app_server_request_handle.clone() else {
            return;
        };
        let preferred_model = if loading_state.provider_id == self.current_model_provider_id {
            self.current_model.clone()
        } else if self.imported_model_provider_id.as_deref()
            == Some(loading_state.provider_id.as_str())
        {
            self.imported_model.clone()
        } else {
            None
        };
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            let result = request_handle
                .request_typed::<ModelListResponse>(ClientRequest::ModelList {
                    request_id: onboarding_request_id(),
                    params: ModelListParams {
                        cursor: None,
                        limit: None,
                        include_hidden: Some(true),
                        model_provider: Some(loading_state.provider_id.clone()),
                    },
                })
                .await;

            let (next_state, next_error) =
                Self::resolve_provider_model_load_state(loading_state, preferred_model, result)
                    .await;
            *error.write().unwrap() = next_error;
            *sign_in_state.write().unwrap() = next_state;
            request_frame.schedule_frame();
        });
    }

    async fn resolve_provider_model_load_state(
        loading_state: LoadingProviderModelsState,
        preferred_model: Option<String>,
        result: Result<ModelListResponse, codex_app_server_client::TypedRequestError>,
    ) -> (SignInState, Option<String>) {
        let resolution = resolve_provider_model_load_resolution(
            loading_state,
            result
                .map(|response| {
                    response
                        .data
                        .into_iter()
                        .map(crate::app_server_session::model_preset_from_api_model)
                        .collect()
                })
                .map_err(|err| err.to_string()),
        )
        .await;
        match resolution {
            ProviderModelLoadResolution::Picker {
                mut state,
                info_message,
            } => {
                if let Some(preferred_model) = preferred_model {
                    state.select_model(preferred_model.as_str());
                }
                (SignInState::ProviderModelSelection(state), info_message)
            }
            ProviderModelLoadResolution::LocalProviderUnavailable(state) => {
                (SignInState::LocalProviderUnavailable(state), None)
            }
            ProviderModelLoadResolution::ManualModelFallback(state) => (
                SignInState::ManualModelEntry(ManualModelEntryState::new(
                    state.provider_id,
                    state.provider_name,
                    state.manual_model_placeholder,
                    state.default_manual_model,
                )),
                Some(state.message),
            ),
        }
    }

    fn persist_provider_model_selection(
        &mut self,
        provider_id: String,
        provider_name: String,
        model: String,
        effort: Option<codex_protocol::openai_models::ReasoningEffort>,
        fallback_state: SignInState,
    ) {
        self.set_error(/*message*/ None);
        let Some(request_handle) = self.app_server_request_handle.clone() else {
            self.defer_app_server_request(
                PendingAppServerAction::SaveProviderModelSelection {
                    provider_id,
                    provider_name: provider_name.clone(),
                    model,
                    effort,
                },
                fallback_state,
                SignInState::ConnectingAppServer(ConnectingAppServerState {
                    message: format!("Saving {provider_name}..."),
                }),
            );
            return;
        };
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        let interpreter_home = self.interpreter_home.clone();
        let configured_provider = self.configured_model_providers.get(&provider_id).cloned();
        *sign_in_state.write().unwrap() =
            SignInState::ConnectingAppServer(ConnectingAppServerState {
                message: format!("Saving {provider_name}..."),
            });
        tokio::spawn(async move {
            let mut edits = configured_provider
                .as_ref()
                .map(|provider| configured_provider_repair_edits(provider_id.as_str(), provider))
                .unwrap_or_default();
            edits.extend(provider_model_selection_edits(
                /*profile*/ None,
                provider_id.as_str(),
                configured_provider.as_ref(),
                Some(model.as_str()),
                effort,
            ));
            match request_handle
                .request_typed::<ConfigWriteResponse>(ClientRequest::ConfigBatchWrite {
                    request_id: onboarding_request_id(),
                    params: ConfigBatchWriteParams {
                        edits,
                        file_path: None,
                        expected_version: None,
                        reload_user_config: true,
                    },
                })
                .await
            {
                Ok(_) => {
                    let onboarding_marker_path =
                        interpreter_home.join(".fresh_home_provider_onboarding");
                    if let Err(err) = std::fs::remove_file(&onboarding_marker_path)
                        && err.kind() != std::io::ErrorKind::NotFound
                    {
                        tracing::warn!(
                            path = %onboarding_marker_path.display(),
                            "failed to clear fresh-home onboarding marker: {err}"
                        );
                    }
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() = SignInState::ProviderConfigured(format!(
                        "Using {provider_name} with {model}."
                    ));
                }
                Err(err) => {
                    *error.write().unwrap() = Some(format!(
                        "Failed to save {provider_name} with {model}: {err}"
                    ));
                    *sign_in_state.write().unwrap() = fallback_state;
                }
            }
            request_frame.schedule_frame();
        });
        self.request_frame.schedule_frame();
    }

    fn is_api_login_allowed(&self) -> bool {
        !matches!(self.forced_login_method, Some(ForcedLoginMethod::Chatgpt))
    }

    fn is_chatgpt_login_allowed(&self) -> bool {
        !matches!(self.forced_login_method, Some(ForcedLoginMethod::Api))
    }

    fn displayed_sign_in_options(&self) -> Vec<SignInOption> {
        if self.selected_provider_uses_browser_auth() {
            return vec![SignInOption::Browser];
        }
        let mut options = vec![SignInOption::ChatGpt];
        if self.is_chatgpt_login_allowed() {
            options.push(SignInOption::DeviceCode);
        }
        if self.is_api_login_allowed() {
            options.push(SignInOption::ApiKey);
        }
        options
    }

    fn selectable_sign_in_options(&self) -> Vec<SignInOption> {
        if self.selected_provider_uses_browser_auth() {
            return vec![SignInOption::Browser];
        }
        let mut options = Vec::new();
        if self.is_chatgpt_login_allowed() {
            options.push(SignInOption::ChatGpt);
            options.push(SignInOption::DeviceCode);
        }
        if self.is_api_login_allowed() {
            options.push(SignInOption::ApiKey);
        }
        options
    }

    fn move_highlight(&mut self, delta: isize) {
        let options = self.selectable_sign_in_options();
        if options.is_empty() {
            return;
        }

        let current_index = options
            .iter()
            .position(|option| *option == self.highlighted_mode)
            .unwrap_or(0);
        let next_index =
            (current_index as isize + delta).rem_euclid(options.len() as isize) as usize;
        self.highlighted_mode = options[next_index];
    }

    fn select_option_by_index(&mut self, index: usize) {
        let options = self.displayed_sign_in_options();
        if let Some(option) = options.get(index).copied() {
            self.handle_sign_in_option(option);
        }
    }

    fn handle_sign_in_option(&mut self, option: SignInOption) {
        match option {
            SignInOption::Browser => {
                if self.selected_provider_uses_browser_auth() {
                    self.start_kimi_code_login();
                }
            }
            SignInOption::ChatGpt => {
                if self.is_chatgpt_login_allowed() {
                    self.start_chatgpt_login();
                }
            }
            SignInOption::DeviceCode => {
                if self.is_chatgpt_login_allowed() {
                    self.start_device_code_login();
                }
            }
            SignInOption::ApiKey => {
                if self.is_api_login_allowed() {
                    self.start_api_key_entry();
                } else {
                    self.disallow_api_login();
                }
            }
        }
    }

    fn disallow_api_login(&mut self) {
        self.highlighted_mode = SignInOption::ChatGpt;
        self.set_error(Some(API_KEY_DISABLED_MESSAGE.to_string()));
        *self.sign_in_state.write().unwrap() = SignInState::PickMode;
        self.request_frame.schedule_frame();
    }

    fn render_provider_picker(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = Vec::new();
        if self.branding.is_open_interpreter {
            lines.push("".into());
        }
        lines.push(Line::from(vec![
            "  ".into(),
            self.branding.auth_intro_primary.into(),
        ]));
        if !self.branding.auth_intro_secondary.is_empty() {
            lines.push(Line::from(vec![
                "  ".into(),
                self.branding.auth_intro_secondary.into(),
            ]));
        }
        let filter_line = if self.provider_filter_query.trim().is_empty() {
            Line::from(vec!["  Filter providers: ".into(), "type to search".dim()])
        } else {
            Line::from(vec![
                "  Filter providers: ".into(),
                self.provider_filter_query.as_str().fg(app_accent_color()),
            ])
        };
        lines.push(filter_line);
        lines.push("".into());

        let filtered_choices = self.filtered_provider_choices();
        if filtered_choices.is_empty() {
            lines.push(
                "  No matching providers. Keep typing or press Backspace to clear."
                    .dim()
                    .into(),
            );
            lines.push("".into());
        }

        for (idx, choice) in filtered_choices.iter().enumerate() {
            let is_selected = self.highlighted_provider_id == choice.preset.provider_id;
            let caret = if is_selected { ">" } else { " " };
            let current_suffix = if choice.is_current { " (current)" } else { "" };
            let title = format!(
                "{caret} {}. {}{}",
                idx + 1,
                choice.preset.title,
                current_suffix
            );
            let description = format!("     {}", choice.description);

            let line_style = if is_selected {
                selected_option_style()
            } else {
                unselected_option_style()
            };
            let title = if is_selected {
                title
            } else {
                format!("  {}. {}{}", idx + 1, choice.preset.title, current_suffix)
            };
            lines.push(title.set_style(line_style).into());
            lines.push(description.set_style(line_style).into());
            if let Some(model_hint) = &choice.model_hint {
                lines.push(
                    format!("     {}: {}", model_hint.label, model_hint.model)
                        .set_style(line_style)
                        .into(),
                );
            }
            lines.push("".into());
        }

        lines.push("  Press Enter to continue".dim().into());
        lines.push("  Type to filter • Backspace clears".dim().into());
        if let Some(err) = self.error_message() {
            lines.push("".into());
            lines.push(err.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_pick_mode(&self, area: Rect, buf: &mut Buffer) {
        let selected_preset = self.selected_provider_preset();
        let (intro_primary, intro_secondary) = if selected_preset
            .as_ref()
            .is_some_and(ProviderPreset::uses_browser_auth)
        {
            (
                "Choose how Kimi Code should authenticate.",
                "Use browser sign-in to access Kimi Code models.",
            )
        } else if self.branding.is_open_interpreter {
            (
                "Choose how the OpenAI provider should authenticate.",
                "Use ChatGPT, Device Code, or your own OpenAI API key.",
            )
        } else {
            (
                self.branding.auth_intro_primary,
                self.branding.auth_intro_secondary,
            )
        };
        let mut lines: Vec<Line> = vec![
            Line::from(vec!["  ".into(), intro_primary.into()]),
            Line::from(vec!["  ".into(), intro_secondary.into()]),
            "".into(),
        ];

        let create_mode_item = |idx: usize,
                                selected_mode: SignInOption,
                                text: &str,
                                description: &str|
         -> Vec<Line<'static>> {
            let is_selected = self.highlighted_mode == selected_mode;
            let caret = if is_selected { ">" } else { " " };

            let line1 = if is_selected {
                Line::from(vec![
                    format!("{caret} {index}. ", index = idx + 1)
                        .set_style(selected_option_style()),
                    text.to_string().set_style(selected_option_style()),
                ])
            } else {
                format!("  {index}. {text}", index = idx + 1)
                    .set_style(unselected_option_style())
                    .into()
            };

            let line2 = if is_selected {
                Line::from(format!("     {description}")).set_style(selected_option_style())
            } else {
                Line::from(format!("     {description}")).set_style(unselected_option_style())
            };

            vec![line1, line2]
        };

        let chatgpt_description = if !self.is_chatgpt_login_allowed() {
            "ChatGPT login is disabled"
        } else {
            "Usage included with Plus, Pro, Business, and Enterprise plans"
        };
        let device_code_description = "Sign in from another device with a one-time code";

        for (idx, option) in self.displayed_sign_in_options().into_iter().enumerate() {
            match option {
                SignInOption::Browser => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Sign in with Kimi Code",
                        "Opens a browser and stores refreshable credentials locally",
                    ));
                }
                SignInOption::ChatGpt => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Sign in with ChatGPT",
                        chatgpt_description,
                    ));
                }
                SignInOption::DeviceCode => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Sign in with Device Code",
                        device_code_description,
                    ));
                }
                SignInOption::ApiKey => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Provide your own API key",
                        "Pay for what you use",
                    ));
                }
            }
            lines.push("".into());
        }

        if !self.is_api_login_allowed() {
            lines.push(
                "  API key login is disabled by this workspace. Sign in with ChatGPT to continue."
                    .dim()
                    .into(),
            );
            lines.push("".into());
        }
        lines.push(
            // Keep user-input tips on the shared accent so the Open Interpreter build has one
            // canonical blue instead of a separate cyan path.
            //     But leaving this for a future cleanup.
            "  Press Enter to continue".dim().into(),
        );
        if let Some(err) = self.error_message() {
            lines.push("".into());
            lines.push(err.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_connecting_app_server(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &ConnectingAppServerState,
    ) {
        let mut lines = vec![
            "  Connecting to the local background service...".into(),
            "".into(),
            format!("  {}", state.message).into(),
            "".into(),
            "  Press Esc to go back".dim().into(),
        ];
        if let Some(error) = self.error_message() {
            lines.push("".into());
            lines.push(error.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_provider_setup(&self, area: Rect, buf: &mut Buffer, state: &ProviderSetupState) {
        let mut intro_lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), state.preset.title.clone().bold()]),
            "".into(),
            format!("  {}", state.preset.description).into(),
            "".into(),
        ];
        if let Some(env_var) = state.api_key_env_var_name.as_deref()
            && state.api_key_prefilled_from_env
        {
            intro_lines.push(format!("  Detected {env_var} in your environment.").into());
            intro_lines.push(
                "  Paste or type a different key if you want to use another account."
                    .dim()
                    .into(),
            );
            intro_lines.push("".into());
        }
        if state.field != ProviderSetupField::ProviderName && !state.provider_name.trim().is_empty()
        {
            intro_lines.push(
                format!("  Provider: {}", state.provider_name.trim())
                    .dim()
                    .into(),
            );
        }
        if state.field != ProviderSetupField::BaseUrl && !state.base_url.trim().is_empty() {
            intro_lines.push(
                format!("  Base URL: {}", state.base_url.trim())
                    .dim()
                    .into(),
            );
        }
        if state.field != ProviderSetupField::ApiKey
            && (state.preset.api_key_required || !state.api_key.trim().is_empty())
        {
            let api_key_status = if state.api_key.trim().is_empty() {
                "not set"
            } else {
                "configured"
            };
            intro_lines.push(format!("  API key: {api_key_status}").dim().into());
        }

        let mut footer_lines: Vec<Line> = vec![
            "  Press Enter to continue".dim().into(),
            "  Press Esc to go back".dim().into(),
        ];
        if let Some(error) = self.error_message() {
            footer_lines.push("".into());
            footer_lines.push(error.red().into());
        }

        let intro_height = u16::try_from(intro_lines.len()).unwrap_or(u16::MAX);
        let footer_height = u16::try_from(footer_lines.len()).unwrap_or(u16::MAX);
        let [intro_area, input_area, footer_area] = Layout::vertical([
            Constraint::Length(intro_height),
            Constraint::Length(3),
            Constraint::Length(footer_height),
        ])
        .areas(area);

        Paragraph::new(intro_lines)
            .wrap(Wrap { trim: false })
            .render(intro_area, buf);

        let visible_width = usize::from(input_area.width.saturating_sub(2));
        let content_line: Line = if state.active_field_value().is_empty() {
            vec![state.active_field_placeholder().dim()].into()
        } else {
            let value = state.active_field_value();
            let visible_value = if value.chars().count() <= visible_width {
                value.to_string()
            } else if visible_width == 0 {
                String::new()
            } else if visible_width == 1 {
                "…".to_string()
            } else {
                let mut truncated = value.chars().take(visible_width - 1).collect::<String>();
                truncated.push('…');
                truncated
            };
            Line::from(visible_value)
        };
        Paragraph::new(content_line)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(state.active_field_label())
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(app_accent_style()),
            )
            .render(input_area, buf);
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .render(footer_area, buf);
    }

    fn render_loading_provider_models(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &LoadingProviderModelsState,
    ) {
        let lines = vec![
            Line::from(format!("> {}", state.provider_name).bold()),
            "".into(),
            format!("  Loading models for {}...", state.provider_name).into(),
            "".into(),
            "  Press Esc to go back".dim().into(),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_provider_model_selection(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &ProviderModelSelectionState,
    ) {
        let header_height = if state.using_unverified_models() {
            7
        } else {
            5
        };
        let footer_height = u16::from(self.error_message().is_some()) + 3;
        let [header_area, list_area, footer_area] = Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(footer_height),
        ])
        .areas(area);

        let filter_value = state.filter_query().trim();
        let filter_line = if filter_value.is_empty() {
            Line::from(vec!["  Filter models: ".into(), "type to search".dim()])
        } else {
            Line::from(vec![
                "  Filter models: ".into(),
                filter_value.fg(app_accent_color()),
            ])
        };
        let header_lines = vec![
            Line::from(format!("> {}", state.provider_name).bold()),
            "".into(),
            "  Choose a model for this new chat.".into(),
            filter_line,
            "".into(),
        ];
        let mut header_lines = header_lines;
        if state.using_unverified_models() {
            header_lines.push(
                "  Showing provider-advertised models even though compatibility metadata is incomplete."
                    .dim()
                    .into(),
            );
            header_lines.push("".into());
        }
        Paragraph::new(header_lines)
            .wrap(Wrap { trim: false })
            .render(header_area, buf);

        let filtered_indices = state.filtered_indices();
        let mut selected_line = 0u16;
        let mut list_lines: Vec<Line> = Vec::new();
        if filtered_indices.is_empty() {
            list_lines.push(
                "  No matching models. Keep typing or press Enter to type a custom model id."
                    .dim()
                    .into(),
            );
        } else {
            for (filtered_idx, original_idx) in filtered_indices.iter().copied().enumerate() {
                let preset = &state.models()[original_idx];
                if filtered_idx == state.selected_idx() {
                    selected_line = u16::try_from(list_lines.len()).unwrap_or(u16::MAX);
                }

                let is_selected = filtered_idx == state.selected_idx();
                let description =
                    (!preset.description.is_empty()).then_some(preset.description.as_str());
                if is_selected {
                    list_lines.push(
                        format!("> {}. {}", filtered_idx + 1, preset.model)
                            .set_style(selected_option_style())
                            .into(),
                    );
                    if let Some(description) = description {
                        list_lines.push(
                            format!("     {description}")
                                .set_style(selected_option_style())
                                .into(),
                        );
                    }
                } else {
                    list_lines.push(
                        format!("  {}. {}", filtered_idx + 1, preset.model)
                            .set_style(unselected_option_style())
                            .into(),
                    );
                    if let Some(description) = description {
                        list_lines.push(
                            format!("     {description}")
                                .set_style(unselected_option_style())
                                .into(),
                        );
                    }
                }
                list_lines.push("".into());
            }
        }

        let list_height = list_area.height;
        let scroll_y = if list_height == 0 || selected_line < list_height {
            0
        } else {
            selected_line.saturating_sub(list_height.saturating_sub(1))
        };
        Paragraph::new(list_lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll_y, 0))
            .render(list_area, buf);

        let mut footer_lines = vec![
            "  Press Enter to continue".dim().into(),
            "  Type to filter • Backspace clears • Esc goes back"
                .dim()
                .into(),
        ];
        if let Some(error) = self.error_message() {
            footer_lines.push("".into());
            footer_lines.push(error.red().into());
        }
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .render(footer_area, buf);
    }

    fn render_provider_reasoning_selection(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &ProviderReasoningSelectionState,
    ) {
        let mut lines = vec![
            Line::from(format!("> {} / {}", state.provider_name, state.model).bold()),
            "".into(),
            "  Choose a reasoning level for this new chat.".into(),
            "".into(),
        ];

        for (idx, choice) in state.choices().iter().enumerate() {
            let is_selected = idx == state.selected_idx();
            let prefix = if is_selected { ">" } else { " " };
            if is_selected {
                lines.push(
                    format!("{prefix} {}. {}", idx + 1, choice.label)
                        .set_style(selected_option_style())
                        .into(),
                );
                if let Some(description) = &choice.description {
                    lines.push(
                        format!("     {description}")
                            .set_style(selected_option_style())
                            .into(),
                    );
                }
            } else {
                lines.push(
                    format!("  {}. {}", idx + 1, choice.label)
                        .set_style(unselected_option_style())
                        .into(),
                );
                if let Some(description) = &choice.description {
                    lines.push(
                        format!("     {description}")
                            .set_style(unselected_option_style())
                            .into(),
                    );
                }
            }
            lines.push("".into());
        }

        lines.push("  Press Enter to continue".dim().into());
        lines.push("  Press Esc to go back".dim().into());
        if let Some(error) = self.error_message() {
            lines.push("".into());
            lines.push(error.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_local_provider_unavailable(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &LocalProviderUnavailableState,
    ) {
        let mut lines = vec![
            Line::from(format!("> {}", state.provider_name).bold()),
            "".into(),
            format!("  {}", state.message).into(),
        ];
        if let Some(hint) = local_provider_start_hint(state.provider_id.as_str()) {
            lines.push(format!("  {hint}").dim().into());
        }
        lines.push("".into());
        lines.push("  Press Enter to type a model manually".dim().into());
        if state.can_start_provider
            && local_provider_start_hint(state.provider_id.as_str()).is_none()
        {
            lines.push("  Press S to start the local server".dim().into());
        }
        lines.push("  Press Esc to go back".dim().into());
        if let Some(error) = self.error_message() {
            lines.push("".into());
            lines.push(error.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_manual_model_entry(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &ManualModelEntryState,
    ) {
        let [intro_area, input_area, footer_area] = Layout::vertical([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(if self.error_message().is_some() { 4 } else { 2 }),
        ])
        .areas(area);

        let intro_lines = vec![
            Line::from(format!("> {}", state.provider_name).bold()),
            "".into(),
            "  Enter a model name manually for this provider.".into(),
            "".into(),
        ];
        Paragraph::new(intro_lines)
            .wrap(Wrap { trim: false })
            .render(intro_area, buf);

        let content_line: Line = if state.value.is_empty() {
            vec![state.placeholder.as_str().dim()].into()
        } else {
            Line::from(state.value.clone())
        };
        Paragraph::new(content_line)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title("Model")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(app_accent_style()),
            )
            .render(input_area, buf);

        let mut footer_lines: Vec<Line> = vec![
            "  Press Enter to save".dim().into(),
            "  Press Esc to go back".dim().into(),
        ];
        if let Some(error) = self.error_message() {
            footer_lines.push("".into());
            footer_lines.push(error.red().into());
        }
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .render(footer_area, buf);
    }

    fn render_continue_in_browser(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = vec!["  ".into()];
        let sign_in_state = self.sign_in_state.read().unwrap();
        let continue_state = match &*sign_in_state {
            SignInState::ContinueInBrowser(state) => state,
            _ => return,
        };
        if self.animations_enabled && !self.animations_suppressed.get() {
            // Schedule a follow-up frame to keep the shimmer animation going.
            self.request_frame
                .schedule_frame_in(std::time::Duration::from_millis(100));
            spans.extend(shimmer_spans(continue_state.title.as_str()));
        } else {
            spans.push(continue_state.title.as_str().into());
        }
        let mut lines = vec![spans.into(), "".into()];

        let auth_url = if !continue_state.auth_url.is_empty() {
            lines.push("  If the link doesn't open automatically, open the following link to authenticate:".into());
            lines.push("".into());
            lines.push(Line::from(vec![
                "  ".into(),
                continue_state
                    .auth_url
                    .as_str()
                    .set_style(app_accent_underlined_style()),
            ]));
            lines.push("".into());
            if let Some(remote_hint) = continue_state.remote_hint.as_deref() {
                lines.push(Line::from(vec![
                    "  ".into(),
                    remote_hint.set_style(app_accent_style()),
                ]));
                lines.push("".into());
            }
            Some(continue_state.auth_url.clone())
        } else {
            None
        };

        lines.push("  Press Esc to cancel".dim().into());
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);

        // Wrap cyan+underlined URL cells with OSC 8 so the terminal treats
        // the entire region as a single clickable hyperlink.
        if let Some(url) = &auth_url {
            mark_url_hyperlink(buf, area, url);
        }
    }

    fn render_chatgpt_success_message(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            "✓ Signed in with your ChatGPT account".fg(Color::Green).into(),
            "".into(),
            "  Before you start:".into(),
            "".into(),
            format!(
                "  Decide how much autonomy you want to grant {}",
                self.branding.display_name
            )
            .into(),
            Line::from(vec![
                "  For more details see the ".into(),
                "\u{1b}]8;;https://developers.openai.com/codex/security\u{7}security docs\u{1b}]8;;\u{7}".underlined(),
            ])
            .dim(),
            "".into(),
            format!("  {} can make mistakes", self.branding.agent_name()).into(),
            "  Review the code it writes and commands it runs".dim().into(),
            "".into(),
            "  Powered by your ChatGPT account".into(),
            Line::from(vec![
                "  Uses your plan's rate limits and ".into(),
                "\u{1b}]8;;https://chatgpt.com/#settings\u{7}training data preferences\u{1b}]8;;\u{7}".underlined(),
            ])
            .dim(),
            "".into(),
            "  Press Enter to continue".fg(app_accent_color()).into(),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_provider_configured(&self, area: Rect, buf: &mut Buffer, message: &str) {
        let lines = vec![
            format!("✓ {message}").fg(Color::Green).into(),
            "".into(),
            "  You can switch models later with /model.".into(),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_api_key_entry(&self, area: Rect, buf: &mut Buffer, state: &ApiKeyInputState) {
        let [intro_area, input_area, footer_area] = Layout::vertical([
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Min(2),
        ])
        .areas(area);

        let mut intro_lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), self.branding.api_key_intro.bold()]),
            "".into(),
            "  Paste or type your API key below. It will be stored locally in auth.json.".into(),
            "".into(),
        ];
        if state.prepopulated_from_env {
            intro_lines.push("  Detected OPENAI_API_KEY environment variable.".into());
            intro_lines.push(
                "  Paste a different key if you prefer to use another account."
                    .dim()
                    .into(),
            );
            intro_lines.push("".into());
        }
        Paragraph::new(intro_lines)
            .wrap(Wrap { trim: false })
            .render(intro_area, buf);

        let content_line: Line = if state.value.is_empty() {
            vec!["Paste or type your API key".dim()].into()
        } else {
            Line::from(state.value.clone())
        };
        Paragraph::new(content_line)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title("API key")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(app_accent_style()),
            )
            .render(input_area, buf);

        let mut footer_lines: Vec<Line> = vec![
            "  Press Enter to save".dim().into(),
            "  Press Esc to go back".dim().into(),
        ];
        if let Some(error) = self.error_message() {
            footer_lines.push("".into());
            footer_lines.push(error.red().into());
        }
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .render(footer_area, buf);
    }

    fn handle_api_key_entry_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let mut should_save: Option<String> = None;
        let mut should_request_frame = false;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            if let SignInState::ApiKeyEntry(state) = &mut *guard {
                match key_event.code {
                    KeyCode::Esc => {
                        *guard = SignInState::PickMode;
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                    KeyCode::Enter => {
                        let trimmed = state.value.trim().to_string();
                        if trimmed.is_empty() {
                            self.set_error(Some("API key cannot be empty".to_string()));
                            should_request_frame = true;
                        } else {
                            should_save = Some(trimmed);
                        }
                    }
                    KeyCode::Backspace => {
                        if state.prepopulated_from_env {
                            state.value.clear();
                            state.prepopulated_from_env = false;
                        } else {
                            state.value.pop();
                        }
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                    KeyCode::Char(c)
                        if key_event.kind == KeyEventKind::Press
                            && !key_event.modifiers.contains(KeyModifiers::SUPER)
                            && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        if state.prepopulated_from_env {
                            state.value.clear();
                            state.prepopulated_from_env = false;
                        }
                        state.value.push(c);
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                    _ => {}
                }
                // handled; let guard drop before potential save
            } else {
                return false;
            }
        }

        if let Some(api_key) = should_save {
            self.save_api_key(api_key);
        } else if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_api_key_entry_paste(&mut self, pasted: String) -> bool {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return false;
        }

        let mut guard = self.sign_in_state.write().unwrap();
        if let SignInState::ApiKeyEntry(state) = &mut *guard {
            if state.prepopulated_from_env {
                state.value = trimmed.to_string();
                state.prepopulated_from_env = false;
            } else {
                state.value.push_str(trimmed);
            }
            self.set_error(/*message*/ None);
        } else {
            return false;
        }

        drop(guard);
        self.request_frame.schedule_frame();
        true
    }

    fn handle_provider_setup_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let mut should_request_frame = false;
        let mut should_save: Option<ProviderSetupState> = None;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            let SignInState::ProviderSetup(state) = &mut *guard else {
                return false;
            };

            match key_event.code {
                KeyCode::Esc => {
                    if let Some(previous_field) = state.previous_field() {
                        state.field = previous_field;
                    } else {
                        *guard = SignInState::ProviderPicker;
                    }
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if let Err(err) = state.validate_active_field() {
                        self.set_error(Some(err));
                        should_request_frame = true;
                    } else if state.advance_field() {
                        if let Err(err) = state.validate() {
                            self.set_error(Some(err));
                            should_request_frame = true;
                        } else {
                            should_save = Some(state.clone());
                        }
                    } else {
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                }
                KeyCode::Up => {
                    if let Some(previous_field) = state.previous_field() {
                        state.field = previous_field;
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                }
                KeyCode::Down => {
                    if state.validate_active_field().is_ok() && !state.advance_field() {
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                }
                KeyCode::Backspace => {
                    state.pop_char();
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                KeyCode::Char(c)
                    if key_event.kind == KeyEventKind::Press
                        && !key_event.modifiers.contains(KeyModifiers::SUPER)
                        && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                {
                    state.push_char(c);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                _ => {}
            }
        }

        if let Some(state) = should_save {
            self.save_provider_setup(state);
        } else if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_provider_setup_paste(&mut self, pasted: String) -> bool {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return false;
        }

        let mut guard = self.sign_in_state.write().unwrap();
        let SignInState::ProviderSetup(state) = &mut *guard else {
            return false;
        };
        if matches!(state.field, ProviderSetupField::ApiKey) && state.api_key_prefilled_from_env {
            state.replace_active_value(trimmed.to_string());
            state.api_key_prefilled_from_env = false;
        } else {
            state.active_field_value_mut().push_str(trimmed);
        }
        self.set_error(/*message*/ None);
        drop(guard);
        self.request_frame.schedule_frame();
        true
    }

    fn handle_loading_provider_models_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let initial_state = self.initial_provider_auth_state();
        let mut guard = self.sign_in_state.write().unwrap();
        let SignInState::LoadingProviderModels(_) = &*guard else {
            return false;
        };
        if matches!(key_event.code, KeyCode::Esc) {
            *guard = initial_state;
            self.set_error(/*message*/ None);
            drop(guard);
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_local_provider_unavailable_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let mut start_state: Option<LocalProviderUnavailableState> = None;
        let mut manual_state: Option<ManualModelEntryState> = None;
        let mut should_request_frame = false;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            let SignInState::LocalProviderUnavailable(state) = &mut *guard else {
                return false;
            };

            match key_event.code {
                KeyCode::Esc => {
                    *guard = SignInState::ProviderPicker;
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                KeyCode::Enter => {
                    manual_state = Some(ManualModelEntryState::new(
                        state.provider_id.clone(),
                        state.provider_name.clone(),
                        state.manual_model_placeholder.clone(),
                        state.default_manual_model.clone(),
                    ));
                    self.set_error(/*message*/ None);
                }
                KeyCode::Char('s') | KeyCode::Char('S') if state.can_start_provider => {
                    start_state = Some(state.clone());
                    self.set_error(/*message*/ None);
                }
                _ => {}
            }
        }

        if let Some(state) = manual_state {
            *self.sign_in_state.write().unwrap() = SignInState::ManualModelEntry(state);
            self.request_frame.schedule_frame();
            return true;
        }

        if let Some(state) = start_state {
            let Some(request_handle) = self.app_server_request_handle.clone() else {
                *self.sign_in_state.write().unwrap() = SignInState::LocalProviderUnavailable(state);
                self.set_error(Some(
                    "The local background service is still starting. Try again in a moment."
                        .to_string(),
                ));
                self.request_frame.schedule_frame();
                return true;
            };
            let sign_in_state = self.sign_in_state.clone();
            let error = self.error.clone();
            let request_frame = self.request_frame.clone();
            let loading_state = LoadingProviderModelsState {
                provider_id: state.provider_id.clone(),
                provider_name: state.provider_name.clone(),
                manual_model_placeholder: state.manual_model_placeholder.clone(),
                default_manual_model: state.default_manual_model.clone(),
            };
            *self.sign_in_state.write().unwrap() =
                SignInState::LoadingProviderModels(loading_state.clone());
            tokio::spawn(async move {
                if let Err(err) = start_local_provider(state.provider_id.as_str()).await {
                    *error.write().unwrap() =
                        Some(format!("Failed to start {}: {err}", state.provider_name));
                    *sign_in_state.write().unwrap() = SignInState::LocalProviderUnavailable(state);
                    request_frame.schedule_frame();
                    return;
                }

                match wait_for_local_provider_running(
                    state.provider_id.as_str(),
                    std::time::Duration::from_secs(8),
                )
                .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        *error.write().unwrap() = Some(format!(
                            "{} did not start in time. You can press Enter to type a model manually.",
                            state.provider_name
                        ));
                        *sign_in_state.write().unwrap() =
                            SignInState::LocalProviderUnavailable(state);
                        request_frame.schedule_frame();
                        return;
                    }
                    Err(err) => {
                        *error.write().unwrap() = Some(format!(
                            "Failed while waiting for {}: {err}",
                            state.provider_name
                        ));
                        *sign_in_state.write().unwrap() =
                            SignInState::LocalProviderUnavailable(state);
                        request_frame.schedule_frame();
                        return;
                    }
                }
                let result = request_handle
                    .request_typed::<ModelListResponse>(ClientRequest::ModelList {
                        request_id: onboarding_request_id(),
                        params: ModelListParams {
                            cursor: None,
                            limit: None,
                            include_hidden: Some(true),
                            model_provider: Some(loading_state.provider_id.clone()),
                        },
                    })
                    .await;
                let (next_state, next_error) = Self::resolve_provider_model_load_state(
                    loading_state,
                    /*preferred_model*/ None,
                    result,
                )
                .await;
                *error.write().unwrap() = next_error;
                *sign_in_state.write().unwrap() = next_state;
                request_frame.schedule_frame();
            });
            self.request_frame.schedule_frame();
            return true;
        }

        if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_provider_model_selection_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let initial_state = self.initial_provider_auth_state();
        let mut should_request_frame = false;
        let mut should_persist = None;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            let SignInState::ProviderModelSelection(state) = &mut *guard else {
                return false;
            };

            match key_event.code {
                KeyCode::Up => {
                    state.move_selection(/*delta*/ -1);
                    should_request_frame = true;
                }
                KeyCode::Down => {
                    state.move_selection(/*delta*/ 1);
                    should_request_frame = true;
                }
                KeyCode::Esc => {
                    if state.has_filter() {
                        state.clear_filter();
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    } else {
                        *guard = initial_state;
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                }
                KeyCode::Enter => {
                    if let Some(model) = state.selected_model() {
                        if let Some(reasoning_state) = ProviderReasoningSelectionState::new(
                            state.provider_id.clone(),
                            state.provider_name.clone(),
                            model.clone(),
                        ) {
                            *guard = SignInState::ProviderReasoningSelection(reasoning_state);
                            self.set_error(/*message*/ None);
                            should_request_frame = true;
                        } else {
                            should_persist = Some((
                                state.provider_id.clone(),
                                state.provider_name.clone(),
                                model.model.clone(),
                                Some(model.default_reasoning_effort),
                                SignInState::ProviderModelSelection(state.clone()),
                            ));
                        }
                    } else if state.has_filter() {
                        *guard = SignInState::ManualModelEntry(ManualModelEntryState::new(
                            state.provider_id.clone(),
                            state.provider_name.clone(),
                            state.manual_model_placeholder.clone(),
                            state.filter_query().to_string(),
                        ));
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                }
                KeyCode::Backspace => {
                    state.pop_filter_char();
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                KeyCode::Char(c)
                    if key_event.kind == KeyEventKind::Press
                        && !key_event.modifiers.contains(KeyModifiers::SUPER)
                        && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                {
                    state.push_filter_char(c);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                _ => {}
            }
        }

        if let Some((provider_id, provider_name, model, effort, fallback_state)) = should_persist {
            self.persist_provider_model_selection(
                provider_id,
                provider_name,
                model,
                effort,
                fallback_state,
            );
        } else if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_provider_reasoning_selection_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let initial_state = self.initial_provider_auth_state();
        let mut should_request_frame = false;
        let mut should_persist = None;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            let SignInState::ProviderReasoningSelection(state) = &mut *guard else {
                return false;
            };

            match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    state.move_selection(/*delta*/ -1);
                    should_request_frame = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    state.move_selection(/*delta*/ 1);
                    should_request_frame = true;
                }
                KeyCode::Esc => {
                    *guard = initial_state;
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                KeyCode::Enter => {
                    should_persist = Some((
                        state.provider_id.clone(),
                        state.provider_name.clone(),
                        state.model.clone(),
                        state.selected_effort(),
                        SignInState::ProviderReasoningSelection(state.clone()),
                    ));
                }
                _ => {}
            }
        }

        if let Some((provider_id, provider_name, model, effort, fallback_state)) = should_persist {
            self.persist_provider_model_selection(
                provider_id,
                provider_name,
                model,
                effort,
                fallback_state,
            );
        } else if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_manual_model_entry_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let initial_state = self.initial_provider_auth_state();
        let mut should_request_frame = false;
        let mut should_persist = None;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            let SignInState::ManualModelEntry(state) = &mut *guard else {
                return false;
            };

            match key_event.code {
                KeyCode::Esc => {
                    *guard = initial_state;
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                KeyCode::Enter => match state.validate() {
                    Ok(()) => {
                        should_persist = Some((
                            state.provider_id.clone(),
                            state.provider_name.clone(),
                            state.selected_model(),
                            SignInState::ManualModelEntry(state.clone()),
                        ));
                    }
                    Err(err) => {
                        self.set_error(Some(err));
                        should_request_frame = true;
                    }
                },
                KeyCode::Backspace => {
                    state.pop_char();
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                KeyCode::Char(c)
                    if key_event.kind == KeyEventKind::Press
                        && !key_event.modifiers.contains(KeyModifiers::SUPER)
                        && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                {
                    state.push_char(c);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
                _ => {}
            }
        }

        if let Some((provider_id, provider_name, model, fallback_state)) = should_persist {
            self.persist_provider_model_selection(
                provider_id,
                provider_name,
                model,
                /*effort*/ None,
                fallback_state,
            );
        } else if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_manual_model_entry_paste(&mut self, pasted: String) -> bool {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return false;
        }

        let mut guard = self.sign_in_state.write().unwrap();
        let SignInState::ManualModelEntry(state) = &mut *guard else {
            return false;
        };
        state.replace_value(trimmed.to_string());
        self.set_error(/*message*/ None);
        drop(guard);
        self.request_frame.schedule_frame();
        true
    }

    fn handle_provider_picker_paste(&mut self, pasted: String) -> bool {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return false;
        }

        let guard = self.sign_in_state.read().unwrap();
        if !matches!(&*guard, SignInState::ProviderPicker) {
            return false;
        }
        drop(guard);

        self.provider_filter_query.push_str(trimmed);
        self.sync_provider_highlight_to_filter();
        self.set_error(/*message*/ None);
        self.request_frame.schedule_frame();
        true
    }

    fn handle_provider_model_selection_paste(&mut self, pasted: String) -> bool {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return false;
        }

        let mut guard = self.sign_in_state.write().unwrap();
        let SignInState::ProviderModelSelection(state) = &mut *guard else {
            return false;
        };
        state.replace_filter_query(trimmed.to_string());
        self.set_error(/*message*/ None);
        drop(guard);
        self.request_frame.schedule_frame();
        true
    }

    fn initial_provider_auth_state(&self) -> SignInState {
        if self.branding.is_open_interpreter {
            SignInState::ProviderPicker
        } else {
            SignInState::PickMode
        }
    }

    fn save_provider_setup(&mut self, state: ProviderSetupState) {
        self.set_error(/*message*/ None);
        let Some(request_handle) = self.app_server_request_handle.clone() else {
            let provider_name = state.provider_identity().name;
            self.defer_app_server_request(
                PendingAppServerAction::SaveProviderSetup(state.clone()),
                SignInState::ProviderSetup(state),
                SignInState::ConnectingAppServer(ConnectingAppServerState {
                    message: format!("Preparing {provider_name}..."),
                }),
            );
            return;
        };
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        let identity = state.provider_identity();
        let preset = state.preset.clone();
        let loading_state = LoadingProviderModelsState {
            provider_id: identity.id,
            provider_name: identity.name,
            manual_model_placeholder: preset.model_placeholder.to_string(),
            default_manual_model: preset.default_model.unwrap_or_default(),
        };
        tokio::spawn(async move {
            match request_handle
                .request_typed::<ConfigWriteResponse>(ClientRequest::ConfigBatchWrite {
                    request_id: onboarding_request_id(),
                    params: ConfigBatchWriteParams {
                        edits: state.provider_definition_edits(),
                        file_path: None,
                        expected_version: None,
                        reload_user_config: true,
                    },
                })
                .await
            {
                Ok(_) => {
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() =
                        SignInState::LoadingProviderModels(loading_state.clone());
                    request_frame.schedule_frame();
                    let models = request_handle
                        .request_typed::<ModelListResponse>(ClientRequest::ModelList {
                            request_id: onboarding_request_id(),
                            params: ModelListParams {
                                cursor: None,
                                limit: None,
                                include_hidden: Some(true),
                                model_provider: Some(loading_state.provider_id.clone()),
                            },
                        })
                        .await;
                    let (next_state, next_error) = Self::resolve_provider_model_load_state(
                        loading_state,
                        /*preferred_model*/ None,
                        models,
                    )
                    .await;
                    *error.write().unwrap() = next_error;
                    *sign_in_state.write().unwrap() = next_state;
                }
                Err(err) => {
                    *error.write().unwrap() = Some(format!("Failed to save provider: {err}"));
                    *sign_in_state.write().unwrap() = SignInState::ProviderSetup(state);
                }
            }
            request_frame.schedule_frame();
        });
        self.request_frame.schedule_frame();
    }

    fn start_api_key_entry(&mut self) {
        if !self.is_api_login_allowed() {
            self.disallow_api_login();
            return;
        }
        self.set_error(/*message*/ None);
        let prefill_from_env = crate::login_support::read_openai_api_key_from_env_trimmed();
        let mut guard = self.sign_in_state.write().unwrap();
        match &mut *guard {
            SignInState::ApiKeyEntry(state) => {
                if state.value.is_empty() {
                    if let Some(prefill) = prefill_from_env {
                        state.value = prefill;
                        state.prepopulated_from_env = true;
                    } else {
                        state.prepopulated_from_env = false;
                    }
                }
            }
            _ => {
                *guard = SignInState::ApiKeyEntry(ApiKeyInputState {
                    value: prefill_from_env.clone().unwrap_or_default(),
                    prepopulated_from_env: prefill_from_env.is_some(),
                });
            }
        }
        drop(guard);
        self.request_frame.schedule_frame();
    }

    fn save_api_key(&mut self, api_key: String) {
        if !self.is_api_login_allowed() {
            self.disallow_api_login();
            return;
        }
        self.set_error(/*message*/ None);
        let fallback_state = SignInState::ApiKeyEntry(ApiKeyInputState {
            value: api_key.clone(),
            prepopulated_from_env: false,
        });
        let Some(request_handle) = self.app_server_request_handle.clone() else {
            self.defer_app_server_request(
                PendingAppServerAction::SaveApiKey(api_key),
                fallback_state,
                SignInState::ConnectingAppServer(ConnectingAppServerState {
                    message: "Preparing OpenAI authentication...".to_string(),
                }),
            );
            return;
        };
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        let preset = provider_presets()
            .into_iter()
            .find(|provider_preset| provider_preset.provider_id == "openai")
            .expect("openai preset should exist");
        let loading_state = LoadingProviderModelsState {
            provider_id: preset.provider_id.to_string(),
            provider_name: preset.title.to_string(),
            manual_model_placeholder: preset.model_placeholder.to_string(),
            default_manual_model: preset.default_model.unwrap_or_default(),
        };
        tokio::spawn(async move {
            match request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params: LoginAccountParams::ApiKey {
                        api_key: api_key.clone(),
                    },
                })
                .await
            {
                Ok(LoginAccountResponse::ApiKey {}) => {
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() =
                        SignInState::LoadingProviderModels(loading_state.clone());
                    request_frame.schedule_frame();
                    let models = request_handle
                        .request_typed::<ModelListResponse>(ClientRequest::ModelList {
                            request_id: onboarding_request_id(),
                            params: ModelListParams {
                                cursor: None,
                                limit: None,
                                include_hidden: Some(true),
                                model_provider: Some(loading_state.provider_id.clone()),
                            },
                        })
                        .await;
                    let (next_state, next_error) = Self::resolve_provider_model_load_state(
                        loading_state,
                        /*preferred_model*/ None,
                        models,
                    )
                    .await;
                    *error.write().unwrap() = next_error;
                    *sign_in_state.write().unwrap() = next_state;
                }
                Ok(other) => {
                    *error.write().unwrap() = Some(format!(
                        "Unexpected account/login/start response: {other:?}"
                    ));
                    *sign_in_state.write().unwrap() = SignInState::ApiKeyEntry(ApiKeyInputState {
                        value: api_key,
                        prepopulated_from_env: false,
                    });
                }
                Err(err) => {
                    *error.write().unwrap() = Some(format!("Failed to save API key: {err}"));
                    *sign_in_state.write().unwrap() = SignInState::ApiKeyEntry(ApiKeyInputState {
                        value: api_key,
                        prepopulated_from_env: false,
                    });
                }
            }
            request_frame.schedule_frame();
        });
        self.request_frame.schedule_frame();
    }

    fn handle_existing_chatgpt_login(&mut self) -> bool {
        if matches!(
            self.login_status,
            LoginStatus::AuthMode(AppServerAuthMode::Chatgpt)
                | LoginStatus::AuthMode(AppServerAuthMode::ChatgptAuthTokens)
        ) {
            self.start_provider_model_selection(
                provider_presets()
                    .into_iter()
                    .find(|preset| preset.provider_id == "openai")
                    .expect("openai preset should exist"),
            );
            true
        } else {
            false
        }
    }

    #[cfg(feature = "direct-login")]
    fn start_kimi_code_login(&mut self) {
        self.set_error(/*message*/ None);
        let Some(request_handle) = self.app_server_request_handle.clone() else {
            self.defer_app_server_request(
                PendingAppServerAction::StartKimiCodeLogin,
                SignInState::PickMode,
                SignInState::ConnectingAppServer(ConnectingAppServerState {
                    message: "Preparing Kimi Code sign-in...".to_string(),
                }),
            );
            return;
        };
        let Some(preset) = provider_presets()
            .into_iter()
            .find(|provider_preset| provider_preset.provider_id == KIMI_FOR_CODING_PROVIDER_ID)
        else {
            self.set_error(Some("Kimi Code provider is unavailable.".to_string()));
            *self.sign_in_state.write().unwrap() = SignInState::PickMode;
            self.request_frame.schedule_frame();
            return;
        };
        if let Some(abort_handle) = self.provider_login_abort.lock().unwrap().take() {
            abort_handle.abort();
        }
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        let interpreter_home = self.interpreter_home.clone();
        let provider_login_abort = self.provider_login_abort.clone();
        let loading_state = LoadingProviderModelsState {
            provider_id: preset.provider_id.to_string(),
            provider_name: preset.title.to_string(),
            manual_model_placeholder: preset.model_placeholder.to_string(),
            default_manual_model: preset.default_model.unwrap_or_default(),
        };
        let task = tokio::spawn(async move {
            let authorization =
                match kimi_code::request_device_authorization(&interpreter_home).await {
                    Ok(authorization) => authorization,
                    Err(err) => {
                        *error.write().unwrap() = Some(err.to_string());
                        *sign_in_state.write().unwrap() = SignInState::PickMode;
                        provider_login_abort.lock().unwrap().take();
                        request_frame.schedule_frame();
                        return;
                    }
                };
            let _ = kimi_code::open_verification_url(&authorization.verification_uri_complete);
            *error.write().unwrap() = None;
            *sign_in_state.write().unwrap() =
                SignInState::ContinueInBrowser(ContinueInBrowserState {
                    login_id: None,
                    auth_url: authorization.verification_uri_complete.clone(),
                    title: "Finish signing in via your browser".to_string(),
                    remote_hint: None,
                });
            request_frame.schedule_frame();

            if let Err(err) =
                kimi_code::complete_device_authorization(&interpreter_home, &authorization).await
            {
                *error.write().unwrap() = Some(err.to_string());
                *sign_in_state.write().unwrap() = SignInState::PickMode;
                provider_login_abort.lock().unwrap().take();
                request_frame.schedule_frame();
                return;
            }

            let command_cwd = std::env::current_dir().unwrap_or_else(|_| interpreter_home.clone());
            let edits =
                browser_auth_provider_definition_edits(KIMI_FOR_CODING_PROVIDER_ID, &command_cwd);
            if let Err(err) = request_handle
                .request_typed::<ConfigWriteResponse>(ClientRequest::ConfigBatchWrite {
                    request_id: onboarding_request_id(),
                    params: ConfigBatchWriteParams {
                        edits,
                        file_path: None,
                        expected_version: None,
                        reload_user_config: true,
                    },
                })
                .await
            {
                *error.write().unwrap() = Some(format!("Failed to save Kimi Code login: {err}"));
                *sign_in_state.write().unwrap() = SignInState::PickMode;
                provider_login_abort.lock().unwrap().take();
                request_frame.schedule_frame();
                return;
            }

            *sign_in_state.write().unwrap() =
                SignInState::LoadingProviderModels(loading_state.clone());
            request_frame.schedule_frame();
            let models = request_handle
                .request_typed::<ModelListResponse>(ClientRequest::ModelList {
                    request_id: onboarding_request_id(),
                    params: ModelListParams {
                        cursor: None,
                        limit: None,
                        include_hidden: Some(true),
                        model_provider: Some(loading_state.provider_id.clone()),
                    },
                })
                .await;
            let (next_state, next_error) = Self::resolve_provider_model_load_state(
                loading_state,
                /*preferred_model*/ None,
                models,
            )
            .await;
            *error.write().unwrap() = next_error;
            *sign_in_state.write().unwrap() = next_state;
            provider_login_abort.lock().unwrap().take();
            request_frame.schedule_frame();
        });
        *self.provider_login_abort.lock().unwrap() = Some(task.abort_handle());
        self.request_frame.schedule_frame();
    }

    #[cfg(not(feature = "direct-login"))]
    fn start_kimi_code_login(&mut self) {
        self.set_error(Some(
            "Kimi Code browser login is unavailable in this build.".to_string(),
        ));
        *self.sign_in_state.write().unwrap() = SignInState::PickMode;
        self.request_frame.schedule_frame();
    }

    /// Kicks off the ChatGPT auth flow and keeps the UI state consistent with the attempt.
    fn start_chatgpt_login(&mut self) {
        // If we're already authenticated with ChatGPT, don't start a new login –
        // just proceed to the success message flow.
        if self.handle_existing_chatgpt_login() {
            return;
        }

        self.set_error(/*message*/ None);
        let Some(request_handle) = self.app_server_request_handle.clone() else {
            self.defer_app_server_request(
                PendingAppServerAction::StartChatGptLogin,
                SignInState::PickMode,
                SignInState::ConnectingAppServer(ConnectingAppServerState {
                    message: "Preparing OpenAI sign-in...".to_string(),
                }),
            );
            return;
        };
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            match request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params: LoginAccountParams::Chatgpt,
                })
                .await
            {
                Ok(LoginAccountResponse::Chatgpt { login_id, auth_url }) => {
                    maybe_open_auth_url_in_browser(&request_handle, &auth_url);
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() =
                        SignInState::ContinueInBrowser(ContinueInBrowserState {
                            login_id: Some(login_id),
                            auth_url,
                            title: "Finish signing in via your browser".to_string(),
                            remote_hint: Some(
                                "On a remote or headless machine? Press Esc and choose Sign in with Device Code."
                                    .to_string(),
                            ),
                        });
                }
                Ok(other) => {
                    *sign_in_state.write().unwrap() = SignInState::PickMode;
                    *error.write().unwrap() = Some(format!(
                        "Unexpected account/login/start response: {other:?}"
                    ));
                }
                Err(err) => {
                    *sign_in_state.write().unwrap() = SignInState::PickMode;
                    *error.write().unwrap() = Some(err.to_string());
                }
            }
            request_frame.schedule_frame();
        });
    }

    fn start_device_code_login(&mut self) {
        if self.handle_existing_chatgpt_login() {
            return;
        }

        self.set_error(/*message*/ None);
        let Some(request_handle) = self.app_server_request_handle.clone() else {
            self.defer_app_server_request(
                PendingAppServerAction::StartDeviceCodeLogin,
                SignInState::PickMode,
                SignInState::ConnectingAppServer(ConnectingAppServerState {
                    message: "Preparing device-code sign-in...".to_string(),
                }),
            );
            return;
        };
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        *sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState {
                login_id: None,
                verification_url: None,
                user_code: None,
            });
        self.request_frame.schedule_frame();
        tokio::spawn(async move {
            match request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params: LoginAccountParams::ChatgptDeviceCode,
                })
                .await
            {
                Ok(LoginAccountResponse::ChatgptDeviceCode {
                    login_id,
                    verification_url,
                    user_code,
                }) => {
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() =
                        SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState {
                            login_id: Some(login_id),
                            verification_url: Some(verification_url),
                            user_code: Some(user_code),
                        });
                }
                Ok(other) => {
                    *sign_in_state.write().unwrap() = SignInState::PickMode;
                    *error.write().unwrap() = Some(format!(
                        "Unexpected account/login/start response: {other:?}"
                    ));
                }
                Err(err) => {
                    *sign_in_state.write().unwrap() = SignInState::PickMode;
                    *error.write().unwrap() = Some(err.to_string());
                }
            }
            request_frame.schedule_frame();
        });
    }

    pub(crate) fn on_account_login_completed(
        &mut self,
        notification: AccountLoginCompletedNotification,
    ) {
        let Some(login_id) = notification.login_id else {
            return;
        };
        let guard = self.sign_in_state.read().unwrap();
        let is_matching_login = matches!(
            &*guard,
            SignInState::ContinueInBrowser(state)
                if state.login_id.as_deref() == Some(login_id.as_str())
        ) || matches!(
            &*guard,
            SignInState::ChatGptDeviceCode(state) if state.login_id.as_deref() == Some(login_id.as_str())
        );
        drop(guard);
        if !is_matching_login {
            return;
        }

        if notification.success {
            self.set_error(/*message*/ None);
            *self.sign_in_state.write().unwrap() = SignInState::ChatGptSuccessMessage;
        } else {
            self.set_error(notification.error);
            *self.sign_in_state.write().unwrap() = SignInState::PickMode;
        }
        self.request_frame.schedule_frame();
    }

    pub(crate) fn on_account_updated(&mut self, notification: AccountUpdatedNotification) {
        self.login_status = notification
            .auth_mode
            .map(LoginStatus::AuthMode)
            .unwrap_or(LoginStatus::NotAuthenticated);
    }
}

impl StepStateProvider for AuthModeWidget {
    fn get_step_state(&self) -> StepState {
        let sign_in_state = self.sign_in_state.read().unwrap();
        match &*sign_in_state {
            SignInState::ProviderPicker
            | SignInState::PickMode
            | SignInState::ConnectingAppServer(_)
            | SignInState::ProviderSetup(_)
            | SignInState::LoadingProviderModels(_)
            | SignInState::LocalProviderUnavailable(_)
            | SignInState::ProviderModelSelection(_)
            | SignInState::ProviderReasoningSelection(_)
            | SignInState::ManualModelEntry(_)
            | SignInState::ApiKeyEntry(_)
            | SignInState::ContinueInBrowser(_)
            | SignInState::ChatGptDeviceCode(_)
            | SignInState::ChatGptSuccessMessage => StepState::InProgress,
            SignInState::ProviderConfigured(_) => StepState::Complete,
        }
    }
}

impl WidgetRef for AuthModeWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let sign_in_state = self.sign_in_state.read().unwrap();
        match &*sign_in_state {
            SignInState::ProviderPicker => {
                self.render_provider_picker(area, buf);
            }
            SignInState::PickMode => {
                self.render_pick_mode(area, buf);
            }
            SignInState::ConnectingAppServer(state) => {
                self.render_connecting_app_server(area, buf, state);
            }
            SignInState::ProviderSetup(state) => {
                self.render_provider_setup(area, buf, state);
            }
            SignInState::LoadingProviderModels(state) => {
                self.render_loading_provider_models(area, buf, state);
            }
            SignInState::LocalProviderUnavailable(state) => {
                self.render_local_provider_unavailable(area, buf, state);
            }
            SignInState::ProviderModelSelection(state) => {
                self.render_provider_model_selection(area, buf, state);
            }
            SignInState::ProviderReasoningSelection(state) => {
                self.render_provider_reasoning_selection(area, buf, state);
            }
            SignInState::ManualModelEntry(state) => {
                self.render_manual_model_entry(area, buf, state);
            }
            SignInState::ProviderConfigured(message) => {
                self.render_provider_configured(area, buf, message);
            }
            SignInState::ContinueInBrowser(_) => {
                self.render_continue_in_browser(area, buf);
            }
            SignInState::ChatGptDeviceCode(state) => {
                headless_chatgpt_login::render_device_code_login(self, area, buf, state);
            }
            SignInState::ChatGptSuccessMessage => {
                self.render_chatgpt_success_message(area, buf);
            }
            SignInState::ApiKeyEntry(state) => {
                self.render_api_key_entry(area, buf, state);
            }
        }
    }
}

pub(super) fn maybe_open_auth_url_in_browser(request_handle: &AppServerRequestHandle, url: &str) {
    #[cfg(not(feature = "system-browser"))]
    {
        let _ = request_handle;
        let _ = url;
        return;
    }

    #[cfg(not(feature = "embedded-app-server"))]
    {
        let _ = request_handle;
        let _ = url;
        return;
    }

    #[cfg(all(feature = "embedded-app-server", feature = "system-browser"))]
    if !matches!(request_handle, AppServerRequestHandle::InProcess(_)) {
        return;
    }

    #[cfg(all(feature = "embedded-app-server", feature = "system-browser"))]
    if let Err(err) = webbrowser::open(url) {
        tracing::warn!("failed to open browser for login URL: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_core::config::ConfigBuilder;
    use codex_app_server_client::AppServerRequestHandle;
    use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
    use codex_app_server_client::InProcessAppServerClient;
    use codex_app_server_client::InProcessClientStartArgs;
    use codex_arg0::Arg0DispatchPaths;
    use codex_cloud_requirements::cloud_requirements_loader_for_storage;
    use codex_config::types::AuthCredentialsStoreMode;
    use codex_exec_server::EnvironmentManager;

    use codex_protocol::openai_models::ModelPreset;
    use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
    use codex_protocol::openai_models::ReasoningEffortPreset;
    use codex_protocol::openai_models::default_input_modalities;
    use codex_protocol::protocol::SessionSource;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::onboarding::provider_setup::default_provider_preset_id;
    use crate::test_backend::VT100Backend;

    async fn widget_forced_chatgpt() -> (AuthModeWidget, TempDir) {
        let codex_home = TempDir::new().unwrap();
        let codex_home_path = codex_home.path().to_path_buf();
        let config = ConfigBuilder::default()
            .codex_home(codex_home_path.clone())
            .build()
            .await
            .unwrap();
        let client = InProcessAppServerClient::start(InProcessClientStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config: Arc::new(config),
            cli_overrides: Vec::new(),
            loader_overrides: Default::default(),
            cloud_requirements: cloud_requirements_loader_for_storage(
                codex_home_path.clone(),
                /*enable_codex_api_key_env*/ false,
                AuthCredentialsStoreMode::File,
                "https://chatgpt.com/backend-api/".to_string(),
            ),
            feedback: CodexFeedback::new(),
            log_db: None,
            environment_manager: Arc::new(EnvironmentManager::new(/*exec_server_url*/ None)),
            config_warnings: Vec::new(),
            session_source: SessionSource::Cli,
            enable_codex_api_key_env: false,
            client_name: "test".to_string(),
            client_version: "test".to_string(),
            experimental_api: true,
            opt_out_notification_methods: Vec::new(),
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await
        .unwrap();
        let widget = AuthModeWidget {
            request_frame: FrameRequester::test_dummy(),
            interpreter_home: codex_home.path().to_path_buf(),
            highlighted_mode: SignInOption::ChatGpt,
            highlighted_provider_id: default_provider_preset_id(),
            provider_filter_query: String::new(),
            configured_model_providers: std::collections::HashMap::new(),
            current_model_provider_id: default_provider_preset_id(),
            current_model: None,
            imported_model_provider_id: None,
            imported_model: None,
            suppress_current_provider: false,
            provider_readiness_snapshot: ProviderReadinessSnapshot::default(),
            error: Arc::new(RwLock::new(None)),
            sign_in_state: Arc::new(RwLock::new(SignInState::PickMode)),
            branding: ProductBranding::for_open_interpreter(/*is_open_interpreter*/ false),
            login_status: LoginStatus::NotAuthenticated,
            app_server_request_handle: Some(AppServerRequestHandle::InProcess(
                client.request_handle(),
            )),
            pending_app_server_request: Arc::new(RwLock::new(None)),
            forced_chatgpt_workspace_id: None,
            forced_login_method: Some(ForcedLoginMethod::Chatgpt),
            animations_enabled: true,
            animations_suppressed: std::cell::Cell::new(false),
            provider_login_abort: Arc::new(Mutex::new(None)),
        };
        (widget, codex_home)
    }

    async fn widget_open_interpreter() -> (AuthModeWidget, TempDir) {
        let (mut widget, tmp) = widget_forced_chatgpt().await;
        widget.branding = ProductBranding::for_open_interpreter(/*is_open_interpreter*/ true);
        widget.forced_login_method = None;
        widget.highlighted_provider_id = default_provider_preset_id();
        *widget.sign_in_state.write().unwrap() =
            initial_sign_in_state(widget.branding, widget.forced_login_method);
        (widget, tmp)
    }

    fn sample_model_preset(model: &str) -> ModelPreset {
        ModelPreset {
            id: model.to_string(),
            model: model.to_string(),
            display_name: model.to_string(),
            description: format!("{model} description"),
            default_reasoning_effort: ReasoningEffortConfig::Medium,
            supported_reasoning_efforts: vec![
                ReasoningEffortPreset {
                    effort: ReasoningEffortConfig::Low,
                    description: "Low reasoning".to_string(),
                },
                ReasoningEffortPreset {
                    effort: ReasoningEffortConfig::Medium,
                    description: "Balanced reasoning".to_string(),
                },
            ],
            supports_personality: false,
            is_default: false,
            upgrade: None,
            show_in_picker: true,
            availability_nux: None,
            supported_in_api: true,
            input_modalities: default_input_modalities(),
            additional_speed_tiers: Vec::new(),
        }
    }

    #[tokio::test]
    async fn api_key_flow_disabled_when_chatgpt_forced() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;

        widget.start_api_key_entry();

        assert_eq!(
            widget.error_message().as_deref(),
            Some(API_KEY_DISABLED_MESSAGE)
        );
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
    }

    #[tokio::test]
    async fn saving_api_key_is_blocked_when_chatgpt_forced() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;

        widget.save_api_key("sk-test".to_string());

        assert_eq!(
            widget.error_message().as_deref(),
            Some(API_KEY_DISABLED_MESSAGE)
        );
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
        assert_eq!(widget.login_status, LoginStatus::NotAuthenticated);
    }

    #[tokio::test]
    async fn existing_chatgpt_auth_tokens_login_counts_as_signed_in() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;
        widget.login_status = LoginStatus::AuthMode(AppServerAuthMode::ChatgptAuthTokens);

        let handled = widget.handle_existing_chatgpt_login();

        assert_eq!(handled, true);
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::LoadingProviderModels(_)
        ));
    }

    #[tokio::test]
    async fn cancel_active_attempt_resets_browser_login_state() {
        let (widget, _tmp) = widget_forced_chatgpt().await;
        *widget.error.write().unwrap() = Some("still logging in".to_string());
        *widget.sign_in_state.write().unwrap() =
            SignInState::ContinueInBrowser(ContinueInBrowserState {
                login_id: Some("login-1".to_string()),
                auth_url: "https://auth.example.com".to_string(),
                title: "Finish signing in via your browser".to_string(),
                remote_hint: None,
            });

        widget.cancel_active_attempt();

        assert_eq!(widget.error_message(), None);
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
    }

    #[tokio::test]
    async fn cancel_active_attempt_notifies_device_code_login() {
        let (widget, _tmp) = widget_forced_chatgpt().await;
        *widget.error.write().unwrap() = Some("still logging in".to_string());
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState {
                login_id: Some("login-1".to_string()),
                verification_url: Some("https://auth.example.com/device".to_string()),
                user_code: Some("ABCD-1234".to_string()),
            });

        widget.cancel_active_attempt();

        assert_eq!(widget.error_message(), None);
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
    }

    #[tokio::test]
    async fn device_code_login_completion_matches_active_login_id() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState {
                login_id: Some("login-1".to_string()),
                verification_url: Some("https://auth.example.com/device".to_string()),
                user_code: Some("ABCD-1234".to_string()),
            });

        widget.on_account_login_completed(AccountLoginCompletedNotification {
            login_id: Some("login-1".to_string()),
            success: true,
            error: None,
        });

        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::ChatGptSuccessMessage
        ));
    }

    /// Collects all buffer cell symbols that contain the OSC 8 open sequence
    /// for the given URL.  Returns the concatenated "inner" characters.
    fn collect_osc8_chars(buf: &Buffer, area: Rect, url: &str) -> String {
        let open = format!("\x1B]8;;{url}\x07");
        let close = "\x1B]8;;\x07";
        let mut chars = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let sym = buf[(x, y)].symbol();
                if let Some(rest) = sym.strip_prefix(open.as_str())
                    && let Some(ch) = rest.strip_suffix(close)
                {
                    chars.push_str(ch);
                }
            }
        }
        chars
    }

    #[test]
    fn continue_in_browser_renders_osc8_hyperlink() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
        let url = "https://auth.example.com/login?state=abc123";
        *widget.sign_in_state.write().unwrap() =
            SignInState::ContinueInBrowser(ContinueInBrowserState {
                login_id: Some("login-1".to_string()),
                auth_url: url.to_string(),
                title: "Finish signing in via your browser".to_string(),
                remote_hint: None,
            });

        // Render into a narrow buffer so the URL wraps across multiple rows.
        let area = Rect::new(0, 0, 30, 20);
        let mut buf = Buffer::empty(area);
        widget.render_continue_in_browser(area, &mut buf);

        // Every character of the URL should be present as an OSC 8 cell.
        let found = collect_osc8_chars(&buf, area, url);
        assert_eq!(found, url, "OSC 8 hyperlink should cover the full URL");
    }

    #[test]
    fn auth_widget_suppresses_animations_when_device_code_is_visible() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
                "request-1".to_string(),
                "login-1".to_string(),
                "https://chatgpt.com/device".to_string(),
                "ABCD-EFGH".to_string(),
            ));

        assert_eq!(widget.should_suppress_animations(), true);
    }

    #[test]
    fn auth_widget_suppresses_animations_while_requesting_device_code() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
        *widget.sign_in_state.write().unwrap() = SignInState::ChatGptDeviceCode(
            ContinueWithDeviceCodeState::pending("request-1".to_string()),
        );

        assert_eq!(widget.should_suppress_animations(), true);
    }

    #[tokio::test]
    async fn device_code_login_completion_advances_to_success_message() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
                "request-1".to_string(),
                "login-1".to_string(),
                "https://chatgpt.com/device".to_string(),
                "ABCD-EFGH".to_string(),
            ));

        widget.on_account_login_completed(AccountLoginCompletedNotification {
            login_id: Some("login-1".to_string()),
            success: true,
            error: None,
        });

        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::ChatGptSuccessMessage
        ));
    }

    #[test]
    fn mark_url_hyperlink_wraps_cyan_underlined_cells() {
        let url = "https://example.com";
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);

        // Manually write some cyan+underlined characters to simulate a rendered URL.
        for (i, ch) in "example".chars().enumerate() {
            let cell = &mut buf[(i as u16, 0)];
            cell.set_symbol(&ch.to_string());
            cell.fg = app_accent_color();
            cell.modifier = Modifier::UNDERLINED;
        }
        // Leave a plain cell that should NOT be marked.
        buf[(7, 0)].set_symbol("X");

        mark_url_hyperlink(&mut buf, area, url);

        // Each cyan+underlined cell should now carry the OSC 8 wrapper.
        let found = collect_osc8_chars(&buf, area, url);
        assert_eq!(found, "example");

        // The plain "X" cell should be untouched.
        assert_eq!(buf[(7, 0)].symbol(), "X");
    }

    #[test]
    fn mark_url_hyperlink_sanitizes_control_chars() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);

        // One cyan+underlined cell to mark.
        let cell = &mut buf[(0, 0)];
        cell.set_symbol("a");
        cell.fg = app_accent_color();
        cell.modifier = Modifier::UNDERLINED;

        // URL contains ESC and BEL that could break the OSC 8 sequence.
        let malicious_url = "https://evil.com/\x1B]8;;\x07injected";
        mark_url_hyperlink(&mut buf, area, malicious_url);

        let sym = buf[(0, 0)].symbol().to_string();
        // The sanitized URL retains `]` (printable) but strips ESC and BEL.
        let sanitized = "https://evil.com/]8;;injected";
        assert!(
            sym.contains(sanitized),
            "symbol should contain sanitized URL, got: {sym:?}"
        );
        // The injected close-sequence must not survive: \x1B and \x07 are gone.
        assert!(
            !sym.contains("\x1B]8;;\x07injected"),
            "symbol must not contain raw control chars from URL"
        );
    }

    #[test]
    fn provider_picker_renders_open_interpreter_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 24)).expect("terminal");
        terminal
            .draw(|f| widget.render_provider_picker(f.area(), f.buffer_mut()))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[tokio::test]
    async fn provider_picker_filters_by_typed_provider_name() {
        let (mut widget, _tmp) = widget_open_interpreter().await;

        for c in "anthropic".chars() {
            widget.handle_key_event(KeyEvent::from(KeyCode::Char(c)));
        }

        assert_eq!(widget.highlighted_provider_id, "anthropic");
        assert_eq!(widget.provider_filter_query, "anthropic");

        widget.handle_key_event(KeyEvent::from(KeyCode::Enter));

        let SignInState::ProviderSetup(state) = &*widget.sign_in_state.read().unwrap() else {
            panic!("expected anthropic provider setup state");
        };
        assert_eq!(state.preset.provider_id, "anthropic");
    }

    #[tokio::test]
    async fn kimi_provider_uses_browser_sign_in_only() {
        let (mut widget, _tmp) = widget_open_interpreter().await;
        widget.highlighted_provider_id = KIMI_FOR_CODING_PROVIDER_ID.to_string();
        widget.highlighted_mode = SignInOption::Browser;

        assert_eq!(
            widget.displayed_sign_in_options(),
            vec![SignInOption::Browser]
        );
        assert_eq!(
            widget.selectable_sign_in_options(),
            vec![SignInOption::Browser]
        );
    }

    #[test]
    fn kimi_pick_mode_renders_browser_sign_in_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (mut widget, _tmp) = runtime.block_on(widget_open_interpreter());
        widget.highlighted_provider_id = KIMI_FOR_CODING_PROVIDER_ID.to_string();
        widget.highlighted_mode = SignInOption::Browser;
        *widget.sign_in_state.write().unwrap() = SignInState::PickMode;

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 20)).expect("terminal");
        terminal
            .draw(|f| widget.render_pick_mode(f.area(), f.buffer_mut()))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn provider_setup_renders_openrouter_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let state = ProviderSetupState::new(
            provider_presets()
                .into_iter()
                .find(|preset| preset.provider_id == "openrouter")
                .expect("openrouter preset"),
        )
        .expect("openrouter setup");

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 20)).expect("terminal");
        terminal
            .draw(|f| widget.render_provider_setup(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn provider_setup_renders_openai_prefilled_api_key_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let mut state = ProviderSetupState::new(
            provider_presets()
                .into_iter()
                .find(|preset| preset.provider_id == "openai_api_key")
                .expect("openai api key preset"),
        )
        .expect("openai api key setup");
        state.api_key =
            "sk-test-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".to_string();
        state.api_key_prefilled_from_env = true;
        state.api_key_env_var_name = Some("OPENAI_API_KEY".to_string());

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 20)).expect("terminal");
        terminal
            .draw(|f| widget.render_provider_setup(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn provider_model_selection_renders_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let state = ProviderModelSelectionState::new(
            "openrouter".to_string(),
            "OpenRouter".to_string(),
            "model-name".to_string(),
            vec![sample_model_preset("google/gemini-2.5-flash")],
        )
        .expect("provider model selection");

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 20)).expect("terminal");
        terminal
            .draw(|f| widget.render_provider_model_selection(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn provider_model_selection_renders_filtered_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let mut state = ProviderModelSelectionState::new(
            "groq".to_string(),
            "Groq".to_string(),
            "model-name".to_string(),
            vec![
                sample_model_preset("llama-3.3-70b-versatile"),
                sample_model_preset("moonshotai/kimi-k2-instruct"),
                sample_model_preset("qwen/qwen3-32b"),
            ],
        )
        .expect("provider model selection");
        state.replace_filter_query("kimi".to_string());

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 16)).expect("terminal");
        terminal
            .draw(|f| widget.render_provider_model_selection(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn provider_model_selection_renders_openai_common_models_first_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let state = ProviderModelSelectionState::new(
            "openai_api_key".to_string(),
            "OpenAI (API key)".to_string(),
            "gpt-5.4-mini".to_string(),
            vec![
                sample_model_preset("gpt-4.1"),
                sample_model_preset("gpt-5.3-codex"),
                sample_model_preset("gpt-5.4-nano"),
                sample_model_preset("gpt-5.4-mini"),
                sample_model_preset("gpt-5.4"),
            ],
        )
        .expect("provider model selection");

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 16)).expect("terminal");
        terminal
            .draw(|f| widget.render_provider_model_selection(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn provider_model_selection_filter_can_fall_back_to_manual_entry() {
        let mut state = ProviderModelSelectionState::new(
            "groq".to_string(),
            "Groq".to_string(),
            "model-name".to_string(),
            vec![
                sample_model_preset("llama-3.3-70b-versatile"),
                sample_model_preset("moonshotai/kimi-k2-instruct"),
            ],
        )
        .expect("provider model selection");

        state.replace_filter_query("my-custom-model".to_string());

        assert_eq!(state.selected_model(), None);
        assert_eq!(state.filter_query(), "my-custom-model");
        assert_eq!(state.manual_model_placeholder, "model-name");
    }

    #[test]
    fn provider_model_selection_filter_retains_j_and_k_characters() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (mut widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let state = ProviderModelSelectionState::new(
            "custom".to_string(),
            "Local Bench".to_string(),
            "model-name".to_string(),
            vec![sample_model_preset("mock-compatible-model")],
        )
        .expect("provider model selection");
        *widget.sign_in_state.write().unwrap() = SignInState::ProviderModelSelection(state);

        widget.handle_key_event(KeyEvent::from(KeyCode::Char('k')));
        widget.handle_key_event(KeyEvent::from(KeyCode::Char('j')));

        let SignInState::ProviderModelSelection(state) = &*widget.sign_in_state.read().unwrap()
        else {
            panic!("expected provider model selection state");
        };
        assert_eq!(state.filter_query(), "kj");
    }

    #[tokio::test]
    async fn provider_model_selection_enter_shows_saving_state_before_completion() {
        let (mut widget, _tmp) = widget_open_interpreter().await;
        let mut model = sample_model_preset("mock-compatible-model");
        model.supported_reasoning_efforts.truncate(1);
        *widget.sign_in_state.write().unwrap() = SignInState::ProviderModelSelection(
            ProviderModelSelectionState::new(
                "compatible_local_bench".to_string(),
                "Local Bench".to_string(),
                "model-name".to_string(),
                vec![model],
            )
            .expect("provider model selection"),
        );

        widget.handle_key_event(KeyEvent::from(KeyCode::Enter));

        let SignInState::ConnectingAppServer(state) = &*widget.sign_in_state.read().unwrap() else {
            panic!("expected saving state after confirming provider model selection");
        };
        assert_eq!(state.message, "Saving Local Bench...");
    }

    #[test]
    fn provider_model_selection_falls_back_to_provider_advertised_models() {
        let mut preset = sample_model_preset("gpt-oss:20b");
        preset.show_in_picker = false;

        let state = ProviderModelSelectionState::new(
            "ollama".to_string(),
            "Ollama".to_string(),
            "gpt-oss:20b".to_string(),
            vec![preset.clone()],
        )
        .expect("provider model selection");

        assert_eq!(state.using_unverified_models(), true);
        assert_eq!(state.models(), &[preset]);
        assert_eq!(
            state.selected_model().expect("selected model").model,
            "gpt-oss:20b"
        );
    }

    #[test]
    fn local_provider_unavailable_renders_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let state = LocalProviderUnavailableState {
            provider_id: "ollama".to_string(),
            provider_name: "Ollama".to_string(),
            manual_model_placeholder: "gpt-oss:20b".to_string(),
            default_manual_model: "gpt-oss:20b".to_string(),
            message: "Ollama is not running on localhost:11434.".to_string(),
            can_start_provider: true,
        };

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 14)).expect("terminal");
        terminal
            .draw(|f| widget.render_local_provider_unavailable(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn manual_model_entry_renders_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let state = ManualModelEntryState::new(
            "custom".to_string(),
            "Acme Gateway".to_string(),
            "your-model-name".to_string(),
            String::new(),
        );

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 16)).expect("terminal");
        terminal
            .draw(|f| widget.render_manual_model_entry(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn provider_reasoning_selection_renders_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (widget, _tmp) = runtime.block_on(widget_open_interpreter());
        let state = ProviderReasoningSelectionState::new(
            "openrouter".to_string(),
            "OpenRouter".to_string(),
            sample_model_preset("google/gemini-2.5-flash"),
        )
        .expect("provider reasoning selection");

        let mut terminal =
            Terminal::new(VT100Backend::new(/*width*/ 96, /*height*/ 20)).expect("terminal");
        terminal
            .draw(|f| widget.render_provider_reasoning_selection(f.area(), f.buffer_mut(), &state))
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }
}
