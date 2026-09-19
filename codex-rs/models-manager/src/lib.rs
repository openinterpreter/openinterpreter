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
    fn open_interpreter_advertises_embedded_codex_compatibility_version() {
        assert_eq!(
            client_version_to_whole_for_product(codex_product_info::Product::OpenInterpreter),
            "0.154.0"
        );
    }
}
