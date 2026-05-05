use crate::cli::AltScreenCli;
use crate::cli::ForkCommand;
use crate::cli::KillCommand;
use crate::cli::LaunchOptions;
use crate::cli::ResumeCommand;
use crate::cli::ServerCli;
use crate::cli::Subcommand;
use crate::cli::apply_root_overrides;
use crate::cli::daemon_startup_overrides;
use crate::cli::finalize_fork_interactive;
use crate::cli::finalize_resume_interactive;
use crate::cli_common::apply_interpreter_alt_screen_default;
use crate::cli_common::apply_interpreter_feature_defaults;
use crate::daemon;
use crate::exec_forward;
use crate::startup_preview::StartupModelPreview;
use crate::startup_trace::record_startup_trace_event;
use anyhow::Context;
use codex_arg0::Arg0DispatchPaths;
use codex_tui::AppExitInfo;
use codex_tui::ExitReason;
use std::io::IsTerminal;
use std::io::Write;

pub async fn run_main(cli: ServerCli, arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    record_startup_trace_event("interpreter.run_main.enter");
    let ServerCli {
        config_overrides,
        feature_toggles,
        launch,
        alt_screen,
        interactive,
        subcommand,
    } = cli;
    let mut root_config_overrides = config_overrides;
    root_config_overrides
        .raw_overrides
        .extend(feature_toggles.into_overrides());
    apply_interpreter_feature_defaults(&mut root_config_overrides);
    let daemon_cli_overrides = daemon_startup_overrides(&root_config_overrides);

    match subcommand {
        None => handle_app_exit(
            run_tui(
                launch,
                apply_default_alt_screen(
                    apply_root_overrides(interactive, root_config_overrides),
                    alt_screen,
                )?,
                arg0_paths,
                daemon_cli_overrides,
            )
            .await?,
        ),
        Some(Subcommand::Resume(resume)) => {
            let ResumeCommand {
                session_id,
                last,
                all,
                include_non_interactive,
                launch: resume_launch,
                interactive: resume_interactive,
            } = resume;
            let interactive = apply_default_alt_screen(
                finalize_resume_interactive(
                    interactive,
                    root_config_overrides,
                    session_id,
                    last,
                    all,
                    include_non_interactive,
                    resume_interactive,
                ),
                alt_screen,
            )?;
            let launch = launch.merged_with(resume_launch);
            handle_app_exit(run_tui(launch, interactive, arg0_paths, daemon_cli_overrides).await?)
        }
        Some(Subcommand::Fork(fork)) => {
            let ForkCommand {
                session_id,
                last,
                all,
                launch: fork_launch,
                interactive: fork_interactive,
            } = fork;
            let interactive = apply_default_alt_screen(
                finalize_fork_interactive(
                    interactive,
                    root_config_overrides,
                    session_id,
                    last,
                    all,
                    fork_interactive,
                ),
                alt_screen,
            )?;
            let launch = launch.merged_with(fork_launch);
            handle_app_exit(run_tui(launch, interactive, arg0_paths, daemon_cli_overrides).await?)
        }
        Some(Subcommand::Exec(exec)) => {
            let status = exec_forward::run_exec_subcommand(
                exec,
                launch,
                root_config_overrides,
                daemon_cli_overrides,
                &arg0_paths,
            )
            .await?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Some(Subcommand::Kill(kill)) => {
            ensure_daemon_command_uses_local_daemon(&launch)?;
            kill_daemon(kill).await
        }
    }
}

fn apply_default_alt_screen(
    mut interactive: codex_tui::Cli,
    alt_screen: AltScreenCli,
) -> anyhow::Result<codex_tui::Cli> {
    apply_interpreter_alt_screen_default(&mut interactive.no_alt_screen, alt_screen)?;
    Ok(interactive)
}

fn handle_app_exit(exit_info: AppExitInfo) -> anyhow::Result<()> {
    match exit_info.exit_reason {
        ExitReason::UserRequested => Ok(()),
        ExitReason::Fatal(message) => anyhow::bail!("{message}"),
    }
}

async fn run_tui(
    launch: LaunchOptions,
    mut interactive: codex_tui::Cli,
    arg0_paths: Arg0DispatchPaths,
    daemon_cli_overrides: Vec<String>,
) -> anyhow::Result<AppExitInfo> {
    if let Some(prompt) = interactive.prompt.take() {
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    let remote = if let Some(remote) = launch.remote.as_deref() {
        record_startup_trace_event("interpreter.remote.selected");
        Some(
            codex_tui::normalize_remote_addr(remote)
                .map_err(|err| anyhow::anyhow!(err.to_string()))?,
        )
    } else {
        None
    };
    let remote_auth_token = launch
        .remote_auth_token_env
        .as_deref()
        .map(read_remote_auth_token_from_env_var)
        .transpose()?;
    if remote_auth_token.is_some() && launch.remote.is_none() {
        anyhow::bail!("`--remote-auth-token-env` requires `--remote`.");
    }

    if let Some(remote) = remote {
        record_startup_trace_event("interpreter.tui.delegate.enter");
        return codex_tui::run_main_with_default_loader_overrides(
            interactive,
            arg0_paths,
            Some(remote),
            remote_auth_token,
        )
        .await
        .map_err(anyhow::Error::from);
    }

    let startup_model_display = {
        let startup_preview = StartupModelPreview::resolve(
            interactive.model.as_deref(),
            interactive.config_profile.as_deref(),
        );
        (startup_preview.model_display != "default").then_some(startup_preview.model_display)
    };
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
                daemon::ensure_local_app_server_url_with_startup_message(
                    app_server_bin,
                    daemon_cli_overrides,
                )
                .await
                .map_err(|err| std::io::Error::other(err.to_string()))
            }
        },
    )
    .await
    .map_err(anyhow::Error::from)
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

