use crate::startup_trace::record_startup_trace_event;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use sysinfo::Pid;
use sysinfo::ProcessesToUpdate;
use sysinfo::Signal;
use sysinfo::System;
use tokio::time::Instant;
use tokio::time::sleep;
use uuid::Uuid;

const SPAWN_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const STALE_SPAWN_LOCK_AGE: Duration = Duration::from_secs(30);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(50);
const HEALTH_REUSE_GRACE_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_WEBSOCKET_HOST: &str = "127.0.0.1";
const MAX_PROCESS_START_TIME_SKEW: Duration = Duration::from_secs(30);
const SHUTDOWN_IDLE_TIMEOUT_SECONDS: u64 = 5;
const STOP_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const INTERPRETER_APP_SERVER_BINARY: &str = if cfg!(windows) {
    "interpreter-app-server.exe"
} else {
    "interpreter-app-server"
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LocalAppServerStatus {
    pub pid: u32,
    pub websocket_url: String,
    pub started_at_unix: i64,
    pub log_path: PathBuf,
    pub health: DaemonHealth,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DaemonHealth {
    Ready,
    Unhealthy,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StopLocalAppServerOutcome {
    NotRunning,
    Stopped(LocalAppServerStatus),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerLockfile {
    pid: u32,
    port: u16,
    nonce: String,
    websocket_url: String,
    started_at_unix: i64,
    server_bin: String,
    server_bin_fingerprint: Option<LaunchArtifactFingerprint>,
}

#[derive(Debug)]
struct DaemonPaths {
    lockfile_path: PathBuf,
    spawn_lock_path: PathBuf,
    log_path: PathBuf,
}

#[derive(Clone, Debug)]
struct LaunchSpec {
    program: PathBuf,
    args: Vec<String>,
    display_name: String,
    artifact_fingerprint: Option<LaunchArtifactFingerprint>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchArtifactFingerprint {
    modified_at_unix_nanos: u128,
    file_len_bytes: u64,
}

#[derive(Debug, Clone)]
enum LockfileReusePolicy {
    MatchingServerBinary {
        server_bin: String,
        artifact_fingerprint: Option<LaunchArtifactFingerprint>,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LockfileReuseDecision {
    Reuse,
    RestartRequired,
    Ignore,
}

impl LockfileReusePolicy {
    fn decision(&self, lockfile: &AppServerLockfile) -> LockfileReuseDecision {
        match self {
            Self::MatchingServerBinary {
                server_bin,
                artifact_fingerprint,
            } => {
                if lockfile.server_bin != *server_bin {
                    return LockfileReuseDecision::Ignore;
                }
                match (artifact_fingerprint, &lockfile.server_bin_fingerprint) {
                    (Some(expected), Some(actual)) if expected == actual => {
                        LockfileReuseDecision::Reuse
                    }
                    (Some(_), _) => LockfileReuseDecision::RestartRequired,
                    (None, _) => LockfileReuseDecision::Reuse,
                }
            }
        }
    }
}

struct SpawnLockGuard {
    _file: File,
    path: PathBuf,
}

impl SpawnLockGuard {
    fn acquire(path: &Path) -> Result<Option<Self>> {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => Ok(Some(Self {
                _file: file,
                path: path.to_path_buf(),
            })),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(err).with_context(|| format!("failed to create {}", path.display())),
        }
    }
}

impl Drop for SpawnLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub async fn ensure_local_app_server_url(
    app_server_bin: Option<PathBuf>,
    cli_overrides: Vec<String>,
) -> Result<String> {
    let codex_home = crate::home::current_interpreter_home()
        .context("failed to resolve Open Interpreter home")?;
    let launch_spec = resolve_launch_spec(app_server_bin)?;
    ensure_app_server_url_with_launch_spec(&codex_home, &launch_spec, cli_overrides).await
}

pub async fn local_app_server_status() -> Result<Option<LocalAppServerStatus>> {
    let codex_home = crate::home::current_interpreter_home()
        .context("failed to resolve Open Interpreter home")?;
    local_app_server_status_for_home(&codex_home).await
}

pub async fn stop_local_app_server() -> Result<StopLocalAppServerOutcome> {
    let codex_home = crate::home::current_interpreter_home()
        .context("failed to resolve Open Interpreter home")?;
    stop_local_app_server_for_home(&codex_home).await
}

async fn ensure_app_server_url_with_launch_spec(
    codex_home: &Path,
    launch_spec: &LaunchSpec,
    cli_overrides: Vec<String>,
) -> Result<String> {
    let paths = daemon_paths_for_home(codex_home);
    ensure_runtime_dirs(&paths)?;
    let reuse_policy = LockfileReusePolicy::MatchingServerBinary {
        server_bin: launch_spec.display_name.clone(),
        artifact_fingerprint: launch_spec.artifact_fingerprint.clone(),
    };

    if let Some(url) = healthy_lockfile_url(&paths, &reuse_policy).await? {
        record_startup_trace_event("interpreter.daemon.reused_existing");
        return Ok(url);
    }

    if let Some(url) = wait_for_existing_spawn(&paths, &reuse_policy).await? {
        record_startup_trace_event("interpreter.daemon.waited_for_existing_spawn");
        return Ok(url);
    }

    let Some(spawn_lock) = SpawnLockGuard::acquire(&paths.spawn_lock_path)? else {
        if let Some(url) = wait_for_existing_spawn(&paths, &reuse_policy).await? {
            return Ok(url);
        }
        bail!("timed out waiting for another interpreter process to start app-server");
    };

    if let Some(url) = healthy_lockfile_url(&paths, &reuse_policy).await? {
        drop(spawn_lock);
        record_startup_trace_event("interpreter.daemon.reused_existing");
        return Ok(url);
    }

    let websocket_url = reserve_websocket_url()?;
    record_startup_trace_event("interpreter.daemon.spawn.start");
    let pid = spawn_app_server(&paths.log_path, launch_spec, &websocket_url, &cli_overrides)?;
    wait_for_healthy_server(&websocket_url)
        .await
        .with_context(|| {
            format!(
                "spawned `{}` but it never became healthy; check {}",
                launch_spec.display_name,
                paths.log_path.display()
            )
        })?;
    record_startup_trace_event("interpreter.daemon.spawn.ready");
    write_lockfile(
        &paths.lockfile_path,
        &AppServerLockfile {
            pid,
            port: websocket_port(&websocket_url)?,
            nonce: Uuid::new_v4().to_string(),
            websocket_url: websocket_url.clone(),
            started_at_unix: now_unix_seconds()?,
            server_bin: launch_spec.display_name.clone(),
            server_bin_fingerprint: launch_spec.artifact_fingerprint.clone(),
        },
    )?;

    drop(spawn_lock);
    Ok(websocket_url)
}

async fn local_app_server_status_for_home(
    codex_home: &Path,
) -> Result<Option<LocalAppServerStatus>> {
    let paths = daemon_paths_for_home(codex_home);
    let Some(lockfile) = read_live_lockfile(&paths)? else {
        return Ok(None);
    };
    let health = if is_server_healthy(&lockfile.websocket_url).await {
        DaemonHealth::Ready
    } else {
        DaemonHealth::Unhealthy
    };
    Ok(Some(LocalAppServerStatus {
        pid: lockfile.pid,
        websocket_url: lockfile.websocket_url,
        started_at_unix: lockfile.started_at_unix,
        log_path: paths.log_path,
        health,
    }))
}

async fn stop_local_app_server_for_home(codex_home: &Path) -> Result<StopLocalAppServerOutcome> {
    let paths = daemon_paths_for_home(codex_home);
    let Some(status) = local_app_server_status_for_home(codex_home).await? else {
        clear_runtime_markers(&paths);
        return Ok(StopLocalAppServerOutcome::NotRunning);
    };

    terminate_process(status.pid, /*force*/ false);
    if !wait_for_process_exit(status.pid, STOP_WAIT_TIMEOUT).await {
        terminate_process(status.pid, /*force*/ true);
        if !wait_for_process_exit(status.pid, SPAWN_WAIT_TIMEOUT).await {
            bail!("failed to stop daemon process {}", status.pid);
        }
    }

    clear_runtime_markers(&paths);
    Ok(StopLocalAppServerOutcome::Stopped(status))
}

fn daemon_paths_for_home(codex_home: &Path) -> DaemonPaths {
    let runtime_dir = codex_home.join("tmp").join("interpreter");
    let log_dir = codex_home.join("log");
    DaemonPaths {
        lockfile_path: runtime_dir.join("app-server.json"),
        spawn_lock_path: runtime_dir.join("spawn.lock"),
        log_path: log_dir.join("codex-app-server.log"),
    }
}

fn ensure_runtime_dirs(paths: &DaemonPaths) -> Result<()> {
    let lockfile_dir = paths
        .lockfile_path
        .parent()
        .context("lockfile path missing parent directory")?;
    std::fs::create_dir_all(lockfile_dir)
        .with_context(|| format!("failed to create {}", lockfile_dir.display()))?;
    let log_dir = paths
        .log_path
        .parent()
        .context("log path missing parent directory")?;
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create {}", log_dir.display()))?;
    Ok(())
}

fn reserve_websocket_url() -> Result<String> {
    let listener = TcpListener::bind((DEFAULT_WEBSOCKET_HOST, 0))
        .context("failed to reserve websocket port")?;
    let port = listener
        .local_addr()
        .context("failed to read reserved websocket port")?
        .port();
    drop(listener);
    Ok(format!("ws://{DEFAULT_WEBSOCKET_HOST}:{port}"))
}

fn websocket_port(websocket_url: &str) -> Result<u16> {
    websocket_url
        .rsplit(':')
        .next()
        .context("websocket url missing port")?
        .parse::<u16>()
        .with_context(|| format!("failed to parse websocket port from `{websocket_url}`"))
}

async fn healthy_lockfile_url(
    paths: &DaemonPaths,
    reuse_policy: &LockfileReusePolicy,
) -> Result<Option<String>> {
    let Some(lockfile) = read_live_lockfile(paths)? else {
        return Ok(None);
    };

    match reuse_policy.decision(&lockfile) {
        LockfileReuseDecision::Ignore => return Ok(None),
        LockfileReuseDecision::RestartRequired => {
            restart_required_lockfile(paths, &lockfile).await?;
            return Ok(None);
        }
        LockfileReuseDecision::Reuse => {}
    }

    if wait_for_healthy_existing_server(&lockfile.websocket_url).await {
        return Ok(Some(lockfile.websocket_url));
    }

    let _ = std::fs::remove_file(&paths.lockfile_path);
    Ok(None)
}

async fn restart_required_lockfile(
    paths: &DaemonPaths,
    lockfile: &AppServerLockfile,
) -> Result<()> {
    terminate_process(lockfile.pid, /*force*/ false);
    if !wait_for_process_exit(lockfile.pid, STOP_WAIT_TIMEOUT).await {
        terminate_process(lockfile.pid, /*force*/ true);
        if !wait_for_process_exit(lockfile.pid, SPAWN_WAIT_TIMEOUT).await {
            bail!(
                "failed to restart stale daemon process {} after binary change",
                lockfile.pid
            );
        }
    }
    clear_runtime_markers(paths);
    Ok(())
}

fn launch_artifact_fingerprint(path: &Path) -> Result<Option<LaunchArtifactFingerprint>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to stat {}", path.display())),
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read modification time for {}", path.display()))?;
    let modified_at_unix_nanos = modified
        .duration_since(UNIX_EPOCH)
        .context("launch artifact modification time is before unix epoch")?
        .as_nanos();
    Ok(Some(LaunchArtifactFingerprint {
        modified_at_unix_nanos,
        file_len_bytes: metadata.len(),
    }))
}

fn read_live_lockfile(paths: &DaemonPaths) -> Result<Option<AppServerLockfile>> {
    let Some(lockfile) = read_lockfile(&paths.lockfile_path)? else {
        return Ok(None);
    };

    if !lockfile_process_matches(&lockfile) {
        let _ = std::fs::remove_file(&paths.lockfile_path);
        return Ok(None);
    }

    Ok(Some(lockfile))
}

fn read_lockfile(path: &Path) -> Result<Option<AppServerLockfile>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };

    match serde_json::from_str(&content) {
        Ok(lockfile) => Ok(Some(lockfile)),
        Err(_) => {
            let _ = std::fs::remove_file(path);
            Ok(None)
        }
    }
}

fn lockfile_process_matches(lockfile: &AppServerLockfile) -> bool {
    let mut system = System::new();
    system.refresh_processes(
        ProcessesToUpdate::Some(&[Pid::from_u32(lockfile.pid)]),
        true,
    );

    let Some(process) = system.process(Pid::from_u32(lockfile.pid)) else {
        return false;
    };

    process_start_matches(lockfile.started_at_unix, process.start_time())
}

fn process_start_matches(lockfile_started_at_unix: i64, process_start_time_unix: u64) -> bool {
    let Ok(lockfile_started_at_unix) = u64::try_from(lockfile_started_at_unix) else {
        return false;
    };

    process_start_time_unix.abs_diff(lockfile_started_at_unix)
        <= MAX_PROCESS_START_TIME_SKEW.as_secs()
}

async fn wait_for_existing_spawn(
    paths: &DaemonPaths,
    reuse_policy: &LockfileReusePolicy,
) -> Result<Option<String>> {
    let Some(spawn_lock_age) = spawn_lock_age(&paths.spawn_lock_path)? else {
        return Ok(None);
    };

    if spawn_lock_age > STALE_SPAWN_LOCK_AGE {
        let _ = std::fs::remove_file(&paths.spawn_lock_path);
        return Ok(None);
    }

    let deadline = Instant::now() + SPAWN_WAIT_TIMEOUT;
    loop {
        if let Some(url) = healthy_lockfile_url(paths, reuse_policy).await? {
            return Ok(Some(url));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        sleep(HEALTH_POLL_INTERVAL).await;
    }
}

fn spawn_lock_age(path: &Path) -> Result<Option<Duration>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to stat {}", path.display())),
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    Ok(Some(age))
}

fn resolve_launch_spec(app_server_bin: Option<PathBuf>) -> Result<LaunchSpec> {
    if let Some(program) = app_server_bin {
        return Ok(LaunchSpec {
            display_name: program.display().to_string(),
            artifact_fingerprint: launch_artifact_fingerprint(&program)?,
            program,
            args: Vec::new(),
        });
    }

    let program = resolve_default_app_server_binary()?;
    Ok(LaunchSpec {
        display_name: program.display().to_string(),
        artifact_fingerprint: launch_artifact_fingerprint(&program)?,
        program,
        args: Vec::new(),
    })
}

fn resolve_default_app_server_binary() -> Result<PathBuf> {
    let current_exe = std::env::current_exe().context("failed to resolve interpreter path")?;
    let sibling = current_exe
        .parent()
        .context("interpreter path missing parent directory")?
        .join(INTERPRETER_APP_SERVER_BINARY);
    if sibling.exists() {
        return Ok(sibling);
    }

    which::which(INTERPRETER_APP_SERVER_BINARY).with_context(|| {
        format!(
            "failed to locate `{INTERPRETER_APP_SERVER_BINARY}` next to `{}` or on PATH",
            current_exe.display()
        )
    })
}

fn spawn_app_server(
    log_path: &Path,
    launch_spec: &LaunchSpec,
    websocket_url: &str,
    cli_overrides: &[String],
) -> Result<u32> {
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("failed to open {}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to clone {}", log_path.display()))?;

    let mut command = std::process::Command::new(&launch_spec.program);
    command
        .args(&launch_spec.args)
        .args(build_app_server_spawn_args(websocket_url, cli_overrides))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.pre_exec(codex_utils_pty::process_group::detach_from_tty);
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }

    let child = command.spawn().with_context(|| {
        format!(
            "failed to spawn app-server binary `{}`",
            launch_spec.display_name
        )
    })?;
    Ok(child.id())
}

fn build_app_server_spawn_args(websocket_url: &str, cli_overrides: &[String]) -> Vec<String> {
    let mut args = vec![
        "--listen".to_string(),
        websocket_url.to_string(),
        "--session-source".to_string(),
        "cli".to_string(),
        "--shutdown-idle-timeout-seconds".to_string(),
        SHUTDOWN_IDLE_TIMEOUT_SECONDS.to_string(),
    ];
    for cli_override in cli_overrides {
        args.push("-c".to_string());
        args.push(cli_override.clone());
    }
    args
}

async fn wait_for_healthy_server(websocket_url: &str) -> Result<()> {
    let deadline = Instant::now() + SPAWN_WAIT_TIMEOUT;
    loop {
        if is_server_healthy(websocket_url).await {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for app-server health");
        }
        sleep(HEALTH_POLL_INTERVAL).await;
    }
}

async fn wait_for_healthy_existing_server(websocket_url: &str) -> bool {
    let deadline = Instant::now() + HEALTH_REUSE_GRACE_TIMEOUT;
    loop {
        if is_server_healthy(websocket_url).await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(HEALTH_POLL_INTERVAL).await;
    }
}

async fn is_server_healthy(websocket_url: &str) -> bool {
    let Ok(response) = reqwest::get(readyz_url(websocket_url)).await else {
        return false;
    };
    response.status().is_success()
}

fn readyz_url(websocket_url: &str) -> String {
    if let Some(rest) = websocket_url.strip_prefix("ws://") {
        return format!("http://{}/readyz", rest.trim_end_matches('/'));
    }
    if let Some(rest) = websocket_url.strip_prefix("wss://") {
        return format!("https://{}/readyz", rest.trim_end_matches('/'));
    }
    format!("{}/readyz", websocket_url.trim_end_matches('/'))
}

fn write_lockfile(path: &Path, lockfile: &AppServerLockfile) -> Result<()> {
    let content = serde_json::to_string_pretty(lockfile)?;
    let parent = path
        .parent()
        .context("lockfile path missing parent directory")?;
    let temp_path = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app-server.json"),
        Uuid::new_v4()
    ));
    std::fs::write(&temp_path, content)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    std::fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to atomically replace {} with {}",
            path.display(),
            temp_path.display()
        )
    })?;
    Ok(())
}

