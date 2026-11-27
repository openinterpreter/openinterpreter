//! Local / custom OpenAI-compatible provider setup within the authentication step.
//!
//! The first-run onboarding only offered hosted account/API-key paths; users
//! starting with Ollama, LM Studio, KCPP or another OpenAI-compatible endpoint
//! had to leave onboarding and learn the provider config format by hand. This
//! state collects a base URL, an optional credential env-key name, and a model,
//! validates the URL, and hands a `Configure` action back to the auth step
//! which persists the provider through `config/batchWrite` — the same source
//! of truth as every other provider, not a second config path.
//!
//! When `model_providers.local` already exists the flow never overwrites it:
//! the auth step surfaces a collision picker so the user can adopt the existing
//! entry or save the form as a new `local-N` provider.
//!
//! Both risky steps are guarded: a config read or write failure keeps the
//! completed form on screen, and the `local-N` id chosen by the picker is
//! revalidated (under the config version the write is pinned to) immediately
//! before `config/batchWrite`, so an entry created by another writer in the
//! meantime is never replaced.

#![allow(clippy::unwrap_used)]

use super::auth::SignInState;
use super::keys;
use crate::key_hint::KeyBindingListExt;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::ConfigEdit;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::sync::Arc;
use std::sync::RwLock;
use url::Url;

/// The provider id written to `model_provider` / `model_providers.<id>` when
/// no entry exists yet.
pub(super) const LOCAL_PROVIDER_ID: &str = "local";

/// Version an empty `config.toml` hashes to. When a config read found no user
/// layer at all — meaning `config.toml` did not exist at read time — there is
/// no real version to pin the write to, so this sentinel stands in for it:
/// `config/batchWrite` creates an empty layer when the file is still absent
/// (whose version matches the sentinel and lets the write through), but a file
/// created by a concurrent writer in the meantime carries real content whose
/// version differs, so the write is rejected instead of silently replacing
/// that writer's entries.
fn empty_user_layer_version() -> String {
    codex_config::version_for_toml(&codex_config::TomlValue::Table(Default::default()))
}

/// How a collision with an existing `model_providers.local` entry is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LocalProviderResolution {
    /// Keep the existing entry untouched; point `model_provider` at it and
    /// apply only the fields the user explicitly typed (model, env-key name).
    UseExisting,
    /// Write the form under a fresh `local-N` provider id; the existing entry
    /// is left completely untouched.
    CreateNew,
}

#[derive(Clone)]
pub(super) struct LocalProviderState {
    /// Text fields: base URL, credential env-key name (optional), model id.
    values: [String; 3],
    selected_field: usize,
}

/// Outcome returned to the auth step once the form is complete.
#[derive(Clone, Debug)]
pub(super) struct LocalProviderConfig {
    pub base_url: String,
    pub env_key: Option<String>,
    pub model: String,
}

pub(super) enum LocalProviderAction {
    BackToAuth,
    Configure(Box<LocalProviderConfig>),
    /// Field validation failed; the message is surfaced as the widget error.
    Invalid(String),
}

