use codex_app_server_protocol::InterpreterHarness;
use codex_app_server_protocol::InterpreterHarnessListParams;
use codex_app_server_protocol::InterpreterHarnessListResponse;
use codex_app_server_protocol::InterpreterProvider;
use codex_app_server_protocol::InterpreterProviderListParams;
use codex_app_server_protocol::InterpreterProviderListResponse;
use codex_app_server_protocol::WireApiDto;
use codex_core::config::Config;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;

#[cfg(test)]
#[path = "interpreter_catalog_tests.rs"]
mod tests;

struct HarnessDefinition {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    wire_apis: &'static [WireApi],
}

impl HarnessDefinition {
    fn supports(&self, wire_api: WireApi) -> bool {
        self.wire_apis.contains(&wire_api)
    }
}

const MESSAGES_WIRE_APIS: &[WireApi] = &[WireApi::Messages];
const CHAT_WIRE_APIS: &[WireApi] = &[WireApi::Chat];
const ALL_WIRE_APIS: &[WireApi] = &[WireApi::Responses, WireApi::Chat, WireApi::Messages];

const HARNESS_DEFINITIONS: &[HarnessDefinition] = &[
    HarnessDefinition {
        id: "",
        label: "Native",
        description: "Use the native Open Interpreter tool harness.",
        wire_apis: ALL_WIRE_APIS,
    },
    HarnessDefinition {
        id: "claude-code",
        label: "Claude Code",
        description: "Use the Claude Code-style tool harness.",
        wire_apis: MESSAGES_WIRE_APIS,
    },
    HarnessDefinition {
        id: "claude-code-bare",
        label: "Claude Code Bare",
        description: "Use the lean Claude Code-style harness.",
        wire_apis: MESSAGES_WIRE_APIS,
    },
    HarnessDefinition {
        id: "zcode",
        label: "ZCode",
        description: "Use the ZCode-style tool harness.",
        wire_apis: MESSAGES_WIRE_APIS,
    },
    HarnessDefinition {
        id: "kimi-cli",
        label: "Kimi CLI",
        description: "Use the Kimi CLI-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "kimi-code",
        label: "Kimi Code",
        description: "Use the Kimi Code-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "little-coder",
        label: "Little Coder",
        description: "Use the Little Coder-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "qwen-code",
        label: "Qwen Code",
        description: "Use the Qwen Code-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "deepseek-tui",
        label: "DeepSeek TUI",
        description: "Use the DeepSeek TUI-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "mini-swe-agent",
        label: "mini-swe-agent",
        description: "Use the mini-swe-agent-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "opencode",
        label: "opencode",
        description: "Use the opencode-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "pi",
        label: "Pi",
        description: "Use the Pi-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "local",
        label: "Local",
        description: "Use the local llama.cpp tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "swe-agent",
        label: "SWE-agent",
        description: "Use the SWE-agent-style tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "terminus-2",
        label: "Terminus 2",
        description: "Use the Terminus 2-style terminal harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
    HarnessDefinition {
        id: "minimal",
        label: "Minimal",
        description: "Use a minimal shell-oriented tool harness.",
        wire_apis: CHAT_WIRE_APIS,
    },
];

pub(crate) fn provider_list(
    config: &Config,
    params: InterpreterProviderListParams,
) -> InterpreterProviderListResponse {
    let include_unconfigured = params.include_unconfigured.unwrap_or(true);
    let mut providers: Vec<_> = config
        .model_providers
        .iter()
        .map(|(provider_id, provider)| {
            let configured = true;
            (provider_id, provider, configured)
        })
        .filter(|(_, _, configured)| include_unconfigured || *configured)
        .collect();
    providers.sort_by(|left, right| {
        let left_current = left.0 == &config.model_provider_id;
        let right_current = right.0 == &config.model_provider_id;
        right_current
            .cmp(&left_current)
            .then_with(|| provider_sort_priority(left.0).cmp(&provider_sort_priority(right.0)))
            .then_with(|| left.1.name.cmp(&right.1.name))
    });

    InterpreterProviderListResponse {
        data: providers
            .into_iter()
            .map(|(provider_id, provider, configured)| InterpreterProvider {
                id: provider_id.clone(),
                name: provider.name.clone(),
                description: provider_description(provider_id, provider),
                is_current: provider_id == &config.model_provider_id,
                base_url: provider.base_url.clone(),
                wire_api: Some(map_wire_api(provider.wire_api)),
                env_key: provider.env_key.clone(),
                configured,
                is_default: provider_id == &config.model_provider_id,
            })
            .collect(),
    }
}

pub(crate) fn harness_list(
    config: &Config,
    params: InterpreterHarnessListParams,
) -> InterpreterHarnessListResponse {
    InterpreterHarnessListResponse {
        data: harness_choices(config, &params.provider_id, params.model.as_deref()),
    }
}

fn map_wire_api(wire_api: WireApi) -> WireApiDto {
    match wire_api {
        WireApi::Responses => WireApiDto::Responses,
        WireApi::Chat => WireApiDto::Chat,
        WireApi::Messages => WireApiDto::Messages,
    }
}

fn provider_sort_priority(provider_id: &str) -> u16 {
    codex_model_provider_info::bundled_provider_catalog_entry(provider_id)
        .map_or(u16::MAX, |entry| entry.sort_priority)
}

fn provider_description(provider_id: &str, provider: &ModelProviderInfo) -> String {
    let description = if provider_id == codex_model_provider_info::LOCAL_OSS_PROVIDER_ID {
        "Bundled llama.cpp • GGUF model download required on first use".to_string()
    } else if provider.requires_openai_auth {
        "ChatGPT login".to_string()
    } else if let Some(env_key) = provider.env_key.as_deref() {
        format!("Environment: {env_key}")
    } else if provider.auth.is_some() || provider.experimental_bearer_token.is_some() {
        "Configured credentials".to_string()
    } else {
        match provider.wire_api {
            WireApi::Responses => "Responses API".to_string(),
            WireApi::Chat => "Chat Completions".to_string(),
            WireApi::Messages => "Anthropic Messages".to_string(),
        }
    };
    codex_model_provider_info::default_harness_for_provider_model(
        provider_id,
        provider,
        /*model*/ None,
    )
    .map_or(description.clone(), |harness| {
        format!("{description} | Harness: {harness}")
    })
}

fn harness_choices(
    config: &Config,
    provider_id: &str,
    model: Option<&str>,
) -> Vec<InterpreterHarness> {
    let provider = config.model_providers.get(provider_id);
    let wire_api = provider
        .map(|provider| provider.wire_api)
        .or_else(|| {
            codex_model_provider_info::bundled_provider_catalog_entry(provider_id)
                .map(|entry| entry.wire_api)
        })
        .unwrap_or(WireApi::Chat);
    let recommended = provider
        .and_then(|provider| {
            codex_model_provider_info::default_harness_for_provider_model(
                provider_id,
                provider,
                model,
            )
        })
        .unwrap_or("");

    let mut choices: Vec<_> = HARNESS_DEFINITIONS
        .iter()
        .filter(|definition| definition.supports(wire_api))
        .collect();
    choices.sort_by_key(|definition| usize::from(definition.id != recommended));
    choices
        .into_iter()
        .map(|definition| harness_choice(definition, definition.id == recommended))
        .collect()
}

fn harness_choice(definition: &HarnessDefinition, is_recommended: bool) -> InterpreterHarness {
    let label = if is_recommended {
        format!("{} (recommended)", definition.label)
    } else {
        definition.label.to_string()
    };
    InterpreterHarness {
        id: (!definition.id.is_empty()).then(|| definition.id.to_string()),
        label,
        description: definition.description.to_string(),
        is_recommended,
    }
}
