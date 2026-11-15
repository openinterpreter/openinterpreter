//! Detects NFS on Linux; other platforms retain their existing filesystem behavior.
//! Closing an extra file descriptor could release process-wide POSIX locks on its inode.

use std::io;
use std::path::Path;

/// On linux we sometimes do a special treatment for NFS related path. We don't support windows NFS
#[cfg(target_os = "linux")]
pub fn is_nfs(path: &Path) -> io::Result<bool> {
    Ok(rustix::fs::statfs(path)?.f_type == rustix::fs::NFS_SUPER_MAGIC)
}

#[cfg(not(target_os = "linux"))]
pub fn is_nfs(_path: &Path) -> io::Result<bool> {
    Ok(false)
}
