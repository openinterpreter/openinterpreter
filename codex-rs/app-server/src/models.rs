use std::sync::Arc;

use codex_app_server_protocol::Model;
use codex_app_server_protocol::ModelUpgradeInfo;
use codex_app_server_protocol::ReasoningEffortOption;
use codex_core::config::Config;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_models_manager::collaboration_mode_presets::CollaborationModesConfig;
use codex_models_manager::manager::ModelsManager;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffortPreset;

pub async fn supported_models(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    include_hidden: bool,
) -> Vec<Model> {
    let collaboration_modes_config = CollaborationModesConfig {
        default_mode_request_user_input: config
            .features
            .enabled(Feature::DefaultModeRequestUserInput),
    };
    ModelsManager::new_with_provider(
        provider_cache_home(config, config.model_provider_id.as_str()),
        auth_manager,
        config.model_catalog.clone(),
        collaboration_modes_config,
        config.model_provider.clone(),
    )
    .list_models(RefreshStrategy::OnlineIfUncached)
    .await
    .into_iter()
    .filter(|preset| include_hidden || preset.show_in_picker)
    .map(model_from_preset)
    .collect()
}

pub async fn supported_models_for_provider(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    provider_id: &str,
    include_hidden: bool,
) -> Result<Vec<Model>, String> {
    let provider = config
        .model_providers
        .get(provider_id)
        .cloned()
        .ok_or_else(|| format!("model provider `{provider_id}` not found"))?;
    let collaboration_modes_config = CollaborationModesConfig {
        default_mode_request_user_input: config
            .features
            .enabled(Feature::DefaultModeRequestUserInput),
    };
    let model_catalog = if provider_id == config.model_provider_id {
        config.model_catalog.clone()
    } else {
        None
    };
    let models = ModelsManager::new_with_provider(
        provider_cache_home(config, provider_id),
        auth_manager,
        model_catalog,
        collaboration_modes_config,
        provider,
    )
    .list_models(RefreshStrategy::OnlineIfUncached)
    .await;

    Ok(models
        .into_iter()
        .filter(|preset| include_hidden || preset.show_in_picker)
        .map(model_from_preset)
        .collect())
}

fn model_from_preset(preset: ModelPreset) -> Model {
    Model {
        id: preset.id.to_string(),
        model: preset.model.to_string(),
        upgrade: preset.upgrade.as_ref().map(|upgrade| upgrade.id.clone()),
        upgrade_info: preset.upgrade.as_ref().map(|upgrade| ModelUpgradeInfo {
            model: upgrade.id.clone(),
            upgrade_copy: upgrade.upgrade_copy.clone(),
            model_link: upgrade.model_link.clone(),
            migration_markdown: upgrade.migration_markdown.clone(),
        }),
        availability_nux: preset.availability_nux.map(Into::into),
        display_name: preset.display_name.to_string(),
        description: preset.description.to_string(),
        hidden: !preset.show_in_picker,
        supported_reasoning_efforts: reasoning_efforts_from_preset(
            preset.supported_reasoning_efforts,
        ),
        default_reasoning_effort: preset.default_reasoning_effort,
        input_modalities: preset.input_modalities,
        supports_personality: preset.supports_personality,
        additional_speed_tiers: preset.additional_speed_tiers,
        is_default: preset.is_default,
    }
}

fn reasoning_efforts_from_preset(
    efforts: Vec<ReasoningEffortPreset>,
) -> Vec<ReasoningEffortOption> {
    efforts
        .iter()
        .map(|preset| ReasoningEffortOption {
            reasoning_effort: preset.effort,
            description: preset.description.to_string(),
        })
        .collect()
}

fn provider_cache_home(config: &Config, provider_id: &str) -> std::path::PathBuf {
    config
        .codex_home
        .join("models-cache")
        .join(sanitize_provider_id(provider_id))
        .to_path_buf()
}

