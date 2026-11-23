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

use super::keys;
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
use url::Url;

/// The provider id written to `model_provider` / `model_providers.<id>` when
/// no entry exists yet.
pub(super) const LOCAL_PROVIDER_ID: &str = "local";

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
#[derive(Clone)]
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
    /// Human-readable summary of the existing entry (e.g. its base URL).
    pub existing_summary: String,
    /// First unused `local-N` provider id for the "create new" choice.
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
        existing_summary: String,
        new_provider_id: String,
    ) -> Self {
        Self {
            config,
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
        if !existing_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(&candidate))
        {
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

#[cfg(test)]
#[path = "local_provider_tests.rs"]
mod tests;
