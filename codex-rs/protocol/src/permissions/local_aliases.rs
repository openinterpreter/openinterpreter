//! Prepare local policy paths in the sandbox's trusted system-alias namespace.
//! Normalize grants and restrictions once per operation without resolving mutable symlinks.
//! Check native macOS ASCII aliases before approving protected metadata writes.
//! Borrow unchanged policies and return normalization failures to fallible callers.

use super::FileSystemAccessMode;
use super::FileSystemPath;
use super::FileSystemSandboxEntry;
use super::FileSystemSandboxPolicy;
use super::FileSystemSandboxPolicyContext;
use super::FileSystemSpecialPath;
#[cfg(target_os = "macos")]
use super::PROTECTED_METADATA_PATH_NAMES;
use super::local_temporary_directories;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path_uri::PathUri;
use std::borrow::Cow;
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::io;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "macos")]
use std::path::Component;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;

/// Owns native environment bindings so one matcher can serve a whole root calculation.
pub(super) struct LocalPolicyContext {
    cwd: PathUri,
    user_home_dir: Option<PathUri>,
    temporary_directories: Vec<PathUri>,
}

impl LocalPolicyContext {
    pub(super) fn new(cwd: &Path) -> Option<Self> {
        Some(Self {
            cwd: PathUri::from(AbsolutePathBuf::from_absolute_path(cwd).ok()?),
            user_home_dir: PathUri::from_host_native_path("~").ok(),
            temporary_directories: local_temporary_directories(),
        })
    }

    pub(super) fn as_context(&self) -> FileSystemSandboxPolicyContext<'_> {
        FileSystemSandboxPolicyContext {
            cwd: &self.cwd,
            workspace_roots: std::slice::from_ref(&self.cwd),
            user_home_dir: self.user_home_dir.as_ref(),
            temporary_directories: Some(&self.temporary_directories),
        }
    }
}

/// An operation-scoped view of a configured policy with local roots resolved.
/// Reuse it for each target in the operation; prepare again when policy or
/// environment bindings change. Metadata checks also account for native ASCII
/// case aliases without changing executor-independent path comparisons.
pub struct LocalFileSystemPolicyMatcher<'a> {
    policy: Cow<'a, FileSystemSandboxPolicy>,
    context: FileSystemSandboxPolicyContext<'a>,
}

impl FileSystemSandboxPolicy {
    /// Prepare local permission matching, resolving trusted aliases on macOS.
    /// Borrows the configured policy on platforms that do not need normalization.
    /// Remote paths must use `can_write_path` instead: the controller's
    /// filesystem cannot establish aliases on another host.
    pub fn prepare_local_matching<'a>(
        &'a self,
        context: &FileSystemSandboxPolicyContext<'a>,
    ) -> io::Result<LocalFileSystemPolicyMatcher<'a>> {
        let mut policy = Cow::Borrowed(self);
        if cfg!(target_os = "macos") {
            let policy = policy.to_mut();
            // Preserve non-path entries, including globs that constrain full-disk
            // grants. Materialize symbolic roots using the selected environment.
            policy.entries.retain(|entry| {
                matches!(
                    entry.path,
                    FileSystemPath::GlobPattern { .. }
                        | FileSystemPath::Special {
                            value: FileSystemSpecialPath::Root
                                | FileSystemSpecialPath::Minimal
                                | FileSystemSpecialPath::Unknown { .. }
                        }
                )
            });
            for (root, access) in self.resolved_entries(context) {
                if root.is_opaque() {
                    continue;
                }
                let normalized =
                    root.to_abs_path()?
                        .normalize_system_aliases()
                        .map_err(|error| {
                            io::Error::new(
                                error.kind(),
                                format!("failed to normalize {root}: {error}"),
                            )
                        })?;
                policy
                    .entries
                    .push(FileSystemSandboxEntry::new(normalized.into(), access));
            }
        }
        Ok(LocalFileSystemPolicyMatcher {
            policy,
            context: *context,
        })
    }
}

