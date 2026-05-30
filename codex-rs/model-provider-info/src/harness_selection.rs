use crate::ModelProviderInfo;
use crate::WireApi;
use crate::bundled_provider_catalog_entry;
use crate::bundled_provider_catalog_entry_for_base_url;
use crate::default_harness_for_provider_model;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessChoice {
    pub stored: Option<String>,
    pub label: String,
    pub description: String,
    pub is_recommended: bool,
}

/// One harness, in picker order. `id == ""` is the native (Codex) harness.
/// Adding a harness is a single edit here: availability (`wire_apis`), `label`,
/// and `description` all live in this one table rather than three parallel matches.
struct HarnessInfo {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    /// Provider wire APIs that offer this harness.
    wire_apis: &'static [WireApi],
}

const HARNESSES: &[HarnessInfo] = &[
    HarnessInfo {
        id: "",
        label: "Codex",
        description: "Use the native Codex tool harness.",
        wire_apis: &[WireApi::Responses, WireApi::Chat],
    },
    HarnessInfo {
        id: "claude-code",
        label: "Claude Code",
        description: "Use the Claude Code-style tool harness.",
        wire_apis: &[WireApi::Messages, WireApi::Chat],
    },
    HarnessInfo {
        id: "claude-code-bare",
        label: "Claude Code Bare",
        description: "Use the lean Claude Code-style harness.",
        wire_apis: &[WireApi::Messages, WireApi::Chat],
    },
    HarnessInfo {
        id: "kimi-cli",
        label: "Kimi CLI",
        description: "Use the Kimi CLI-style tool harness.",
        wire_apis: &[WireApi::Chat],
    },
    HarnessInfo {
        id: "qwen-code",
        label: "Qwen Code",
        description: "Use the Qwen Code-style tool harness.",
        wire_apis: &[WireApi::Chat],
    },
    HarnessInfo {
        id: "deepseek-tui",
        label: "DeepSeek TUI",
        description: "Use the DeepSeek TUI-style tool harness.",
        wire_apis: &[WireApi::Chat],
    },
    HarnessInfo {
        id: "mini-swe-agent",
        label: "mini-swe-agent",
        description: "Use the mini-swe-agent-style tool harness.",
        wire_apis: &[WireApi::Chat],
    },
    HarnessInfo {
        id: "opencode",
        label: "opencode",
        description: "Use the opencode-style tool harness.",
        wire_apis: &[WireApi::Chat],
    },
    HarnessInfo {
        id: "swe-agent",
        label: "SWE-agent",
        description: "Use the SWE-agent-style tool harness.",
        wire_apis: &[WireApi::Chat],
    },
    HarnessInfo {
        id: "terminus-2",
        label: "Terminus 2",
        description: "Use the Terminus 2-style terminal harness.",
        wire_apis: &[WireApi::Chat],
    },
    HarnessInfo {
        id: "minimal",
        label: "Minimal",
        description: "Use a minimal shell-oriented tool harness.",
        wire_apis: &[WireApi::Chat],
    },
];

impl HarnessInfo {
    fn to_choice(&self, is_recommended: bool) -> HarnessChoice {
        let label = if is_recommended {
            format!("{} (recommended)", self.label)
        } else {
            self.label.to_string()
        };
        HarnessChoice {
            // The native harness is stored as `None`; every other id as-is.
            stored: (!self.id.is_empty()).then(|| self.id.to_string()),
            label,
            description: self.description.to_string(),
            is_recommended,
        }
    }
}

pub fn harness_choices_for_provider_model(
    provider_id: &str,
    provider_name: Option<&str>,
    base_url: Option<&str>,
    wire_api: Option<WireApi>,
    model: Option<&str>,
) -> Vec<HarnessChoice> {
    // Determine the provider's wire API from a single authoritative source so
    // every screen (onboarding, `/harness`, model switcher) offers the same set
    // of harnesses for the same provider. The bundled catalog — looked up by id,
    // then by base URL — is authoritative for built-in providers; only fall back
    // to the caller-supplied `wire_api` for custom providers that aren't in the
    // catalog. Without this, call sites that don't know a provider's wire (e.g.
    // the new-chat onboarding flow, where built-in providers are reserved and
    // absent from `model_providers`) defaulted to `Responses` and only ever
    // offered the Codex harness.
    let wire_api = bundled_provider_catalog_entry(provider_id)
        .or_else(|| base_url.and_then(bundled_provider_catalog_entry_for_base_url))
        .map(|entry| entry.wire_api)
        .or(wire_api)
        .unwrap_or_default();
    let provider = ModelProviderInfo {
        name: provider_name.unwrap_or_default().to_string(),
        base_url: base_url.map(ToOwned::to_owned),
        wire_api,
        ..Default::default()
    };
    let recommended =
        default_harness_for_provider_model(provider_id, &provider, model).unwrap_or("");
    let mut choices: Vec<&HarnessInfo> = HARNESSES
        .iter()
        .filter(|harness| harness.wire_apis.contains(&wire_api))
        .collect();
    // Show the recommended harness first; preserve table order otherwise.
    choices.sort_by_key(|harness| usize::from(harness.id != recommended));
    choices
        .into_iter()
        .map(|harness| harness.to_choice(harness.id == recommended))
        .collect()
}