/// A valid environment-variable identifier (`[A-Za-z_][A-Za-z0-9_]*`), which is
/// the only shape shells can export for provider `env_key` values.
fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl LocalProviderState {
    pub(super) fn new() -> Self {
        Self {
            values: Default::default(),
            selected_field: 0,
        }
    }

    /// Rebuild a form from a validated config.
    ///
    /// Used wherever the completed form has to survive a state that is not the
    /// form itself — backing out of the collision picker, or a failed config
    /// read/write — so the user never has to retype what they already entered.
    pub(super) fn from_config(config: &LocalProviderConfig) -> Self {
        Self {
            values: [
                config.base_url.clone(),
                config.env_key.clone().unwrap_or_default(),
                config.model.clone(),
            ],
            // Focus the last field, which is where Enter submits.
            selected_field: 2,
        }
    }

    pub(super) fn is_text_entry_active(&self) -> bool {
        true
    }

    fn active_input_mut(&mut self) -> &mut String {
        &mut self.values[self.selected_field]
    }

    /// Validate the three fields and build the config to persist.
    /// Returns the config or the error message to surface in the footer.
    fn validate(&self) -> Result<LocalProviderConfig, String> {
        let base_url = self.values[0].trim();
        if base_url.is_empty() {
            return Err("Base URL is required (e.g. http://localhost:11434/v1)".to_string());
        }
        match Url::parse(base_url) {
            Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => {
                // Credentials belong in the env-key field, not embedded in the
                // URL where they would be persisted to config.toml in the clear
                // and can leak into logs.
                if !url.username().is_empty() || url.password().is_some() {
                    return Err(
                        "Base URL must not embed credentials; use the env var field instead"
                            .to_string(),
                    );
                }
            }
            Ok(_) => {
                return Err(format!("Base URL '{base_url}' is not an http(s) endpoint"));
            }
            Err(err) => {
                return Err(format!("Malformed base URL '{base_url}': {err}"));
            }
        }
        let model = self.values[2].trim();
        if model.is_empty() {
            return Err("Model id is required (e.g. llama3.2)".to_string());
        }
        let env_key = self.values[1].trim();
        if !env_key.is_empty() && !is_valid_env_var_name(env_key) {
            return Err(format!(
                "'{env_key}' is not a valid environment variable name"
            ));
        }
        Ok(LocalProviderConfig {
            base_url: base_url.to_string(),
            env_key: (!env_key.is_empty()).then(|| env_key.to_string()),
            model: model.to_string(),
        })
    }

    /// Returns the action the auth step should take, if any.
    pub(super) fn handle_key_event(&mut self, key_event: &KeyEvent) -> Option<LocalProviderAction> {
        if keys::CANCEL.is_pressed(*key_event) {
            return Some(LocalProviderAction::BackToAuth);
        }
        if key_event.code == KeyCode::Up || key_event.code == KeyCode::BackTab {
            self.selected_field = (self.selected_field + self.values.len() - 1) % self.values.len();
            return None;
        }
        if key_event.code == KeyCode::Down || key_event.code == KeyCode::Tab {
            self.selected_field = (self.selected_field + 1) % self.values.len();
            return None;
        }
        if keys::CONFIRM.is_pressed(*key_event) {
            // Advance to the next empty required field first, like the Bedrock
            // access-key form, so Enter works as a natural "next" as well.
            if self.selected_field < self.values.len() - 1
                && !self.values[self.selected_field].trim().is_empty()
            {
                self.selected_field += 1;
                return None;
            }
            match self.validate() {
                Ok(config) => return Some(LocalProviderAction::Configure(Box::new(config))),
                Err(message) => return Some(LocalProviderAction::Invalid(message)),
            }
        }
        match key_event.code {
            KeyCode::Backspace => {
                self.active_input_mut().pop();
            }
            KeyCode::Char(character)
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                    && !key_event.modifiers.intersects(
                        KeyModifiers::SUPER | KeyModifiers::CONTROL | KeyModifiers::ALT,
                    ) =>
            {
                self.active_input_mut().push(character);
            }
            _ => {}
        }
        None
    }

    pub(super) fn handle_paste(&mut self, pasted: &str) {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return;
        }
        self.active_input_mut().push_str(trimmed);
    }

    /// Render the form. `error` carries validation/config-write failures from
    /// the auth widget so they render in the same place as other auth errors.
    pub(super) fn render(&self, area: Rect, buf: &mut Buffer, error: Option<String>) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Use a local / custom provider".bold()]),
            "".into(),
            "  Point Open Interpreter at an OpenAI-compatible endpoint — Ollama,".into(),
            "  LM Studio, KCPP or any server speaking the chat completions API.".into(),
            "".into(),
        ];

        let fields: [(&str, &str); 3] = [
            ("Base URL", "http://localhost:11434/v1"),
            (
                "Env var for API key (optional)",
                "OLLAMA_API_KEY — leave empty if none needed",
            ),
            ("Model", "llama3.2"),
        ];
        for (index, (label, placeholder)) in fields.iter().enumerate() {
            let selected = index == self.selected_field;
            let caret = if selected { ">" } else { " " };
            let value = &self.values[index];
            let rendered = if value.is_empty() {
                placeholder.to_string().dim()
            } else {
                value.clone().into()
            };
            lines.push(Line::from(vec![
                format!("{caret} {label}: ").into(),
                rendered,
            ]));
        }
        lines.push("".into());
        lines.push(
            "  Up/Down or Tab/Shift-Tab move between fields; Enter advances then saves."
                .dim()
                .into(),
        );
        lines.push(Line::from(vec![
            "  Press ".dim(),
            keys::CANCEL[0].into(),
            " to go back".dim(),
        ]));
        if let Some(error) = error {
            lines.push("".into());
            lines.push(error.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

/// Collision-picker state shown when `model_providers.local` already exists.
/// The existing entry is never overwritten from this screen: the user either
/// adopts it or saves the form under a fresh `local-N` id.
#[derive(Clone)]
pub(super) struct LocalProviderCollisionState {
    /// The validated form values awaiting persistence.
    pub config: LocalProviderConfig,
    /// The completed form itself, restored verbatim when the user backs out of
    /// the picker or the write fails, so no typed value is lost.
    pub form: LocalProviderState,
    /// The actual key of the existing `local` entry — usually `local`, but a
    /// config authored by hand can hold `LOCAL`, `Local`, and so on. Adoption
    /// must point `model_provider` at this exact key, not the lowercase
    /// canonical form, or the selection would reference a provider id that does
    /// not exist.
    pub existing_provider_id: String,
    /// Human-readable summary of the existing entry (e.g. its base URL).
    pub existing_summary: String,
    /// First unused `local-N` provider id for the "create new" choice. It is
    /// only a proposal: the id is revalidated against the effective config right
    /// before the write.
    pub new_provider_id: String,
    selected: usize,
}

/// What the collision picker can decide.
pub(super) enum LocalProviderCollisionAction {
    BackToForm,
    Resolve(LocalProviderResolution),
}

impl LocalProviderCollisionState {
    pub(super) fn new(
        config: LocalProviderConfig,
        existing_provider_id: String,
        existing_summary: String,
        new_provider_id: String,
    ) -> Self {
        Self {
            form: LocalProviderState::from_config(&config),
            config,
            existing_provider_id,
            existing_summary,
            new_provider_id,
            // Default to the non-destructive choice.
            selected: 1,
        }
    }

    pub(super) fn handle_key_event(
        &mut self,
        key_event: &KeyEvent,
    ) -> Option<LocalProviderCollisionAction> {
        if keys::CANCEL.is_pressed(*key_event) {
            return Some(LocalProviderCollisionAction::BackToForm);
        }
        if keys::MOVE_UP.is_pressed(*key_event) {
            self.selected = self.selected.saturating_sub(1);
            return None;
        }
        if keys::MOVE_DOWN.is_pressed(*key_event) {
            self.selected = (self.selected + 1).min(1);
            return None;
        }
        if keys::SELECT_FIRST.is_pressed(*key_event) {
            return Some(LocalProviderCollisionAction::Resolve(
                LocalProviderResolution::UseExisting,
            ));
        }
        if keys::SELECT_SECOND.is_pressed(*key_event) {
            return Some(LocalProviderCollisionAction::Resolve(
                LocalProviderResolution::CreateNew,
            ));
        }
        if keys::CONFIRM.is_pressed(*key_event) {
            let resolution = if self.selected == 0 {
                LocalProviderResolution::UseExisting
            } else {
                LocalProviderResolution::CreateNew
            };
            return Some(LocalProviderCollisionAction::Resolve(resolution));
        }
        None
    }

    pub(super) fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Local provider already exists".bold()]),
            "".into(),
            format!("  {existing}", existing = self.existing_summary).into(),
            "  It will not be overwritten.".into(),
            "".into(),
        ];
        let options = [
            format!("Use the existing provider with model {}", self.config.model),
            format!("Save as a new provider ({id})", id = self.new_provider_id),
        ];
        for (index, option) in options.iter().enumerate() {
            let caret = if index == self.selected { ">" } else { " " };
            lines.push(Line::from(vec![
                format!("{caret} {}. ", index + 1).into(),
                option.clone().into(),
            ]));
        }
        lines.push("".into());
        lines.push(Line::from(vec![
            "  Press ".dim(),
            keys::CANCEL[0].into(),
            " to go back".dim(),
        ]));
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

/// Whether `provider_id` is still unused in `existing_ids`. Provider ids are
/// compared case-insensitively because the config loader resolves them that way.
pub(super) fn is_provider_id_free(provider_id: &str, existing_ids: &[String]) -> bool {
    !existing_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(provider_id))
}

/// Pick the first free `local`, `local-2`, `local-3`, ... provider id given the
/// ids already present in the effective config.
pub(super) fn next_local_provider_id(existing_ids: &[String]) -> String {
    let mut suffix = 1u32;
    loop {
        let candidate = if suffix == 1 {
            LOCAL_PROVIDER_ID.to_string()
        } else {
            format!("{LOCAL_PROVIDER_ID}-{suffix}")
        };
        if is_provider_id_free(&candidate, existing_ids) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Convert a validated form into config edits persisted through the same
/// `config/batchWrite` path every other provider write uses.
///
/// `provider_id` is the entry being written — `local` for a fresh install or
/// `local-N` after a collision. An existing `env_key` on the target entry is
/// only replaced when the user typed a new name; a left-empty field never
/// clears a credential that is already configured.
pub(super) fn local_provider_edits(
    provider_id: &str,
    config: &LocalProviderConfig,
) -> Vec<codex_app_server_protocol::ConfigEdit> {
    use crate::config_update::replace_config_value;

    let mut edits = vec![
        replace_config_value(
            format!("model_providers.{provider_id}.name"),
            serde_json::json!("Local / custom endpoint"),
        ),
        replace_config_value(
            format!("model_providers.{provider_id}.base_url"),
            serde_json::json!(config.base_url),
        ),
        // OpenAI-compatible servers speak the chat completions wire API and do
        // not need Codex auth.
        replace_config_value(
            format!("model_providers.{provider_id}.wire_api"),
            serde_json::json!("chat"),
        ),
        replace_config_value(
            format!("model_providers.{provider_id}.requires_openai_auth"),
            serde_json::json!(false),
        ),
        replace_config_value("model_provider", serde_json::json!(provider_id)),
        replace_config_value("model", serde_json::json!(config.model)),
    ];
    // Only set env_key when the user supplied one. A left-empty field is not a
    // request to clear — dropping a credential the user already configured
    // would silently break their endpoint on the next run.
    if let Some(env_key) = &config.env_key {
        edits.push(replace_config_value(
            format!("model_providers.{provider_id}.env_key"),
            serde_json::json!(env_key),
        ));
    }
    edits
}

/// Edits for adopting an existing `local` provider entry: keep the entry
/// itself untouched, select it as the active provider/model, and only update
/// `env_key` when the user explicitly entered a name.
pub(super) fn adopt_existing_provider_edits(
    provider_id: &str,
    config: &LocalProviderConfig,
) -> Vec<codex_app_server_protocol::ConfigEdit> {
    use crate::config_update::replace_config_value;

    let mut edits = vec![
        replace_config_value("model_provider", serde_json::json!(provider_id)),
        replace_config_value("model", serde_json::json!(config.model)),
    ];
    if let Some(env_key) = &config.env_key {
        edits.push(replace_config_value(
            format!("model_providers.{provider_id}.env_key"),
            serde_json::json!(env_key),
        ));
    }
    edits
}

/// The provider-relevant slice of the effective config, plus the version of the
/// user layer it was read from.
///
/// `expected_version` is passed back to `config/batchWrite`, so a write that
/// was computed from this snapshot is rejected by the server once another
/// writer has changed the config in the meantime, instead of silently
/// replacing that writer's entry. It is never `None` here: when the read found
/// no user layer, [`empty_user_layer_version`] pins the write to "the file was
/// absent", so a first write still races safely — a concurrent writer that
/// created `config.toml` in between is detected rather than overwritten.
pub(super) struct ProviderSnapshot {
    pub existing_ids: Vec<String>,
    /// The actual key of an existing `local` entry, matched
    /// case-insensitively so a hand-authored `model_providers.LOCAL` is found
    /// by the same rule [`is_provider_id_free`] uses to allocate ids.
    pub existing_local_id: Option<String>,
    /// Summary of an existing `model_providers.local` entry, if any.
    pub local_summary: Option<String>,
    /// The version `config/batchWrite` should pin the write to.
    pub expected_version: String,
}

impl ProviderSnapshot {
    pub(super) async fn load(
        request_handle: &AppServerRequestHandle,
    ) -> color_eyre::eyre::Result<Self> {
        let cwd = std::env::current_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let (response, user_config_version) =
            crate::config_update::read_effective_config_with_user_version(
                request_handle.clone(),
                cwd,
            )
            .await?;
        let providers = response
            .config
            .additional
            .get("model_providers")
            .and_then(|value| value.as_object());
        let existing_ids: Vec<String> = providers
            .map(|providers| providers.keys().cloned().collect())
            .unwrap_or_default();
        // The collision rule and the id allocator must agree on what "exists"
        // means. `is_provider_id_free` compares case-insensitively because the
        // config loader can resolve provider ids that way, so a `LOCAL` or
        // `Local` entry counts as an existing local provider here too — not
        // only the exact lowercase key a `get("local")` lookup would find.
        let existing_local_id = existing_ids
            .iter()
            .find(|id| id.eq_ignore_ascii_case(LOCAL_PROVIDER_ID))
            .cloned();
        let local_summary = existing_local_id
            .as_ref()
            .and_then(|id| providers.and_then(|providers| providers.get(id)))
            .map(|existing| {
                let name = existing
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Local / custom endpoint");
                existing
                    .get("base_url")
                    .and_then(|value| value.as_str())
                    .map_or_else(
                        || format!("Existing '{name}' provider is already configured."),
                        |base_url| format!("Existing '{name}' provider points at {base_url}."),
                    )
            });
        Ok(Self {
            existing_ids,
            existing_local_id,
            local_summary,
            expected_version: user_config_version.unwrap_or_else(empty_user_layer_version),
        })
    }
}

/// The handles the local-provider persistence flow drives: the app-server
/// connection plus the auth step's shared sign-in state and error slot.
#[derive(Clone)]
pub(super) struct LocalProviderFlow {
    pub request_handle: AppServerRequestHandle,
    pub sign_in_state: Arc<RwLock<SignInState>>,
    pub error: Arc<RwLock<Option<String>>>,
}

impl LocalProviderFlow {
    /// Save a completed form: read the current provider configuration, then
    /// either surface the collision picker or write the provider outright.
    pub(super) async fn save(&self, config: LocalProviderConfig) {
        let form = LocalProviderState::from_config(&config);
        let snapshot = match ProviderSnapshot::load(&self.request_handle).await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                self.fail(
                    form,
                    format!("Failed to check existing provider configuration: {err}"),
                );
                return;
            }
        };
        let ProviderSnapshot {
            existing_ids,
            existing_local_id,
            local_summary,
            expected_version,
        } = snapshot;
        match local_summary {
            Some(summary) => {
                let picker = LocalProviderCollisionState::new(
                    config,
                    // The collision summary only exists when a local entry was
                    // found, so its id is present here.
                    existing_local_id.unwrap_or_else(|| LOCAL_PROVIDER_ID.to_string()),
                    summary,
                    next_local_provider_id(&existing_ids),
                );
                *self.error.write().unwrap() = None;
                *self.sign_in_state.write().unwrap() = SignInState::LocalProviderCollision(picker);
            }
            None => {
                let edits = local_provider_edits(LOCAL_PROVIDER_ID, &config);
                self.persist(form, expected_version, edits).await;
            }
        }
    }

    /// Resolve a collision: adopt the existing entry, or save the form under the
    /// `local-N` id the picker proposed.
    ///
    /// Both paths revalidate the entry they are about to touch against the
    /// effective config immediately before the write — the "use existing" id
    /// must still be present, and the proposed `local-N` id must still be free —
    /// and the write itself is pinned to the version that revalidation read, so
    /// an entry removed or an id claimed by another writer between the picker
    /// and the write is reported instead of silently selected or overwritten.
    pub(super) async fn resolve_collision(
        &self,
        collision: LocalProviderCollisionState,
        resolution: LocalProviderResolution,
    ) {
        let LocalProviderCollisionState {
            config,
            form,
            existing_provider_id,
            new_provider_id,
            ..
        } = collision;
        let prepared = match ProviderSnapshot::load(&self.request_handle).await {
            Ok(snapshot) => match resolution {
                LocalProviderResolution::UseExisting => {
                    // Re-check with the same case-insensitive rule that showed
                    // the picker: another writer may have removed or renamed the
                    // entry while the picker was open, and adopting a provider
                    // id that no longer resolves leaves `model_provider`
                    // dangling.
                    match snapshot.existing_local_id {
                        Some(existing_id) => Ok((
                            snapshot.expected_version,
                            adopt_existing_provider_edits(&existing_id, &config),
                        )),
                        None => Err(format!(
                            "The '{existing_provider_id}' provider was removed by another \
                             configuration change before it could be selected, so nothing was \
                             written. Your entries are kept — press Enter to save as a new \
                             provider instead."
                        )),
                    }
                }
                LocalProviderResolution::CreateNew
                    if !is_provider_id_free(&new_provider_id, &snapshot.existing_ids) =>
                {
                    Err(format!(
                        "'{new_provider_id}' was created by another configuration change before \
                         this form could be saved, so nothing was written. Your entries are kept — \
                         press Enter to save under the next free id."
                    ))
                }
                LocalProviderResolution::CreateNew => Ok((
                    snapshot.expected_version,
                    local_provider_edits(&new_provider_id, &config),
                )),
            },
            Err(err) => Err(format!("Failed to re-check provider configuration: {err}")),
        };
        match prepared {
            Ok((expected_version, edits)) => self.persist(form, expected_version, edits).await,
            Err(message) => self.fail(form, message),
        }
    }

    /// Run `config/batchWrite` for the form and record the outcome.
    ///
    /// A failed write keeps the user's entries: the form is put back on screen
    /// with the error, so the next Enter retries the save instead of asking for
    /// the whole form again.
    pub(super) async fn persist(
        &self,
        form: LocalProviderState,
        expected_version: String,
        edits: Vec<ConfigEdit>,
    ) {
        match crate::config_update::write_config_batch_checked(
            self.request_handle.clone(),
            edits,
            Some(expected_version),
        )
        .await
        {
            Ok(_) => {
                *self.error.write().unwrap() = None;
                *self.sign_in_state.write().unwrap() = SignInState::LocalProviderConfigured;
            }
            Err(err) if is_config_version_conflict(&err) => self.fail(
                form,
                "Your configuration changed while this form was open, so the local provider was \
                 not saved. Your entries are kept — press Enter to try again."
                    .to_string(),
            ),
            Err(err) => self.fail(
                form,
                format!("Failed to save local provider configuration: {err}"),
            ),
        }
    }

    /// Surface `message` and restore the completed form.
    fn fail(&self, form: LocalProviderState, message: String) {
        *self.error.write().unwrap() = Some(message);
        *self.sign_in_state.write().unwrap() = SignInState::LocalProvider(form);
    }
}

/// True when the server rejected a write because the user config no longer
/// matched the version the write was based on.
fn is_config_version_conflict(err: &color_eyre::eyre::Report) -> bool {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<TypedRequestError>())
        .is_some_and(|err| match err {
            TypedRequestError::Server { source, .. } => {
                source
                    .data
                    .as_ref()
                    .and_then(|data| data.get("config_write_error_code"))
                    .and_then(serde_json::Value::as_str)
                    == Some("configVersionConflict")
            }
            TypedRequestError::Transport { .. } | TypedRequestError::Deserialize { .. } => false,
        })
}

#[cfg(test)]
#[path = "local_provider_tests.rs"]
mod tests;
