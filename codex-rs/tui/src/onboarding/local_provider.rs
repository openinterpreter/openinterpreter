//! Local / custom OpenAI-compatible provider setup within the authentication step.
//!
//! The first-run onboarding only offered hosted account/API-key paths; users
//! starting with Ollama, LM Studio, KCPP or another OpenAI-compatible endpoint
//! had to leave onboarding and learn the provider config format by hand. This
//! state collects a base URL, an optional credential env-key name, and a model,
//! validates the URL, and hands a `Configure` action back to the auth step
//! which persists the provider through `config/batchWrite` — the same source
//! of truth as every other provider, not a second config path.

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

/// The provider id written to `model_provider` / `model_providers.<id>`.
pub(super) const LOCAL_PROVIDER_ID: &str = "local";

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
            Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => {}
            Ok(_) => {
                return Err(format!(
                    "Base URL '{base_url}' is not an http(s) endpoint"
                ));
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
                    && !key_event
                        .modifiers
                        .intersects(KeyModifiers::SUPER | KeyModifiers::CONTROL | KeyModifiers::ALT) =>
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
            Line::from(vec![
                "> ".into(),
                "Use a local / custom provider".bold(),
            ]),
            "".into(),
            "  Point Open Interpreter at an OpenAI-compatible endpoint — Ollama,".into(),
            "  LM Studio, KCPP or any server speaking the chat completions API.".into(),
            "".into(),
        ];

        let fields: [(&str, &str); 3] = [
            ("Base URL", "http://localhost:11434/v1"),
            ("Env var for API key (optional)", "OLLAMA_API_KEY — leave empty if none needed"),
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

/// Convert a validated form into config edits persisted through the same
/// `config/batchWrite` path every other provider write uses.
pub(super) fn local_provider_edits(
    config: &LocalProviderConfig,
) -> Vec<codex_app_server_protocol::ConfigEdit> {
    use crate::config_update::replace_config_value;

    let mut edits = vec![
        replace_config_value(
            format!("model_providers.{LOCAL_PROVIDER_ID}.name"),
            serde_json::json!("Local / custom endpoint"),
        ),
        replace_config_value(
            format!("model_providers.{LOCAL_PROVIDER_ID}.base_url"),
            serde_json::json!(config.base_url),
        ),
        // OpenAI-compatible servers speak the chat completions wire API and do
        // not need Codex auth.
        replace_config_value(
            format!("model_providers.{LOCAL_PROVIDER_ID}.wire_api"),
            serde_json::json!("chat"),
        ),
        replace_config_value(
            format!("model_providers.{LOCAL_PROVIDER_ID}.requires_openai_auth"),
            serde_json::json!(false),
        ),
        replace_config_value("model_provider", serde_json::json!(LOCAL_PROVIDER_ID)),
        replace_config_value("model", serde_json::json!(config.model)),
    ];
    match &config.env_key {
        Some(env_key) => edits.push(replace_config_value(
            format!("model_providers.{LOCAL_PROVIDER_ID}.env_key"),
            serde_json::json!(env_key),
        )),
        // Local endpoints like Ollama usually need no key at all; clear any
        // stale env_key so a re-run does not keep a ghost credential.
        None => edits.push(crate::config_update::clear_config_value(format!(
            "model_providers.{LOCAL_PROVIDER_ID}.env_key"
        ))),
    }
    edits
}

#[cfg(test)]
#[path = "local_provider_tests.rs"]
mod tests;
