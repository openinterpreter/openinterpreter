use codex_config::CloudRequirementsLoader;
use std::path::PathBuf;

#[cfg(feature = "cloud-requirements")]
pub(crate) fn build_cloud_requirements_loader(
    codex_home: PathBuf,
    auth_credentials_store_mode: codex_login::AuthCredentialsStoreMode,
    chatgpt_base_url: String,
) -> CloudRequirementsLoader {
    codex_cloud_requirements::cloud_requirements_loader_for_storage(
        codex_home,
        /*enable_codex_api_key_env*/ false,
        auth_credentials_store_mode,
        chatgpt_base_url,
    )
}

#[cfg(not(feature = "cloud-requirements"))]
pub(crate) fn build_cloud_requirements_loader<S>(
    codex_home: PathBuf,
    auth_credentials_store_mode: S,
    chatgpt_base_url: String,
) -> CloudRequirementsLoader {
    let _ = (codex_home, auth_credentials_store_mode, chatgpt_base_url);
    CloudRequirementsLoader::default()
}
