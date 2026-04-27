#![cfg(all(not(debug_assertions), feature = "startup-network"))]

use crate::legacy_core::config::Config;
use crate::update_action::UpdateAction;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use codex_login::default_client::create_client;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;

use crate::version::CODEX_CLI_VERSION;

pub fn get_upgrade_version(config: &Config) -> Option<String> {
    if !config.check_for_update_on_startup {
        return None;
    }

    let version_file = version_filepath(config);
    let info = read_version_info(&version_file).ok();

    if match &info {
        None => true,
        Some(info) => info.last_checked_at < Utc::now() - Duration::hours(20),
    } {
        // Refresh the cached latest version in the background so TUI startup
        // isn't blocked by a network call.
        tokio::spawn(async move {
            check_for_update(&version_file)
                .await
                .inspect_err(|e| tracing::error!("Failed to update version: {e}"))
        });
    }

    info.and_then(|info| {
        if is_newer(&info.latest_version, CODEX_CLI_VERSION).unwrap_or(false) {
            Some(info.latest_version)
        } else {
            None
        }
    })
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct VersionInfo {
    latest_version: String,
    // ISO-8601 timestamp (RFC3339)
    last_checked_at: DateTime<Utc>,
}

const VERSION_FILENAME: &str = "version.json";
const AUTO_UPDATE_MARKER_FILENAME: &str = "update-installed.json";
const AUTO_UPDATE_LOCK_FILENAME: &str = "update-running.lock";
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/KillianLucas/oix/releases/latest";
const RELEASES_URL: &str = "https://api.github.com/repos/KillianLucas/oix/releases";

#[derive(Deserialize, Debug, Clone)]
struct ReleaseInfo {
    tag_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct InstalledUpdateInfo {
    version: String,
}

fn version_filepath(config: &Config) -> PathBuf {
    config.codex_home.join(VERSION_FILENAME).into_path_buf()
}

fn read_version_info(version_file: &Path) -> anyhow::Result<VersionInfo> {
    let contents = std::fs::read_to_string(version_file)?;
    Ok(serde_json::from_str(&contents)?)
}

async fn check_for_update(version_file: &Path) -> anyhow::Result<()> {
    let latest_tag_name = latest_release_tag_name().await?;
    let latest_version = extract_version_from_latest_tag(&latest_tag_name)?;

    let info = VersionInfo {
        latest_version,
        last_checked_at: Utc::now(),
    };

    let json_line = format!("{}\n", serde_json::to_string(&info)?);
    if let Some(parent) = version_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(version_file, json_line).await?;
    Ok(())
}

async fn latest_release_tag_name() -> anyhow::Result<String> {
    let client = create_client();
    let latest_response = client.get(LATEST_RELEASE_URL).send().await?;
    if latest_response.status().as_u16() != 404 {
        let ReleaseInfo { tag_name } = latest_response
            .error_for_status()?
            .json::<ReleaseInfo>()
            .await?;
        return Ok(tag_name);
    }

    // GitHub's /latest endpoint excludes prereleases. During early 0.x release
    // testing, fall back to the release list so self-update still has a channel.
    let releases = client
        .get(RELEASES_URL)
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<ReleaseInfo>>()
        .await?;
    releases
        .into_iter()
        .map(|release| release.tag_name)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No Open Interpreter releases found"))
}

fn is_newer(latest: &str, current: &str) -> Option<bool> {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => Some(l > c),
        _ => None,
    }
}

fn extract_version_from_latest_tag(latest_tag_name: &str) -> anyhow::Result<String> {
    latest_tag_name
        .strip_prefix('v')
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse latest tag name '{latest_tag_name}'"))
}

pub fn spawn_auto_update_if_needed(config: &Config) {
    if !config.check_for_update_on_startup {
        return;
    }
    let Some(update_action) = crate::update_action::get_update_action() else {
        return;
    };
    let marker_file = update_marker_filepath(config);
    let Some(latest_version) = get_upgrade_version(config) else {
        return;
    };
    let lock_file = update_lock_filepath(config);
    if !try_create_update_lock(&lock_file) {
        return;
    }
    spawn_update_command(update_action, latest_version, marker_file, Some(lock_file));
}

pub fn spawn_manual_update(config: &Config) -> anyhow::Result<()> {
    let Some(update_action) = crate::update_action::get_update_action() else {
        anyhow::bail!(
            "This installation cannot self-update. Install with the standalone installer to enable updates."
        );
    };
    let marker_file = update_marker_filepath(config);
    let lock_file = update_lock_filepath(config);
    if !try_create_update_lock(&lock_file) {
        anyhow::bail!("An Open Interpreter update is already running.");
    }
    spawn_update_command(
        update_action,
        "latest".to_string(),
        marker_file,
        Some(lock_file),
    );
    Ok(())
}

pub fn take_installed_update_notice(config: &Config) -> Option<String> {
    let marker_file = update_marker_filepath(config);
    let contents = std::fs::read_to_string(&marker_file).ok()?;
    let _ = std::fs::remove_file(&marker_file);
    let info: InstalledUpdateInfo = serde_json::from_str(&contents).ok()?;
    Some(format!("Updated to Open Interpreter {}.", info.version))
}

fn update_marker_filepath(config: &Config) -> PathBuf {
    config
        .codex_home
        .join(AUTO_UPDATE_MARKER_FILENAME)
        .into_path_buf()
}

fn update_lock_filepath(config: &Config) -> PathBuf {
    config
        .codex_home
        .join(AUTO_UPDATE_LOCK_FILENAME)
        .into_path_buf()
}

fn try_create_update_lock(lock_file: &Path) -> bool {
    if let Some(parent) = lock_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_file)
        .is_ok()
}

