//! The single integration point between the model client and
//! chat-completions harness emulation.
//!
//! `client.rs` resolves a [`ChatHarnessRoute`] and calls
//! [`build_chat_harness_request`]; everything harness-specific — guidance
//! injection, per-harness request building, and response-stream
//! postprocessing — lives here. Adding a chat harness means adding a route
//! arm in this module, not editing the client.

use codex_chat_wire_compat::ToolKinds;
use codex_protocol::error::CodexErr;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::SessionSource;
use codex_tools::Harness;
use serde_json::Value;

use crate::client_common::Prompt;
use crate::client_common::ResponseStream;
use crate::harness::deepseek_tui::build_request as build_deepseek_tui_request;
use crate::harness::guidance::guidance_for_harness;
use crate::harness::kimi_cli::build_request as build_kimi_cli_request;
use crate::harness::kimi_code::build_request as build_kimi_code_request;
use crate::harness::kimi_code::preserve_reasoning_content as preserve_kimi_code_reasoning_content;
use crate::harness::little_coder::build_request as build_little_coder_request;
use crate::harness::mini_swe_agent::build_request as build_mini_swe_agent_request;
use crate::harness::mini_swe_agent::inject_no_tool_call_format_error as inject_mini_swe_agent_no_tool_call_format_error;
use crate::harness::minimal::build_request as build_minimal_request;
use crate::harness::opencode::build_request as build_opencode_request;
use crate::harness::opencode::build_title_request as build_opencode_title_request;
use crate::harness::opencode::should_generate_title as should_generate_opencode_title;
use crate::harness::pi::build_request as build_pi_request;
use crate::harness::qwen_code::build_request as build_qwen_code_request;
use crate::harness::routing::ChatHarnessRoute;
use crate::harness::swe_agent::build_request as build_swe_agent_request;
use crate::harness::swe_agent::inject_action_calls as inject_swe_agent_action_calls;
use crate::harness::swe_agent::prompt_has_submit_review as swe_agent_prompt_has_submit_review;
use crate::harness::terminus_2::Terminus2RequestKind;
use crate::harness::terminus_2::build_request as build_terminus_2_request;
use crate::harness::terminus_2::inject_action_calls as inject_terminus_2_action_calls;
use crate::harness::terminus_2::prompt_has_completion_confirmation as terminus_2_prompt_has_completion_confirmation;

/// Inputs a chat harness needs to shape one provider request.
pub(crate) struct ChatHarnessTurn<'a> {
    pub(crate) prompt: &'a Prompt,
    pub(crate) harness: &'a Harness,
    pub(crate) harness_guidance: bool,
    pub(crate) model_info: &'a ModelInfo,
    pub(crate) effort: Option<ReasoningEffortConfig>,
    pub(crate) thread_id: &'a str,
    pub(crate) session_source: Option<&'a SessionSource>,
}

/// A fully shaped chat-completions harness request.
pub(crate) struct ChatHarnessRequest {
    pub(crate) request_body: Value,
    pub(crate) tool_kinds: ToolKinds,
    /// Some harnesses fire a separate title-generation request first.
    pub(crate) title_request: Option<Value>,
    pub(crate) postprocess: ChatHarnessPostprocess,
}

/// Harness-specific shaping applied to the provider response stream.
pub(crate) enum ChatHarnessPostprocess {
    None,
    KimiCodeReasoningContent,
    MiniSweAgentNoToolCallFormatError,
    SweAgentActionCalls {
        has_submit_review: bool,
    },
    Terminus2ActionCalls {
        request_kind: Terminus2RequestKind,
        pending_completion: bool,
    },
}

