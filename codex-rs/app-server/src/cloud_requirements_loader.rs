use codex_core::config_loader::CloudRequirementsLoader;
use codex_login::AuthManager;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "cloud-requirements")]
pub(crate) fn build_cloud_requirements_loader(
    auth_manager: Arc<AuthManager>,
    chatgpt_base_url: String,
    codex_home: PathBuf,
) -> CloudRequirementsLoader {
    codex_cloud_requirements::cloud_requirements_loader(auth_manager, chatgpt_base_url, codex_home)
}

#[cfg(not(feature = "cloud-requirements"))]
pub(crate) fn build_cloud_requirements_loader(
    auth_manager: Arc<AuthManager>,
    chatgpt_base_url: String,
    codex_home: PathBuf,
) -> CloudRequirementsLoader {
    let _ = (auth_manager, chatgpt_base_url, codex_home);
    CloudRequirementsLoader::default()
}
