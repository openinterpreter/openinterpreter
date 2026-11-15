//! WSL text copies with Windows interop use the Windows clipboard without contacting the display.
//! Bound discovery, input, and exit; give fallback native setup its own short deadline.
//! Never release the worker while a writer can still run.

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::mpsc;
#[cfg(any(target_os = "linux", all(test, unix)))]
use std::time::Instant;

#[cfg(target_os = "linux")]
type Pending<T> = Mutex<Option<mpsc::Receiver<Result<T, String>>>>;

#[cfg(target_os = "linux")]
pub(super) fn copy(text: &str, deadline: Instant) -> Result<(), String> {
    static DISCOVERY: Pending<(PathBuf, OsString)> = Mutex::new(None);
    let (executable, path) = read_only_setup(&DISCOVERY, deadline, || {
        let executable = codex_utils_path::system_executable("powershell.exe")
            .ok_or("PowerShell is unavailable in the system PATH")?;
        let path = codex_utils_path::system_path()
            .map_err(|error| format!("failed to resolve system PATH: {error}"))?;
        Ok((executable, path))
    })?;
    let status = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start Windows clipboard writer: {error}"))?
        .block_on(write_command(command(&executable, &path), text, deadline))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("powershell.exe exited with status {status}"))
    }
}

#[cfg(any(target_os = "linux", all(test, unix)))]
fn command(executable: &std::path::Path, path: &std::ffi::OsStr) -> tokio::process::Command {
    // Spawn only the native shell here: directly spawning a Windows exe can block on WSL.
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .env("PATH", path)
        .args(["-c", r#"exec "$@""#, "codex-wsl-copy"])
        .arg(executable)
        // Preserve the released command signature: trusted adapters may match it exactly.
        .args([
            "-NoProfile",
            "-Command",
            "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; $ErrorActionPreference = 'Stop'; $text = [Console]::In.ReadToEnd(); Set-Clipboard -Value $text",
        ]);
    command
}

#[cfg(not(target_os = "linux"))]
pub(super) fn copy(_text: &str, _deadline: std::time::Instant) -> Result<(), String> {
    Err("Windows clipboard unavailable on this platform".into())
}

#[cfg(target_os = "linux")]
pub(super) fn native_clipboard() -> Result<arboard::Clipboard, String> {
    static NATIVE: Pending<arboard::Clipboard> = Mutex::new(None);
    // Windows discovery may have exhausted its budget. Do not starve a healthy WSLg.
    let deadline = Instant::now() + std::time::Duration::from_secs(/*secs*/ 1);
    read_only_setup(&NATIVE, deadline, || {
        arboard::Clipboard::new().map_err(|error| format!("clipboard unavailable: {error}"))
    })
}

// Discovery and WSLg setup may block forever. Keep at most one outstanding task of each
// kind and reuse its result after a timeout. Neither task may capture text or write a clipboard.
#[cfg(target_os = "linux")]
fn read_only_setup<T: Send + 'static>(
    pending: &Pending<T>,
    deadline: Instant,
    setup: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or("clipboard setup timed out")?;
    let mut pending = pending.lock().map_err(|error| error.to_string())?;
    if pending.is_none() {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("wsl-clipboard-setup".into())
            .spawn(move || {
                let _ = tx.send(setup());
            })
            .map_err(|error| format!("could not start clipboard setup: {error}"))?;
        *pending = Some(rx);
    }
    let result = pending
        .as_ref()
        .ok_or("clipboard setup unavailable")?
        .recv_timeout(remaining);
    if !matches!(result, Err(mpsc::RecvTimeoutError::Timeout)) {
        *pending = None;
    }
    result.map_err(|error| format!("clipboard setup: {error}"))?
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(super) fn native_clipboard() -> Result<arboard::Clipboard, String> {
    Err("WSL native clipboard unavailable on this platform".into())
}

#[cfg(any(target_os = "linux", all(test, unix)))]
async fn write_command(
    mut command: tokio::process::Command,
    text: &str,
    deadline: Instant,
) -> Result<std::process::ExitStatus, String> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    if Instant::now() >= deadline {
        return Err("Windows clipboard writer timed out".into());
    }
    let mut child = command
        .process_group(/*pgroup*/ 0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        // Do not expose clipboard contents from PowerShell errors or let its stderr block input.
        .stderr(Stdio::null())
        .kill_on_drop(/*kill_on_drop*/ true)
        .spawn()
        .map_err(|error| format!("failed to spawn powershell.exe: {error}"))?;
    // WSL process spawn itself may stall. Do not write after a late spawn.
    let result = tokio::time::timeout_at(deadline.into(), async {
        if Instant::now() >= deadline {
            return Err("Windows clipboard writer timed out".into());
        }
        child
            .stdin
            .as_mut()
            .ok_or("failed to open powershell.exe stdin")?
            .write_all(text.as_bytes())
            .await
            .map_err(|error| format!("failed to write to powershell.exe: {error}"))?;
        drop(child.stdin.take());
        child
            .wait()
            .await
            .map_err(|error| format!("failed to wait for powershell.exe: {error}"))
    })
    .await
    .unwrap_or_else(|_| Err("Windows clipboard writer timed out".into()));
    match result {
        Ok(status) => Ok(status),
        Err(error) => {
            // Keep partially written stdin open until kill: PowerShell copies on EOF.
            // If kill fails, still wait. Releasing the worker earlier could allow an old copy
            // to overwrite a newer one. kill_on_drop alone does not guarantee reaping.
            if let Some(pid) = child.id() {
                // SAFETY: process_group(0) assigned the owned, unreaped child its own group.
                // An installed adapter may have spawned its own writer. Stop the whole group.
                unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
            }
            let _ = child.start_kill();
            // wait() closes child.stdin itself, even if kill failed. Never give the
            // writer EOF with partial input while waiting for it to stop.
            let _stdin = child.stdin.take();
            child
                .wait()
                .await
                .map_err(|error| format!("failed to reap powershell.exe: {error}"))?;
            Err(error)
        }
    }
}

#[cfg(all(test, unix))]
#[path = "wsl_tests.rs"]
mod tests;
