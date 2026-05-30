//! Shared provider-selection logic for the `/model` provider picker.
//!
//! This module is the single source of truth for the list of providers the
//! user can pick from: configured providers plus quick-add presets, with
//! readiness labels, the OpenAI ChatGPT-vs-API-key split, and the readiness
//! sort. Both the TUI (`tui/src/provider_model_flow.rs`,
//! `tui/src/provider_readiness.rs`, `tui/src/onboarding/provider_setup.rs`) and
//! the app-server (`interpreter/provider/list`) build their lists from here so
//! the two surfaces never diverge. It mirrors how `harness_selection` is shared.
//!
//! This crate must not depend on `codex-core`, so the readiness snapshot and
//! the choice builders take `&HashMap<String, ModelProviderInfo>`,
//! `current_provider_id: &str`, and `codex_home: &Path` rather than a `Config`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use codex_app_server_protocol::ConfigEdit;
use codex_app_server_protocol::MergeStrategy;
use serde_json::Value as JsonValue;
use serde_json::json;

use crate::BundledProviderCatalogEntry;
use crate::LMSTUDIO_OSS_PROVIDER_ID;
use crate::ModelProviderInfo;
use crate::OLLAMA_OSS_PROVIDER_ID;
use crate::OPENAI_PROVIDER_ID;
use crate::WireApi;
use crate::bundled_provider_catalog;
use crate::default_harness_for_provider_model;

const OPENAI_CHATGPT_PROVIDER_ID: &str = "openai";
pub const OPENAI_API_KEY_PROVIDER_ID: &str = "openai_api_key";
const OPENCODE_PROVIDER_ID: &str = "opencode";
const OPENCODE_GO_PROVIDER_ID: &str = "opencode-go";
const OPENAI_API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";
const ADD_COMPATIBLE_PROVIDER_ID: &str = "openinterpreter_add_compatible_provider";
const CUSTOM_PROVIDER_ID_PREFIX: &str = "compatible_";
const LMSTUDIO_BASE_URL: &str = "http://localhost:1234/v1";
const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";
pub const KIMI_FOR_CODING_PROVIDER_ID: &str = "kimi-for-coding";

// ---------------------------------------------------------------------------
// Config-edit helpers (crate-local equivalents of the TUI's `config_write_edits`).
// ---------------------------------------------------------------------------

pub fn set_path(key_path: impl Into<String>, value: JsonValue) -> ConfigEdit {
    ConfigEdit {
        key_path: key_path.into(),
        value,
        merge_strategy: MergeStrategy::Replace,
    }
}

pub fn clear_path(key_path: impl Into<String>) -> ConfigEdit {
    set_path(key_path, JsonValue::Null)
}

/// Dotted config key for a `model_providers.<id>.<field>` entry.
fn provider_key(provider_id: &str, field: &str) -> String {
    format!("model_providers.{provider_id}.{field}")
}

/// Default OSS model for the built-in local providers (LM Studio / Ollama).
fn default_model_for_oss_provider(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        LMSTUDIO_OSS_PROVIDER_ID => Some("openai/gpt-oss-20b"),
        OLLAMA_OSS_PROVIDER_ID => Some("gpt-oss:20b"),
        _ => None,
    }
}

fn read_env_var_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------
// Readiness
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderReadiness {
    LoggedIn,
    Ready,
    Installed,
    NeedsSetup,
}

impl ProviderReadiness {
    pub fn sort_rank(self) -> u8 {
        match self {
            Self::LoggedIn => 0,
            Self::Ready => 1,
            Self::Installed => 2,
            Self::NeedsSetup => 3,
        }
    }

