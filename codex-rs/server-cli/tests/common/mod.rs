#![allow(dead_code, unused_imports)]

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use codex_utils_cargo_bin::cargo_bin;
use codex_utils_cargo_bin::repo_root;
use codex_utils_cargo_bin::runfiles_available;
use tokio::time::sleep;

mod mock_anthropic_server;
mod mock_responses_server;

pub(crate) use mock_anthropic_server::MockAnthropicResponse;
pub(crate) use mock_anthropic_server::MockAnthropicServer;
pub(crate) use mock_responses_server::MockResponsesServer;

static CODEX_BIN: OnceLock<PathBuf> = OnceLock::new();
static EXEC_BIN: OnceLock<PathBuf> = OnceLock::new();
static INTERPRETER_BIN: OnceLock<PathBuf> = OnceLock::new();
static TMUX_SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
const KEYBOARD_ENHANCEMENT_QUERY: &[u8] = b"\x1b[?u";
const PRIMARY_DEVICE_ATTRIBUTES_QUERY: &[u8] = b"\x1b[c";
const FOREGROUND_COLOR_QUERY: &[u8] = b"\x1b]10;?\x1b\\";
const BACKGROUND_COLOR_QUERY: &[u8] = b"\x1b]11;?\x1b\\";

const CURSOR_POSITION_RESPONSE: &[u8] = b"\x1b[1;1R";
const KEYBOARD_ENHANCEMENT_RESPONSE: &[u8] = b"\x1b[?0u";
const PRIMARY_DEVICE_ATTRIBUTES_RESPONSE: &[u8] = b"\x1b[?1;2c";
const FOREGROUND_COLOR_RESPONSE: &[u8] = b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\";
const BACKGROUND_COLOR_RESPONSE: &[u8] = b"\x1b]11;rgb:0000/0000/0000\x1b\\";

const TYPED_KEY_DELAY: Duration = Duration::from_millis(12);

pub(crate) fn is_prompt_visible_screen(pane: &str) -> bool {
    !pane.trim().is_empty() && pane.contains('›')
}

pub(crate) fn is_session_ready_screen(pane: &str) -> bool {
    let has_session_status_surface = pane.contains("/model to change")
        || pane
            .lines()
            .any(|line| line.contains(" · ") && !line.contains("Choose a"));
    is_prompt_visible_screen(pane)
        && has_session_status_surface
        && !pane.contains("(starting...)")
        && !pane.contains("model:     loading")
        && !pane.contains("Enter queue")
}

pub(crate) fn tmux_is_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(crate) fn resolve_codex_bin() -> Result<PathBuf> {
    ensure_workspace_binary(
        &CODEX_BIN,
        /*package*/ "codex-cli",
        /*bin_name*/ codex_bin_name(),
    )
}

pub(crate) fn resolve_exec_bin() -> Result<PathBuf> {
    ensure_workspace_binary(
        &EXEC_BIN,
        /*package*/ "codex-exec",
        /*bin_name*/ exec_bin_name(),
    )
}

pub(crate) fn resolve_interpreter_bin() -> Result<PathBuf> {
    ensure_workspace_binary(
        &INTERPRETER_BIN,
        /*package*/ "codex-server-cli",
        /*bin_name*/ interpreter_bin_name(),
    )
}

fn ensure_workspace_binary(
    cache: &OnceLock<PathBuf>,
    package: &str,
    bin_name: &str,
) -> Result<PathBuf> {
    if let Some(path) = cache.get() {
        return Ok(path.clone());
    }

    if runfiles_available() {
        let path = cargo_bin(bin_name).with_context(|| {
            format!("failed to resolve `{bin_name}` from Cargo/Bazel binary metadata")
        })?;
        let _ = cache.set(path.clone());
        return Ok(path);
    }

    let repo_root = repo_root()?;
    let workspace_root = repo_root.join("codex-rs");
    let workspace_debug_bin = workspace_root.join("target").join("debug").join(bin_name);
    if workspace_debug_bin.exists() {
        let _ = cache.set(workspace_debug_bin.clone());
        return Ok(workspace_debug_bin);
    }

    for key in cargo_bin_env_keys(bin_name) {
        if let Some(path) = std::env::var_os(&key).map(PathBuf::from) {
            let resolved = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .with_context(|| format!("reading current directory for `{bin_name}`"))?
                    .join(path)
            };
            if resolved.exists() {
                let _ = cache.set(resolved.clone());
                return Ok(resolved);
            }
        }
    }

    anyhow::bail!(
        "could not locate prebuilt `{package}` binary `{bin_name}`; looked for Cargo-provided test env vars and {}. Build it first with `cargo build -p {package} --bin {bin_name}`",
        workspace_debug_bin.display()
    )
}