fn spawn_update_command(
    update_action: UpdateAction,
    version: String,
    marker_file: PathBuf,
    lock_file: Option<PathBuf>,
) {
    let marker_parent = marker_file.parent().map(Path::to_path_buf);
    std::thread::spawn(move || {
        let (command, args) = update_action.command_args();
        let command_status = std::process::Command::new(command)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match command_status {
            Ok(status) if status.success() => {
                if let Some(parent) = marker_parent {
                    let _ = std::fs::create_dir_all(parent);
                }
                let marker = InstalledUpdateInfo { version };
                if let Ok(json_line) =
                    serde_json::to_string(&marker).map(|line| format!("{line}\n"))
                {
                    let _ = std::fs::write(marker_file, json_line);
                }
            }
            Ok(status) => {
                tracing::warn!("Open Interpreter update command exited with status {status}");
            }
            Err(err) => {
                tracing::warn!("Failed to start Open Interpreter update command: {err}");
            }
        }
        if let Some(lock_file) = lock_file {
            let _ = std::fs::remove_file(lock_file);
        }
    });
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut iter = v.trim().split('.');
    let maj = iter.next()?.parse::<u64>().ok()?;
    let min = iter.next()?.parse::<u64>().ok()?;
    let pat = iter.next()?.parse::<u64>().ok()?;
    Some((maj, min, pat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_version_from_open_interpreter_latest_tag() {
        assert_eq!(
            extract_version_from_latest_tag("v1.5.0").expect("failed to parse version"),
            "1.5.0"
        );
    }

    #[test]
    fn latest_tag_without_known_prefix_is_invalid() {
        assert!(extract_version_from_latest_tag("1.5.0").is_err());
    }

    #[test]
    fn prerelease_version_is_not_considered_newer() {
        assert_eq!(is_newer("0.11.0-beta.1", "0.11.0"), None);
        assert_eq!(is_newer("1.0.0-rc.1", "1.0.0"), None);
    }

    #[test]
    fn plain_semver_comparisons_work() {
        assert_eq!(is_newer("0.11.1", "0.11.0"), Some(true));
        assert_eq!(is_newer("0.11.0", "0.11.1"), Some(false));
        assert_eq!(is_newer("1.0.0", "0.9.9"), Some(true));
        assert_eq!(is_newer("0.9.9", "1.0.0"), Some(false));
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(parse_version(" 1.2.3 \n"), Some((1, 2, 3)));
        assert_eq!(is_newer(" 1.2.3 ", "1.2.2"), Some(true));
    }
}