    pub fn decorate_description(self, description: String) -> String {
        match self {
            Self::LoggedIn => format!("{description} · Logged in"),
            Self::Ready => format!("{description} · Ready"),
            Self::Installed => format!("{description} · Installed"),
            Self::NeedsSetup => description,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderReadinessSnapshot {
    present_env_keys: HashSet<String>,
    auth_mode: Option<String>,
    has_openai_api_key_auth: bool,
    has_ollama_binary: bool,
    has_opencode_binary: bool,
}

impl ProviderReadinessSnapshot {
    pub fn from_system(codex_home: &Path) -> Self {
        let auth = read_auth_json(codex_home);
        Self {
            present_env_keys: std::env::vars_os()
                .filter_map(|(key, value)| (!value.is_empty()).then_some(key))
                .map(|key| key.to_string_lossy().to_string())
                .collect(),
            auth_mode: auth
                .as_ref()
                .and_then(|auth| auth.get("auth_mode"))
                .and_then(JsonValue::as_str)
                .map(str::to_string),
            has_openai_api_key_auth: auth
                .as_ref()
                .and_then(|auth| auth.get("OPENAI_API_KEY"))
                .and_then(JsonValue::as_str)
                .is_some_and(|value| !value.trim().is_empty()),
            has_ollama_binary: provider_binary_exists("ollama", /*fallback*/ None),
            has_opencode_binary: provider_binary_exists(
                "opencode",
                default_opencode_binary_path().as_ref(),
            ),
        }
    }

    fn env_var_present(&self, key: &str) -> bool {
        self.present_env_keys.contains(key)
    }
}

pub fn readiness_for_configured_provider(
    provider_id: &str,
    provider: &ModelProviderInfo,
    snapshot: &ProviderReadinessSnapshot,
) -> ProviderReadiness {
    if provider.requires_openai_auth && snapshot.auth_mode.as_deref() == Some("chatgpt") {
        return ProviderReadiness::LoggedIn;
    }
    if provider_id == OPENAI_API_KEY_PROVIDER_ID
        && (snapshot.env_var_present(OPENAI_API_KEY_ENV_VAR) || snapshot.has_openai_api_key_auth)
    {
        return ProviderReadiness::Ready;
    }
    if provider
        .env_key
        .as_deref()
        .is_some_and(|env_key| snapshot.env_var_present(env_key))
    {
        return ProviderReadiness::Ready;
    }
    if provider.experimental_bearer_token.is_some() || provider.auth.is_some() {
        return ProviderReadiness::Ready;
    }
    readiness_for_local_provider(provider_id, snapshot)
}

pub fn readiness_for_provider_preset(
    preset: &ProviderPreset,
    snapshot: &ProviderReadinessSnapshot,
) -> ProviderReadiness {
    if preset.provider_id == OPENAI_CHATGPT_PROVIDER_ID
        && snapshot.auth_mode.as_deref() == Some("chatgpt")
    {
        return ProviderReadiness::LoggedIn;
    }
    if preset.provider_id == OPENAI_API_KEY_PROVIDER_ID
        && (snapshot.env_var_present(OPENAI_API_KEY_ENV_VAR) || snapshot.has_openai_api_key_auth)
    {
        return ProviderReadiness::Ready;
    }
    if preset
        .api_key_env_var_name(/*provider_name*/ None)
        .as_deref()
        .is_some_and(|env_key| snapshot.env_var_present(env_key))
    {
        return ProviderReadiness::Ready;
    }
    readiness_for_local_provider(preset.provider_id.as_str(), snapshot)
}

fn readiness_for_local_provider(
    provider_id: &str,
    snapshot: &ProviderReadinessSnapshot,
) -> ProviderReadiness {
    if provider_id == OLLAMA_OSS_PROVIDER_ID && snapshot.has_ollama_binary {
        return ProviderReadiness::Installed;
    }
    if matches!(provider_id, OPENCODE_PROVIDER_ID | OPENCODE_GO_PROVIDER_ID)
        && snapshot.has_opencode_binary
    {
        return ProviderReadiness::Installed;
    }
    if provider_id == LMSTUDIO_OSS_PROVIDER_ID {
        return ProviderReadiness::NeedsSetup;
    }
    ProviderReadiness::NeedsSetup
}

fn read_auth_json(codex_home: &Path) -> Option<JsonValue> {
    let auth_path = codex_home.join("auth.json");
    let contents = std::fs::read_to_string(auth_path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn provider_binary_exists(binary_name: &str, fallback: Option<&PathBuf>) -> bool {
    path_binary_exists(binary_name) || fallback.is_some_and(|path| path.exists())
}

fn default_opencode_binary_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".opencode").join("bin").join("opencode"))
}

fn path_binary_exists(binary_name: &str) -> bool {
    env::var_os("PATH")
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .map(|dir| dir.join(binary_name))
        .any(|candidate| is_executable_path(candidate.as_os_str()))
}

fn is_executable_path(path: &OsStr) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderPresetKind {
    OpenAi,
    BrowserAuth,
    BuiltIn,
    Compatible,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderPresetQuickAddAction {
    WriteEdits(Vec<ConfigEdit>),
    PromptForApiKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderPreset {
    pub title: String,
    pub description: String,
    provider_kind: ProviderPresetKind,
    pub provider_id: String,
    pub base_url: String,
    pub base_url_editable: bool,
    pub api_key_required: bool,
    pub api_key_env_var: Option<String>,
    pub model_placeholder: String,
    pub default_model: Option<String>,
    wire_api: WireApi,
    pub sort_priority: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderIdentity {
    pub id: String,
    pub name: String,
}

pub fn provider_presets() -> Vec<ProviderPreset> {
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

pub fn provider_preset_by_id(provider_id: &str) -> Option<ProviderPreset> {
    provider_presets()
        .into_iter()
        .find(|preset| preset.provider_id == provider_id)
}

pub fn default_provider_preset_id() -> String {
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

pub fn browser_auth_provider_definition_edits(
    provider_id: &str,
    command_cwd: &Path,
) -> Vec<ConfigEdit> {
    if provider_id != KIMI_FOR_CODING_PROVIDER_ID {
        return Vec::new();
    }
    vec![
        set_path(provider_key(provider_id, "wire_api"), json!("chat")),
        set_path(
            provider_key(provider_id, "requires_openai_auth"),
            json!(false),
        ),
        clear_path(provider_key(provider_id, "env_key")),
        clear_path(provider_key(provider_id, "env_key_instructions")),
        clear_path(provider_key(provider_id, "experimental_bearer_token")),
        set_path(
            provider_key(provider_id, "auth"),
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
    pub fn wire_api(&self) -> WireApi {
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

    pub fn uses_openai_auth(&self) -> bool {
        matches!(self.provider_kind, ProviderPresetKind::OpenAi)
    }

    pub fn uses_browser_auth(&self) -> bool {
        matches!(self.provider_kind, ProviderPresetKind::BrowserAuth)
    }

    pub fn api_key_env_var_name(&self, provider_name: Option<&str>) -> Option<String> {
        if self.base_url_editable {
            return compatible_provider_env_key(provider_name);
        }

        self.api_key_env_var.clone()
    }

    pub fn supports_model_picker_quick_add(&self) -> bool {
        true
    }

    pub fn quick_add_action(&self) -> Option<ProviderPresetQuickAddAction> {
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

        if read_env_var_trimmed(env_var_name.as_str()).is_some() {
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

    pub fn configured_provider_name(&self, provider_name: Option<&str>) -> String {
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

    pub fn configured_provider_id(&self, provider_name: Option<&str>) -> String {
        if !self.base_url_editable {
            return self.provider_id.clone();
        }

        format!(
            "{CUSTOM_PROVIDER_ID_PREFIX}{}",
            slugify_provider_name(provider_name.unwrap_or_default())
        )
    }

    pub fn provider_definition_edits(
        &self,
        provider_id: &str,
        provider_name: &str,
        base_url: &str,
        api_key: &str,
        api_key_prefilled_from_env: bool,
    ) -> Vec<ConfigEdit> {
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
    ) -> Vec<ConfigEdit> {
        let mut edits = vec![
            set_path(provider_key(provider_id, "name"), json!(provider_name)),
            set_path(provider_key(provider_id, "base_url"), json!(base_url)),
            set_path(
                provider_key(provider_id, "wire_api"),
                json!(self.wire_api.to_string()),
            ),
            set_path(
                provider_key(provider_id, "requires_openai_auth"),
                json!(false),
            ),
            set_path(
                provider_key(provider_id, "supports_websockets"),
                json!(false),
            ),
            clear_path(provider_key(provider_id, "env_key")),
            clear_path(provider_key(provider_id, "env_key_instructions")),
            clear_path(provider_key(provider_id, "experimental_bearer_token")),
            clear_path(provider_key(provider_id, "auth")),
        ];

        match auth_storage {
            AuthStorageChoice::Environment(env_var) => {
                if !env_var.trim().is_empty() {
                    edits.push(set_path(
                        provider_key(provider_id, "env_key"),
                        json!(env_var),
                    ));
                    edits.push(set_path(
                        provider_key(provider_id, "env_key_instructions"),
                        json!(format!("Set {env_var} in your environment.")),
                    ));
                }
            }
            AuthStorageChoice::BearerToken(token) => {
                if !token.trim().is_empty() {
                    edits.push(set_path(
                        provider_key(provider_id, "experimental_bearer_token"),
                        json!(token.trim()),
                    ));
                }
            }
            AuthStorageChoice::None => {}
        }

        edits
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

// ---------------------------------------------------------------------------
// Choices
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderChoice {
    pub id: String,
    pub name: String,
    pub description: String,
    pub readiness: ProviderReadiness,
    pub is_current: bool,
    pub starts_new_chat: bool,
    pub action: ProviderChoiceAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderChoiceAction {
    Existing,
    QuickAdd(ProviderPreset),
}

pub fn model_picker_provider_choices(
    providers: &HashMap<String, ModelProviderInfo>,
    current_provider_id: &str,
    codex_home: &Path,
) -> Vec<ProviderChoice> {
    model_picker_provider_choices_with_snapshot(
        providers,
        current_provider_id,
        &ProviderReadinessSnapshot::from_system(codex_home),
    )
}

pub fn model_picker_provider_choices_with_snapshot(
    providers: &HashMap<String, ModelProviderInfo>,
    current_provider_id: &str,
    snapshot: &ProviderReadinessSnapshot,
) -> Vec<ProviderChoice> {
    let mut choices: Vec<ProviderChoice> = providers
        .iter()
        .filter(|(provider_id, _)| provider_id.as_str() != OPENAI_PROVIDER_ID)
        .map(|(provider_id, provider)| {
            let is_current = provider_id.as_str() == current_provider_id;
            let starts_new_chat = !is_current;
            let name = provider_display_name(provider_id, provider);
            let readiness =
                readiness_for_configured_provider(provider_id.as_str(), provider, snapshot);
            ProviderChoice {
                id: provider_id.clone(),
                name,
                description: readiness
                    .decorate_description(provider_choice_description(provider_id, provider)),
                readiness,
                is_current,
                starts_new_chat,
                action: ProviderChoiceAction::Existing,
            }
        })
        .collect();

    let configured_ids: HashSet<&str> = providers.keys().map(String::as_str).collect();
    for preset in provider_presets() {
        if configured_ids.contains(preset.provider_id.as_str())
            || !preset.supports_model_picker_quick_add()
        {
            continue;
        }

        let readiness = readiness_for_provider_preset(&preset, snapshot);
        choices.push(ProviderChoice {
            id: preset.provider_id.clone(),
            name: preset.title.clone(),
            description: readiness
                .decorate_description(provider_preset_choice_description(&preset)),
            readiness,
            is_current: false,
            starts_new_chat: true,
            action: ProviderChoiceAction::QuickAdd(preset),
        });
    }

    if let Some(provider) = providers.get(OPENAI_PROVIDER_ID) {
        let provider_name = provider_display_name(OPENAI_PROVIDER_ID, provider);
        let starts_new_chat = current_provider_id != OPENAI_PROVIDER_ID;
        let preset =
            provider_preset_by_id(OPENAI_PROVIDER_ID).expect("openai chatgpt preset should exist");
        let readiness = readiness_for_provider_preset(&preset, snapshot);
        choices.push(ProviderChoice {
            id: format!("{OPENAI_PROVIDER_ID}::chatgpt"),
            name: provider_name,
            description: readiness
                .decorate_description(provider_choice_description(OPENAI_PROVIDER_ID, provider)),
            readiness,
            is_current: current_provider_id == OPENAI_PROVIDER_ID,
            starts_new_chat,
            action: ProviderChoiceAction::QuickAdd(preset),
        });
    }

    choices.sort_by(|left, right| {
        provider_sort_key(left.readiness, left.id.as_str(), left.name.as_str()).cmp(
            &provider_sort_key(right.readiness, right.id.as_str(), right.name.as_str()),
        )
    });
    choices
}

fn provider_display_name(provider_id: &str, provider: &ModelProviderInfo) -> String {
    provider_preset_by_id(provider_id)
        .map(|preset| preset.title)
        .unwrap_or_else(|| provider.name.clone())
}

pub fn provider_choice_description(provider_id: &str, provider: &ModelProviderInfo) -> String {
    let description = if provider.requires_openai_auth {
        "Sign in with ChatGPT".to_string()
    } else if provider_id == KIMI_FOR_CODING_PROVIDER_ID {
        if provider.auth.is_some() {
            "Signed in with Kimi Code".to_string()
        } else {
            "Sign in with Kimi Code".to_string()
        }
    } else if provider_id == LMSTUDIO_OSS_PROVIDER_ID {
        "Connect to localhost:1234".to_string()
    } else if provider_id == OLLAMA_OSS_PROVIDER_ID {
        "Connect to localhost:11434".to_string()
    } else if let Some(env_key) = provider.env_key.as_deref() {
        format!("Use {env_key} or paste a key")
    } else if provider
        .experimental_bearer_token
        .as_deref()
        .is_some_and(|token| !token.trim().is_empty())
        || provider.auth.is_some()
    {
        "Auth configured".to_string()
    } else {
        match provider.wire_api {
            WireApi::Responses => "No API key required".to_string(),
            WireApi::Chat => "Chat-compatible endpoint".to_string(),
            WireApi::Messages => "Anthropic Messages endpoint".to_string(),
        }
    };

    decorate_harness_description(
        description,
        default_harness_for_provider_model(provider_id, provider, None),
    )
}

pub fn provider_preset_choice_description(preset: &ProviderPreset) -> String {
    let description = if preset.uses_openai_auth() {
        "Sign in with ChatGPT".to_string()
    } else if preset.uses_browser_auth() {
        "Sign in with Kimi Code".to_string()
    } else if preset.provider_id == LMSTUDIO_OSS_PROVIDER_ID {
        "Connect to localhost:1234".to_string()
    } else if preset.provider_id == OLLAMA_OSS_PROVIDER_ID {
        "Connect to localhost:11434".to_string()
    } else if preset.base_url_editable {
        "Name it, set a base URL, and optionally add a key".to_string()
    } else if let Some(env_key) = preset.api_key_env_var_name(/*provider_name*/ None) {
        format!("Use {env_key} or paste a key")
    } else {
        "No API key required".to_string()
    };

    decorate_harness_description(
        description,
        default_harness_for_provider_model(
            preset.provider_id.as_str(),
            &ModelProviderInfo {
                name: preset.title.clone(),
                base_url: Some(preset.base_url.clone()),
                wire_api: preset.wire_api(),
                ..Default::default()
            },
            None,
        ),
    )
}

fn decorate_harness_description(description: String, harness: Option<&str>) -> String {
    match harness {
        Some(harness) => format!("{description} | Harness: {harness}"),
        None => description,
    }
}

fn provider_sort_key(
    readiness: ProviderReadiness,
    provider_id: &str,
    provider_name: &str,
) -> (u8, u16, String) {
    let normalized_provider_id = provider_id
        .split_once("::")
        .map(|(base_provider_id, _)| base_provider_id)
        .unwrap_or(provider_id);
    let priority = provider_preset_by_id(normalized_provider_id)
        .map(|preset| preset.sort_priority)
        .unwrap_or(100);

    (
        readiness.sort_rank(),
        priority,
        provider_name.to_ascii_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // -- Readiness ---------------------------------------------------------

    fn ready_snapshot() -> ProviderReadinessSnapshot {
        ProviderReadinessSnapshot {
            present_env_keys: HashSet::from([
                "GROQ_API_KEY".to_string(),
                OPENAI_API_KEY_ENV_VAR.to_string(),
            ]),
            auth_mode: Some("apikey".to_string()),
            has_openai_api_key_auth: true,
            has_ollama_binary: true,
            has_opencode_binary: false,
        }
    }

    #[test]
    fn configured_openai_api_key_provider_is_ready_with_auth_json() {
        let readiness = readiness_for_configured_provider(
            OPENAI_API_KEY_PROVIDER_ID,
            &ModelProviderInfo {
                name: "OpenAI (API key)".to_string(),
                base_url: Some("https://api.openai.com/v1".to_string()),
                env_key: Some(OPENAI_API_KEY_ENV_VAR.to_string()),
                env_key_instructions: None,
                experimental_bearer_token: None,
                auth: None,
                aws: None,
                wire_api: WireApi::Responses,
                query_params: None,
                http_headers: None,
                env_http_headers: None,
                request_max_retries: None,
                stream_max_retries: None,
                stream_idle_timeout_ms: None,
                websocket_connect_timeout_ms: None,
                requires_openai_auth: false,
                supports_websockets: false,
            },
            &ready_snapshot(),
        );

        assert_eq!(readiness, ProviderReadiness::Ready);
    }

    #[test]
    fn ollama_preset_is_installed_when_binary_exists() {
        let readiness = readiness_for_local_provider(
            OLLAMA_OSS_PROVIDER_ID,
            &ProviderReadinessSnapshot {
                has_ollama_binary: true,
                ..ProviderReadinessSnapshot::default()
            },
        );

        assert_eq!(readiness, ProviderReadiness::Installed);
    }

    // -- Presets -----------------------------------------------------------

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
    fn custom_provider_definition_edits_route_through_chat_wire() {
        let preset = custom_compatible_preset();
        let edits = preset.provider_definition_edits(
            "compatible_acme_gateway",
            "Acme Gateway",
            "https://example.com/v1",
            "sk-custom",
            /*api_key_prefilled_from_env*/ false,
        );

        assert!(edits.contains(&set_path(
            "model_providers.compatible_acme_gateway.name",
            json!("Acme Gateway")
        )));
        assert!(edits.contains(&set_path(
            "model_providers.compatible_acme_gateway.wire_api",
            json!("chat")
        )));
        assert!(edits.contains(&set_path(
            "model_providers.compatible_acme_gateway.experimental_bearer_token",
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
            "model_providers.test_provider.env_key",
            json!("PATH")
        )));
    }

    // -- Choices -----------------------------------------------------------

    fn empty_snapshot() -> ProviderReadinessSnapshot {
        ProviderReadinessSnapshot::default()
    }

    fn provider_with_env_key(
        name: &str,
        base_url: &str,
        env_key: &str,
        wire_api: WireApi,
    ) -> ModelProviderInfo {
        ModelProviderInfo {
            name: name.to_string(),
            base_url: Some(base_url.to_string()),
            env_key: Some(env_key.to_string()),
            wire_api,
            ..Default::default()
        }
    }

    #[test]
    fn model_picker_provider_choices_sort_known_providers_and_mark_current() {
        // A real `Config` always carries the built-in `openai` provider, which the
        // picker renders as the `openai::chatgpt` split rather than a bare preset.
        let mut providers: HashMap<String, ModelProviderInfo> = HashMap::new();
        providers.insert(
            OPENAI_PROVIDER_ID.to_string(),
            ModelProviderInfo::create_openai_provider(None),
        );
        providers.insert(
            "groq".to_string(),
            provider_with_env_key(
                "Groq",
                "https://api.groq.com/openai/v1",
                "GROQ_API_KEY",
                WireApi::Chat,
            ),
        );

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "groq", &empty_snapshot());

        assert_eq!(
            choices
                .iter()
                .map(|choice| choice.id.as_str())
                .take(5)
                .collect::<Vec<_>>(),
            vec![
                "openai::chatgpt",
                "openai_api_key",
                "anthropic",
                "openrouter",
                "groq",
            ]
        );
        assert_eq!(
            choices
                .iter()
                .find(|choice| choice.id == "groq")
                .expect("groq choice"),
            &ProviderChoice {
                id: "groq".to_string(),
                name: "Groq".to_string(),
                description: "Use GROQ_API_KEY or paste a key".to_string(),
                readiness: ProviderReadiness::NeedsSetup,
                is_current: true,
                starts_new_chat: false,
                action: ProviderChoiceAction::Existing,
            }
        );
    }

    #[test]
    fn model_picker_provider_choices_include_both_openai_auth_modes() {
        let providers: HashMap<String, ModelProviderInfo> = HashMap::new();

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "openai", &empty_snapshot());
        let api_key = choices
            .iter()
            .find(|choice| choice.id == "openai_api_key")
            .expect("openai api key choice");

        assert_eq!(
            api_key,
            &ProviderChoice {
                id: "openai_api_key".to_string(),
                name: "OpenAI (API key)".to_string(),
                description: "Use OPENAI_API_KEY or paste a key".to_string(),
                readiness: ProviderReadiness::NeedsSetup,
                is_current: false,
                starts_new_chat: true,
                action: ProviderChoiceAction::QuickAdd(
                    provider_preset_by_id("openai_api_key").expect("openai api key preset")
                ),
            }
        );
    }

    #[test]
    fn model_picker_provider_choices_include_configured_openai_chatgpt_split() {
        let mut providers: HashMap<String, ModelProviderInfo> = HashMap::new();
        providers.insert(
            OPENAI_PROVIDER_ID.to_string(),
            ModelProviderInfo::create_openai_provider(None),
        );

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "openai", &empty_snapshot());
        let chatgpt = choices
            .iter()
            .find(|choice| choice.id == "openai::chatgpt")
            .expect("openai chatgpt choice");

        assert_eq!(
            chatgpt,
            &ProviderChoice {
                id: "openai::chatgpt".to_string(),
                name: "OpenAI (ChatGPT sign-in)".to_string(),
                description: "Sign in with ChatGPT".to_string(),
                readiness: ProviderReadiness::NeedsSetup,
                is_current: true,
                starts_new_chat: false,
                action: ProviderChoiceAction::QuickAdd(
                    provider_preset_by_id("openai").expect("openai chatgpt preset")
                ),
            }
        );
    }

    #[test]
    fn model_picker_provider_choices_include_anthropic_with_claude_code_harness() {
        let providers: HashMap<String, ModelProviderInfo> = HashMap::new();

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "openai", &empty_snapshot());
        let anthropic = choices
            .iter()
            .find(|choice| choice.id == "anthropic")
            .expect("anthropic choice");

        assert_eq!(anthropic.name, "Anthropic".to_string());
        assert_eq!(
            anthropic.description,
            "Use ANTHROPIC_API_KEY or paste a key | Harness: claude-code".to_string()
        );
    }

    #[test]
    fn model_picker_provider_choices_include_kimi_with_kimi_cli_harness() {
        let providers: HashMap<String, ModelProviderInfo> = HashMap::new();

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "openai", &empty_snapshot());
        let kimi = choices
            .iter()
            .find(|choice| choice.id == "kimi-for-coding")
            .expect("kimi provider choice");

        assert_eq!(kimi.name, "Kimi For Coding".to_string());
        assert_eq!(
            kimi.description,
            "Sign in with Kimi Code | Harness: kimi-cli".to_string()
        );
    }

    #[test]
    fn model_picker_provider_choices_include_moonshot_with_api_key_auth() {
        let providers: HashMap<String, ModelProviderInfo> = HashMap::new();

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "openai", &empty_snapshot());
        let moonshot = choices
            .iter()
            .find(|choice| choice.id == "moonshotai")
            .expect("moonshot provider choice");

        assert_eq!(moonshot.name, "Moonshot AI".to_string());
        assert_eq!(
            moonshot.description,
            "Use MOONSHOT_API_KEY or paste a key | Harness: kimi-cli".to_string()
        );
    }

    #[test]
    fn model_picker_provider_choices_include_addable_presets() {
        let providers: HashMap<String, ModelProviderInfo> = HashMap::new();

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "openai", &empty_snapshot());
        let openrouter = choices
            .iter()
            .find(|choice| choice.id == "openrouter")
            .expect("openrouter choice");

        assert_eq!(
            openrouter,
            &ProviderChoice {
                id: "openrouter".to_string(),
                name: "OpenRouter".to_string(),
                description: "Use OPENROUTER_API_KEY or paste a key".to_string(),
                readiness: ProviderReadiness::NeedsSetup,
                is_current: false,
                starts_new_chat: true,
                action: ProviderChoiceAction::QuickAdd(
                    provider_preset_by_id("openrouter").expect("openrouter preset")
                ),
            }
        );
    }

    #[test]
    fn model_picker_provider_choices_include_custom_endpoint_preset() {
        let providers: HashMap<String, ModelProviderInfo> = HashMap::new();

        let choices =
            model_picker_provider_choices_with_snapshot(&providers, "openai", &empty_snapshot());
        let custom = choices
            .iter()
            .find(|choice| choice.id == "openinterpreter_add_compatible_provider")
            .expect("custom choice");

        assert_eq!(
            custom,
            &ProviderChoice {
                id: "openinterpreter_add_compatible_provider".to_string(),
                name: "Add compatible provider".to_string(),
                description: "Name it, set a base URL, and optionally add a key".to_string(),
                readiness: ProviderReadiness::NeedsSetup,
                is_current: false,
                starts_new_chat: true,
                action: ProviderChoiceAction::QuickAdd(
                    provider_preset_by_id("openinterpreter_add_compatible_provider")
                        .expect("custom compatible preset")
                ),
            }
        );
    }

    #[test]
    fn model_picker_provider_choices_show_auth_configured_for_custom_provider() {
        let mut providers: HashMap<String, ModelProviderInfo> = HashMap::new();
        providers.insert(
            "compatible_acme_gateway".to_string(),
            ModelProviderInfo {
                name: "Acme Gateway".to_string(),
                base_url: Some("https://example.com/v1".to_string()),
                experimental_bearer_token: Some("sk-acme".to_string()),
                wire_api: WireApi::Chat,
                ..Default::default()
            },
        );

        let choices = model_picker_provider_choices_with_snapshot(
            &providers,
            "compatible_acme_gateway",
            &empty_snapshot(),
        );
        let custom = choices
            .iter()
            .find(|choice| choice.id == "compatible_acme_gateway")
            .expect("custom choice");

        assert_eq!(custom.description, "Auth configured · Ready".to_string());
        assert_eq!(custom.readiness, ProviderReadiness::Ready);
    }
}
