//! Windows deny-glob scan bounds shared by policy validation and native ACL expansion.
use codex_utils_path_uri::PathConvention;

/// Literal scan root and maximum traversal depth for a Windows deny glob.
pub struct WindowsDenyReadGlobScan<'a> {
    pub root: &'a str,
    pub pattern_suffix: &'a str,
    pub max_depth: Option<usize>,
}

/// Plans lexical scan bounds without accessing the controller's filesystem.
pub fn windows_deny_read_glob_scan(
    pattern: &str,
    configured_max_depth: Option<usize>,
) -> WindowsDenyReadGlobScan<'_> {
    let first_glob = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
    let literal_prefix = &pattern[..first_glob];
    let (root, pattern_suffix) = match literal_prefix.rfind(['/', '\\']) {
        Some(index) => {
            let drive_root = index > 0 && literal_prefix.as_bytes()[index - 1] == b':';
            let end = if index == 0 || drive_root {
                index + 1
            } else {
                index
            };
            (&literal_prefix[..end], &pattern[index + 1..])
        }
        None => (".", pattern),
    };
    let components = PathConvention::Windows
        .path_segments(pattern_suffix)
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let max_depth = if components.contains(&"**") {
        configured_max_depth
    } else {
        Some(configured_max_depth.map_or(components.len(), |depth| depth.min(components.len())))
    };
    WindowsDenyReadGlobScan {
        root,
        pattern_suffix,
        max_depth,
    }
}
