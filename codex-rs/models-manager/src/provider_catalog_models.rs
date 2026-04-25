use codex_model_provider_info::BundledProviderModelEntry;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::bundled_provider_catalog;
use codex_model_provider_info::bundled_provider_catalog_entry;
use codex_model_provider_info::bundled_provider_catalog_entry_for_base_url;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ReasoningEffort;

pub(crate) fn bundled_provider_model_infos(provider: &ModelProviderInfo) -> Vec<ModelInfo> {
    let entry = if provider.is_anthropic_provider() {
        bundled_provider_catalog_entry("anthropic")
    } else {
        provider
            .base_url
            .as_deref()
            .and_then(bundled_provider_catalog_entry_for_base_url)
            .or_else(|| {
                bundled_provider_catalog().iter().find(|entry| {
                    entry.name.eq_ignore_ascii_case(provider.name.as_str())
                        || entry.env_key.as_deref() == provider.env_key.as_deref()
                })
            })
    };
    let Some(entry) = entry else {
        return Vec::new();
    };

    entry
        .models
        .iter()
        .map(model_info_from_bundled_provider_model)
        .collect()
}

fn model_info_from_bundled_provider_model(model: &BundledProviderModelEntry) -> ModelInfo {
    let mut fallback = crate::model_info::model_info_from_slug(model.id.as_str());
    fallback.slug = model.id.clone();
    fallback.display_name = model.display_name.clone();
    fallback.description = model.description.clone();
    fallback.default_reasoning_level = model.reasoning.then_some(ReasoningEffort::Medium);
    fallback.supported_reasoning_levels = Vec::new();
    fallback.visibility = ModelVisibility::List;
    fallback.supported_in_api = true;
    fallback.priority = model.priority;
    fallback.context_window = model.context_window.or(fallback.context_window);
    if !model.input_modalities.is_empty() {
        fallback.input_modalities = model.input_modalities.clone();
    }
    fallback.used_fallback_model_metadata = false;
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

    #[test]
    fn bundled_provider_models_seed_deepseek() {
        let provider = ModelProviderInfo {
            name: "DeepSeek".to_string(),
            base_url: Some("https://api.deepseek.com".to_string()),
            env_key: Some("DEEPSEEK_API_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
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
        };

        let models = bundled_provider_model_infos(&provider);
        assert!(models.iter().any(|model| model.slug == "deepseek-chat"));
        assert_eq!(
            models
                .iter()
                .find(|model| model.slug == "deepseek-chat")
                .expect("deepseek-chat model")
                .visibility,
            ModelVisibility::List
        );
    }

    #[test]
    fn bundled_provider_models_seed_anthropic_with_reasoning_and_vision() {
        let provider = ModelProviderInfo {
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

        let models = bundled_provider_model_infos(&provider);
        let sonnet = models
            .iter()
            .find(|model| model.slug == "claude-sonnet-4-6")
            .expect("claude-sonnet-4-6 model");
        assert_eq!(sonnet.visibility, ModelVisibility::List);
        assert_eq!(
            sonnet.default_reasoning_level,
            Some(ReasoningEffort::Medium)
        );
        assert!(
            sonnet.supported_reasoning_levels.is_empty(),
            "generated provider catalog should not invent effort levels from a boolean reasoning flag"
        );
        assert!(
            sonnet.input_modalities.iter().any(|modality| {
                matches!(
                    modality,
                    codex_protocol::openai_models::InputModality::Image
                )
            }),
            "expected anthropic model to advertise image input from the generated catalog"
        );
    }

    #[test]
    fn bundled_provider_models_seed_anthropic_for_proxy_base_url() {
        let provider = ModelProviderInfo {
            name: "Anthropic".to_string(),
            base_url: Some("http://127.0.0.1:9000".to_string()),
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

        let models = bundled_provider_model_infos(&provider);
        assert!(
            models.iter().any(|model| model.slug == "claude-sonnet-4-6"),
            "expected Anthropic proxy provider to reuse bundled Anthropic catalog"
        );
    }

    #[test]
    fn bundled_provider_models_seed_openrouter_for_proxy_base_url() {
        let provider = ModelProviderInfo {
            name: "OpenRouter".to_string(),
            base_url: Some("http://127.0.0.1:4010".to_string()),
            env_key: Some("OPENROUTER_API_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
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
        };

        let models = bundled_provider_model_infos(&provider);
        assert!(
            models
                .iter()
                .any(|model| model.slug == "anthropic/claude-sonnet-4.6"),
            "expected OpenRouter proxy provider to reuse bundled OpenRouter catalog"
        );
    }
}
