use super::HARNESS_DEFINITIONS;
use codex_tools::Harness;
use pretty_assertions::assert_eq;

#[test]
fn app_server_lists_all_known_product_harnesses() {
    let mut actual = HARNESS_DEFINITIONS
        .iter()
        .filter_map(|definition| (!definition.id.is_empty()).then_some(definition.id))
        .collect::<Vec<_>>();
    actual.sort_unstable();

    let mut expected = vec![
        "claude-code",
        "claude-code-bare",
        "deepseek-tui",
        "kimi-cli",
        "kimi-code",
        "little-coder",
        "local",
        "mini-swe-agent",
        "minimal",
        "opencode",
        "pi",
        "qwen-code",
        "swe-agent",
        "terminus-2",
        "zcode",
    ];
    expected.sort_unstable();

    assert_eq!(expected, actual);
    for id in expected {
        assert!(
            !matches!(Harness::from_config_name(Some(id)), Harness::Other(_)),
            "{id} should parse as a known codex_tools::Harness"
        );
    }
}
