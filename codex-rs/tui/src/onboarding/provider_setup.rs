//! Provider preset data and the TUI provider-setup form.
//!
//! The preset catalog and quick-add logic now live in
//! `codex_model_provider_info::provider_selection` so the TUI and the
//! app-server share one source of truth. This module re-exports those symbols
//! (preserving the existing `crate::onboarding::provider_setup::*` import
//! paths) and keeps the TUI-only form: [`ProviderSetupState`],
//! [`ProviderSetupField`], and the form-navigation extension trait.

use codex_app_server_protocol::ConfigEdit as AppServerConfigEdit;

pub(crate) use codex_model_provider_info::provider_selection::KIMI_FOR_CODING_PROVIDER_ID;
pub(crate) use codex_model_provider_info::provider_selection::ProviderIdentity;
pub(crate) use codex_model_provider_info::provider_selection::ProviderPreset;
pub(crate) use codex_model_provider_info::provider_selection::ProviderPresetQuickAddAction;
pub(crate) use codex_model_provider_info::provider_selection::browser_auth_provider_definition_edits;
pub(crate) use codex_model_provider_info::provider_selection::default_provider_preset_id;
pub(crate) use codex_model_provider_info::provider_selection::provider_preset_by_id;
pub(crate) use codex_model_provider_info::provider_selection::provider_presets;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderSetupField {
    ProviderName,
    BaseUrl,
    ApiKey,
}

/// Form-navigation helpers for the provider-setup wizard.
///
/// `ProviderPreset` is a foreign type (it now lives in
/// `codex-model-provider-info`), so these field-order helpers, which return the
/// TUI-only [`ProviderSetupField`], are defined as a fork-local extension trait
/// rather than inherent methods.
pub(crate) trait ProviderPresetFormNav {
    fn first_field(&self) -> ProviderSetupField;
    fn next_field(&self, field: ProviderSetupField) -> Option<ProviderSetupField>;
    fn previous_field(&self, field: ProviderSetupField) -> Option<ProviderSetupField>;
}

impl ProviderPresetFormNav for ProviderPreset {
    fn first_field(&self) -> ProviderSetupField {
        if self.base_url_editable {
            return ProviderSetupField::ProviderName;
        }
        ProviderSetupField::ApiKey
    }

    fn next_field(&self, field: ProviderSetupField) -> Option<ProviderSetupField> {
        match field {
            ProviderSetupField::ProviderName => Some(ProviderSetupField::BaseUrl),
            ProviderSetupField::BaseUrl => Some(ProviderSetupField::ApiKey),
            ProviderSetupField::ApiKey => None,
        }
    }

