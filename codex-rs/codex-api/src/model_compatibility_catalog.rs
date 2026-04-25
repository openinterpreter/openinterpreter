use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ReasoningEffortPreset;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CompatibleModelCatalogEntry {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) aliases: Vec<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    pub(crate) visibility: ModelVisibility,
    #[serde(default)]
    pub(crate) supported_parameters: Vec<String>,
    #[serde(default)]
    pub(crate) supported_reasoning_levels: Vec<ReasoningEffortPreset>,
    #[serde(default)]
    pub(crate) supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub(crate) supports_search_tool: bool,
    #[serde(default)]
    pub(crate) input_modalities: Vec<InputModality>,
    #[serde(default)]
    pub(crate) context_window: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CompatibleModelCatalogFile {
    entries: Vec<CompatibleModelCatalogEntry>,
}

pub(crate) fn compatible_model_catalog_entry(
    slug: &str,
) -> Option<&'static CompatibleModelCatalogEntry> {
    static INDEX: OnceLock<HashMap<String, CompatibleModelCatalogEntry>> = OnceLock::new();

    INDEX
        .get_or_init(build_catalog_index)
        .get(&normalize_slug(slug))
}

fn build_catalog_index() -> HashMap<String, CompatibleModelCatalogEntry> {
    let catalog = serde_json::from_str::<CompatibleModelCatalogFile>(include_str!(
        "../model_compatibility_catalog.json"
    ))
    .unwrap_or_else(|err| panic!("bundled model compatibility catalog should parse: {err}"));

    let mut by_alias = HashMap::new();
    for entry in catalog.entries {
        let canonical_key = normalize_slug(entry.id.as_str());
        by_alias.insert(canonical_key, entry.clone());
        for alias in &entry.aliases {
            by_alias.insert(normalize_slug(alias.as_str()), entry.clone());
        }
    }
    by_alias
}

fn normalize_slug(slug: &str) -> String {
    slug.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn catalog_resolves_exact_openai_entries() {
        let entry = compatible_model_catalog_entry("gpt-5").expect("gpt-5 entry");
        assert_eq!(entry.visibility, ModelVisibility::List);
        assert!(
            entry
                .supported_parameters
                .iter()
                .any(|parameter| parameter == "tools")
        );
    }

    #[test]
    fn catalog_resolves_unique_suffix_aliases() {
        let entry = compatible_model_catalog_entry("openai/gpt-5-codex")
            .expect("openrouter suffix alias should resolve");
        assert_eq!(entry.id, "openrouter/openai/gpt-5-codex".to_string());
    }
}
