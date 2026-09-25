use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;

pub(crate) const MAX_WALK_ENTRIES: usize = 50_000;
pub(crate) const MAX_WALK_DEPTH: usize = 64;
const MAX_SEARCH_FILE_BYTES: u64 = 1_048_576;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkEntryKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WalkEntry {
    pub(crate) path: PathBuf,
    pub(crate) kind: WalkEntryKind,
}

pub(crate) fn primary_cwd(invocation: &ToolInvocation) -> PathBuf {
    invocation
        .step_context
        .environments
        .primary()
        .and_then(|environment| environment.cwd().to_abs_path().ok())
        .map(codex_utils_absolute_path::AbsolutePathBuf::into_path_buf)
        .unwrap_or_else(|| {
            #[allow(deprecated)]
            invocation.step_context.turn.cwd.as_path().to_path_buf()
        })
}

pub(crate) fn resolve_model_path(
    invocation: &ToolInvocation,
    path: &str,
) -> Result<PathBuf, FunctionCallError> {
    let path = normalize_model_path_text(path);
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(primary_cwd(invocation).join(path))
    }
}

pub(crate) fn checked_read_path(
    invocation: &ToolInvocation,
    path: &str,
    operation: &str,
) -> Result<PathBuf, FunctionCallError> {
    let path = resolve_model_path(invocation, path)?;
    ensure_read_allowed(invocation, &path, operation)?;
    Ok(path)
}

pub(crate) fn checked_write_path(
    invocation: &ToolInvocation,
    path: &str,
    operation: &str,
) -> Result<PathBuf, FunctionCallError> {
    let path = resolve_model_path(invocation, path)?;
    ensure_write_allowed(invocation, &path, operation)?;
    Ok(path)
}

pub(crate) fn ensure_read_allowed(
    invocation: &ToolInvocation,
    path: &Path,
    operation: &str,
) -> Result<(), FunctionCallError> {
    ensure_allowed(invocation, path, AccessKind::Read, operation)
}

pub(crate) fn ensure_write_allowed(
    invocation: &ToolInvocation,
    path: &Path,
    operation: &str,
) -> Result<(), FunctionCallError> {
    ensure_allowed(invocation, path, AccessKind::Write, operation)
}

pub(crate) fn read_search_file(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    let file_type = metadata.file_type();
    if !file_type.is_file() || file_type.is_symlink() || metadata.len() > MAX_SEARCH_FILE_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

pub(crate) fn bounded_walk(root: &Path) -> io::Result<Vec<WalkEntry>> {
    let root_metadata = fs::symlink_metadata(root)?;
    if root_metadata.file_type().is_symlink() {
        return Ok(Vec::new());
    }
    if root_metadata.is_file() {
        return Ok(vec![WalkEntry {
            path: root.to_path_buf(),
            kind: WalkEntryKind::File,
        }]);
    }
    if !root_metadata.is_dir() || is_ignored_dir(root) {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if depth >= MAX_WALK_DEPTH || entries.len() >= MAX_WALK_ENTRIES {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_file() {
                entries.push(WalkEntry {
                    path,
                    kind: WalkEntryKind::File,
                });
            } else if file_type.is_dir() && !is_ignored_dir(&path) {
                entries.push(WalkEntry {
                    path: path.clone(),
                    kind: WalkEntryKind::Directory,
                });
                queue.push_back((path, depth + 1));
            }
            if entries.len() >= MAX_WALK_ENTRIES {
                break;
            }
        }
    }
    Ok(entries)
}

pub(crate) fn is_ignored_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "node_modules" | "target" | ".venv"))
}

#[derive(Clone, Copy)]
enum AccessKind {
    Read,
    Write,
}

fn ensure_allowed(
    invocation: &ToolInvocation,
    path: &Path,
    access: AccessKind,
    operation: &str,
) -> Result<(), FunctionCallError> {
    let cwd = primary_cwd(invocation);
    let policy = invocation.turn.file_system_sandbox_policy();
    let candidates = policy_candidates_for_path(path);
    let allowed = match access {
        AccessKind::Read => candidates
            .iter()
            .all(|candidate| policy.can_read_local_path_with_cwd(candidate, &cwd)),
        AccessKind::Write => candidates
            .iter()
            .all(|candidate| policy.can_write_local_path_with_cwd(candidate, &cwd)),
    };
    if allowed {
        return Ok(());
    }
    let access_text = match access {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
    };
    Err(FunctionCallError::RespondToModel(format!(
        "{operation} failed: sandbox policy denied {access_text} access to {}",
        path.display()
    )))
}

pub(crate) fn policy_candidates_for_path(path: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![path.to_path_buf()];
    if let Ok(canonical) = fs::canonicalize(path) {
        if canonical != path {
            candidates.push(canonical);
        }
        return candidates;
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name())
        && let Ok(canonical_parent) = fs::canonicalize(parent)
    {
        let candidate = canonical_parent.join(file_name);
        if candidate != path {
            candidates.push(candidate);
        }
    }
    candidates
}

pub(crate) fn normalize_model_path_text(text: &str) -> String {
    text.replace("/private/private/", "/private/")
}
