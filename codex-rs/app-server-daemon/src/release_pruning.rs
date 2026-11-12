//! Tracks dedicated selections under the installer lock. Pruning requires both lifecycle and
//! installer locks and valid history; inspect idle packages before a single best-effort retirement.

use std::io::Write;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem as winfs;

use crate::Daemon;
use crate::backend;
use crate::managed_install;
use crate::settings::DaemonSettings;

#[derive(serde::Deserialize)]
struct Manifest {
    version: semver::Version,
    target: String,
    entrypoint: String,
}

#[cfg(unix)]
fn selection_identity(current: &Path) -> Result<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = current.symlink_metadata()?;
    Ok(format!(
        "{} {} {}.{:09}",
        meta.dev(),
        meta.ino(),
        meta.ctime(),
        meta.ctime_nsec()
    ))
}

#[cfg(windows)]
fn selection_identity(current: &Path) -> Result<String> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let file = std::fs::OpenOptions::new()
        .access_mode(winfs::FILE_READ_ATTRIBUTES)
        .share_mode(winfs::FILE_SHARE_READ | winfs::FILE_SHARE_WRITE | winfs::FILE_SHARE_DELETE)
        .custom_flags(winfs::FILE_FLAG_BACKUP_SEMANTICS | winfs::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(current)?;
    // FSCTL_READ_FILE_USN_DATA, with default V2 record version. The input is always
    // "current", so 1024 bytes accommodates its journal record including the file name.
    let mut record = [0u8; 1024];
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x0009_00eb,
            std::ptr::null(),
            0,
            record.as_mut_ptr().cast(),
            record.len() as u32,
            &raw mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let usn = i64::from_le_bytes(record[24..32].try_into()?);
    anyhow::ensure!(
        returned >= 32 && u16::from_le_bytes(record[4..6].try_into()?) == 2 && usn > 0,
        "no reliable daemon selection journal record"
    );
    let file_id = u64::from_le_bytes(record[8..16].try_into()?);
    Ok(format!("{file_id:x} {usn:x}"))
}

/// Call after changing the selection, while holding the installer lock.
pub(crate) fn record_selection(root: &Path, previous: Option<&Path>) -> Result<()> {
    let current = root.join("current");
    let selected = current.canonicalize()?;
    let previous = previous
        .and_then(Path::file_name)
        .unwrap_or_default()
        .to_string_lossy();
    // If previous came from a shared install, conservatively retain its name here too.
    let name = selected.file_name().unwrap_or_default().to_string_lossy();
    let identity = selection_identity(&current)?;
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    write!(file, "{name}\n{previous}\n{identity}\n")?;
    file.persist(root.join("selection-history"))?;
    Ok(())
}

/// Only ordinary files/directories can belong to a release. Linux device IDs alone
/// cannot distinguish same-device bind mounts; missing procfs disables pruning.
fn owned_mount(path: &Path) -> Result<u64> {
    let meta = path.symlink_metadata()?;
    anyhow::ensure!(
        meta.is_dir() || meta.is_file(),
        "not an ordinary package entry"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        anyhow::ensure!(
            meta.file_attributes() & winfs::FILE_ATTRIBUTE_REPARSE_POINT == 0,
            "package entry is a reparse point"
        );
        // Volume mount points on Windows are reparse points.
        Ok(0)
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", file.as_raw_fd()))?;
        info.lines()
            .find_map(|line| line.strip_prefix("mnt_id:"))
            .context("unknown package mount")?
            .trim()
            .parse()
            .context("invalid package mount")
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(meta.dev())
    }
}

pub(crate) async fn prune(daemon: &Daemon, root: &Path, settings: &DaemonSettings) -> Result<()> {
    let home = daemon
        .settings_file
        .parent()
        .and_then(Path::parent)
        .context("missing Codex home")?;
    // Require a real dedicated root, never a shared alias or an in-progress migration.
    let releases = root.join("releases");
    let mount = owned_mount(&home.join("packages"))?;
    anyhow::ensure!(
        root == home.join("packages/app-server-daemon")
            && owned_mount(root)? == mount
            && owned_mount(&releases)? == mount
            && root.join(".migration-current").symlink_metadata().is_err(),
        "invalid daemon package directory"
    );
    let releases = releases.canonicalize()?;
    let current = root.join("current");
    let selected = current.canonicalize()?;
    let history = std::fs::read_to_string(root.join("selection-history"))?;
    let previous = history
        .lines()
        .nth(1)
        .context("missing previous selection")?;
    let name = selected
        .file_name()
        .context("missing selection")?
        .to_string_lossy();
    anyhow::ensure!(
        selected.parent() == Some(releases.as_path())
            && history == format!("{name}\n{previous}\n{}\n", selection_identity(&current)?)
            && (previous.is_empty()
                || Path::new(previous).file_name().and_then(|n| n.to_str()) == Some(previous)),
        "invalid or stale daemon selection"
    );
    let paths = daemon.current_installation()?.backend_paths(settings);
    let server = backend::pid_backend(paths.clone());
    let updater = backend::pid_update_loop_backend(paths);
    anyhow::ensure!(
        server.is_starting_or_running().await?
            || crate::client::probe(&daemon.socket_path).await.is_err(),
        "unmanaged daemon"
    );
    let mut identities = Vec::new();
    for worker in [server, updater] {
        if worker.is_starting_or_running().await? {
            identities.push(
                worker
                    .running_executable_identity()
                    .await?
                    .and_then(|identity| identity.path_digest)
                    .context("unknown running package")?,
            );
        }
    }
    let caller = std::env::current_exe()?.canonicalize()?;
    'releases: for entry in std::fs::read_dir(&releases)? {
        let entry = entry?;
        let path = entry.path();
        if path == selected
            || entry.file_name() == previous
            || caller.starts_with(&path)
            || !owned_mount(&path).is_ok_and(|id| id == mount)
        {
            continue;
        }
        let Some(manifest) = std::fs::File::open(path.join("codex-package.json"))
            .ok()
            .and_then(|file| serde_json::from_reader::<_, Manifest>(file).ok())
        else {
            continue;
        };
        // Accept only managed release names, never arbitrary or legacy folders.
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let local = name
            .strip_prefix("local-")
            .and_then(|s| s.strip_suffix(&format!("-{}", manifest.target)));
        if !(name == format!("{}-{}", manifest.version, manifest.target)
            || local.is_some_and(|digest| blake3::Hash::from_hex(digest).is_ok()))
            || !matches!(manifest.entrypoint.as_str(), "bin/codex" | "bin/codex.exe")
        {
            continue;
        }
        let Ok(executable) = path.join(manifest.entrypoint).canonicalize() else {
            continue;
        };
        if identities.contains(&managed_install::path_digest(&executable)) {
            continue;
        }
        let mut pending = vec![path.clone()];
        while let Some(child) = pending.pop() {
            // The Unix installer adds this alias; it never traverses outside the release.
            #[cfg(unix)]
            if child == path.join("codex")
                && std::fs::read_link(&child).ok().as_deref() == Some(Path::new("bin/codex"))
            {
                continue;
            }
            if !owned_mount(&child).is_ok_and(|id| id == mount) {
                continue 'releases;
            }
            if child.is_dir() {
                for entry in std::fs::read_dir(child)? {
                    pending.push(entry?.path());
                }
            }
        }
        // Clean up a failed rename, but never retry deletion after retirement.
        let retired = tempfile::Builder::new()
            .prefix(".retired.")
            .tempdir_in(&releases)?;
        std::fs::rename(&path, retired.path().join("release"))?;
        std::fs::remove_dir_all(retired.keep())?;
    }
    Ok(())
}
