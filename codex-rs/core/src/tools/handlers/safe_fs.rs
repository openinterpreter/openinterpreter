//! Filesystem-safety helpers shared by emulated-harness tool handlers.
//!
//! Several harness emulations hand-rolled their own recursive directory walks
//! for `file_search`/`grep`-style tools. Those walks used `fs::metadata`
//! (which follows symlinks), had no depth or entry cap, and pushed every
//! discovered path into a `Vec`. A single search rooted at a large directory
//! such as `$HOME` could therefore follow a symlink cycle and recurse without
//! bound, exhausting memory. These helpers keep ad-hoc walks bounded and
//! symlink-safe so one tool call can no longer take down the daemon.

use std::path::Path;
use std::path::PathBuf;

/// Maximum number of files a bounded workspace walk collects before it stops
/// descending. A backstop against pathological roots; real project trees are
/// far smaller than this.
pub(super) const BOUNDED_WALK_MAX_FILES: usize = 50_000;

/// Maximum directory depth a bounded workspace walk descends.
pub(super) const BOUNDED_WALK_MAX_DEPTH: usize = 64;

/// Largest file a harness search tool reads into memory while scanning for
/// matches. Larger files are skipped rather than buffered whole.
pub(super) const BOUNDED_SEARCH_MAX_FILE_BYTES: u64 = 1 << 20; // 1 MiB

/// Recursively collect regular files under `root`, bounded and symlink-safe.
///
/// - Never follows symlinks: the directory entry's own file type is used, so a
///   symlinked directory is neither descended into nor collected. This removes
///   the symlink-cycle infinite-recursion hazard.
/// - Stops after [`BOUNDED_WALK_MAX_FILES`] files or [`BOUNDED_WALK_MAX_DEPTH`]
///   levels of depth.
/// - Skips `.git` directories, matching the previous hand-rolled behavior.
///
/// If `root` is itself a regular file the returned vector contains just that
/// path. Unreadable directories are skipped rather than aborting the walk. The
/// traversal is iterative so a deep tree cannot overflow the stack.
pub(super) fn bounded_collect_files(root: &Path) -> Vec<PathBuf> {
    bounded_collect_paths(root, /* include_dirs */ false)
}

/// Bounded, symlink-safe walk of `root`. Collects regular files, and directory
/// paths too when `include_dirs` is set (needed by glob matching). Shares all
/// the safety properties of [`bounded_collect_files`]: never follows symlinks,
/// caps at [`BOUNDED_WALK_MAX_FILES`]/[`BOUNDED_WALK_MAX_DEPTH`], skips `.git`,
/// and is iterative so a deep tree cannot overflow the stack. A file `root`
/// yields just that file.
pub(super) fn bounded_collect_paths(root: &Path, include_dirs: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();

    // `symlink_metadata` so a symlinked root is not silently followed.
    match std::fs::symlink_metadata(root) {
        Ok(meta) if meta.is_file() => {
            out.push(root.to_path_buf());
            return out;
        }
        Ok(meta) if meta.is_dir() => {}
        // Symlink root, missing path, or unreadable: nothing to collect.
        _ => return out,
    }

    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= BOUNDED_WALK_MAX_FILES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // `DirEntry::file_type` reflects the entry itself and does not
            // follow symlinks, so a symlinked directory reports as a symlink.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if out.len() >= BOUNDED_WALK_MAX_FILES {
                break;
            }
            let path = entry.path();
            if file_type.is_dir() {
                let is_git = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == ".git");
                if is_git {
                    continue;
                }
                if include_dirs {
                    out.push(path.clone());
                }
                if depth + 1 <= BOUNDED_WALK_MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if file_type.is_file() {
                out.push(path);
            }
        }
    }

    out
}

