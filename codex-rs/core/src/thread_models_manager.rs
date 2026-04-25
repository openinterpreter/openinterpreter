use crate::config::Config;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_models_manager::collaboration_mode_presets::CollaborationModesConfig;
use codex_models_manager::manager::ModelsManager;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) fn models_manager_for_config(
    config: &Config,
    auth_manager: Arc<AuthManager>,
) -> ModelsManager {
    ModelsManager::new_with_provider(
        provider_cache_home(config),
        auth_manager,
        config.model_catalog.clone(),
        CollaborationModesConfig {
            default_mode_request_user_input: config
                .features
                .enabled(Feature::DefaultModeRequestUserInput),
        },
        config.model_provider.clone(),
    )
}

fn provider_cache_home(config: &Config) -> PathBuf {
    config
        .codex_home
        .join("models-cache")
        .join(sanitize_provider_id(config.model_provider_id.as_str()))
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
    use crate::config::ConfigBuilder;
    use codex_login::CodexAuth;
    use codex_model_provider_info::ModelProviderInfo;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[tokio::test]
    async fn models_manager_for_config_uses_selected_anthropic_provider_catalog() {
        let temp_dir = tempdir().expect("tempdir");
        let mut config = ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        config.model_provider_id = "anthropic".to_string();
        config.model_provider = ModelProviderInfo {
            name: "Anthropic".to_string(),
            base_url: Some("https://api.anthropic.com".to_string()),
            env_key: Some("ANTHROPIC_API_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            wire_api: WireApi::Messages,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        };
        config
            .model_providers
            .insert("anthropic".to_string(), config.model_provider.clone());

        let manager = models_manager_for_config(
            &config,
            AuthManager::from_auth_for_testing(CodexAuth::from_api_key("test")),
        );
        let model_info = manager
            .get_model_info("claude-opus-4-7", &config.to_models_manager_config())
            .await;

        assert_eq!(model_info.slug, "claude-opus-4-7".to_string());
        assert!(
            !model_info.used_fallback_model_metadata,
            "selected Anthropic provider should seed Claude model metadata"
        );
    }
}