fn cargo_bin_env_keys(bin_name: &str) -> Vec<String> {
    let target_name = bin_name.strip_suffix(".exe").unwrap_or(bin_name);
    let mut keys = vec![format!("CARGO_BIN_EXE_{target_name}")];
    let underscore_name = target_name.replace('-', "_");
    if underscore_name != target_name {
        keys.push(format!("CARGO_BIN_EXE_{underscore_name}"));
    }
    keys
}

fn codex_bin_name() -> &'static str {
    #[cfg(windows)]
    {
        "codex.exe"
    }

    #[cfg(not(windows))]
    {
        "codex"
    }
}

fn exec_bin_name() -> &'static str {
    #[cfg(windows)]
    {
        "codex-exec.exe"
    }

    #[cfg(not(windows))]
    {
        "codex-exec"
    }
}

fn interpreter_bin_name() -> &'static str {
    #[cfg(windows)]
    {
        "interpreter.exe"
    }

    #[cfg(not(windows))]
    {
        "interpreter"
    }
}

pub(crate) struct TmuxSession {
    name: String,
}

impl TmuxSession {
    pub(crate) fn start(
        prefix: &str,
        binary: &Path,
        args: &[String],
        workdir: &Path,
        env: &[(String, String)],
    ) -> Result<Self> {
        let session_name = unique_tmux_session_name(prefix);
        let command = build_tmux_command(binary, args, workdir, env);
        let status = Command::new("tmux")
            .args(["new-session", "-d", "-s", &session_name, &command])
            .status()
            .with_context(|| format!("starting tmux session `{session_name}`"))?;
        anyhow::ensure!(
            status.success(),
            "tmux new-session failed for `{session_name}` with status {status}"
        );

        let remain_status = Command::new("tmux")
            .args(["set-option", "-t", &session_name, "remain-on-exit", "on"])
            .status()
            .with_context(|| format!("enabling remain-on-exit for `{session_name}`"))?;
        anyhow::ensure!(
            remain_status.success(),
            "tmux set-option remain-on-exit failed for `{session_name}` with status {remain_status}"
        );

        Ok(Self { name: session_name })
    }

    pub(crate) fn send_literal(&self, text: &str) -> Result<()> {
        let status = Command::new("tmux")
            .args(["send-keys", "-t", &self.name, "-l", text])
            .status()
            .with_context(|| format!("sending literal keys to `{}`", self.name))?;
        anyhow::ensure!(
            status.success(),
            "tmux send-keys -l failed for `{}` with status {status}",
            self.name
        );
        Ok(())
    }

    pub(crate) async fn type_like_user(&self, text: &str) -> Result<()> {
        for character in text.chars() {
            self.send_literal(&character.to_string())?;
            sleep(TYPED_KEY_DELAY).await;
        }

        sleep(TYPED_KEY_DELAY).await;
        Ok(())
    }

    pub(crate) fn send_enter(&self) -> Result<()> {
        let status = Command::new("tmux")
            .args(["send-keys", "-t", &self.name, "C-m"])
            .status()
            .with_context(|| format!("sending Enter to `{}`", self.name))?;
        anyhow::ensure!(
            status.success(),
            "tmux send-keys C-m failed for `{}` with status {status}",
            self.name
        );
        Ok(())
    }

    pub(crate) fn send_escape(&self) -> Result<()> {
        let status = Command::new("tmux")
            .args(["send-keys", "-t", &self.name, "Escape"])
            .status()
            .with_context(|| format!("sending Escape to `{}`", self.name))?;
        anyhow::ensure!(
            status.success(),
            "tmux send-keys Escape failed for `{}` with status {status}",
            self.name
        );
        Ok(())
    }

