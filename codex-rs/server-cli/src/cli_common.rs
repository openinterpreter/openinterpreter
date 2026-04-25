use anyhow::bail;
use clap::ArgAction;
use clap::Args;
use clap::Parser;
use clap::ValueHint;
use codex_utils_cli::CliConfigOverrides;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone, Default)]
pub struct FeatureToggles {
    /// Enable a feature. Equivalent to `-c features.<name>=true`.
    #[arg(long = "enable", value_name = "FEATURE", action = ArgAction::Append, global = true)]
    pub enable: Vec<String>,

    /// Disable a feature. Equivalent to `-c features.<name>=false`.
    #[arg(long = "disable", value_name = "FEATURE", action = ArgAction::Append, global = true)]
    pub disable: Vec<String>,
}

impl FeatureToggles {
    pub fn into_overrides(self) -> Vec<String> {
        let mut overrides = Vec::with_capacity(self.enable.len() + self.disable.len());
        overrides.extend(
            self.enable
                .into_iter()
                .map(|feature| format!("features.{feature}=true")),
        );
        overrides.extend(
            self.disable
                .into_iter()
                .map(|feature| format!("features.{feature}=false")),
        );
        overrides
    }
}

#[derive(Debug, Args, Clone, Copy, Default, Eq, PartialEq)]
pub struct AltScreenCli {
    /// Use fullscreen alternate-screen mode instead of Open Interpreter's inline default.
    #[arg(long = "alt-screen", default_value_t = false, global = true)]
    pub alt_screen: bool,
}

pub fn apply_interpreter_alt_screen_default(
    no_alt_screen: &mut bool,
    alt_screen: AltScreenCli,
) -> anyhow::Result<()> {
    if alt_screen.alt_screen && *no_alt_screen {
        bail!("`--alt-screen` conflicts with `--no-alt-screen`");
    }

    *no_alt_screen = !alt_screen.alt_screen;

    Ok(())
}

#[derive(Debug, Args, Clone, Copy, Default, Eq, PartialEq)]
pub struct KillCommand {
    /// Stop the daemon without prompting even if it may disconnect active sessions.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Parser, Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Connect to a remote app server websocket endpoint.
    #[arg(long = "remote", alias = "url", value_name = "ADDR")]
    pub remote: Option<String>,

    /// Name of the environment variable containing the bearer token to send to
    /// a remote app server websocket.
    #[arg(long = "remote-auth-token-env", value_name = "ENV_VAR")]
    pub remote_auth_token_env: Option<String>,

    /// Path to the local app-server binary to spawn when `--remote` is not used.
    #[arg(
        long,
        hide = true,
        value_name = "PATH",
        value_hint = ValueHint::ExecutablePath
    )]
    pub app_server_bin: Option<PathBuf>,
}

impl LaunchOptions {
    pub fn merged_with(self, override_options: LaunchOptions) -> LaunchOptions {
        LaunchOptions {
            remote: override_options.remote.or(self.remote),
            remote_auth_token_env: override_options
                .remote_auth_token_env
                .or(self.remote_auth_token_env),
            app_server_bin: override_options.app_server_bin.or(self.app_server_bin),
        }
    }
}

const DAEMON_STARTUP_OVERRIDE_KEYS: &[&str] = &[
    "features.apps",
    "features.plugins",
    "features.default_mode_request_user_input",
];
const DEFAULT_MODE_REQUEST_USER_INPUT_OVERRIDE: &str =
    "features.default_mode_request_user_input=true";

pub fn daemon_startup_overrides(config_overrides: &CliConfigOverrides) -> Vec<String> {
    config_overrides
        .raw_overrides
        .iter()
        .filter(|override_entry| {
            DAEMON_STARTUP_OVERRIDE_KEYS
                .iter()
                .any(|key| override_entry_key(override_entry) == Some(*key))
        })
        .cloned()
        .collect()
}

pub fn apply_interpreter_feature_defaults(config_overrides: &mut CliConfigOverrides) {
    if config_overrides.raw_overrides.iter().all(|override_entry| {
        override_entry_key(override_entry) != Some("features.default_mode_request_user_input")
    }) {
        config_overrides
            .raw_overrides
            .push(DEFAULT_MODE_REQUEST_USER_INPUT_OVERRIDE.to_string());
    }
}

fn override_entry_key(override_entry: &str) -> Option<&str> {
    Some(
        override_entry
            .split_once('=')
            .map_or(override_entry, |(path, _)| path)
            .trim(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn alt_screen_defaults_interpreter_to_inline_mode() {
        let mut no_alt_screen = false;

        apply_interpreter_alt_screen_default(&mut no_alt_screen, AltScreenCli::default())
            .expect("apply default alt-screen mode");

        assert_eq!(no_alt_screen, true);
    }

    #[test]
    fn alt_screen_flag_restores_fullscreen_mode() {
        let mut no_alt_screen = false;

        apply_interpreter_alt_screen_default(&mut no_alt_screen, AltScreenCli { alt_screen: true })
            .expect("apply alt-screen override");

        assert_eq!(no_alt_screen, false);
    }

    #[test]
    fn conflicting_alt_screen_flags_error() {
        let mut no_alt_screen = true;

        let err = apply_interpreter_alt_screen_default(
            &mut no_alt_screen,
            AltScreenCli { alt_screen: true },
        )
        .expect_err("conflicting flags should fail");

        assert_eq!(
            err.to_string(),
            "`--alt-screen` conflicts with `--no-alt-screen`"
        );
    }

    #[test]
    fn interpreter_enables_request_user_input_by_default() {
        let mut config_overrides = CliConfigOverrides::default();

        apply_interpreter_feature_defaults(&mut config_overrides);

        assert_eq!(
            config_overrides.raw_overrides,
            vec![DEFAULT_MODE_REQUEST_USER_INPUT_OVERRIDE.to_string()]
        );
    }

    #[test]
    fn explicit_request_user_input_override_is_preserved() {
        let mut config_overrides = CliConfigOverrides {
            raw_overrides: vec!["features.default_mode_request_user_input=false".to_string()],
        };

        apply_interpreter_feature_defaults(&mut config_overrides);

        assert_eq!(
            config_overrides.raw_overrides,
            vec!["features.default_mode_request_user_input=false".to_string()]
        );
    }
}
