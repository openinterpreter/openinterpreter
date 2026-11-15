//! Verifies local configuration precedence over the preinstalled cleanup default.

use anyhow::Result;
use pretty_assertions::assert_eq;

use super::clean_uninstall_enabled;

/// Existing preinstalls keep clean uninstall when no user preference exists.
#[test]
fn missing_preference_uses_preinstall_default() -> Result<()> {
    let home = tempfile::tempdir()?;
    for preinstalled in [false, true] {
        assert_eq!(
            clean_uninstall_enabled(home.path(), preinstalled),
            preinstalled
        );
    }
    for contents in ["", "log_dir = 'logs'", "[desktop]\ntheme = 'dark'"] {
        std::fs::write(home.path().join("config.toml"), contents)?;
        for preinstalled in [false, true] {
            assert_eq!(
                clean_uninstall_enabled(home.path(), preinstalled),
                preinstalled
            );
        }
    }
    Ok(())
}

/// A user's explicit choice overrides both ordinary and preinstalled defaults.
#[test]
fn explicit_preference_overrides_preinstall_default() -> Result<()> {
    let home = tempfile::tempdir()?;
    for enabled in [false, true] {
        std::fs::write(
            home.path().join("config.toml"),
            format!("log_dir = 'logs'\n[desktop]\nuninstall_codex_home = {enabled}"),
        )?;
        for preinstalled in [false, true] {
            assert_eq!(clean_uninstall_enabled(home.path(), preinstalled), enabled);
        }
    }
    Ok(())
}

/// Invalid configuration never falls back to destructive cleanup, even on a preload.
#[test]
fn malformed_or_invalid_preference_preserves_home() -> Result<()> {
    let home = tempfile::tempdir()?;
    for contents in [
        "[desktop",
        "desktop = false",
        "[desktop]\nuninstall_codex_home = 'true'",
        "[desktop]\nuninstall_codex_home = 1",
        "[desktop]\nuninstall_codex_home = []",
    ] {
        std::fs::write(home.path().join("config.toml"), contents)?;
        for preinstalled in [false, true] {
            assert!(!clean_uninstall_enabled(home.path(), preinstalled));
        }
    }
    Ok(())
}

/// Failure to read an existing config must not restore the preinstalled deletion default.
#[test]
fn unreadable_config_preserves_home() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::create_dir(home.path().join("config.toml"))?;
    for preinstalled in [false, true] {
        assert!(!clean_uninstall_enabled(home.path(), preinstalled));
    }
    Ok(())
}
