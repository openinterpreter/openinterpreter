use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use codex_core::config::Config;
use codex_model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use serde_json::Value as JsonValue;

use crate::onboarding::provider_setup::ProviderPreset;

const OPENAI_CHATGPT_PROVIDER_ID: &str = "openai";
const OPENAI_API_KEY_PROVIDER_ID: &str = "openai_api_key";
const OPENCODE_PROVIDER_ID: &str = "opencode";
const OPENCODE_GO_PROVIDER_ID: &str = "opencode-go";
const OPENAI_API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderReadiness {
    LoggedIn,
    Ready,
    Installed,
    NeedsSetup,
}

impl ProviderReadiness {
    pub(crate) fn sort_rank(self) -> u8 {
        match self {
            Self::LoggedIn => 0,
            Self::Ready => 1,
            Self::Installed => 2,
            Self::NeedsSetup => 3,
        }
    }

    pub(crate) fn decorate_description(self, description: String) -> String {
        match self {
            Self::LoggedIn => format!("{description} · Logged in"),
            Self::Ready => format!("{description} · Ready"),
            Self::Installed => format!("{description} · Installed"),
            Self::NeedsSetup => description,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProviderReadinessSnapshot {
    present_env_keys: HashSet<String>,
    auth_mode: Option<String>,
    has_openai_api_key_auth: bool,
    has_ollama_binary: bool,
    has_opencode_binary: bool,
}

impl ProviderReadinessSnapshot {
    pub(crate) fn from_system(config: &Config) -> Self {
        let auth = read_auth_json(&config.codex_home);
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

pub(crate) fn readiness_for_configured_provider(
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

pub(crate) fn readiness_for_provider_preset(
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

#[cfg(test)]
mod tests {
    use super::*;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

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
}