fn now_unix_seconds() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(duration.as_secs()).context("unix timestamp overflowed i64")
}

fn clear_runtime_markers(paths: &DaemonPaths) {
    let _ = std::fs::remove_file(&paths.lockfile_path);
    let _ = std::fs::remove_file(&paths.spawn_lock_path);
}

fn terminate_process(pid: u32, force: bool) -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let Some(process) = system.process(Pid::from_u32(pid)) else {
        return false;
    };

    if force {
        return process.kill();
    }

    process
        .kill_with(Signal::Term)
        .unwrap_or_else(|| process.kill())
}

async fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
        if system.process(Pid::from_u32(pid)).is_none() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn process_start_matches_accepts_small_skew() {
        assert!(process_start_matches(
            /*lockfile_started_at_unix*/ 1_000, /*process_start_time_unix*/ 1_000,
        ));
        assert!(process_start_matches(
            /*lockfile_started_at_unix*/ 1_000,
            1_000 + MAX_PROCESS_START_TIME_SKEW.as_secs()
        ));
        assert!(process_start_matches(
            /*lockfile_started_at_unix*/
            1_000 + i64::try_from(MAX_PROCESS_START_TIME_SKEW.as_secs()).expect("fits"),
            /*process_start_time_unix*/ 1_000,
        ));
    }

    #[test]
    fn process_start_matches_rejects_large_skew() {
        assert!(!process_start_matches(
            /*lockfile_started_at_unix*/ 1_000,
            1_000 + MAX_PROCESS_START_TIME_SKEW.as_secs() + 1
        ));
    }

    #[test]
    fn read_lockfile_treats_invalid_json_as_stale() {
        let tempdir = TempDir::new().expect("tempdir");
        let path = tempdir.path().join("app-server.json");
        std::fs::write(&path, "{not-json").expect("write invalid lockfile");

        let lockfile = read_lockfile(&path).expect("read should succeed");
        assert!(lockfile.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn read_lockfile_treats_missing_nonce_as_stale() {
        let tempdir = TempDir::new().expect("tempdir");
        let path = tempdir.path().join("app-server.json");
        std::fs::write(
            &path,
            r#"{
  "pid": 123,
  "port": 4567,
  "websocketUrl": "ws://127.0.0.1:4567",
  "startedAtUnix": 1000,
  "serverBin": "codex-app-server"
}"#,
        )
        .expect("write stale lockfile");

        let lockfile = read_lockfile(&path).expect("read should succeed");
        assert!(lockfile.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn readyz_url_maps_websocket_urls_to_http() {
        assert_eq!(
            readyz_url("ws://127.0.0.1:8080"),
            "http://127.0.0.1:8080/readyz"
        );
        assert_eq!(
            readyz_url("wss://example.com:443/"),
            "https://example.com:443/readyz"
        );
    }

    #[test]
    fn build_spawn_args_include_cli_overrides() {
        assert_eq!(
            build_app_server_spawn_args(
                "ws://127.0.0.1:8080",
                &[
                    "features.apps=false".to_string(),
                    "features.plugins=false".to_string(),
                ],
            ),
            vec![
                "--listen".to_string(),
                "ws://127.0.0.1:8080".to_string(),
                "--session-source".to_string(),
                "cli".to_string(),
                "--shutdown-idle-timeout-seconds".to_string(),
                SHUTDOWN_IDLE_TIMEOUT_SECONDS.to_string(),
                "-c".to_string(),
                "features.apps=false".to_string(),
                "-c".to_string(),
                "features.plugins=false".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_launch_spec_uses_explicit_override_binary() {
        let custom_binary = PathBuf::from("/tmp/custom-interpreter-app-server");
        let launch_spec =
            resolve_launch_spec(Some(custom_binary.clone())).expect("resolve custom launch spec");

        assert_eq!(launch_spec.program, custom_binary);
        assert_eq!(launch_spec.args, Vec::<String>::new());
        assert_eq!(
            launch_spec.display_name,
            "/tmp/custom-interpreter-app-server"
        );
        assert_eq!(launch_spec.artifact_fingerprint, None);
    }

    #[tokio::test]
    async fn ensure_app_server_url_reuses_existing_daemon_for_same_home() {
        let tempdir = TempDir::new().expect("tempdir");
        let launch_spec =
            readyz_test_server_launch_spec(tempdir.path()).expect("resolve test app-server");

        let first_url =
            ensure_app_server_url_with_launch_spec(tempdir.path(), &launch_spec, Vec::new())
                .await
                .expect("start first daemon");
        let paths = daemon_paths_for_home(tempdir.path());
        let first_lockfile = read_lockfile(&paths.lockfile_path)
            .expect("read first lockfile")
            .expect("first lockfile exists");

        let second_url =
            ensure_app_server_url_with_launch_spec(tempdir.path(), &launch_spec, Vec::new())
                .await
                .expect("reuse daemon");
        let second_lockfile = read_lockfile(&paths.lockfile_path)
            .expect("read second lockfile")
            .expect("second lockfile exists");

        assert_eq!(first_url, second_url);
        assert_eq!(first_lockfile.websocket_url, second_lockfile.websocket_url);
        assert_eq!(first_lockfile.pid, second_lockfile.pid);

        terminate_process(second_lockfile.pid, /*force*/ true);
        assert!(wait_for_process_exit(second_lockfile.pid, Duration::from_secs(10)).await);
    }

    #[tokio::test]
    async fn ensure_app_server_url_serializes_concurrent_launchers_to_one_daemon() {
        let tempdir = TempDir::new().expect("tempdir");
        let launch_spec =
            readyz_test_server_launch_spec(tempdir.path()).expect("resolve test app-server");

        let (first_url, second_url) = tokio::join!(
            ensure_app_server_url_with_launch_spec(tempdir.path(), &launch_spec, Vec::new()),
            ensure_app_server_url_with_launch_spec(tempdir.path(), &launch_spec, Vec::new()),
        );

        let first_url = first_url.expect("first launcher should connect");
        let second_url = second_url.expect("second launcher should connect");
        let paths = daemon_paths_for_home(tempdir.path());
        let lockfile = read_lockfile(&paths.lockfile_path)
            .expect("read lockfile")
            .expect("lockfile exists");

        assert_eq!(first_url, second_url);
        assert_eq!(first_url, lockfile.websocket_url);

        terminate_process(lockfile.pid, /*force*/ true);
        assert!(wait_for_process_exit(lockfile.pid, Duration::from_secs(10)).await);
    }

    #[tokio::test]
    async fn ensure_app_server_url_restarts_daemon_when_launch_artifact_changes() {
        let tempdir = TempDir::new().expect("tempdir");
        let first_spec = readyz_test_server_launch_spec_with_marker(tempdir.path(), "v1")
            .expect("resolve first test app-server");

        let first_url =
            ensure_app_server_url_with_launch_spec(tempdir.path(), &first_spec, Vec::new())
                .await
                .expect("start first daemon");
        let paths = daemon_paths_for_home(tempdir.path());
        let first_lockfile = read_lockfile(&paths.lockfile_path)
            .expect("read first lockfile")
            .expect("first lockfile exists");

        let second_spec = readyz_test_server_launch_spec_with_marker(tempdir.path(), "v2")
            .expect("resolve second test app-server");
        let second_url =
            ensure_app_server_url_with_launch_spec(tempdir.path(), &second_spec, Vec::new())
                .await
                .expect("restart daemon after launch artifact change");
        let second_lockfile = read_lockfile(&paths.lockfile_path)
            .expect("read second lockfile")
            .expect("second lockfile exists");

        assert_ne!(first_url, second_url);
        assert_ne!(first_lockfile.pid, second_lockfile.pid);
        assert!(
            wait_for_process_exit(first_lockfile.pid, Duration::from_secs(10)).await,
            "stale daemon should exit after restart"
        );

        terminate_process(second_lockfile.pid, /*force*/ true);
        assert!(wait_for_process_exit(second_lockfile.pid, Duration::from_secs(10)).await);
    }

    #[tokio::test]
    async fn local_app_server_status_reports_running_daemon_for_home() {
        let tempdir = TempDir::new().expect("tempdir");
        let launch_spec =
            readyz_test_server_launch_spec(tempdir.path()).expect("resolve test app-server");

        let websocket_url =
            ensure_app_server_url_with_launch_spec(tempdir.path(), &launch_spec, Vec::new())
                .await
                .expect("start daemon");

        let status = local_app_server_status_for_home(tempdir.path())
            .await
            .expect("status should load")
            .expect("daemon should be running");

        assert_eq!(status.websocket_url, websocket_url);
        assert_eq!(status.health, DaemonHealth::Ready);
        assert!(status.log_path.ends_with("codex-app-server.log"));

        terminate_process(status.pid, /*force*/ true);
        assert!(wait_for_process_exit(status.pid, Duration::from_secs(10)).await);
    }

    #[tokio::test]
    async fn stop_local_app_server_stops_running_daemon_and_clears_lockfile() {
        let tempdir = TempDir::new().expect("tempdir");
        let launch_spec =
            readyz_test_server_launch_spec(tempdir.path()).expect("resolve test app-server");

        let websocket_url =
            ensure_app_server_url_with_launch_spec(tempdir.path(), &launch_spec, Vec::new())
                .await
                .expect("start daemon");
        let lockfile_path = daemon_paths_for_home(tempdir.path()).lockfile_path;

        let outcome = stop_local_app_server_for_home(tempdir.path())
            .await
            .expect("stop daemon");

        let StopLocalAppServerOutcome::Stopped(status) = outcome else {
            panic!("expected stopped daemon outcome");
        };
        assert_eq!(status.websocket_url, websocket_url);
        assert!(!lockfile_path.exists());
        assert!(
            local_app_server_status_for_home(tempdir.path())
                .await
                .expect("status check")
                .is_none()
        );
    }

    #[tokio::test]
    async fn stop_local_app_server_returns_not_running_without_lockfile() {
        let tempdir = TempDir::new().expect("tempdir");

        let outcome = stop_local_app_server_for_home(tempdir.path())
            .await
            .expect("stop with no daemon");

        assert_eq!(outcome, StopLocalAppServerOutcome::NotRunning);
    }

    fn readyz_test_server_launch_spec(root: &Path) -> Result<LaunchSpec> {
        readyz_test_server_launch_spec_with_marker(root, "")
    }

    fn readyz_test_server_launch_spec_with_marker(root: &Path, marker: &str) -> Result<LaunchSpec> {
        let program = which::which("python3")
            .or_else(|_| which::which("python"))
            .context("could not find python interpreter for daemon test helper")?;
        let script_path = root.join("readyz_test_server.py");
        std::fs::write(&script_path, readyz_test_server_script(marker)).with_context(|| {
            format!(
                "failed to write daemon test helper script to {}",
                script_path.display()
            )
        })?;
        Ok(LaunchSpec {
            display_name: script_path.display().to_string(),
            artifact_fingerprint: launch_artifact_fingerprint(&script_path)?,
            program,
            args: vec![script_path.display().to_string()],
        })
    }

    fn readyz_test_server_script(marker: &str) -> String {
        format!(
            r#"# marker: {marker}
import http.server
import socketserver
import sys
import urllib.parse


def parse_listen_url(argv):
    for index, arg in enumerate(argv):
        if arg == "--listen" and index + 1 < len(argv):
            return argv[index + 1]
    raise SystemExit("missing --listen argument")


listen_url = parse_listen_url(sys.argv[1:])
parsed = urllib.parse.urlparse(listen_url)
if parsed.hostname is None or parsed.port is None:
    raise SystemExit(f"invalid listen url: {{listen_url}}")


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/readyz":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, format, *args):
        return


class ReusableTCPServer(socketserver.TCPServer):
    allow_reuse_address = True


with ReusableTCPServer((parsed.hostname, parsed.port), Handler) as server:
    server.serve_forever()
"#
        )
    }
}
