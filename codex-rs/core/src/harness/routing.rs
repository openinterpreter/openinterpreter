use codex_model_provider_info::WireApi;
use codex_protocol::error::CodexErr;
use codex_tools::Harness;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum MessagesHarnessRoute {
    ClaudeCode,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ChatHarnessRoute {
    KimiCli,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum StreamTransportRoute {
    ResponsesApi,
    ChatCompletionsCompat,
    ChatHarness(ChatHarnessRoute),
    MessagesHarness(MessagesHarnessRoute),
}

impl StreamTransportRoute {
    pub(crate) fn supports_responses_websocket(self) -> bool {
        matches!(self, Self::ResponsesApi)
    }
}

pub(crate) fn resolve_stream_transport_route(
    wire_api: WireApi,
    harness: &Harness,
) -> Result<StreamTransportRoute, CodexErr> {
    match (wire_api, harness) {
        (WireApi::Responses, Harness::ClaudeCode) | (WireApi::Chat, Harness::ClaudeCode) => {
            Err(CodexErr::InvalidRequest(
                "harness = \"claude-code\" requires a provider with wire_api = \"messages\""
                    .to_string(),
            ))
        }
        (WireApi::Responses, _) => Ok(StreamTransportRoute::ResponsesApi),
        (WireApi::Chat, Harness::KimiCli) => {
            Ok(StreamTransportRoute::ChatHarness(ChatHarnessRoute::KimiCli))
        }
        (WireApi::Chat, _) => Ok(StreamTransportRoute::ChatCompletionsCompat),
        (WireApi::Messages, Harness::ClaudeCode) => Ok(StreamTransportRoute::MessagesHarness(
            MessagesHarnessRoute::ClaudeCode,
        )),
        (WireApi::Messages, Harness::KimiCli) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"kimi-cli\"".to_string(),
        )),
        (WireApi::Messages, Harness::Native) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" requires a harness-native transport; configure harness = \"claude-code\" for Anthropic-style sessions"
                .to_string(),
        )),
        (WireApi::Messages, Harness::Other(harness_name)) => Err(CodexErr::InvalidRequest(
            format!(
                "wire_api = \"messages\" is not supported by harness = \"{harness_name}\""
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn responses_wire_uses_native_responses_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Responses, &Harness::Native)
                .expect("responses route"),
            StreamTransportRoute::ResponsesApi
        );
    }

    #[test]
    fn chat_wire_uses_chat_compat_route_for_non_claude_harnesses() {
        assert_eq!(
            resolve_stream_transport_route(
                WireApi::Chat,
                &Harness::Other("custom-harness".to_string()),
            )
            .expect("chat route"),
            StreamTransportRoute::ChatCompletionsCompat
        );
    }

    #[test]
    fn kimi_cli_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::KimiCli).expect("kimi route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::KimiCli)
        );
    }

    #[test]
    fn messages_wire_requires_claude_code_harness() {
        let err = resolve_stream_transport_route(WireApi::Messages, &Harness::Native)
            .expect_err("messages without harness should fail");

        assert_eq!(
            err.to_string(),
            "wire_api = \"messages\" requires a harness-native transport; configure harness = \"claude-code\" for Anthropic-style sessions"
        );
    }

    #[test]
    fn claude_code_harness_requires_messages_wire() {
        let err = resolve_stream_transport_route(WireApi::Responses, &Harness::ClaudeCode)
            .expect_err("claude-code on responses should fail");

        assert_eq!(
            err.to_string(),
            "harness = \"claude-code\" requires a provider with wire_api = \"messages\""
        );
    }

    #[test]
    fn kimi_cli_harness_rejects_messages_wire() {
        let err = resolve_stream_transport_route(WireApi::Messages, &Harness::KimiCli)
            .expect_err("kimi-cli on messages should fail");

        assert_eq!(
            err.to_string(),
            "wire_api = \"messages\" is not supported by harness = \"kimi-cli\""
        );
    }
}