pub(crate) fn build_chat_harness_request(
    route: ChatHarnessRoute,
    turn: ChatHarnessTurn<'_>,
) -> Result<ChatHarnessRequest, CodexErr> {
    let ChatHarnessTurn {
        prompt,
        harness,
        harness_guidance,
        model_info,
        effort,
        thread_id,
        session_source,
    } = turn;
    let guided_prompt = prompt_with_harness_guidance(prompt, harness, harness_guidance);
    let yolo_mode = prompt
        .base_instructions
        .text
        .contains("Approval policy is currently never.");
    let (request_body, tool_kinds, title_request, postprocess) = match route {
        ChatHarnessRoute::DeepSeekTui => {
            let (request_body, tool_kinds) = build_deepseek_tui_request(&guided_prompt, model_info)
                .map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid deepseek-tui request: {err}"))
                })?;
            (request_body, tool_kinds, None, ChatHarnessPostprocess::None)
        }
        ChatHarnessRoute::KimiCli => {
            let (request_body, tool_kinds) = build_kimi_cli_request(
                &guided_prompt,
                model_info,
                effort,
                thread_id,
                session_source,
                yolo_mode,
            )
            .map_err(|err| CodexErr::InvalidRequest(format!("invalid kimi-cli request: {err}")))?;
            (request_body, tool_kinds, None, ChatHarnessPostprocess::None)
        }
        ChatHarnessRoute::KimiCode => {
            let (request_body, tool_kinds) =
                build_kimi_code_request(&guided_prompt, model_info, thread_id).map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid kimi-code request: {err}"))
                })?;
            (
                request_body,
                tool_kinds,
                None,
                ChatHarnessPostprocess::KimiCodeReasoningContent,
            )
        }
        ChatHarnessRoute::LittleCoder => {
            let (request_body, tool_kinds) = build_little_coder_request(&guided_prompt, model_info)
                .map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid little-coder request: {err}"))
                })?;
            (request_body, tool_kinds, None, ChatHarnessPostprocess::None)
        }
        ChatHarnessRoute::MiniSweAgent => {
            let (request_body, tool_kinds) =
                build_mini_swe_agent_request(&guided_prompt, model_info).map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid mini-swe-agent request: {err}"))
                })?;
            (
                request_body,
                tool_kinds,
                None,
                ChatHarnessPostprocess::MiniSweAgentNoToolCallFormatError,
            )
        }
        ChatHarnessRoute::Minimal => {
            let (request_body, tool_kinds) =
                build_minimal_request(&guided_prompt, model_info, effort).map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid minimal request: {err}"))
                })?;
            (request_body, tool_kinds, None, ChatHarnessPostprocess::None)
        }
        ChatHarnessRoute::OpenCode => {
            let title_request = should_generate_opencode_title(&guided_prompt)
                .then(|| build_opencode_title_request(&guided_prompt, model_info));
            let (request_body, tool_kinds) = build_opencode_request(&guided_prompt, model_info)
                .map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid opencode request: {err}"))
                })?;
            (
                request_body,
                tool_kinds,
                title_request,
                ChatHarnessPostprocess::None,
            )
        }
        ChatHarnessRoute::Pi => {
            let (request_body, tool_kinds) = build_pi_request(&guided_prompt, model_info)
                .map_err(|err| CodexErr::InvalidRequest(format!("invalid pi request: {err}")))?;
            (request_body, tool_kinds, None, ChatHarnessPostprocess::None)
        }
        ChatHarnessRoute::QwenCode => {
            let (request_body, tool_kinds) =
                build_qwen_code_request(&guided_prompt, model_info, effort, thread_id, yolo_mode)
                    .map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid qwen-code request: {err}"))
                })?;
            (request_body, tool_kinds, None, ChatHarnessPostprocess::None)
        }
        ChatHarnessRoute::SweAgent => {
            let (request_body, tool_kinds) = build_swe_agent_request(&guided_prompt, model_info)
                .map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid swe-agent request: {err}"))
                })?;
            (
                request_body,
                tool_kinds,
                None,
                ChatHarnessPostprocess::SweAgentActionCalls {
                    has_submit_review: swe_agent_prompt_has_submit_review(&guided_prompt),
                },
            )
        }
        ChatHarnessRoute::Terminus2 => {
            let (request_body, tool_kinds, request_kind) =
                build_terminus_2_request(&guided_prompt, model_info).map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid terminus-2 request: {err}"))
                })?;
            (
                request_body,
                tool_kinds,
                None,
                ChatHarnessPostprocess::Terminus2ActionCalls {
                    request_kind,
                    pending_completion: terminus_2_prompt_has_completion_confirmation(
                        &guided_prompt,
                    ),
                },
            )
        }
    };

    Ok(ChatHarnessRequest {
        request_body,
        tool_kinds,
        title_request,
        postprocess,
    })
}

pub(crate) fn apply_chat_harness_postprocess(
    stream: ResponseStream,
    postprocess: ChatHarnessPostprocess,
) -> ResponseStream {
    match postprocess {
        ChatHarnessPostprocess::None => stream,
        ChatHarnessPostprocess::KimiCodeReasoningContent => {
            preserve_kimi_code_reasoning_content(stream)
        }
        ChatHarnessPostprocess::MiniSweAgentNoToolCallFormatError => {
            inject_mini_swe_agent_no_tool_call_format_error(stream)
        }
        ChatHarnessPostprocess::SweAgentActionCalls { has_submit_review } => {
            inject_swe_agent_action_calls(stream, has_submit_review)
        }
        ChatHarnessPostprocess::Terminus2ActionCalls {
            request_kind,
            pending_completion,
        } => inject_terminus_2_action_calls(stream, request_kind, pending_completion),
    }
}

pub(crate) fn prompt_with_harness_guidance<'a>(
    prompt: &'a Prompt,
    harness: &Harness,
    enabled: bool,
) -> std::borrow::Cow<'a, Prompt> {
    if !enabled {
        return std::borrow::Cow::Borrowed(prompt);
    }
    let Some(guidance) = guidance_for_harness(harness) else {
        return std::borrow::Cow::Borrowed(prompt);
    };
    let mut prompt = prompt.clone();
    prompt.base_instructions.text = format!("{guidance}\n\n{}", prompt.base_instructions.text);
    std::borrow::Cow::Owned(prompt)
}
