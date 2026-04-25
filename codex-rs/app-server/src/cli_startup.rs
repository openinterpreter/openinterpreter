use anyhow::Result;
use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_core::config_loader::LoaderOverrides;
use codex_protocol::protocol::SessionSource;
use codex_utils_cli::CliConfigOverrides;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::AppServerTransport;
use crate::AppServerWebsocketAuthArgs;
use crate::RunMainOptions;
use crate::run_main_with_transport;

// Debug-only test hook: lets integration tests point the server at a temporary
// managed config file without writing to /etc.
const MANAGED_CONFIG_PATH_ENV_VAR: &str = "CODEX_APP_SERVER_MANAGED_CONFIG_PATH";

#[derive(Debug, Parser)]
struct AppServerArgs {
    /// Transport endpoint URL. Supported values: `stdio://` (default),
    /// `ws://IP:PORT`.
    #[arg(
        long = "listen",
        value_name = "URL",
        default_value = AppServerTransport::DEFAULT_LISTEN_URL
    )]
    listen: AppServerTransport,

    /// Session source used to derive product restrictions and metadata.
    #[arg(
        long = "session-source",
        value_name = "SOURCE",
        default_value = "vscode",
        value_parser = SessionSource::from_startup_arg
    )]
    session_source: SessionSource,

    #[command(flatten)]
    auth: AppServerWebsocketAuthArgs,

    #[command(flatten)]
    config_overrides: CliConfigOverrides,

    /// Exit after this many idle seconds once the last websocket client disconnects.
    #[arg(long = "shutdown-idle-timeout-seconds", value_name = "SECONDS")]
    shutdown_idle_timeout_seconds: Option<u64>,
}

pub async fn run_main_from_cli_args<I, T>(arg0_paths: Arg0DispatchPaths, cli_args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = AppServerArgs::parse_from(cli_args);
    let managed_config_path = managed_config_path_from_debug_env();
    let loader_overrides = LoaderOverrides {
        managed_config_path,
        ..Default::default()
    };
    let auth = args.auth.try_into_settings()?;

    run_main_with_transport(
        arg0_paths,
        args.config_overrides,
        loader_overrides,
        RunMainOptions {
            default_analytics_enabled: false,
            shutdown_idle_timeout: args.shutdown_idle_timeout_seconds.map(Duration::from_secs),
            transport: args.listen,
            session_source: args.session_source,
            ..RunMainOptions::default()
        },
        auth,
    )
    .await?;
    Ok(())
}

fn managed_config_path_from_debug_env() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var(MANAGED_CONFIG_PATH_ENV_VAR) {
            return if value.is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            };
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::net::SocketAddr;

    #[test]
    fn app_server_args_default_to_stdio_and_vscode() {
        let args = AppServerArgs::parse_from(["codex-app-server"]);

        assert_eq!(args.listen, AppServerTransport::Stdio);
        assert_eq!(args.session_source, SessionSource::VSCode);
        assert_eq!(args.shutdown_idle_timeout_seconds, None);
    }

    #[test]
    fn app_server_args_parse_websocket_cli_launch_options() {
        let args = AppServerArgs::parse_from([
            "codex-app-server",
            "--listen",
            "ws://127.0.0.1:8123",
            "--session-source",
            "cli",
            "--shutdown-idle-timeout-seconds",
            "7",
        ]);

        assert_eq!(
            args.listen,
            AppServerTransport::WebSocket {
                bind_address: "127.0.0.1:8123"
                    .parse::<SocketAddr>()
                    .expect("parse websocket socket addr"),
            }
        );
        assert_eq!(args.session_source, SessionSource::Cli);
        assert_eq!(args.shutdown_idle_timeout_seconds, Some(7));
    }
}