/// Match `joined_pattern` (a glob pattern already joined to `base`) against a
/// bounded, symlink-safe walk of `base`. This is a drop-in replacement for
/// `glob::glob` that cannot run away: the `glob` crate follows symlinks and a
/// `**` pattern over a symlink cycle can loop forever, whereas this walks with
/// [`bounded_collect_paths`] (no symlink following, depth/entry caps) and then
/// filters with `glob::Pattern`. `require_literal_separator` is set so `*` does
/// not cross `/`, matching `glob::glob` semantics. `include_dirs` controls
/// whether directory paths are eligible matches.
pub(super) fn bounded_glob_paths(
    base: &Path,
    joined_pattern: &str,
    include_dirs: bool,
) -> Vec<PathBuf> {
    let Ok(pattern) = glob::Pattern::new(joined_pattern) else {
        return Vec::new();
    };
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..glob::MatchOptions::new()
    };
    let mut matches: Vec<PathBuf> = bounded_collect_paths(base, include_dirs)
        .into_iter()
        .filter(|path| pattern.matches_path_with(path, options))
        .collect();
    // Sorted output to match `glob::glob`'s ordering (it sorts each level).
    matches.sort();
    matches
}

/// Read a file for in-memory match scanning, skipping anything that is not a
/// regular file or is larger than [`BOUNDED_SEARCH_MAX_FILE_BYTES`]. Returns
/// `None` for oversized, unreadable, symlinked, or non-UTF-8 files so a single
/// huge file cannot blow up a harness search tool.
pub(super) fn read_searchable_file(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > BOUNDED_SEARCH_MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[cfg(unix)]
    #[test]
    fn bounded_collect_files_does_not_follow_symlink_cycles() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/a.txt"), b"hello").unwrap();
        // A symlink pointing back at the root would cause unbounded recursion
        // if the walk followed it.
        symlink(root, root.join("sub/loop")).unwrap();

        let files = bounded_collect_files(root);

        assert!(files.iter().any(|path| path.ends_with("sub/a.txt")));
        // The symlink itself is never collected or descended into.
        assert!(
            !files
                .iter()
                .any(|path| path.to_string_lossy().contains("loop"))
        );
        assert!(files.len() < BOUNDED_WALK_MAX_FILES);
    }

    #[test]
    fn bounded_collect_files_skips_git_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), b"x").unwrap();
        fs::write(root.join("keep.txt"), b"y").unwrap();

        let files = bounded_collect_files(root);

        assert!(files.iter().any(|path| path.ends_with("keep.txt")));
        assert!(
            !files
                .iter()
                .any(|path| path.to_string_lossy().contains(".git"))
        );
    }

    #[test]
    fn bounded_collect_files_returns_single_file_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file = temp.path().join("only.txt");
        fs::write(&file, b"z").unwrap();

        assert_eq!(bounded_collect_files(&file), vec![file]);
    }

    #[cfg(unix)]
    #[test]
    fn bounded_glob_does_not_follow_symlink_cycles() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), b"fn main() {}").unwrap();
        // `**` over a symlink cycle would loop forever in the `glob` crate.
        symlink(root, root.join("src/loop")).unwrap();

        let joined = root.join("**/*.rs");
        let matches = bounded_glob_paths(root, &joined.to_string_lossy(), false);

        assert!(matches.iter().any(|path| path.ends_with("src/main.rs")));
        assert!(matches.len() < BOUNDED_WALK_MAX_FILES);
    }

    #[test]
    fn bounded_glob_star_does_not_cross_separator() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("top.rs"), b"").unwrap();
        fs::write(root.join("src/main.rs"), b"").unwrap();

        let joined = root.join("*.rs");
        let matches = bounded_glob_paths(root, &joined.to_string_lossy(), false);

        // `*` must not cross `/`, matching `glob::glob` semantics.
        assert!(matches.iter().any(|path| path.ends_with("top.rs")));
        assert!(!matches.iter().any(|path| path.ends_with("src/main.rs")));
    }

    #[test]
    fn read_searchable_file_skips_oversized_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let big = temp.path().join("big.bin");
        fs::write(
            &big,
            vec![b'a'; (BOUNDED_SEARCH_MAX_FILE_BYTES + 1) as usize],
        )
        .unwrap();
        let small = temp.path().join("small.txt");
        fs::write(&small, b"ok").unwrap();

        assert_eq!(read_searchable_file(&big), None);
        assert_eq!(read_searchable_file(&small).as_deref(), Some("ok"));
    }
}
