//! Reads the owner's current home-deletion preference; existing OEM preinstalls retain their default.

use std::io;
use std::path::Path;

use codex_config::AbsolutePathBufGuard;
use codex_config::config_toml::ConfigToml;

pub(crate) fn clean_uninstall_enabled(home: &Path, preinstalled: bool) -> bool {
    let contents = match std::fs::read_to_string(home.join("config.toml")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return preinstalled,
        Err(_) => return false,
    };
    let _guard = AbsolutePathBufGuard::new(home);
    let Ok(config) = toml::from_str::<ConfigToml>(&contents) else {
        return false;
    };
    config
        .desktop
        .as_ref()
        .and_then(|desktop| desktop.get("uninstall_codex_home"))
        .map_or(preinstalled, |value| value.as_bool().unwrap_or(false))
}

#[cfg(test)]
#[path = "uninstall_policy_tests.rs"]
mod tests;
