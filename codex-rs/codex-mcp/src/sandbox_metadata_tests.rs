//! Preserve sandbox policy wire semantics while launcher metadata evolves separately.

use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn sandbox_state_serializes_skip_missing_entries_as_missing_path_behavior() {
    let sandbox_cwd = PathUri::from_host_native_path(
        std::env::current_dir().expect("current directory should be available"),
    )
    .expect("current directory should convert to a URI");
    let sandbox_state = SandboxState {
        permission_profile: PermissionProfile::workspace_write(),
        codex_linux_sandbox_exe: None,
        sandbox_cwd,
        use_legacy_landlock: false,
        use_mxc: false,
    };

    let serialized = serde_json::to_value(&sandbox_state).expect("serialize sandbox state");
    let serialized_text = serde_json::to_string(&serialized).expect("serialize JSON text");
    assert!(
        !serialized_text.contains("generated_default_path"),
        "MCP sandbox metadata must preserve FileSystemPath's stable wire variants"
    );
    assert!(
        !serialized_text.contains("generated_default_special"),
        "MCP sandbox metadata must preserve FileSystemPath's stable wire variants"
    );

    let entries = serialized
        .pointer("/permissionProfile/file_system/entries")
        .and_then(Value::as_array)
        .expect("workspace-write profile should contain filesystem entries");
    let skip_missing_entries = entries
        .iter()
        .filter(|entry| entry.get("missing_path_behavior").and_then(Value::as_str) == Some("skip"))
        .collect::<Vec<_>>();
    assert!(
        !skip_missing_entries.is_empty(),
        "skip-missing entries should be represented as optional missing_path_behavior"
    );
    assert!(
        skip_missing_entries.iter().all(|entry| {
            matches!(
                entry.pointer("/path/type").and_then(Value::as_str),
                Some("path" | "special")
            )
        }),
        "skip-missing entries should use the stable path/special variants"
    );

    let deserialized: SandboxState =
        serde_json::from_value(serialized.clone()).expect("deserialize legacy sandbox state");
    assert_eq!(deserialized, sandbox_state);

    let mut extended = serialized;
    extended["futureField"] = serde_json::json!({"preservedByOpaqueConsumers": true});
    let deserialized: SandboxState = serde_json::from_value(extended).expect("extended state");
    assert_eq!(deserialized, sandbox_state);
}
