//! Verifies plugin contents at execution, including one-time legacy approval migration.
//!
//! Runtime discovery stores declarations and approvals without scanning bundles. Each
//! execution verifies fresh contents; dropping its future cancels the scan. Legacy
//! approvals share one immutable bundle baseline per plugin and config home, across
//! hooks, profiles, versions, and sessions.

use codex_utils_absolute_path::AbsolutePathBuf;
use std::fs;
use std::io;
use std::io::Write;

use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use sha2::Digest;
use sha2::Sha256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginHookTrust {
    pub(super) root: AbsolutePathBuf,
    pub(super) declaration_hash: String,
    pub(super) trusted_hash: String,
    pub(super) trusted_content_hash: Option<String>,
    pub(super) legacy_path: Option<AbsolutePathBuf>,
}

impl PluginHookTrust {
    pub(super) async fn verify(&self) -> io::Result<()> {
        let trust = self.clone();
        // Receiver lifetime propagates cancellation even to a queued blocking job.
        let (cancelled, _cancel_on_drop) = tokio::sync::oneshot::channel::<()>();
        tokio::task::spawn_blocking(move || {
            let content_hash =
                super::plugin_fingerprint::plugin_content_hash(&trust.root, &|| {
                    cancelled.is_closed()
                })?;
            if cancelled.is_closed() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            if !trust.matches_hash(&content_hash)? {
                return Err(io::Error::other(
                    "plugin contents changed; hook approval required",
                ));
            }
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }

    pub(super) fn matches_hash(&self, content_hash: &str) -> io::Result<bool> {
        if self.trusted_hash != self.declaration_hash {
            return Ok(false);
        }
        if let Some(expected) = &self.trusted_content_hash {
            if expected != content_hash {
                return Ok(false);
            }
            if let Some(path) = &self.legacy_path {
                // A modern approval also pins the baseline for legacy siblings.
                migrate_legacy_trust(path, content_hash)?;
            }
            return Ok(true);
        }
        if let Some(path) = &self.legacy_path {
            return Ok(migrate_legacy_trust(path, content_hash)? == content_hash);
        }
        Ok(false)
    }
}

pub(super) fn legacy_trust_path(
    stack: &ConfigLayerStack,
    plugin_id: &str,
) -> Option<AbsolutePathBuf> {
    let config_file = stack.all_layers_low_to_high().find_map(|layer| {
        if let ConfigLayerSource::User { file, .. } = &layer.name {
            Some(file)
        } else {
            None
        }
    });
    let config_file = config_file?;
    let parent = config_file.parent()?;
    let mut identity = Sha256::new();
    for part in [
        config_file.as_os_str().as_encoded_bytes(),
        plugin_id.as_bytes(),
    ] {
        identity.update((part.len() as u64).to_le_bytes());
        identity.update(part);
    }
    let directory = parent.join(".hook-trust");
    Some(directory.join(format!("{:x}", identity.finalize())))
}

fn migrate_legacy_trust(path: &AbsolutePathBuf, current_hash: &str) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(baseline) => return Ok(baseline),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("hook trust baseline has no parent"))?;
    fs::create_dir_all(directory.as_path())?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(current_hash.as_bytes())?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(current_hash.to_owned()),
        Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => fs::read_to_string(path),
        Err(err) => Err(err.error),
    }
}