    fn previous_field(&self, field: ProviderSetupField) -> Option<ProviderSetupField> {
        match field {
            ProviderSetupField::ProviderName => None,
            ProviderSetupField::BaseUrl => {
                if self.base_url_editable {
                    Some(ProviderSetupField::ProviderName)
                } else {
                    None
                }
            }
            ProviderSetupField::ApiKey => {
                if self.base_url_editable {
                    Some(ProviderSetupField::BaseUrl)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderSetupState {
    pub(crate) preset: ProviderPreset,
    pub(crate) field: ProviderSetupField,
    pub(crate) provider_name: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) api_key_prefilled_from_env: bool,
    pub(crate) api_key_env_var_name: Option<String>,
}

impl ProviderSetupState {
    pub(crate) fn new(preset: ProviderPreset) -> Option<Self> {
        if preset.uses_openai_auth()
            || preset.uses_browser_auth()
            || (!preset.base_url_editable && !preset.api_key_required)
        {
            return None;
        }
        let api_key_env_var_name = preset.api_key_env_var_name(/*provider_name*/ None);
        let api_key = api_key_env_var_name
            .as_deref()
            .and_then(crate::login_support::read_env_var_trimmed)
            .unwrap_or_default();
        Some(Self {
            field: preset.first_field(),
            provider_name: String::new(),
            base_url: preset.base_url.clone(),
            api_key_prefilled_from_env: !api_key.is_empty(),
            api_key,
            api_key_env_var_name,
            preset,
        })
    }

    pub(crate) fn previous_field(&self) -> Option<ProviderSetupField> {
        self.preset.previous_field(self.field)
    }

    pub(crate) fn advance_field(&mut self) -> bool {
        if let Some(next_field) = self.preset.next_field(self.field) {
            self.field = next_field;
            if matches!(self.field, ProviderSetupField::ApiKey) {
                self.refresh_api_key_prefill();
            }
            false
        } else {
            true
        }
    }

    fn refresh_api_key_prefill(&mut self) {
        if self.api_key_prefilled_from_env {
            self.api_key.clear();
        }

        self.api_key_env_var_name = self
            .preset
            .api_key_env_var_name(Some(self.provider_name.as_str()));

        self.api_key_prefilled_from_env = false;
        if self.api_key.is_empty()
            && let Some(env_var_name) = self.api_key_env_var_name.as_deref()
            && let Some(api_key) = crate::login_support::read_env_var_trimmed(env_var_name)
        {
            self.api_key = api_key;
            self.api_key_prefilled_from_env = true;
        }
    }

    pub(crate) fn active_field_label(&self) -> &'static str {
        match self.field {
            ProviderSetupField::ProviderName => "Provider name",
            ProviderSetupField::BaseUrl => "Base URL",
            ProviderSetupField::ApiKey => "API key",
        }
    }

    pub(crate) fn active_field_placeholder(&self) -> &str {
        match self.field {
            ProviderSetupField::ProviderName => "Acme Gateway",
            ProviderSetupField::BaseUrl => "https://api.example.com/v1",
            ProviderSetupField::ApiKey => "Paste or type your API key",
        }
    }

    pub(crate) fn active_field_value(&self) -> &str {
        match self.field {
            ProviderSetupField::ProviderName => &self.provider_name,
            ProviderSetupField::BaseUrl => &self.base_url,
            ProviderSetupField::ApiKey => &self.api_key,
        }
    }

    pub(crate) fn push_char(&mut self, c: char) {
        self.active_field_value_mut().push(c);
    }

    pub(crate) fn replace_active_value(&mut self, value: String) {
        *self.active_field_value_mut() = value;
    }

    pub(crate) fn pop_char(&mut self) {
        match self.field {
            ProviderSetupField::ApiKey if self.api_key_prefilled_from_env => {
                self.api_key.clear();
                self.api_key_prefilled_from_env = false;
            }
            _ => {
                self.active_field_value_mut().pop();
            }
        }
    }

    pub(crate) fn active_field_value_mut(&mut self) -> &mut String {
        match self.field {
            ProviderSetupField::ProviderName => &mut self.provider_name,
            ProviderSetupField::BaseUrl => &mut self.base_url,
            ProviderSetupField::ApiKey => &mut self.api_key,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.preset.base_url_editable && self.provider_name.trim().is_empty() {
            return Err("Provider name cannot be empty".to_string());
        }
        if self.preset.base_url_editable && self.base_url.trim().is_empty() {
            return Err("Base URL cannot be empty".to_string());
        }
        if self.preset.api_key_required && self.api_key.trim().is_empty() {
            return Err("API key cannot be empty".to_string());
        }
        Ok(())
    }

    pub(crate) fn validate_active_field(&self) -> Result<(), String> {
        match self.field {
            ProviderSetupField::ProviderName if self.provider_name.trim().is_empty() => {
                Err("Provider name cannot be empty".to_string())
            }
            ProviderSetupField::BaseUrl if self.base_url.trim().is_empty() => {
                Err("Base URL cannot be empty".to_string())
            }
            ProviderSetupField::ApiKey
                if self.preset.api_key_required && self.api_key.trim().is_empty() =>
            {
                Err("API key cannot be empty".to_string())
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn provider_identity(&self) -> ProviderIdentity {
        ProviderIdentity {
            id: self
                .preset
                .configured_provider_id(Some(self.provider_name.as_str())),
            name: self
                .preset
                .configured_provider_name(Some(self.provider_name.as_str())),
        }
    }

    pub(crate) fn provider_definition_edits(&self) -> Vec<AppServerConfigEdit> {
        let identity = self.provider_identity();
        self.preset.provider_definition_edits(
            identity.id.as_str(),
            identity.name.as_str(),
            self.base_url.as_str(),
            self.api_key.as_str(),
            self.api_key_prefilled_from_env,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn openai_api_key_preset_requires_api_key_setup() {
        let preset = provider_preset_by_id("openai_api_key").expect("openai api key preset");
        let state = ProviderSetupState::new(preset).expect("openai api key setup");

        assert_eq!(state.field, ProviderSetupField::ApiKey);
        assert_eq!(state.preset.provider_id, "openai_api_key".to_string());
    }

    #[test]
    fn custom_provider_setup_builds_chat_wire_edits() {
        let preset = provider_preset_by_id("openinterpreter_add_compatible_provider")
            .expect("custom compatible preset");
        let mut state = ProviderSetupState::new(preset).expect("custom setup");
        state.provider_name = "Acme Gateway".to_string();
        state.base_url = "https://example.com/v1".to_string();
        state.api_key = "sk-custom".to_string();
        state.api_key_prefilled_from_env = false;

        let edits = state.provider_definition_edits();

        assert!(
            edits
                .iter()
                .any(|edit| edit.key_path == "model_providers.compatible_acme_gateway.name")
        );
        assert!(edits.iter().any(|edit| edit.key_path
            == "model_providers.compatible_acme_gateway.experimental_bearer_token"));
    }
}
