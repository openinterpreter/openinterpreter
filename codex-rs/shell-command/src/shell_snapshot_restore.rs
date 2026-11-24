//! Restore brokered values after shell startup and snapshot replay.

use std::collections::HashMap;

pub const SNAPSHOT_BROKERED_VALUE_ENV_PREFIX: &str = "CODEX_NETWORK_PROXY_SNAPSHOT_BROKERED_VALUE_";
pub const SNAPSHOT_BROKERED_UNSET_ENV_PREFIX: &str = "CODEX_NETWORK_PROXY_SNAPSHOT_BROKERED_UNSET_";

/// Restore protected credential copies and unsets after shell startup.
/// Keep the copies for another replay stage, or remove them before the user command.
pub fn build_brokered_credential_exports(
    env: &HashMap<String, String>,
    remove_copies: bool,
) -> String {
    let mut value_copies = env
        .keys()
        .filter_map(|copy_key| {
            let key = copy_key.strip_prefix(SNAPSHOT_BROKERED_VALUE_ENV_PREFIX)?;
            is_valid_shell_variable_name(key).then_some((key, copy_key))
        })
        .collect::<Vec<_>>();
    value_copies.sort_unstable();
    let value_exports = value_copies
        .into_iter()
        .map(|(key, copy_key)| {
            let restore = format!(
                "if [ -z \"${{{copy_key}+x}}\" ]; then exit 1; fi\nif [ -n \"${{{key}+x}}\" ] && [ -n \"${{{key}}}\" ] && [ \"${{{key}}}\" != \"${{{copy_key}}}\" ]; then export {key}=\"${{{copy_key}}}\" || exit 1; fi\nif [ -n \"${{{key}+x}}\" ] && [ -n \"${{{key}}}\" ] && [ \"${{{key}}}\" != \"${{{copy_key}}}\" ]; then exit 1; fi"
            );
            if remove_copies {
                format!("{restore}\nunset {copy_key} || exit 1")
            } else {
                restore
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut unset_markers = env
        .keys()
        .filter_map(|marker_key| {
            let key = marker_key.strip_prefix(SNAPSHOT_BROKERED_UNSET_ENV_PREFIX)?;
            is_valid_shell_variable_name(key).then_some((key, marker_key))
        })
        .collect::<Vec<_>>();
    unset_markers.sort_unstable();
    let unset_exports = unset_markers
        .into_iter()
        .map(|(key, marker_key)| {
            let restore = format!(
                "if [ -z \"${{{marker_key}+x}}\" ]; then exit 1; fi\nif [ -n \"${{{key}+x}}\" ] && [ -n \"${{{key}}}\" ]; then unset {key} || exit 1; fi\nif [ -n \"${{{key}+x}}\" ] && [ -n \"${{{key}}}\" ]; then exit 1; fi"
            );
            if remove_copies {
                format!("{restore}\nunset {marker_key} || exit 1")
            } else {
                restore
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let exports = [value_exports, unset_exports]
        .into_iter()
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if exports.is_empty() {
        return exports;
    }
    format!(
        "case $- in\n  *x*) __CODEX_SNAPSHOT_BROKER_XTRACE=1; set +x ;;\n  *) __CODEX_SNAPSHOT_BROKER_XTRACE= ;;\nesac\n{exports}\nif [ -n \"$__CODEX_SNAPSHOT_BROKER_XTRACE\" ]; then\n  unset __CODEX_SNAPSHOT_BROKER_XTRACE\n  set -x\nelse\n  unset __CODEX_SNAPSHOT_BROKER_XTRACE\nfi"
    )
}

fn is_valid_shell_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
