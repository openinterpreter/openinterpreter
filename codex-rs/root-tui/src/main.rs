use anyhow::Context;
use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else_current_thread;
use codex_tui::AppExitInfo;
use codex_tui::ExitReason;
use codex_utils_cli::CliConfigOverrides;
use std::io::IsTerminal;
use std::io::Write;

#[path = "../../server-cli/src/cli_common.rs"]
mod cli_common;
#[path = "../../server-cli/src/home.rs"]
mod home;
#[path = "../../server-cli/src/startup_preview.rs"]
mod startup_preview;
#[path = "../../server-cli/src/startup_trace.rs"]
mod startup_trace;
#[path = "../../server-cli/src/system_import.rs"]
mod system_import;

use cli_common::AltScreenCli;
use cli_common::FeatureToggles;
use cli_common::LaunchOptions;
use cli_common::apply_interpreter_alt_screen_default;
use cli_common::apply_interpreter_feature_defaults;
use cli_common::daemon_startup_overrides;
use home::ensure_interpreter_home_env;
use startup_preview::StartupModelPreview;
use startup_trace::record_startup_trace_event;

#[derive(Parser, Debug)]
struct RootTuiCli {
    #[command(flatten)]
    config_overrides: CliConfigOverrides,

    #[command(flatten)]
    feature_toggles: FeatureToggles,

    #[command(flatten)]
    launch: LaunchOptions,

    #[command(flatten)]
    alt_screen: AltScreenCli,

    #[command(flatten)]
    interactive: codex_tui::Cli,
}

fn main() -> anyhow::Result<()> {
    record_startup_trace_event("interpreter.main.enter");
    ensure_interpreter_home_env()?;
    record_startup_trace_event("interpreter.main.home.ready");
    arg0_dispatch_or_else_current_thread(|arg0_paths: Arg0DispatchPaths| async move {
        let RootTuiCli {
            config_overrides,
            feature_toggles,
            launch,
            alt_screen,
            mut interactive,
        } = RootTuiCli::parse();
        interactive.config_overrides = config_overrides;
        interactive
            .config_overrides
            .raw_overrides
            .extend(feature_toggles.into_overrides());
        apply_interpreter_feature_defaults(&mut interactive.config_overrides);
        apply_interpreter_alt_screen_default(&mut interactive.no_alt_screen, alt_screen)?;
        record_startup_trace_event("interpreter.main.cli.parsed");
        run_root_tui(launch, interactive, arg0_paths).await
    })
}

async fn run_root_tui(
    launch: LaunchOptions,
    mut interactive: codex_tui::Cli,
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<()> {
    let startup_model_display = {
        let startup_preview = StartupModelPreview::resolve(
            interactive.model.as_deref(),
            interactive.config_profile.as_deref(),
        );
        (startup_preview.model_display != "default").then_some(startup_preview.model_display)
    };

    if let Some(prompt) = interactive.prompt.take() {
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    let exit_info = if let Some(remote) = launch.remote.as_deref() {
        let remote = codex_tui::normalize_remote_addr(remote)
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        let remote_auth_token = launch
            .remote_auth_token_env
            .as_deref()
            .map(read_remote_auth_token_from_env_var)
            .transpose()?;
        record_startup_trace_event("interpreter.remote.selected");
        record_startup_trace_event("interpreter.tui.delegate.enter");
        codex_tui::run_main_with_default_loader_overrides(
            interactive,
            arg0_paths,
            Some(remote),
            remote_auth_token,
        )
        .await?
    } else {
        if launch.remote_auth_token_env.is_some() {
            anyhow::bail!("`--remote-auth-token-env` requires `--remote`.");
        }
        let daemon_cli_overrides = daemon_startup_overrides(&interactive.config_overrides);
        print_root_tui_startup_message();
        record_startup_trace_event("interpreter.tui.delegate.enter");
        codex_tui::run_main_with_deferred_remote(
            interactive,
            arg0_paths,
            startup_model_display,
            /*startup_requires_provider_setup_override*/ None,
            move || {
                let app_server_bin = launch.app_server_bin.clone();
                let daemon_cli_overrides = daemon_cli_overrides.clone();
                async move {
                    let codex_home = home::current_interpreter_home()?;
                    codex_app_server_launcher::ensure_local_app_server_url(
                        &codex_home,
                        app_server_bin,
                        daemon_cli_overrides,
                    )
                    .await
                    .map_err(|err| std::io::Error::other(err.to_string()))
                }
            },
        )
        .await?
    };
    handle_app_exit(exit_info)
}

fn print_root_tui_startup_message() {
    if std::io::stderr().is_terminal()
        && std::env::var_os("OPEN_INTERPRETER_STARTUP_MESSAGE_SHOWN").is_none()
    {
        eprint!("\rStarting Open Interpreter daemon. This only happens once...");
        let _ = std::io::stderr().flush();
    }
}

fn handle_app_exit(exit_info: AppExitInfo) -> anyhow::Result<()> {
    match exit_info.exit_reason {
        ExitReason::UserRequested => Ok(()),
        ExitReason::Fatal(message) => anyhow::bail!("{message}"),
    }
}

fn read_remote_auth_token_from_env_var(env_var_name: &str) -> anyhow::Result<String> {
    let token = std::env::var(env_var_name).with_context(|| {
        format!("failed to read remote auth token from environment variable `{env_var_name}`")
    })?;
    if token.trim().is_empty() {
        anyhow::bail!("environment variable `{env_var_name}` contained an empty auth token");
    }
    Ok(token)
}
