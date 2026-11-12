//! Isolated installed-CLI fixtures and PID cleanup for Unix daemon integration tests.
//! Every test owns its home and CLI selection; only immutable executable contents are shared.

use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use codex_utils_cargo_bin::copy_executable;
use serde_json::Value;
use tempfile::TempDir;

pub(super) struct TestDaemon {
    pub(super) home: TempDir,
    pub(super) codex: PathBuf,
    pub(super) unmanaged: Option<Child>,
}

impl TestDaemon {
    pub(super) fn new() -> Result<Self> {
        let home = tempfile::Builder::new().tempdir_in("/tmp")?;
        let codex = codex_utils_cargo_bin::cargo_bin("codex")?;
        let codex_source = std::fs::canonicalize(&codex)?;
        let target = if cfg!(target_os = "macos") {
            format!("{}-apple-darwin", std::env::consts::ARCH)
        } else {
            format!("{}-unknown-linux-musl", std::env::consts::ARCH)
        };
        let standalone = home.path().join("packages/standalone");
        let release_name = format!("0.0.0-{target}");
        let managed = standalone
            .join("releases")
            .join(&release_name)
            .join("bin/codex");
        std::fs::create_dir_all(managed.parent().context("managed bin parent")?)?;
        // Preserve the installed path without invalidating the shared CLI's Rosetta cache.
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            copy_executable(&codex_source, &managed)?;
            // Translate the fixture before timed daemon capability and readiness checks.
            ensure!(
                Command::new(&managed)
                    .env("CODEX_HOME", home.path())
                    .arg("--version")
                    .output()?
                    .status
                    .success(),
                "failed to prepare managed test executable"
            );
        }
        #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
        if std::fs::hard_link(&codex_source, &managed).is_err() {
            copy_executable(&codex_source, &managed)?;
        }
        std::fs::write(standalone.join("auto-update-version"), &release_name)?;
        std::os::unix::fs::symlink(
            PathBuf::from("releases").join(release_name),
            standalone.join("current"),
        )?;
        // Model a daemon that was previously launched and is currently stopped.
        let state = home.path().join("app-server-daemon");
        std::fs::create_dir(&state)?;
        std::fs::write(state.join("app-server.stderr.log"), b"")?;
        Ok(Self {
            home,
            codex,
            unmanaged: None,
        })
    }

    pub(super) fn command(&self) -> Command {
        let mut command = Command::new(&self.codex);
        command.env("CODEX_HOME", self.home.path());
        command
    }

    pub(super) fn lifecycle(&self, action: &str) -> Result<Value> {
        let output = self
            .command()
            .args(["app-server", "daemon", action])
            .output()?;
        ensure!(
            output.status.success(),
            "daemon {action} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    pub(super) fn pid(&self, name: &str) -> Result<u32> {
        let record = std::fs::read(self.home.path().join("app-server-daemon").join(name))
            .with_context(|| format!("failed to read {name}"))?;
        Ok(serde_json::from_slice::<Value>(&record)?["pid"]
            .as_u64()
            .context("pid missing")? as u32)
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if let Some(mut child) = self.unmanaged.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.lifecycle("stop");
        if let Ok(pid) = self.pid("app-server-updater.pid") {
            let _ = signal(pid, libc::SIGTERM);
        }
    }
}

pub(super) fn signal(pid: u32, signal: libc::c_int) -> Result<()> {
    let raw_pid = libc::pid_t::try_from(pid).context("pid out of range")?;
    ensure!(
        unsafe { libc::kill(raw_pid, signal) } == 0,
        "failed to signal pid {pid}: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

pub(super) fn wait_for_exit(pid: u32) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .output()
            .context("failed to invoke ps")?;
        let state = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || state.trim().starts_with('Z') {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "pid {pid} did not exit: {state}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