    pub(crate) fn send_ctrl_c(&self) -> Result<()> {
        let status = Command::new("tmux")
            .args(["send-keys", "-t", &self.name, "C-c"])
            .status()
            .with_context(|| format!("sending Ctrl-C to `{}`", self.name))?;
        anyhow::ensure!(
            status.success(),
            "tmux send-keys C-c failed for `{}` with status {status}",
            self.name
        );
        Ok(())
    }

    pub(crate) fn pane_text(&self) -> Result<String> {
        let output = Command::new("tmux")
            .args(["capture-pane", "-p", "-t", &self.name])
            .output()
            .with_context(|| format!("capturing tmux pane for `{}`", self.name))?;
        anyhow::ensure!(
            output.status.success(),
            "tmux capture-pane failed for `{}` with status {}",
            self.name,
            output.status
        );
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub(crate) fn pane_ansi(&self) -> Result<String> {
        let output = Command::new("tmux")
            .args(["capture-pane", "-e", "-p", "-t", &self.name])
            .output()
            .with_context(|| format!("capturing ANSI tmux pane for `{}`", self.name))?;
        anyhow::ensure!(
            output.status.success(),
            "tmux capture-pane -e failed for `{}` with status {}",
            self.name,
            output.status
        );
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub(crate) fn pane_dead_status(&self) -> Result<Option<i32>> {
        let output = Command::new("tmux")
            .args([
                "display-message",
                "-p",
                "-t",
                &self.name,
                "#{?pane_dead,#{pane_dead_status},}",
            ])
            .output()
            .with_context(|| format!("reading tmux pane status for `{}`", self.name))?;
        anyhow::ensure!(
            output.status.success(),
            "tmux display-message failed for `{}` with status {}",
            self.name,
            output.status
        );
        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if status.is_empty() {
            return Ok(None);
        }
        Ok(Some(status.parse().with_context(|| {
            format!(
                "parsing tmux pane exit status `{status}` for `{}`",
                self.name
            )
        })?))
    }

    pub(crate) fn pane_pid(&self) -> Result<u32> {
        let output = Command::new("tmux")
            .args(["display-message", "-p", "-t", &self.name, "#{pane_pid}"])
            .output()
            .with_context(|| format!("reading tmux pane pid for `{}`", self.name))?;
        anyhow::ensure!(
            output.status.success(),
            "tmux display-message pane_pid failed for `{}` with status {}",
            self.name,
            output.status
        );
        let pid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        pid.parse()
            .with_context(|| format!("parsing tmux pane pid `{pid}` for `{}`", self.name))
    }

    pub(crate) async fn wait_for_screen<F>(
        &self,
        timeout_duration: Duration,
        predicate: F,
    ) -> Result<String>
    where
        F: Fn(&str) -> bool,
    {
        let start = tokio::time::Instant::now();
        loop {
            let pane = self.pane_text()?;
            if predicate(&pane) {
                return Ok(pane);
            }

            if let Some(exit_status) = self.pane_dead_status()? {
                let ansi = self.pane_ansi()?;
                anyhow::bail!(
                    "tmux session `{}` exited with status {exit_status}; visible pane:\n{pane}\nANSI pane:\n{ansi}",
                    self.name
                );
            }

            if start.elapsed() > timeout_duration {
                let ansi = self.pane_ansi()?;
                anyhow::bail!(
                    "timed out waiting for tmux session `{}`; visible pane:\n{pane}\nANSI pane:\n{ansi}",
                    self.name
                );
            }

            sleep(Duration::from_millis(50)).await;
        }
    }

    pub(crate) async fn wait_for_exit(&self, timeout_duration: Duration) -> Result<()> {
        let start = tokio::time::Instant::now();
        loop {
            if self.pane_dead_status()?.is_some() {
                return Ok(());
            }
            if start.elapsed() > timeout_duration {
                let pane = self.pane_text()?;
                let ansi = self.pane_ansi()?;
                anyhow::bail!(
                    "timed out waiting for tmux session `{}` to exit; visible pane:\n{pane}\nANSI pane:\n{ansi}",
                    self.name
                );
            }
            sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &self.name])
            .status();
    }
}

pub(crate) struct PtyTerminalResponder {
    answered_cursor_query: bool,
}

impl PtyTerminalResponder {
    pub(crate) fn new() -> Self {
        Self {
            answered_cursor_query: false,
        }
    }

