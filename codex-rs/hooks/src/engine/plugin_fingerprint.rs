//! Best-effort source identity for installed plugins used by hook trust.
//!
//! Hash bundle files, excluding generated Python bytecode, virtual environments,
//! and package metadata. This detects source changes, not tampering inside those
//! runtime artifacts. Cancellation is checked between entries and bounded reads.

use std::fs;
use std::io;
use std::io::Read;

use codex_utils_absolute_path::AbsolutePathBuf;
use sha2::Digest;
use sha2::Sha256;

pub(super) fn plugin_content_hash(
    root: &AbsolutePathBuf,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<String> {
    let check_cancelled = || {
        if cancelled() {
            Err(io::Error::from(io::ErrorKind::Interrupted))
        } else {
            Ok(())
        }
    };
    check_cancelled()?;
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(io::Error::other("plugin root is not a directory"));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut hash = Sha256::new();
    hash.update(b"codex-plugin-bundle-v1");
    let mut buffer = [0_u8; 64 * 1024];
    while let Some(path) = pending.pop() {
        check_cancelled()?;
        let relative_path = path.strip_prefix(root).map_err(io::Error::other)?;
        if relative_path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| {
                matches!(name, "__pycache__" | ".venv")
                    || name.ends_with(".pyc")
                    || name.ends_with(".egg-info")
            })
        {
            continue;
        }
        let name = relative_path.as_os_str().as_encoded_bytes();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            hash.update(b"directory");
            let mut entries = Vec::new();
            for entry in fs::read_dir(&path)? {
                check_cancelled()?;
                entries.push(entry?);
            }
            entries.sort_by_key(fs::DirEntry::file_name);
            pending.extend(entries.iter().rev().map(fs::DirEntry::path));
        } else if metadata.is_file() {
            hash.update(b"file");
            let length = metadata.len();
            hash.update(length.to_le_bytes());
            // Bound each read to the observed size, detecting growth or truncation
            // instead of trusting a partial fingerprint of a changing file.
            check_cancelled()?;
            let mut file = fs::File::open(&path)?.take(length.saturating_add(1));
            let mut bytes_read = 0_u64;
            loop {
                check_cancelled()?;
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                bytes_read += count as u64;
                hash.update(&buffer[..count]);
            }
            if bytes_read != length {
                return Err(io::Error::other("plugin file changed while fingerprinting"));
            }
        } else {
            return Err(io::Error::other(format!(
                "unsupported plugin entry type: {}",
                path.display()
            )));
        }
    }
    check_cancelled()?;
    Ok(format!("sha256:{:x}", hash.finalize()))
}
