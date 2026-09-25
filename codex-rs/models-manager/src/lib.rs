pub mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod compatibility_enrichment;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;
pub mod provider_catalog_models;
pub mod test_support;

pub use codex_protocol::auth::AuthMode;
pub use config::ModelsManagerConfig;

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../models.json"))
}

/// Convert the client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3").
pub fn client_version_to_whole() -> String {
    client_version_to_whole_for_product(codex_product_info::Product::current())
}

fn client_version_to_whole_for_product(product: codex_product_info::Product) -> String {
    let compatibility_version = product.codex_compatibility_version();
    compatibility_version
        .split_once('-')
        .map_or(compatibility_version, |(whole, _)| whole)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_visible_models_do_not_require_a_newer_codex_client() {
        let catalog = bundled_models_response().expect("bundled model catalog");
        let version =
            client_version_to_whole_for_product(codex_product_info::Product::OpenInterpreter);
        let parse_version = |value: &str| {
            value
                .split('.')
                .map(|part| part.parse::<u32>().expect("numeric version component"))
                .collect::<Vec<_>>()
        };
        let client_version = parse_version(&version);
        let models = serde_json::to_value(catalog).expect("serializable catalog");
        for model in models["models"].as_array().expect("model list") {
            if model["visibility"] != "list" {
                continue;
            }
            let Some(minimum) = model["minimal_client_version"].as_str() else {
                continue;
            };
            assert!(
                client_version >= parse_version(minimum),
                "{} requires Codex {minimum}, but OIX advertises {version}",
                model["slug"],
            );
        }
    }

    #[test]
    fn open_interpreter_advertises_embedded_codex_compatibility_version() {
        assert_eq!(
            client_version_to_whole_for_product(codex_product_info::Product::OpenInterpreter),
            "0.156.1"
        );
    }
}
