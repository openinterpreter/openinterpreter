use anyhow::Context;
use clap::Parser;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else_current_thread;
use codex_server_cli::cli_common::AltScreenCli;
use codex_server_cli::cli_common::FeatureToggles;
use codex_server_cli::cli_common::LaunchOptions;
use codex_server_cli::cli_common::apply_interpreter_alt_screen_default;
use codex_server_cli::cli_common::apply_interpreter_feature_defaults;
use codex_server_cli::cli_common::daemon_startup_overrides;
use codex_server_cli::daemon;
use codex_server_cli::home::ensure_interpreter_home_env;
use codex_server_cli::startup_preview::StartupModelPreview;
use codex_server_cli::startup_trace::record_startup_trace_event;
use codex_tui::AppExitInfo;
use codex_tui::ExitReason;
use codex_utils_cli::CliConfigOverrides;

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
                    daemon::ensure_local_app_server_url(app_server_bin, daemon_cli_overrides)
                        .await
                        .map_err(|err| std::io::Error::other(err.to_string()))
                }
            },
        )
        .await?
    };
    handle_app_exit(exit_info)
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
