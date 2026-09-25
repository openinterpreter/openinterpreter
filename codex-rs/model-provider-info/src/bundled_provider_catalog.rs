use codex_protocol::openai_models::InputModality;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::WireApi;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BundledProviderCatalogEntry {
    pub id: String,
    pub name: String,
    pub env_key: Option<String>,
    pub base_url: String,
    pub wire_api: WireApi,
    #[serde(default)]
    pub env_http_headers: Option<HashMap<String, String>>,
    pub models: Vec<BundledProviderModelEntry>,
    pub sort_priority: u16,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BundledProviderModelEntry {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub thinking_toggle: bool,
    #[serde(default)]
    pub input_modalities: Vec<InputModality>,
    pub context_window: Option<i64>,
    pub priority: i32,
}

#[derive(Debug, Deserialize)]
struct BundledProviderCatalogFile {
    providers: Vec<BundledProviderCatalogEntry>,
}

pub fn bundled_provider_catalog() -> &'static [BundledProviderCatalogEntry] {
    static CATALOG: OnceLock<Vec<BundledProviderCatalogEntry>> = OnceLock::new();

    CATALOG
        .get_or_init(|| {
            serde_json::from_str::<BundledProviderCatalogFile>(include_str!(
                "../provider_catalog.json"
            ))
            .unwrap_or_else(|err| panic!("bundled provider catalog should parse: {err}"))
            .providers
        })
        .as_slice()
}

pub fn bundled_provider_catalog_entry(
    provider_id: &str,
) -> Option<&'static BundledProviderCatalogEntry> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();

    let index = INDEX.get_or_init(|| {
        bundled_provider_catalog()
            .iter()
            .enumerate()
            .map(|(idx, entry)| (entry.id.clone(), idx))
            .collect()
    });

    index
        .get(provider_id)
        .and_then(|idx| bundled_provider_catalog().get(*idx))
}

pub fn bundled_provider_catalog_entry_for_base_url(
    base_url: &str,
) -> Option<&'static BundledProviderCatalogEntry> {
    let normalized = normalize_base_url(base_url);
    bundled_provider_catalog()
        .iter()
        .find(|entry| normalize_base_url(entry.base_url.as_str()) == normalized)
}

fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn bundled_catalog_contains_expected_cloud_providers() {
        let provider_ids = bundled_provider_catalog()
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>();
        assert!(provider_ids.contains(&"github-models"));
        assert!(provider_ids.contains(&"github-copilot"));
        assert!(provider_ids.contains(&"anthropic"));
        assert!(provider_ids.contains(&"deepseek"));
        assert!(provider_ids.contains(&"moonshotai"));
        assert!(provider_ids.contains(&"zhipuai"));
        assert!(provider_ids.contains(&"zai"));
        assert!(provider_ids.contains(&"zai-zcode"));
        assert!(provider_ids.contains(&"siliconflow"));
        assert!(provider_ids.contains(&"alibaba"));
        assert!(provider_ids.contains(&"modelscope"));
        assert!(provider_ids.contains(&"opencode"));
        assert!(provider_ids.contains(&"opencode-go"));
        assert!(provider_ids.contains(&"ollama-cloud"));
        assert!(provider_ids.contains(&"llama"));
        assert!(provider_ids.contains(&"upstage"));
        assert!(provider_ids.contains(&"xiaomi"));
        assert!(provider_ids.contains(&"zhipuai-coding-plan"));
        assert!(!provider_ids.contains(&"lmstudio"));
        assert!(!provider_ids.contains(&"privatemode-ai"));
        assert!(!provider_ids.contains(&"cloudflare-workers-ai"));
    }

    #[test]
    fn anthropic_catalog_entry_uses_messages_wire_api() {
        let provider = bundled_provider_catalog_entry("anthropic").expect("anthropic provider");
        assert_eq!(provider.wire_api, WireApi::Messages);
    }

    #[test]
    fn openai_catalog_entry_contains_only_current_gpt6_models() {
        let provider = bundled_provider_catalog_entry("openai").expect("OpenAI provider");
        assert_eq!(provider.base_url, "https://api.openai.com/v1");

        let mut model_ids = provider
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        model_ids.sort_unstable();
        assert_eq!(
            model_ids,
            vec!["gpt-6-astra", "gpt-6-luna", "gpt-6-sol"]
        );
    }

    #[test]
    fn zai_zcode_catalog_entry_has_messages_endpoint_auth_metadata() {
        let provider = bundled_provider_catalog_entry("zai-zcode").expect("ZCode provider");
        assert_eq!(provider.base_url, "https://api.z.ai/api/anthropic");
        assert_eq!(provider.wire_api, WireApi::Messages);
        assert!(provider.env_http_headers.is_none());
    }

    #[test]
    fn kimi_for_coding_catalog_entry_uses_chat_wire_api() {
        let provider =
            bundled_provider_catalog_entry("kimi-for-coding").expect("kimi-for-coding provider");
        assert_eq!(provider.wire_api, WireApi::Chat);
    }

    #[test]
    fn bundled_catalog_contains_kimi_k3_for_both_official_provider_paths() {
        for (provider_id, model_id, display_name) in [
            ("kimi-for-coding", "k3", "Kimi K3"),
            ("moonshotai", "kimi-k3", "Kimi K3"),
        ] {
            let provider =
                bundled_provider_catalog_entry(provider_id).expect("Kimi provider should exist");
            let model = provider
                .models
                .iter()
                .find(|model| model.id == model_id)
                .expect("Kimi K3 model should exist");

            assert_eq!(
                model,
                &BundledProviderModelEntry {
                    id: model_id.to_string(),
                    display_name: display_name.to_string(),
                    description: Some(
                        "kimi-k3 • Reasoning • Tool calling • Image input • Video input"
                            .to_string(),
                    ),
                    reasoning: true,
                    thinking_toggle: true,
                    input_modalities: vec![InputModality::Text, InputModality::Image],
                    context_window: Some(1_048_576),
                    priority: -1,
                }
            );
        }
    }

    #[test]
    fn bundled_catalog_matches_base_url_without_trailing_slash() {
        let provider =
            bundled_provider_catalog_entry_for_base_url("https://api.fireworks.ai/inference/v1")
                .expect("fireworks provider");
        assert_eq!(provider.id, "fireworks-ai");
    }
}