impl LocalFileSystemPolicyMatcher<'_> {
    /// Check write access, preserving failures to establish native path equivalence.
    pub fn can_write_path(&self, path: &PathUri) -> io::Result<bool> {
        self.with_path(path, |path| -> io::Result<bool> {
            let writable = self.policy.can_write_path(path, &self.context);
            #[cfg(target_os = "macos")]
            if writable
                && !self
                    .policy
                    .has_full_disk_write_access_with_context(&self.context)
            {
                let native_path = path.to_abs_path()?;
                for (root, access) in self.policy.resolved_entries(&self.context) {
                    if !access.can_write() || root.is_opaque() {
                        continue;
                    }
                    let root = root.to_abs_path()?;
                    let Some(candidate) = normalize_metadata_case_alias(&native_path, &root)?
                    else {
                        continue;
                    };
                    // This second policy view may only deny an already allowed write.
                    // Normalize both candidate and entries so explicit metadata grants
                    // and narrower restrictions keep their precedence.
                    let mut policy = self.policy.clone().into_owned();
                    for entry in &mut policy.entries {
                        if let FileSystemPath::Path { path } = &mut entry.path
                            && let Some(normalized) =
                                normalize_metadata_case_alias(&path.to_abs_path()?, &root)?
                        {
                            *path = PathUri::from(normalized);
                        }
                    }
                    if !policy.can_write_path(&PathUri::from(candidate), &self.context) {
                        return Ok(false);
                    }
                }
            }
            Ok(writable)
        })?
    }

    pub(super) fn resolve_access(&self, path: &PathUri) -> FileSystemAccessMode {
        self.with_path(path, |path| self.policy.resolve_access(path, &self.context))
            .unwrap_or(FileSystemAccessMode::Deny)
    }

    fn with_path<T>(&self, path: &PathUri, evaluate: impl FnOnce(&PathUri) -> T) -> io::Result<T> {
        let path = if cfg!(target_os = "macos") {
            Cow::Owned(PathUri::from(
                path.to_abs_path()?
                    .normalize_system_aliases()
                    .map_err(|error| {
                        io::Error::new(error.kind(), format!("failed to normalize {path}: {error}"))
                    })?,
            ))
        } else {
            Cow::Borrowed(path)
        };
        Ok(evaluate(&path))
    }
}

// Match only ASCII case aliases leading to protected metadata. Each differing
// component is checked against its containing filesystem, including ancestors
// above mount points. Mutable symlink targets are never substituted into paths.
#[cfg(target_os = "macos")]
fn normalize_metadata_case_alias(
    path: &AbsolutePathBuf,
    root: &AbsolutePathBuf,
) -> io::Result<Option<AbsolutePathBuf>> {
    let mut relative = path.components();
    let Some(Component::Normal(name)) = relative.nth(root.components().count()) else {
        return Ok(None);
    };
    let Some(metadata_name) = PROTECTED_METADATA_PATH_NAMES
        .iter()
        .find(|metadata| name.as_bytes().eq_ignore_ascii_case(metadata.as_bytes()))
    else {
        return Ok(None);
    };
    let mut parent = PathBuf::new();
    for (expected, actual) in root.components().zip(path.components()) {
        if expected != actual
            && (!expected
                .as_os_str()
                .as_bytes()
                .eq_ignore_ascii_case(actual.as_os_str().as_bytes())
                || !is_case_insensitive_filesystem(&parent)?)
        {
            return Ok(None);
        }
        parent.push(expected.as_os_str());
    }
    // Exact metadata names still need root alias normalization even when the
    // metadata's own filesystem is case-sensitive.
    if name.as_bytes() != metadata_name.as_bytes()
        && !is_case_insensitive_filesystem(root.as_path())?
    {
        return Ok(None);
    }
    Ok(Some(root.join(*metadata_name).join(relative.as_path())))
}

#[cfg(target_os = "macos")]
fn is_case_insensitive_filesystem(path: &Path) -> io::Result<bool> {
    // Missing paths inherit their nearest existing ancestor's semantics.
    // An unknown result must never authorize a protected metadata write.
    for ancestor in path.ancestors() {
        let path = CString::new(ancestor.as_os_str().as_bytes())?;
        // SAFETY: path is NUL-terminated and __error points to this thread's
        // errno. Reset it to distinguish an unavailable value from a failure.
        let sensitivity = unsafe {
            *libc::__error() = 0;
            libc::pathconf(path.as_ptr(), libc::_PC_CASE_SENSITIVE)
        };
        match sensitivity {
            0 => return Ok(true),
            1 => return Ok(false),
            -1 => {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(0) {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "filesystem case sensitivity is unavailable",
                    ));
                }
                if error.kind() != io::ErrorKind::NotFound {
                    return Err(error);
                }
            }
            value => {
                return Err(io::Error::other(format!(
                    "unexpected filesystem case sensitivity: {value}",
                )));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no existing metadata parent",
    ))
}

#[cfg(all(test, target_os = "macos"))]
#[path = "local_aliases_tests.rs"]
mod tests;
