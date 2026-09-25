use super::MAX_TRUSTED_TOOL_CONTEXT_TOKENS;
use super::TRUSTED_TOOL_PREFIX;
use super::TrustedTool;
use codex_context_fragments::ContextualUserFragment;
use codex_protocol::protocol::TruncationPolicy;

#[test]
fn trusted_tool_context_has_a_hard_token_budget() {
    let fragment = TrustedTool {
        server: "server".into(),
        connector_id: None,
        source: "unbounded instructions ".repeat(1_000),
    };
    let context = fragment.render();
    assert!(context.starts_with(TRUSTED_TOOL_PREFIX));
    assert!(
        context.len() <= TruncationPolicy::Tokens(MAX_TRUSTED_TOOL_CONTEXT_TOKENS).byte_budget()
    );
    assert!(context.contains("<truncated omitted_approx_tokens="));
}