fn sanitize_provider_id(provider_id: &str) -> String {
    provider_id
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_login::AuthCredentialsStoreMode;
    use codex_login::CodexAuth;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::header_regex;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[test]
    fn sanitize_provider_id_replaces_non_filesystem_chars() {
        assert_eq!(
            sanitize_provider_id("custom/provider:alpha"),
            "custom_provider_alpha".to_string()
        );
    }

    #[tokio::test]
    async fn supported_models_for_provider_uses_requested_provider() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header_regex("Authorization", "Bearer Test API Key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    {
                        "id": "llama-3.3-70b-versatile",
                        "object": "model",
                        "context_window": 128000
                    }
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let temp_dir = tempdir().expect("tempdir");
        let mut config = codex_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        let mut provider = config.model_provider.clone();
        provider.name = "Groq".to_string();
        provider.base_url = Some(server.uri());
        provider.env_key = None;
        provider.experimental_bearer_token = Some("Test API Key".to_string());
        provider.requires_openai_auth = false;
        provider.supports_websockets = false;
        config.model_providers.insert("groq".to_string(), provider);

        let models = supported_models_for_provider(
            &config,
            AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key")),
            "groq",
            /*include_hidden*/ false,
        )
        .await
        .expect("provider models");

        assert!(
            models
                .iter()
                .any(|model| model.model == "llama-3.3-70b-versatile"),
            "expected provider-specific model to be present in merged catalog"
        );
    }

    #[tokio::test]
    async fn supported_models_for_provider_allows_public_catalog_without_provider_env_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(serde_json::json!({
                        "data": [
                            {
                                "id": "anthropic/claude-sonnet-4.6",
                                "name": "Anthropic: Claude Sonnet 4.6",
                                "description": "High-end Anthropic reasoning model",
                                "supported_parameters": [
                                    "tools",
                                    "tool_choice",
                                    "reasoning",
                                    "temperature"
                                ],
                                "architecture": {
                                    "input_modalities": ["text", "image"]
                                }
                            }
                        ]
                    })),
            )
            .mount(&server)
            .await;

        let temp_dir = tempdir().expect("tempdir");
        let mut config = codex_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        let mut provider = config.model_provider.clone();
        provider.name = "OpenRouter".to_string();
        provider.base_url = Some(server.uri());
        provider.env_key = Some("OPENROUTER_API_KEY".to_string());
        provider.env_key_instructions = Some("Set OPENROUTER_API_KEY".to_string());
        provider.experimental_bearer_token = None;
        provider.auth = None;
        provider.query_params = None;
        provider.http_headers = None;
        provider.env_http_headers = None;
        provider.request_max_retries = Some(0);
        provider.stream_max_retries = Some(0);
        provider.stream_idle_timeout_ms = Some(5_000);
        provider.websocket_connect_timeout_ms = None;
        provider.requires_openai_auth = false;
        provider.supports_websockets = false;
        config
            .model_providers
            .insert("openrouter".to_string(), provider);

        let auth_manager = Arc::new(AuthManager::new(
            temp_dir.path().to_path_buf(),
            /*enable_codex_api_key_env*/ false,
            AuthCredentialsStoreMode::File,
        ));
        let direct_models = codex_models_manager::manager::ModelsManager::with_provider_for_tests(
            temp_dir.path().to_path_buf(),
            Arc::clone(&auth_manager),
            config
                .model_providers
                .get("openrouter")
                .cloned()
                .expect("openrouter provider"),
        )
        .list_models(codex_models_manager::manager::RefreshStrategy::OnlineIfUncached)
        .await;
        assert!(
            direct_models
                .iter()
                .any(|model| model.model == "anthropic/claude-sonnet-4.6"),
            "expected direct manager fallback to expose the public OpenRouter catalog"
        );
        let models = supported_models_for_provider(
            &config,
            auth_manager,
            "openrouter",
            /*include_hidden*/ false,
        )
        .await
        .expect("provider models");
        let requests = server.received_requests().await.unwrap_or_default();

        assert!(
            models
                .iter()
                .any(|model| model.model == "anthropic/claude-sonnet-4.6"),
            "expected public provider catalog model to be present without OPENROUTER_API_KEY"
        );
        assert!(
            requests.is_empty(),
            "expected bundled public provider catalog fallback without a live /models request"
        );
    }
}
