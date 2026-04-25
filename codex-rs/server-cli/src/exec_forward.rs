use crate::cli::ExecCommand;
use crate::cli::LaunchOptions;
use crate::daemon;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use codex_arg0::Arg0DispatchPaths;
use codex_utils_cli::CliConfigOverrides;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::Stdio;

const OPEN_INTERPRETER_EXEC_BIN_ENV_VAR: &str = "OPEN_INTERPRETER_EXEC_BIN";
const OPEN_INTERPRETER_EXEC_TRACE_PATH_ENV_VAR: &str = "OPEN_INTERPRETER_EXEC_TRACE_PATH";

pub async fn run_exec_subcommand(
    exec: ExecCommand,
    launch: LaunchOptions,
    root_config_overrides: CliConfigOverrides,
    daemon_cli_overrides: Vec<String>,
    arg0_paths: &Arg0DispatchPaths,
) -> Result<ExitStatus> {
    if launch.remote_auth_token_env.is_some() && launch.remote.is_none() {
        bail!("`--remote-auth-token-env` requires `--remote`.");
    }

    let remote = if let Some(remote) = launch.remote.as_deref() {
        codex_tui::normalize_remote_addr(remote).map_err(|err| anyhow::anyhow!(err.to_string()))?
    } else {
        daemon::ensure_local_app_server_url(launch.app_server_bin, daemon_cli_overrides).await?
    };

    let exec_binary = resolve_exec_binary(arg0_paths)?;
    let codex_home = crate::home::current_interpreter_home()
        .context("failed to resolve Open Interpreter home")?;
    let mut command = std::process::Command::new(&exec_binary);
    command
        .env("CODEX_HOME", &codex_home)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .arg("--remote")
        .arg(&remote);
    if let Some(interpreter_home) = std::env::var_os(crate::home::INTERPRETER_HOME_ENV_VAR) {
        command.env(crate::home::INTERPRETER_HOME_ENV_VAR, interpreter_home);
    }
    if let Some(open_interpreter_home) =
        std::env::var_os(crate::home::OPEN_INTERPRETER_HOME_ENV_VAR)
    {
        command.env(
            crate::home::OPEN_INTERPRETER_HOME_ENV_VAR,
            open_interpreter_home,
        );
    }

    if let Some(env_name) = launch.remote_auth_token_env {
        command.arg("--remote-auth-token-env").arg(env_name);
    }

    for override_arg in root_config_overrides.raw_overrides {
        command.arg("-c").arg(override_arg);
    }
    command.args(exec.args);

    if let Some(trace_path) = std::env::var_os(OPEN_INTERPRETER_EXEC_TRACE_PATH_ENV_VAR) {
        let mut trace = format!(
            "exec_binary={}\ncodex_home={}\nremote={remote}\n",
            exec_binary.display(),
            codex_home.display()
        );
        let arguments: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        trace.push_str(&format!("args={arguments:?}\n"));
        let _ = std::fs::write(trace_path, trace);
    }

    command
        .status()
        .with_context(|| format!("failed to launch {}", exec_binary.display()))
}

fn resolve_exec_binary(arg0_paths: &Arg0DispatchPaths) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(OPEN_INTERPRETER_EXEC_BIN_ENV_VAR)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return Ok(path);
    }

    if let Some(program) = sibling_exec_binary(arg0_paths)? {
        return Ok(program);
    }

    if let Ok(program) = which::which(exec_binary_name()) {
        return Ok(program);
    }

    bail!(
        "could not find a local `{}` binary; build it or install it alongside `interpreter`",
        exec_binary_name()
    )
}

fn sibling_exec_binary(arg0_paths: &Arg0DispatchPaths) -> Result<Option<PathBuf>> {
    let current_exe = arg0_paths
        .codex_self_exe
        .clone()
        .or_else(|| std::env::current_exe().ok())
        .context("failed to resolve interpreter path")?;
    let Some(bin_dir) = current_exe.parent() else {
        return Ok(None);
    };

    for candidate_dir in std::iter::once(bin_dir).chain(bin_dir.parent()) {
        let candidate = candidate_dir.join(exec_binary_name());
        if candidate.exists() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn exec_binary_name() -> &'static str {
    #[cfg(windows)]
    {
        "codex-exec.exe"
    }

    #[cfg(not(windows))]
    {
        "codex-exec"
    }
}
