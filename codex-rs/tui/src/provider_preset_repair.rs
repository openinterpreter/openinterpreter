use codex_app_server_protocol::ConfigEdit as AppServerConfigEdit;
use codex_model_provider_info::ModelProviderInfo;
use serde_json::json;

use crate::config_write_edits::set_path;
use crate::onboarding::provider_setup::provider_preset_by_id;

pub(crate) fn configured_provider_repair_edits(
    provider_id: &str,
    provider: &ModelProviderInfo,
) -> Vec<AppServerConfigEdit> {
    let Some(preset) = provider_preset_by_id(provider_id) else {
        return Vec::new();
    };
    if preset.uses_openai_auth() || preset.uses_browser_auth() || preset.base_url_editable {
        return Vec::new();
    }

    let provider_segments = |tail: &str| {
        vec![
            "model_providers".to_string(),
            provider_id.to_string(),
            tail.to_string(),
        ]
    };

    let mut edits = Vec::new();
    if provider.wire_api != preset.wire_api() {
        edits.push(set_path(
            provider_segments("wire_api"),
            json!(preset.wire_api().to_string()),
        ));
    }
    if provider.requires_openai_auth {
        edits.push(set_path(
            provider_segments("requires_openai_auth"),
            json!(false),
        ));
    }
    if provider.supports_websockets {
        edits.push(set_path(
            provider_segments("supports_websockets"),
            json!(false),
        ));
    }

    let has_auth = provider
        .experimental_bearer_token
        .as_deref()
        .is_some_and(|token| !token.trim().is_empty())
        || provider.auth.is_some()
        || provider
            .env_key
            .as_deref()
            .is_some_and(|env_key| !env_key.trim().is_empty());
    if !has_auth && let Some(env_key) = preset.api_key_env_var_name(/*provider_name*/ None) {
        if provider.env_key.as_deref() != Some(env_key.as_str()) {
            edits.push(set_path(provider_segments("env_key"), json!(env_key)));
        }
        let env_key_instructions = format!("Set {env_key} in your environment.");
        if provider.env_key_instructions.as_deref() != Some(env_key_instructions.as_str()) {
            edits.push(set_path(
                provider_segments("env_key_instructions"),
                json!(env_key_instructions),
            ));
        }
    }

    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

    #[test]
    fn anthropic_repair_updates_stale_wire_api_without_dropping_auth() {
        let provider = ModelProviderInfo {
            name: "Anthropic".to_string(),
            base_url: Some("https://api.anthropic.com".to_string()),
            env_key: Some("ANTHROPIC_API_KEY".to_string()),
            env_key_instructions: Some("Set ANTHROPIC_API_KEY in your environment.".to_string()),
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
        };

        let edits = configured_provider_repair_edits("anthropic", &provider);

        assert_eq!(
            edits,
            vec![set_path(
                vec![
                    "model_providers".to_string(),
                    "anthropic".to_string(),
                    "wire_api".to_string(),
                ],
                json!("messages"),
            )]
        );
    }
}
