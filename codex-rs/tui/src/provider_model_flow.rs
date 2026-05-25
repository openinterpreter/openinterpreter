use codex_core::config::Config;
use codex_model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use codex_model_provider_info::OPENAI_PROVIDER_ID;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::default_harness_for_provider_model;
use std::collections::HashSet;

use crate::onboarding::provider_setup::KIMI_FOR_CODING_PROVIDER_ID;
use crate::onboarding::provider_setup::ProviderPreset;
use crate::onboarding::provider_setup::provider_preset_by_id;
use crate::onboarding::provider_setup::provider_presets;
use crate::provider_readiness::ProviderReadiness;
use crate::provider_readiness::ProviderReadinessSnapshot;
use crate::provider_readiness::readiness_for_configured_provider;
use crate::provider_readiness::readiness_for_provider_preset;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderChoice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) readiness: ProviderReadiness,
    pub(crate) is_current: bool,
    pub(crate) starts_new_chat: bool,
    pub(crate) action: ProviderChoiceAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProviderChoiceAction {
    Existing,
    QuickAdd(ProviderPreset),
}

pub(crate) fn model_picker_provider_choices(config: &Config) -> Vec<ProviderChoice> {
    model_picker_provider_choices_with_snapshot(
        config,
        &ProviderReadinessSnapshot::from_system(config),
    )
}

pub(crate) fn model_picker_provider_choices_with_snapshot(
    config: &Config,
    snapshot: &ProviderReadinessSnapshot,
) -> Vec<ProviderChoice> {
    let mut choices: Vec<ProviderChoice> = config
        .model_providers
        .iter()
        .filter(|(provider_id, _)| provider_id.as_str() != OPENAI_PROVIDER_ID)
        .map(|(provider_id, provider)| {
            let is_current = provider_id == &config.model_provider_id;
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

    let configured_ids: HashSet<&str> = config.model_providers.keys().map(String::as_str).collect();
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

    if let Some(provider) = config.model_providers.get(OPENAI_PROVIDER_ID) {
        let provider_name = provider_display_name(OPENAI_PROVIDER_ID, provider);
        let starts_new_chat = config.model_provider_id != OPENAI_PROVIDER_ID;
        let preset =
            provider_preset_by_id(OPENAI_PROVIDER_ID).expect("openai chatgpt preset should exist");
        let readiness = readiness_for_provider_preset(&preset, snapshot);
        choices.push(ProviderChoice {
            id: format!("{OPENAI_PROVIDER_ID}::chatgpt"),
            name: provider_name,
            description: readiness
                .decorate_description(provider_choice_description(OPENAI_PROVIDER_ID, provider)),
            readiness,
            is_current: config.model_provider_id == OPENAI_PROVIDER_ID,
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

pub(crate) fn provider_choice_description(
    provider_id: &str,
    provider: &ModelProviderInfo,
) -> String {
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

pub(crate) fn provider_preset_choice_description(preset: &ProviderPreset) -> String {
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
    use codex_core::config::ConfigBuilder;
    use codex_model_provider_info::ModelProviderInfo;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn empty_snapshot() -> ProviderReadinessSnapshot {
        ProviderReadinessSnapshot::default()
    }

    #[tokio::test]
    async fn model_picker_provider_choices_sort_known_providers_and_mark_current() {
        let temp_dir = tempdir().expect("tempdir");
        let mut config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        config.model_provider_id = "groq".to_string();
        config.model_providers.insert(
            "groq".to_string(),
            ModelProviderInfo {
                name: "Groq".to_string(),
                base_url: Some("https://api.groq.com/openai/v1".to_string()),
                env_key: Some("GROQ_API_KEY".to_string()),
                env_key_instructions: None,
                experimental_bearer_token: None,
                auth: None,
                aws: None,
                wire_api: WireApi::Chat,
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
        );

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());

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

    #[tokio::test]
    async fn model_picker_provider_choices_include_both_openai_auth_modes() {
        let temp_dir = tempdir().expect("tempdir");
        let config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());
        let chatgpt = choices
            .iter()
            .find(|choice| choice.id == "openai::chatgpt")
            .expect("openai chatgpt choice");
        let api_key = choices
            .iter()
            .find(|choice| choice.id == "openai_api_key")
            .expect("openai api key choice");

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

    #[tokio::test]
    async fn model_picker_provider_choices_include_anthropic_with_claude_code_harness() {
        let temp_dir = tempdir().expect("tempdir");
        let config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());
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

    #[tokio::test]
    async fn model_picker_provider_choices_include_kimi_with_kimi_cli_harness() {
        let temp_dir = tempdir().expect("tempdir");
        let config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());
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

    #[tokio::test]
    async fn model_picker_provider_choices_include_moonshot_with_api_key_auth() {
        let temp_dir = tempdir().expect("tempdir");
        let config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());
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

    #[tokio::test]
    async fn model_picker_provider_choices_include_addable_presets() {
        let temp_dir = tempdir().expect("tempdir");
        let config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());
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

    #[tokio::test]
    async fn model_picker_provider_choices_include_custom_endpoint_preset() {
        let temp_dir = tempdir().expect("tempdir");
        let config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());
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

    #[tokio::test]
    async fn model_picker_provider_choices_show_auth_configured_for_custom_provider() {
        let temp_dir = tempdir().expect("tempdir");
        let mut config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        config.model_provider_id = "compatible_acme_gateway".to_string();
        config.model_providers.insert(
            "compatible_acme_gateway".to_string(),
            ModelProviderInfo {
                name: "Acme Gateway".to_string(),
                base_url: Some("https://example.com/v1".to_string()),
                env_key: None,
                env_key_instructions: None,
                experimental_bearer_token: Some("sk-acme".to_string()),
                auth: None,
                aws: None,
                wire_api: WireApi::Chat,
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
        );

        let choices = model_picker_provider_choices_with_snapshot(&config, &empty_snapshot());
        let custom = choices
            .iter()
            .find(|choice| choice.id == "compatible_acme_gateway")
            .expect("custom choice");

        assert_eq!(custom.description, "Auth configured · Ready".to_string());
        assert_eq!(custom.readiness, ProviderReadiness::Ready);
    }
}
