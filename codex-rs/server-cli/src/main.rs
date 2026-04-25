mod cli_common;
mod daemon;
mod home;
mod startup_trace;
mod system_import;

use codex_arg0::arg0_dispatch_or_else_current_thread;
use codex_login::KIMI_CODE_PROVIDER_ID;
use startup_trace::record_startup_trace_event;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::io::Write;
use std::path::PathBuf;

const INTERPRETER_CLI_BINARY: &str = if cfg!(windows) {
    "interpreter-tui.exe"
} else {
    "interpreter-tui"
};

const INTERPRETER_ROOT_TUI_BINARY: &str = if cfg!(windows) {
    "interpreter-root-tui.exe"
} else {
    "interpreter-root-tui"
};

fn main() -> anyhow::Result<()> {
    record_startup_trace_event("interpreter.main.enter");
    home::ensure_interpreter_home_env()?;
    record_startup_trace_event("interpreter.main.home.ready");

    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if should_delegate_directly(&raw_args) {
        return exec_interpreter_cli(raw_args);
    }

    if let Some(command) = scan_top_level_command(&raw_args) {
        return match command {
            TopLevelCommand::Passthrough => exec_interpreter_cli(raw_args),
            TopLevelCommand::Kill {
                force,
                remote_present,
                remote_auth_token_present,
            } => {
                let launch = crate::cli_common::LaunchOptions {
                    remote: remote_present.then_some(String::new()),
                    remote_auth_token_env: remote_auth_token_present.then_some(String::new()),
                    app_server_bin: None,
                };
                ensure_daemon_command_uses_local_daemon(&launch)?;
                arg0_dispatch_or_else_current_thread(|_| async move { kill_daemon(force).await })
            }
            TopLevelCommand::ProviderAuth { provider_id } => {
                arg0_dispatch_or_else_current_thread(|_| async move {
                    print_provider_auth_token(provider_id).await
                })
            }
        };
    }

    exec_interpreter_root_tui(raw_args)
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum TopLevelCommand {
    Passthrough,
    Kill {
        force: bool,
        remote_present: bool,
        remote_auth_token_present: bool,
    },
    ProviderAuth {
        provider_id: String,
    },
}

fn scan_top_level_command(raw_args: &[OsString]) -> Option<TopLevelCommand> {
    let mut remote_present = false;
    let mut remote_auth_token_present = false;
    let mut index = 0usize;
    while index < raw_args.len() {
        let arg = raw_args[index].to_string_lossy();
        if arg == "--" {
            return None;
        }

        if matches!(
            arg.as_ref(),
            "-c" | "--config"
                | "--enable"
                | "--disable"
                | "--remote"
                | "--url"
                | "--remote-auth-token-env"
                | "--app-server-bin"
                | "--image"
                | "-i"
                | "--model"
                | "-m"
                | "--local-provider"
                | "--profile"
                | "-p"
                | "--sandbox"
                | "-s"
                | "--ask-for-approval"
                | "-a"
                | "--cd"
                | "-C"
                | "--add-dir"
        ) {
            if matches!(arg.as_ref(), "--remote" | "--url") {
                remote_present = true;
            }
            if arg == "--remote-auth-token-env" {
                remote_auth_token_present = true;
            }
            index += 2;
            continue;
        }

        if arg.starts_with("--remote=") || arg.starts_with("--url=") {
            remote_present = true;
            index += 1;
            continue;
        }
        if arg.starts_with("--remote-auth-token-env=") {
            remote_auth_token_present = true;
            index += 1;
            continue;
        }
        if arg.starts_with("--")
            || matches!(
                arg.as_ref(),
                "--oss"
                    | "--alt-screen"
                    | "--search"
                    | "--no-alt-screen"
                    | "--full-auto"
                    | "--dangerously-bypass-approvals-and-sandbox"
                    | "--yolo"
            )
        {
            index += 1;
            continue;
        }

        return match arg.as_ref() {
            "resume" | "fork" | "exec" => Some(TopLevelCommand::Passthrough),
            "kill" => Some(TopLevelCommand::Kill {
                force: raw_args[index + 1..]
                    .iter()
                    .map(|arg| arg.to_string_lossy())
                    .any(|arg| matches!(arg.as_ref(), "-f" | "--force")),
                remote_present,
                remote_auth_token_present,
            }),
            "provider-auth" => {
                raw_args
                    .get(index + 1)
                    .map(|provider_id| TopLevelCommand::ProviderAuth {
                        provider_id: provider_id.to_string_lossy().to_string(),
                    })
            }
            _ => None,
        };
    }

    None
}

fn should_delegate_directly(raw_args: &[OsString]) -> bool {
    raw_args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .any(|arg| matches!(arg.as_ref(), "-h" | "--help" | "-V" | "--version"))
}

fn resolve_interpreter_cli_binary() -> anyhow::Result<PathBuf> {
    resolve_binary(INTERPRETER_CLI_BINARY)
}

fn resolve_interpreter_root_tui_binary() -> anyhow::Result<PathBuf> {
    resolve_binary(INTERPRETER_ROOT_TUI_BINARY)
}

fn resolve_binary(binary_name: &str) -> anyhow::Result<PathBuf> {
    let current_exe = std::env::current_exe()?;
    let sibling = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("interpreter path missing parent directory"))?
        .join(binary_name);
    if sibling.exists() {
        return Ok(sibling);
    }

    which::which(binary_name).map_err(anyhow::Error::from)
}

fn exec_interpreter_cli(raw_args: Vec<OsString>) -> anyhow::Result<()> {
    exec_binary(resolve_interpreter_cli_binary()?, raw_args)
}

fn exec_interpreter_root_tui(raw_args: Vec<OsString>) -> anyhow::Result<()> {
    exec_binary(resolve_interpreter_root_tui_binary()?, raw_args)
}

fn exec_binary(program: PathBuf, raw_args: Vec<OsString>) -> anyhow::Result<()> {
    let mut command = std::process::Command::new(&program);
    command.args(raw_args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let err = command.exec();
        Err(anyhow::Error::from(err))
    }

    #[cfg(not(unix))]
    {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn ensure_daemon_command_uses_local_daemon(
    launch: &crate::cli_common::LaunchOptions,
) -> anyhow::Result<()> {
    if launch.remote.is_some() || launch.remote_auth_token_env.is_some() {
        anyhow::bail!("daemon commands only manage the local Open Interpreter daemon");
    }
    Ok(())
}

async fn print_provider_auth_token(provider_id: String) -> anyhow::Result<()> {
    let interpreter_home = crate::home::current_interpreter_home()?;
    match provider_id.as_str() {
        KIMI_CODE_PROVIDER_ID => {
            let access_token = codex_login::kimi_code::ensure_access_token(
                &interpreter_home,
                /*open_browser*/ true,
            )
            .await
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
            print!("{access_token}");
            Ok(())
        }
        _ => anyhow::bail!("unsupported provider auth command for `{provider_id}`"),
    }
}

async fn kill_daemon(force: bool) -> anyhow::Result<()> {
    let status = daemon::local_app_server_status().await?;
    let Some(_status) = status else {
        println!("Open Interpreter daemon is not running.");
        return Ok(());
    };

    if !force && !confirm_daemon_stop()? {
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
