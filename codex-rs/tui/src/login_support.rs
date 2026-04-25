use codex_core::config::Config;

#[cfg(feature = "direct-login")]
pub(crate) fn configure_default_client_residency(config: &Config) {
    codex_login::default_client::set_default_client_residency_requirement(
        config.enforce_residency.value(),
    );
}

#[cfg(not(feature = "direct-login"))]
pub(crate) fn configure_default_client_residency(_config: &Config) {}

#[cfg(feature = "direct-login")]
pub(crate) fn enforce_embedded_login_restrictions(config: &Config) -> std::io::Result<()> {
    codex_login::enforce_login_restrictions(&codex_login::AuthConfig {
        codex_home: config.codex_home.clone().to_path_buf(),
        auth_credentials_store_mode: config.cli_auth_credentials_store_mode,
        forced_login_method: config.forced_login_method,
        forced_chatgpt_workspace_id: config.forced_chatgpt_workspace_id.clone(),
    })
}

#[cfg(not(feature = "direct-login"))]
pub(crate) fn enforce_embedded_login_restrictions(_config: &Config) -> std::io::Result<()> {
    Ok(())
}

#[cfg(feature = "direct-login")]
pub(crate) fn originator_value() -> String {
    codex_login::default_client::originator().value
}

#[cfg(not(feature = "direct-login"))]
pub(crate) fn originator_value() -> String {
    std::env::var(CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ORIGINATOR.to_string())
}

#[cfg(feature = "direct-login")]
pub(crate) fn read_openai_api_key_from_env_trimmed() -> Option<String> {
    read_env_var_trimmed("OPENAI_API_KEY")
}

#[cfg(not(feature = "direct-login"))]
pub(crate) fn read_openai_api_key_from_env_trimmed() -> Option<String> {
    read_env_var_trimmed(OPENAI_API_KEY_ENV_VAR)
}

pub(crate) fn read_env_var_trimmed(env_var_name: &str) -> Option<String> {
    std::env::var(env_var_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(not(feature = "direct-login"))]
const CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";
#[cfg(not(feature = "direct-login"))]
const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
#[cfg(not(feature = "direct-login"))]
const OPENAI_API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";