fn ensure_daemon_command_uses_local_daemon(launch: &LaunchOptions) -> anyhow::Result<()> {
    if launch.remote.is_some() || launch.remote_auth_token_env.is_some() {
        anyhow::bail!("daemon commands only manage the local Open Interpreter daemon");
    }
    Ok(())
}

async fn kill_daemon(kill: KillCommand) -> anyhow::Result<()> {
    let status = daemon::local_app_server_status().await?;
    let Some(_status) = status else {
        println!("Open Interpreter daemon is not running.");
        return Ok(());
    };

    if !kill.force && !confirm_daemon_stop()? {
        println!("Aborted.");
        return Ok(());
    }

    match daemon::stop_local_app_server().await? {
        daemon::StopLocalAppServerOutcome::NotRunning => {
            println!("Open Interpreter daemon is not running.");
        }
        daemon::StopLocalAppServerOutcome::Stopped(status) => {
            println!("Stopped Open Interpreter daemon (pid {}).", status.pid);
        }
    }
    Ok(())
}

fn confirm_daemon_stop() -> anyhow::Result<bool> {
    let mut stderr = std::io::stderr();
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "daemon is running; rerun with `interpreter kill --force` to stop it non-interactively"
        );
    }

    write!(
        stderr,
        "This will stop the Open Interpreter daemon and disconnect any running sessions. Continue? [y/N] "
    )?;
    stderr.flush()?;

    let mut response = String::new();
    std::io::stdin().read_line(&mut response)?;
    Ok(is_confirmation_response(&response))
}

fn is_confirmation_response(response: &str) -> bool {
    matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn remote_auth_token_reader_rejects_empty_value() {
        unsafe {
            std::env::set_var("CODEX_SERVER_CLI_EMPTY_TOKEN", "");
        }

        let err = read_remote_auth_token_from_env_var("CODEX_SERVER_CLI_EMPTY_TOKEN")
            .expect_err("empty token should fail");
        assert!(err.to_string().contains("contained an empty auth token"));
    }

    #[test]
    fn remote_auth_token_reader_returns_value() {
        unsafe {
            std::env::set_var("CODEX_SERVER_CLI_TOKEN", "abc123");
        }

        let token = read_remote_auth_token_from_env_var("CODEX_SERVER_CLI_TOKEN")
            .expect("non-empty token should parse");
        assert_eq!(token, "abc123");
    }

    #[test]
    fn daemon_commands_reject_remote_options() {
        let err = ensure_daemon_command_uses_local_daemon(&LaunchOptions {
            remote: Some("ws://127.0.0.1:7777".to_string()),
            remote_auth_token_env: None,
            app_server_bin: None,
        })
        .expect_err("remote daemon management should be rejected");

        assert!(err.to_string().contains("local Open Interpreter daemon"));
    }

    #[test]
    fn daemon_commands_allow_default_launch_options() {
        ensure_daemon_command_uses_local_daemon(&LaunchOptions::default())
            .expect("local daemon management should be allowed");
    }

    #[test]
    fn confirm_daemon_stop_accepts_yes_variants() {
        assert!(is_confirmation_response("y"));
        assert!(is_confirmation_response("Y"));
        assert!(is_confirmation_response("yes"));
        assert!(is_confirmation_response("Yes"));
    }

    #[test]
    fn confirm_daemon_stop_rejects_default_and_other_values() {
        assert!(!is_confirmation_response(""));
        assert!(!is_confirmation_response("n"));
        assert!(!is_confirmation_response("no"));
        assert!(!is_confirmation_response("anything else"));
    }
}
