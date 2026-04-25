use codex_app_server_protocol::ConfigEdit as AppServerConfigEdit;
use codex_model_provider_info::BundledProviderCatalogEntry;
use codex_model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use codex_model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::bundled_provider_catalog;
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;

use crate::config_write_edits::clear_path;
use crate::config_write_edits::set_path;
use crate::oss_provider_bootstrap::default_model_for_oss_provider;

const OPENAI_API_KEY_PROVIDER_ID: &str = "openai_api_key";
const ADD_COMPATIBLE_PROVIDER_ID: &str = "openinterpreter_add_compatible_provider";
const CUSTOM_PROVIDER_ID_PREFIX: &str = "compatible_";
const LMSTUDIO_BASE_URL: &str = "http://localhost:1234/v1";
const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";
pub(crate) const KIMI_FOR_CODING_PROVIDER_ID: &str = "kimi-for-coding";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderSetupField {
    ProviderName,
    BaseUrl,
    ApiKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderPresetKind {
    OpenAi,
    BrowserAuth,
    BuiltIn,
    Compatible,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ProviderPresetQuickAddAction {
    WriteEdits(Vec<AppServerConfigEdit>),
    PromptForApiKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderPreset {
    pub(crate) title: String,
    pub(crate) description: String,
    provider_kind: ProviderPresetKind,
    pub(crate) provider_id: String,
    pub(crate) base_url: String,
    pub(crate) base_url_editable: bool,
    pub(crate) api_key_required: bool,
    pub(crate) api_key_env_var: Option<String>,
    pub(crate) model_placeholder: String,
    pub(crate) default_model: Option<String>,
    wire_api: WireApi,
    pub(crate) sort_priority: u16,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderIdentity {
    pub(crate) id: String,
    pub(crate) name: String,
}

pub(crate) fn provider_presets() -> Vec<ProviderPreset> {
    let mut presets = vec![openai_chatgpt_preset(), openai_api_key_preset()];
    presets.extend(
        bundled_provider_catalog()
            .iter()
            .map(ProviderPreset::from_catalog_entry),
    );
    presets.extend([
        lmstudio_preset(),
        ollama_preset(),
        custom_compatible_preset(),
    ]);
    let mut seen_provider_ids = HashSet::new();
    presets.retain(|preset| seen_provider_ids.insert(preset.provider_id.clone()));
    presets.sort_by(|left, right| {
        left.sort_priority.cmp(&right.sort_priority).then_with(|| {
            left.title
                .to_ascii_lowercase()
                .cmp(&right.title.to_ascii_lowercase())
        })
    });
    presets
}

pub(crate) fn provider_preset_by_id(provider_id: &str) -> Option<ProviderPreset> {
    provider_presets()
        .into_iter()
        .find(|preset| preset.provider_id == provider_id)
}

pub(crate) fn default_provider_preset_id() -> String {
    provider_presets()
        .into_iter()
        .next()
        .map(|preset| preset.provider_id)
        .unwrap_or_else(|| "openai".to_string())
}

fn openai_chatgpt_preset() -> ProviderPreset {
    ProviderPreset {
        title: "OpenAI (ChatGPT sign-in)".to_string(),
        description: "Use your ChatGPT account to access OpenAI models.".to_string(),
        provider_kind: ProviderPresetKind::OpenAi,
        provider_id: "openai".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        base_url_editable: false,
        api_key_required: false,
        api_key_env_var: None,
        model_placeholder: "gpt-5.4-mini".to_string(),
        default_model: None,
        wire_api: WireApi::Responses,
        sort_priority: 0,
    }
}

pub(crate) fn browser_auth_provider_definition_edits(
    provider_id: &str,
    command_cwd: &Path,
) -> Vec<AppServerConfigEdit> {
    if provider_id != KIMI_FOR_CODING_PROVIDER_ID {
        return Vec::new();
    }
    let provider_segments = |tail: &str| {
        vec![
            "model_providers".to_string(),
            provider_id.to_string(),
            tail.to_string(),
        ]
    };
    vec![
        set_path(provider_segments("wire_api"), json!("chat")),
        set_path(provider_segments("requires_openai_auth"), json!(false)),
        clear_path(provider_segments("env_key")),
        clear_path(provider_segments("env_key_instructions")),
        clear_path(provider_segments("experimental_bearer_token")),
        set_path(
            provider_segments("auth"),
            json!({
                "command": "interpreter",
                "args": ["provider-auth", provider_id],
                "cwd": command_cwd.display().to_string(),
            }),
        ),
    ]
}

fn openai_api_key_preset() -> ProviderPreset {
    ProviderPreset {
        title: "OpenAI (API key)".to_string(),
        description: "Use an OpenAI API key with the OpenAI model catalog.".to_string(),
        provider_kind: ProviderPresetKind::Compatible,
        provider_id: OPENAI_API_KEY_PROVIDER_ID.to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        base_url_editable: false,
        api_key_required: true,
        api_key_env_var: Some("OPENAI_API_KEY".to_string()),
        model_placeholder: "gpt-5.4-mini".to_string(),
        default_model: None,
        wire_api: WireApi::Responses,
        sort_priority: 1,
    }
}

fn lmstudio_preset() -> ProviderPreset {
    ProviderPreset {
        title: "LM Studio".to_string(),
        description: "Connect to localhost:1234".to_string(),
        provider_kind: ProviderPresetKind::BuiltIn,
        provider_id: LMSTUDIO_OSS_PROVIDER_ID.to_string(),
        base_url: LMSTUDIO_BASE_URL.to_string(),
        base_url_editable: false,
        api_key_required: false,
        api_key_env_var: None,
        model_placeholder: "openai/gpt-oss-20b".to_string(),
        default_model: default_model_for_oss_provider(LMSTUDIO_OSS_PROVIDER_ID).map(str::to_string),
        wire_api: WireApi::Responses,
        sort_priority: 22,
    }
}

fn ollama_preset() -> ProviderPreset {
    ProviderPreset {
        title: "Ollama".to_string(),
        description: "Connect to localhost:11434".to_string(),
        provider_kind: ProviderPresetKind::BuiltIn,
        provider_id: OLLAMA_OSS_PROVIDER_ID.to_string(),
        base_url: OLLAMA_BASE_URL.to_string(),
        base_url_editable: false,
        api_key_required: false,
        api_key_env_var: None,
        model_placeholder: "gpt-oss:20b".to_string(),
        default_model: default_model_for_oss_provider(OLLAMA_OSS_PROVIDER_ID).map(str::to_string),
        wire_api: WireApi::Responses,
        sort_priority: 23,
    }
}

fn custom_compatible_preset() -> ProviderPreset {
    ProviderPreset {
        title: "Add compatible provider".to_string(),
        description: "Name a provider, set a base URL, and optionally add an API key.".to_string(),
        provider_kind: ProviderPresetKind::Compatible,
        provider_id: ADD_COMPATIBLE_PROVIDER_ID.to_string(),
        base_url: String::new(),
        base_url_editable: true,
        api_key_required: false,
        api_key_env_var: None,
        model_placeholder: "your-model-name".to_string(),
        default_model: None,
        wire_api: WireApi::Chat,
        sort_priority: 999,
    }
}

impl ProviderPreset {
    pub(crate) fn wire_api(&self) -> WireApi {
        self.wire_api
    }

    fn from_catalog_entry(entry: &BundledProviderCatalogEntry) -> Self {
        let default_model = entry.models.first().map(|model| model.id.clone());
        let model_placeholder = default_model
            .clone()
            .unwrap_or_else(|| "model-name".to_string());
        if entry.id == KIMI_FOR_CODING_PROVIDER_ID {
            return Self {
                title: entry.name.clone(),
                description: "Sign in with Kimi Code in your browser.".to_string(),
                provider_kind: ProviderPresetKind::BrowserAuth,
                provider_id: entry.id.clone(),
                base_url: entry.base_url.clone(),
                base_url_editable: false,
                api_key_required: false,
                api_key_env_var: None,
                model_placeholder,
                default_model,
                wire_api: entry.wire_api,
                sort_priority: entry.sort_priority,
            };
        }
        let description = entry
            .env_key
            .as_deref()
            .map(|env_key| format!("Use {env_key} or paste a key"))
            .unwrap_or_else(|| "No API key required".to_string());

        Self {
            title: entry.name.clone(),
            description,
            provider_kind: ProviderPresetKind::Compatible,
            provider_id: entry.id.clone(),
            base_url: entry.base_url.clone(),
            base_url_editable: false,
            api_key_required: entry.env_key.is_some(),
            api_key_env_var: entry.env_key.clone(),
            model_placeholder,
            default_model,
            wire_api: entry.wire_api,
            sort_priority: entry.sort_priority,
        }
    }

    pub(crate) fn uses_openai_auth(&self) -> bool {
        matches!(self.provider_kind, ProviderPresetKind::OpenAi)
    }

    pub(crate) fn uses_browser_auth(&self) -> bool {
        matches!(self.provider_kind, ProviderPresetKind::BrowserAuth)
    }

    pub(crate) fn api_key_env_var_name(&self, provider_name: Option<&str>) -> Option<String> {
        if self.base_url_editable {
            return compatible_provider_env_key(provider_name);
        }

        self.api_key_env_var.clone()
    }

    pub(crate) fn supports_model_picker_quick_add(&self) -> bool {
        true
    }

    pub(crate) fn quick_add_action(&self) -> Option<ProviderPresetQuickAddAction> {
        if !self.supports_model_picker_quick_add() {
            return None;
        }
        if self.base_url_editable {
            return None;
        }
        if matches!(
            self.provider_kind,
            ProviderPresetKind::BuiltIn | ProviderPresetKind::OpenAi
        ) {
            return Some(ProviderPresetQuickAddAction::WriteEdits(Vec::new()));
        }
        if matches!(self.provider_kind, ProviderPresetKind::BrowserAuth) {
            let command_cwd = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
            return Some(ProviderPresetQuickAddAction::WriteEdits(
                browser_auth_provider_definition_edits(self.provider_id.as_str(), &command_cwd),
            ));
        }

        let Some(env_var_name) = self.api_key_env_var_name(/*provider_name*/ None) else {
            return Some(ProviderPresetQuickAddAction::WriteEdits(Vec::new()));
        };

        if crate::login_support::read_env_var_trimmed(env_var_name.as_str()).is_some() {
            return Some(ProviderPresetQuickAddAction::WriteEdits(
                self.provider_definition_edits_with_auth(
                    self.provider_id.as_str(),
                    self.title.as_str(),
                    self.base_url.as_str(),
                    AuthStorageChoice::Environment(env_var_name),
                ),
            ));
        }

        if self.api_key_required {
            Some(ProviderPresetQuickAddAction::PromptForApiKey)
        } else {
            Some(ProviderPresetQuickAddAction::WriteEdits(Vec::new()))
        }
    }

    pub(crate) fn configured_provider_name(&self, provider_name: Option<&str>) -> String {
        if !self.base_url_editable {
            return self.title.clone();
        }

        let trimmed = provider_name.unwrap_or_default().trim();
        if trimmed.is_empty() {
            "Compatible provider".to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub(crate) fn configured_provider_id(&self, provider_name: Option<&str>) -> String {
        if !self.base_url_editable {
            return self.provider_id.clone();
        }

        format!(
            "{CUSTOM_PROVIDER_ID_PREFIX}{}",
            slugify_provider_name(provider_name.unwrap_or_default())
        )
    }

    pub(crate) fn provider_definition_edits(
        &self,
        provider_id: &str,
        provider_name: &str,
        base_url: &str,
        api_key: &str,
        api_key_prefilled_from_env: bool,
    ) -> Vec<AppServerConfigEdit> {
        if matches!(self.provider_kind, ProviderPresetKind::BuiltIn) {
            return Vec::new();
        }

        let env_var_name = self.api_key_env_var_name(Some(provider_name));
        let auth_storage = if api_key_prefilled_from_env {
            env_var_name
                .map(AuthStorageChoice::Environment)
                .unwrap_or(AuthStorageChoice::None)
        } else if api_key.trim().is_empty() {
            AuthStorageChoice::None
        } else {
            AuthStorageChoice::BearerToken(api_key)
        };

        let effective_base_url = if self.base_url_editable {
            base_url.trim()
        } else {
            self.base_url.as_str()
        };

        self.provider_definition_edits_with_auth(
            provider_id,
            provider_name,
            effective_base_url,
            auth_storage,
        )
    }

    fn provider_definition_edits_with_auth(
        &self,
        provider_id: &str,
        provider_name: &str,
        base_url: &str,
        auth_storage: AuthStorageChoice<'_>,
    ) -> Vec<AppServerConfigEdit> {
        let provider_segments = |tail: &str| {
            vec![
                "model_providers".to_string(),
                provider_id.to_string(),
                tail.to_string(),
            ]
        };
        let mut edits = vec![
            set_path(provider_segments("name"), json!(provider_name)),
            set_path(provider_segments("base_url"), json!(base_url)),
            set_path(
                provider_segments("wire_api"),
                json!(self.wire_api.to_string()),
            ),
            set_path(provider_segments("requires_openai_auth"), json!(false)),
            set_path(provider_segments("supports_websockets"), json!(false)),
            clear_path(provider_segments("env_key")),
            clear_path(provider_segments("env_key_instructions")),
            clear_path(provider_segments("experimental_bearer_token")),
            clear_path(provider_segments("auth")),
        ];

        match auth_storage {
            AuthStorageChoice::Environment(env_var) => {
                if !env_var.trim().is_empty() {
                    edits.push(set_path(provider_segments("env_key"), json!(env_var)));
                    edits.push(set_path(
                        provider_segments("env_key_instructions"),
                        json!(format!("Set {env_var} in your environment.")),
                    ));
                }
            }
            AuthStorageChoice::BearerToken(token) => {
                if !token.trim().is_empty() {
                    edits.push(set_path(
                        provider_segments("experimental_bearer_token"),
                        json!(token.trim()),
                    ));
                }
            }
            AuthStorageChoice::None => {}
        }

        edits
    }

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

fn slugify_provider_name(input: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for ch in input.trim().chars() {
        let lowercase = ch.to_ascii_lowercase();
        if lowercase.is_ascii_alphanumeric() {
            slug.push(lowercase);
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('_');
            last_was_separator = true;
        }
    }

    let trimmed = slug.trim_matches('_');
    if trimmed.is_empty() {
        "provider".to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AuthStorageChoice<'a> {
    Environment(String),
    BearerToken(&'a str),
    None,
}

fn compatible_provider_env_key(provider_name: Option<&str>) -> Option<String> {
    let trimmed = provider_name.unwrap_or_default().trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(format!(
        "{}_API_KEY",
        slugify_provider_name(trimmed).to_ascii_uppercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn provider_presets_include_generated_cloud_providers() {
        let preset_ids = provider_presets()
            .into_iter()
            .map(|preset| preset.provider_id)
            .collect::<Vec<_>>();
        assert!(preset_ids.contains(&"anthropic".to_string()));
        assert!(preset_ids.contains(&"openrouter".to_string()));
        assert!(preset_ids.contains(&"groq".to_string()));
        assert!(preset_ids.contains(&"github-models".to_string()));
        assert!(preset_ids.contains(&"poe".to_string()));
        assert!(preset_ids.contains(&"deepseek".to_string()));
        assert!(preset_ids.contains(&"moonshotai".to_string()));
        assert!(preset_ids.contains(&"zhipuai".to_string()));
        assert!(preset_ids.contains(&"zai".to_string()));
        assert!(preset_ids.contains(&"modelscope".to_string()));
        assert!(preset_ids.contains(&"opencode".to_string()));
    }

    #[test]
    fn anthropic_preset_uses_messages_wire_api() {
        let preset = provider_preset_by_id("anthropic").expect("anthropic preset");
        assert_eq!(preset.wire_api, WireApi::Messages);
    }

    #[test]
    fn kimi_for_coding_preset_uses_chat_wire_api() {
        let preset = provider_preset_by_id("kimi-for-coding").expect("kimi preset");
        assert_eq!(preset.wire_api, WireApi::Chat);
    }

    #[test]
    fn kimi_for_coding_preset_uses_browser_auth() {
        let preset = provider_preset_by_id(KIMI_FOR_CODING_PROVIDER_ID).expect("kimi preset");
        assert_eq!(preset.uses_browser_auth(), true);
        assert_eq!(preset.api_key_env_var_name(/*provider_name*/ None), None);
    }

    #[test]
    fn kimi_for_coding_quick_add_writes_browser_auth_config() {
        let preset = provider_preset_by_id(KIMI_FOR_CODING_PROVIDER_ID).expect("kimi preset");
        let Some(ProviderPresetQuickAddAction::WriteEdits(edits)) = preset.quick_add_action()
        else {
            panic!("expected browser auth edits");
        };
        assert_eq!(
            edits
                .iter()
                .any(|edit| edit.key_path == "model_providers.kimi-for-coding.auth"),
            true
        );
    }

    #[test]
    fn moonshot_preset_stays_api_key_backed() {
        let preset = provider_preset_by_id("moonshotai").expect("moonshot preset");
        assert_eq!(preset.uses_browser_auth(), false);
        assert_eq!(
            preset.api_key_env_var_name(/*provider_name*/ None),
            Some("MOONSHOT_API_KEY".to_string())
        );
    }

    #[test]
    fn provider_presets_do_not_duplicate_provider_ids() {
        let preset_ids = provider_presets()
            .into_iter()
            .map(|preset| preset.provider_id)
            .collect::<Vec<_>>();
        let unique_count = preset_ids.iter().cloned().collect::<HashSet<_>>().len();
        assert_eq!(unique_count, preset_ids.len());
    }

    #[test]
    fn openai_chatgpt_preset_uses_openai_auth() {
        assert!(openai_chatgpt_preset().uses_openai_auth());
    }

    #[test]
    fn openai_api_key_preset_requires_api_key_setup() {
        let state = ProviderSetupState::new(openai_api_key_preset()).expect("openai api key setup");

        assert_eq!(state.field, ProviderSetupField::ApiKey);
        assert_eq!(
            state.preset.provider_id,
            OPENAI_API_KEY_PROVIDER_ID.to_string()
        );
    }

    #[test]
    fn custom_provider_definition_edits_route_through_chat_wire() {
        let mut state = ProviderSetupState::new(custom_compatible_preset()).expect("custom setup");
        state.provider_name = "Acme Gateway".to_string();
        state.base_url = "https://example.com/v1".to_string();
        state.api_key = "sk-custom".to_string();
        state.api_key_prefilled_from_env = false;

        let edits = state.provider_definition_edits();

        assert!(edits.contains(&set_path(
            vec![
                "model_providers".to_string(),
                "compatible_acme_gateway".to_string(),
                "name".to_string(),
            ],
            json!("Acme Gateway")
        )));
        assert!(edits.contains(&set_path(
            vec![
                "model_providers".to_string(),
                "compatible_acme_gateway".to_string(),
                "wire_api".to_string(),
            ],
            json!("chat")
        )));
        assert!(edits.contains(&set_path(
            vec![
                "model_providers".to_string(),
                "compatible_acme_gateway".to_string(),
                "experimental_bearer_token".to_string(),
            ],
            json!("sk-custom")
        )));
    }

    #[test]
    fn generated_provider_quick_add_uses_present_env_key_reference() {
        let preset = ProviderPreset {
            title: "Test Provider".to_string(),
            description: "test".to_string(),
            provider_kind: ProviderPresetKind::Compatible,
            provider_id: "test_provider".to_string(),
            base_url: "https://example.com/v1".to_string(),
            base_url_editable: false,
            api_key_required: true,
            api_key_env_var: Some("PATH".to_string()),
            model_placeholder: "test-model".to_string(),
            default_model: None,
            wire_api: WireApi::Chat,
            sort_priority: 99,
        };

        let Some(ProviderPresetQuickAddAction::WriteEdits(edits)) = preset.quick_add_action()
        else {
            panic!("expected quick-add edits");
        };

        assert!(edits.contains(&set_path(
            vec![
                "model_providers".to_string(),
                "test_provider".to_string(),
                "env_key".to_string(),
            ],
            json!("PATH")
        )));
    }
}
