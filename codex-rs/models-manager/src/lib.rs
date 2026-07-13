pub(crate) mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod compatibility_enrichment;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;
pub mod provider_catalog_models;
pub mod test_support;

pub use codex_protocol::auth::AuthMode;
use codex_protocol::openai_models::ClientVersion;
pub use config::ModelsManagerConfig;

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../models.json"))
}

/// Convert the client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3").
pub fn client_version_to_whole() -> String {
    let version = client_version();
    format!("{}.{}.{}", version.0, version.1, version.2)
}

/// Return the client version used for model compatibility checks.
pub(crate) fn client_version() -> ClientVersion {
    ClientVersion(
        parse_package_version_component(env!("CARGO_PKG_VERSION_MAJOR")),
        parse_package_version_component(env!("CARGO_PKG_VERSION_MINOR")),
        parse_package_version_component(env!("CARGO_PKG_VERSION_PATCH")),
    )
}

fn parse_package_version_component(component: &str) -> i32 {
    match component.parse() {
        Ok(value) => value,
        Err(_) => unreachable!("Cargo package version components are numeric"),
    }
}
