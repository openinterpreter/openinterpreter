//! Real package installation and replacement through the daemon CLI.
//! Keep these I/O-heavy cases separate from ordinary lifecycle coverage.
#![cfg(unix)]

mod support;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use pretty_assertions::assert_eq;
use serde_json::Value;
use support::TestDaemon;
use support::wait_for_exit;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InitialDaemon {
    Missing,
    Legacy,
}

fn packaged_daemon_launch(action: &str, initial: InitialDaemon) -> Result<()> {
    let mut daemon = TestDaemon::new()?;
    let standalone = daemon.home.path().join("packages/standalone");
    let package = if action == "start" && initial == InitialDaemon::Missing {
        standalone.join("releases/caller")
    } else {
        daemon.home.path().join("cli-package")
    };
    for directory in ["bin", "codex-path", "codex-resources"] {
        std::fs::create_dir_all(package.join(directory))?;
    }
    // Both paths are in this test's temp home and therefore on the same filesystem.
    // Reuse its prepared (and on Intel macOS, translated) executable. The installer
    // still makes a real independent package copy; only fixture setup shares this inode.
    std::fs::hard_link(
        standalone.join("current/bin/codex"),
        package.join("bin/codex"),
    )?;
    daemon.codex = package.join("bin/codex");
    for helper in [
        "bin/codex-code-mode-host",
        "codex-path/rg",
        "codex-resources/bwrap",
    ] {
        codex_utils_cargo_bin::write_executable(&package.join(helper), "runtime fixture")?;
    }
    let target = format!(
        "{}-{}",
        std::env::consts::ARCH,
        if cfg!(target_os = "macos") {
            "apple-darwin"
        } else if cfg!(target_env = "gnu") {
            "unknown-linux-gnu"
        } else {
            "unknown-linux-musl"
        }
    );
    std::fs::write(
        package.join("codex-package.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"), "target": target, "entrypoint": "bin/codex"
        }))?,
    )?;
    if action == "start" && initial == InitialDaemon::Missing {
        std::fs::remove_file(standalone.join("current"))?;
        std::os::unix::fs::symlink(&package, standalone.join("current"))?;
    }
    let cli_selection = standalone.join("current").canonicalize()?;
    let state = daemon.home.path().join("app-server-daemon");
    if initial == InitialDaemon::Missing {
        std::fs::remove_file(state.join("app-server.stderr.log"))?;
    }
    std::fs::write(
        state.join("settings.json"),
        br#"{"shutdownGraceSeconds":0}"#,
    )?;
    let cli_before = daemon.codex.canonicalize()?;
    let result = daemon
        .command()
        .args(["app-server", "daemon", action])
        .output()
        .context("failed to launch packaged daemon fixture")?;
    ensure!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stderr).contains("Installing daemon from CLI version"),
        initial == InitialDaemon::Missing
    );
    let output: Value = serde_json::from_slice(&result.stdout)?;
    let dedicated = daemon
        .home
        .path()
        .canonicalize()?
        .join("packages/app-server-daemon");
    let (initial_root, initial_pid_file) = match initial {
        InitialDaemon::Missing => (&dedicated, "daemon.pid"),
        InitialDaemon::Legacy => (&standalone, "app-server.pid"),
    };
    assert_eq!(
        output["managedCodexPath"],
        initial_root
            .canonicalize()?
            .join("current/bin/codex")
            .to_str()
            .context("managed path is not UTF-8")?
    );
    assert_eq!(daemon.codex.canonicalize()?, cli_before);
    assert_eq!(standalone.join("current").canonicalize()?, cli_selection);
    assert!(state.join(initial_pid_file).exists());
    let legacy_updater = match initial {
        InitialDaemon::Missing => {
            assert!(!state.join("app-server-updater.pid").exists());
            None
        }
        InitialDaemon::Legacy => Some(daemon.pid("app-server-updater.pid")?),
    };
    if action == "start" {
        let initial_current = initial_root.join("current");
        let current = dedicated.join("current");
        let original = initial_current.canonicalize()?;
        let original_pid = daemon.pid(initial_pid_file)?;
        let settings_before = std::fs::read(state.join("settings.json"))?;
        let refused = daemon
            .command()
            .args(["app-server", "daemon", "update", "--from-cli"])
            .output()?;
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("rerun with --yes"));
        assert_eq!(daemon.pid(initial_pid_file)?, original_pid);
        assert_eq!(initial_current.canonicalize()?, original);
        if initial == InitialDaemon::Legacy {
            assert!(!current.exists());
        }

        std::fs::remove_file(package.join("bin/codex-code-mode-host"))?;
        let invalid = daemon
            .command()
            .args(["app-server", "daemon", "update", "--from-cli", "--yes"])
            .output()?;
        assert!(!invalid.status.success());
        assert_eq!(daemon.pid(initial_pid_file)?, original_pid);
        assert_eq!(initial_current.canonicalize()?, original);
        if initial == InitialDaemon::Legacy {
            assert!(!current.exists());
        }
        codex_utils_cargo_bin::write_executable(
            &package.join("bin/codex-code-mode-host"),
            "replacement helper",
        )?;
        let replaced = daemon
            .command()
            .args(["app-server", "daemon", "update", "--from-cli", "--yes"])
            .output()?;
        ensure!(
            replaced.status.success(),
            "{}",
            String::from_utf8_lossy(&replaced.stderr)
        );
        let output: Value = serde_json::from_slice(&replaced.stdout)?;
        assert_eq!(output["status"], "updated");
        assert_ne!(daemon.pid("daemon.pid")?, original_pid);
        wait_for_exit(original_pid)?;
        if let Some(pid) = legacy_updater {
            wait_for_exit(pid)?;
        }
        assert!(!state.join("app-server.pid").exists());
        assert!(!state.join("app-server-updater.pid").exists());
        assert_eq!(
            output["managedCodexPath"],
            current
                .join("bin/codex")
                .to_str()
                .context("managed path is not UTF-8")?
        );
        assert_ne!(current.canonicalize()?, original);
        assert!(!dedicated.join("auto-update-version").exists());
        assert_eq!(std::fs::read(state.join("settings.json"))?, settings_before);
        assert_eq!(
            std::fs::read(current.join("bin/codex-code-mode-host"))?,
            b"replacement helper"
        );
        if initial == InitialDaemon::Missing {
            assert_eq!(
                std::fs::read(original.join("bin/codex-code-mode-host"))?,
                b"runtime fixture"
            );
        }
        assert_eq!(daemon.lifecycle("start")?["status"], "alreadyRunning");
        assert_eq!(daemon.lifecycle("stop")?["status"], "stopped");
        assert_eq!(standalone.join("current").canonicalize()?, cli_selection);
    }
    if action == "bootstrap" {
        assert_eq!(output["autoUpdateEnabled"], false);
    }
    Ok(())
}

#[test]
fn packaged_daemon_start_and_explicit_replacement() -> Result<()> {
    packaged_daemon_launch("start", InitialDaemon::Missing)
}

#[test]
fn packaged_daemon_restart_seeds_local_package() -> Result<()> {
    packaged_daemon_launch("restart", InitialDaemon::Missing)
}

#[test]
fn packaged_daemon_bootstrap_seeds_local_package() -> Result<()> {
    packaged_daemon_launch("bootstrap", InitialDaemon::Missing)
}

#[test]
fn packaged_daemon_explicit_replacement_migrates_running_legacy() -> Result<()> {
    packaged_daemon_launch("start", InitialDaemon::Legacy)
}
