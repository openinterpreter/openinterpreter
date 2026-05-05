use std::path::PathBuf;

use anyhow::Context;
use std::io::IsTerminal;
use std::io::Write;

pub use codex_app_server_launcher::LocalAppServerStatus;
pub use codex_app_server_launcher::StopLocalAppServerOutcome;

pub async fn ensure_local_app_server_url(
    app_server_bin: Option<PathBuf>,
    cli_overrides: Vec<String>,
) -> anyhow::Result<String> {
    let codex_home = crate::home::current_interpreter_home()
        .map_err(anyhow::Error::from)
        .context("failed to resolve Open Interpreter home")?;
    codex_app_server_launcher::ensure_local_app_server_url(
        &codex_home,
        app_server_bin,
        cli_overrides,
    )
    .await
}

pub async fn ensure_local_app_server_url_with_startup_message(
    app_server_bin: Option<PathBuf>,
    cli_overrides: Vec<String>,
) -> anyhow::Result<String> {
    let show_starting_message = std::io::stderr().is_terminal();
    let startup_message = match local_app_server_status().await {
        Ok(None) => "Starting Open Interpreter daemon. This only happens once...",
        Ok(Some(_)) => "Connecting to Open Interpreter daemon...",
        Err(_) => "Starting Open Interpreter daemon...",
    };
    if show_starting_message {
        eprint!("\r{startup_message}");
        let _ = std::io::stderr().flush();
    }

    ensure_local_app_server_url(app_server_bin, cli_overrides).await
}

pub async fn local_app_server_status() -> anyhow::Result<Option<LocalAppServerStatus>> {
    let codex_home = crate::home::current_interpreter_home()
        .map_err(anyhow::Error::from)
        .context("failed to resolve Open Interpreter home")?;
    codex_app_server_launcher::local_app_server_status(&codex_home).await
}

pub async fn stop_local_app_server() -> anyhow::Result<StopLocalAppServerOutcome> {
    let codex_home = crate::home::current_interpreter_home()
        .map_err(anyhow::Error::from)
        .context("failed to resolve Open Interpreter home")?;
    codex_app_server_launcher::stop_local_app_server(&codex_home).await
}
