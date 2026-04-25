use std::fs;

#[test]
fn interpreter_uses_upstream_tui_thin_client_feature() {
    let manifest = codex_utils_cargo_bin::find_resource!("Cargo.toml")
        .unwrap_or_else(|err| panic!("failed to resolve Cargo.toml runfile: {err}"));
    let contents = fs::read_to_string(&manifest)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest.display()));
    let tui_manifest = manifest
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| {
            panic!(
                "failed to resolve tui manifest parent from {}",
                manifest.display()
            )
        })
        .join("tui/Cargo.toml");
    let tui_contents = fs::read_to_string(&tui_manifest)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", tui_manifest.display()));

    assert!(
        contents.contains(
            r#"codex-tui = { path = "../tui", default-features = false, features = ["thin-client"] }"#
        ),
        "expected thin-client codex-tui dependency in {}",
        manifest.display()
    );
    assert!(
        tui_contents.contains(r#"clipboard = ["dep:arboard"]"#),
        "expected clipboard feature declaration in {}",
        tui_manifest.display()
    );
    assert!(
        tui_contents.contains(r#"thin-client = ["system-browser"]"#),
        "expected thin-client feature to stay minimal in {}",
        tui_manifest.display()
    );
    assert!(
        tui_contents.contains(r#"terminal-default-color-probe = []"#),
        "expected a dedicated terminal-default-color-probe feature in {}",
        tui_manifest.display()
    );
    assert!(
        tui_contents.contains(r#""terminal-default-color-probe","#),
        "expected default codex-tui features to keep terminal-default-color-probe enabled in {}",
        tui_manifest.display()
    );
    assert!(
        tui_contents.contains(r#"cross-session-history = ["codex-core/message-history"]"#),
        "expected cross-session-history to opt into codex-core/message-history in {}",
        tui_manifest.display()
    );
}