    pub(crate) fn answered_cursor_query(&self) -> bool {
        self.answered_cursor_query
    }

    pub(crate) async fn handle_output_chunk(
        &mut self,
        writer_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
        chunk: &[u8],
    ) {
        if chunk
            .windows(CURSOR_POSITION_QUERY.len())
            .any(|window| window == CURSOR_POSITION_QUERY)
        {
            let _ = writer_tx.send(CURSOR_POSITION_RESPONSE.to_vec()).await;
            self.answered_cursor_query = true;
        }

        if chunk
            .windows(KEYBOARD_ENHANCEMENT_QUERY.len())
            .any(|window| window == KEYBOARD_ENHANCEMENT_QUERY)
        {
            let _ = writer_tx.send(KEYBOARD_ENHANCEMENT_RESPONSE.to_vec()).await;
        }

        if chunk
            .windows(PRIMARY_DEVICE_ATTRIBUTES_QUERY.len())
            .any(|window| window == PRIMARY_DEVICE_ATTRIBUTES_QUERY)
        {
            let _ = writer_tx
                .send(PRIMARY_DEVICE_ATTRIBUTES_RESPONSE.to_vec())
                .await;
        }

        if chunk
            .windows(FOREGROUND_COLOR_QUERY.len())
            .any(|window| window == FOREGROUND_COLOR_QUERY)
        {
            let _ = writer_tx.send(FOREGROUND_COLOR_RESPONSE.to_vec()).await;
        }

        if chunk
            .windows(BACKGROUND_COLOR_QUERY.len())
            .any(|window| window == BACKGROUND_COLOR_QUERY)
        {
            let _ = writer_tx.send(BACKGROUND_COLOR_RESPONSE.to_vec()).await;
        }
    }
}

pub(crate) async fn type_prompt_like_user(
    writer_tx: &tokio::sync::mpsc::Sender<Vec<u8>>,
    prompt: &str,
) {
    for character in prompt.chars() {
        let _ = writer_tx.send(character.to_string().into_bytes()).await;
        sleep(TYPED_KEY_DELAY).await;
    }

    sleep(TYPED_KEY_DELAY).await;
}

pub(crate) async fn press_enter_like_user(writer_tx: &tokio::sync::mpsc::Sender<Vec<u8>>) {
    let _ = writer_tx.send(vec![b'\r']).await;
}

fn unique_tmux_session_name(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let counter = TMUX_SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{millis}-{counter}", std::process::id())
}

fn build_tmux_command(
    binary: &Path,
    args: &[String],
    workdir: &Path,
    env: &[(String, String)],
) -> String {
    let explicit_env_keys = env
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut command_parts = vec![
        "cd".to_string(),
        shell_quote(workdir.to_string_lossy().as_ref()),
        "&&".to_string(),
        "env".to_string(),
        "-i".to_string(),
    ];
    for key in ["HOME", "PATH", "TMPDIR", "SHELL", "LANG", "LC_ALL"] {
        if explicit_env_keys.contains(key) {
            continue;
        }
        if let Some(value) = std::env::var_os(key).filter(|value| !value.is_empty()) {
            command_parts.push(format!(
                "{key}={}",
                shell_quote(value.to_string_lossy().as_ref())
            ));
        }
    }
    for (key, value) in env {
        command_parts.push(format!("{key}={}", shell_quote(value)));
    }
    command_parts.push(shell_quote(binary.to_string_lossy().as_ref()));
    command_parts.extend(args.iter().map(|arg| shell_quote(arg)));
    command_parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tmux_command_prefers_explicit_home_over_inherited_home() {
        let command = build_tmux_command(
            Path::new("/tmp/interpreter"),
            &[],
            Path::new("/tmp/workdir"),
            &[("HOME".to_string(), "/tmp/custom-home".to_string())],
        );

        assert!(command.contains("HOME='/tmp/custom-home'"));
        assert_eq!(command.matches("HOME=").count(), 1);
    }
}
