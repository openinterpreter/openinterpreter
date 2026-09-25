//! Measures local session storage once at standalone app-server startup.
//!
//! The background scan only visits sessions and archived_sessions under CODEX_HOME.
//! It sums regular-file lengths without reading file contents or following symlinks.
//! Incomplete scans emit no samples, and shutdown cancels the scan.
//! The compression tag reflects the effective startup config, not compression completion.

use codex_core::config::Config;
use codex_features::Feature;
use codex_otel::MetricsClient;
use codex_rollout::ARCHIVED_SESSIONS_SUBDIR;
use codex_rollout::SESSIONS_SUBDIR;
use std::io;
use std::path::Path;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const SIZE_BYTES_METRIC: &str = "codex.app_server.codex_home.size_bytes";
// Bucket sizes range from 1 MiB through 1 TiB; larger directories use the overflow bucket.
const SIZE_BYTES_BOUNDARIES: &[f64] = &[
    1_048_576.0,
    10_485_760.0,
    104_857_600.0,
    1_073_741_824.0,
    10_737_418_240.0,
    107_374_182_400.0,
    1_099_511_627_776.0,
];

pub(crate) fn spawn(
    config: &Config,
    metrics: MetricsClient,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    let codex_home = config.codex_home.to_path_buf();
    let compression_enabled = config
        .features
        .enabled(Feature::LocalThreadStoreCompression)
        .to_string();
    tokio::task::spawn_blocking(move || {
        let sizes = match directory_sizes(&codex_home, &shutdown) {
            Ok(sizes) => sizes,
            Err(error) => {
                tracing::debug!(error_kind = ?error.kind(), "Skipping CODEX_HOME size metrics");
                return;
            }
        };
        for (directory, bytes) in [
            (SESSIONS_SUBDIR, sizes.sessions),
            (ARCHIVED_SESSIONS_SUBDIR, sizes.archived_sessions),
        ] {
            let _ = metrics.histogram_with_boundaries(
                SIZE_BYTES_METRIC,
                i64::try_from(bytes).unwrap_or(i64::MAX),
                SIZE_BYTES_BOUNDARIES,
                &[
                    ("directory", directory),
                    ("compression_enabled", compression_enabled.as_str()),
                ],
            );
        }
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DirectorySizes {
    sessions: u64,
    archived_sessions: u64,
}

fn directory_sizes(codex_home: &Path, shutdown: &CancellationToken) -> io::Result<DirectorySizes> {
    let sessions = codex_home.join(SESSIONS_SUBDIR);
    let archived_sessions = codex_home.join(ARCHIVED_SESSIONS_SUBDIR);
    let mut sizes = DirectorySizes::default();
    let mut pending = Vec::new();
    for directory in [&sessions, &archived_sessions] {
        if shutdown.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        // Inspect the roots without following symlinks, just like entries below them.
        let metadata = match std::fs::symlink_metadata(directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if metadata.is_dir() {
            pending.push(directory.to_path_buf());
        }
    }
    while let Some(directory) = pending.pop() {
        if shutdown.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        for entry in std::fs::read_dir(directory)? {
            if shutdown.is_cancelled() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let entry = entry?;
            // DirEntry::metadata does not follow symbolic links.
            let metadata = entry.metadata()?;
            let path = entry.path();
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                let bytes = metadata.len();
                if path.starts_with(&sessions) {
                    sizes.sessions = sizes.sessions.saturating_add(bytes);
                } else if path.starts_with(&archived_sessions) {
                    sizes.archived_sessions = sizes.archived_sessions.saturating_add(bytes);
                }
            }
        }
    }
    Ok(sizes)
}
