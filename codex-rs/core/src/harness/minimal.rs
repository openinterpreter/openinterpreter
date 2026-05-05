use crate::client_common::Prompt;
use crate::event_mapping::is_contextual_user_message_content;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ImageDetail;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningControl;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::Value;
use serde_json::json;

const MINIMAL_SYSTEM_PROMPT: &str = "You are an expert software engineer working in the user's current directory.\nUse the tools to investigate the codebase and complete the task. Read code before changing it, match existing conventions, and verify your work when possible.";

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let mut messages = vec![json!({
        "role": "system",
        "content": MINIMAL_SYSTEM_PROMPT,
    })];
    messages.extend(build_messages(&prompt.get_formatted_input())?);
    let tools = build_tools(&prompt.tools)?;
    let tool_kinds = prompt
        .tools
        .iter()
        .map(|tool| (tool.name().to_string(), ToolOutputKind::Function))
        .collect();

    let mut request = json!({
        "model": model_info.slug,
        "messages": messages,
        "stream": true,
        "stream_options": {
            "include_usage": true,
        },
        "tools": tools,
    });

    if model_info.reasoning_control == ReasoningControl::ThinkingToggle {
        request["thinking"] = json!({
            "type": if matches!(effort, Some(ReasoningEffortConfig::None)) {
                "disabled"
            } else {
                "enabled"
            },
        });
    }

    Ok((request, tool_kinds))
}

fn build_messages(
    items: &[ResponseItem],
) -> Result<impl Iterator<Item = Value>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut pending_tool_calls = Vec::new();
    let mut pending_tool_call_content = String::new();
    let mut pending_reasoning_content = String::new();

    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    if let Some(message_content) = convert_message_content(content) {
                        if message_content.as_str().is_some_and(str::is_empty) {
                            continue;
                        }
                        if !pending_tool_calls.is_empty() {
                            append_message_text(&mut pending_tool_call_content, &message_content);
                            continue;
                        }
                        let mut message = json!({
                            "role": "assistant",
                            "content": message_content,
                        });
                        attach_pending_reasoning_content(
                            &mut message,
                            &mut pending_reasoning_content,
                        );
                        messages.push(message);
                    }
                }
                "user" => {
                    if is_contextual_user_message_content(content) {
                        continue;
                    }
                    flush_pending_tool_calls(
                        &mut messages,
                        &mut pending_tool_calls,
                        &mut pending_tool_call_content,
                        &mut pending_reasoning_content,
                    );
                    if let Some(message_content) = convert_message_content(content) {
                        messages.push(json!({
                            "role": "user",
                            "content": message_content,
                        }));
                    }
                }
                "developer" => {
                    flush_pending_tool_calls(
                        &mut messages,
                        &mut pending_tool_calls,
                        &mut pending_tool_call_content,
                        &mut pending_reasoning_content,
                    );
                    if let Some(message_content) = convert_message_content(content) {
                        messages.push(json!({
                            "role": "user",
                            "content": message_content,
                        }));
                    }
                }
                _ => {}
            },
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => pending_tool_calls.push(json!({
                "type": "function",
                "id": call_id,
                "function": {
                    "name": name,
                    "arguments": arguments,
                }
            })),
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => pending_tool_calls.push(json!({
                "type": "function",
                "id": call_id,
                "function": {
                    "name": name,
                    "arguments": json!({ "input": input }).to_string(),
                }
            })),
            ResponseItem::LocalShellCall {
                id,
                call_id,
                action,
                ..
            } => {
                let call_id = call_id.clone().or_else(|| id.clone()).ok_or_else(|| {
                    serde_json::Error::io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "local_shell history item missing call id",
                    ))
                })?;
                let arguments = match action {
                    LocalShellAction::Exec(exec) => json!({
                        "command": exec.command,
                        "timeout": exec.timeout_ms.map(|timeout_ms| timeout_ms / 1000),
                    })
                    .to_string(),
                };
                pending_tool_calls.push(json!({
                    "type": "function",
                    "id": call_id,
                    "function": {
                        "name": "bash",
                        "arguments": arguments,
                    }
                }));
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut pending_tool_call_content,
                    &mut pending_reasoning_content,
                );
                messages.push(json!({
                    "role": "tool",
                    "content": minimal_tool_output_content(output),
                    "tool_call_id": call_id,
                }));
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut pending_tool_call_content,
                    &mut pending_reasoning_content,
                );
                messages.push(json!({
                    "role": "tool",
                    "content": minimal_tool_output_content(output),
                    "tool_call_id": call_id,
                }));
            }
            ResponseItem::Reasoning { content, .. } => {
                if let Some(content) = content {
                    for entry in content {
                        let text = match entry {
                            ReasoningItemContent::ReasoningText { text }
                            | ReasoningItemContent::Text { text } => text,
                        };
                        pending_reasoning_content.push_str(text);
                    }
                }
            }
            ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::GhostSnapshot { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::Other => {}
        }
    }

    flush_pending_tool_calls(
        &mut messages,
        &mut pending_tool_calls,
        &mut pending_tool_call_content,
        &mut pending_reasoning_content,
    );
    Ok(messages.into_iter())
}

fn build_tools(tools: &[ToolSpec]) -> Result<Vec<Value>, serde_json::Error> {
    let mut converted = Vec::new();
    for tool in tools {
        let ToolSpec::Function(ResponsesApiTool {
            name,
            description,
            parameters,
            ..
        }) = tool
        else {
            continue;
        };
        converted.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
            }
        }));
    }
    Ok(converted)
}

fn flush_pending_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_tool_call_content: &mut String,
    pending_reasoning_content: &mut String,
) {
    if pending_tool_calls.is_empty() {
        return;
    }
    let mut message = json!({
        "role": "assistant",
        "content": std::mem::take(pending_tool_call_content),
        "tool_calls": std::mem::take(pending_tool_calls),
    });
    attach_pending_reasoning_content(&mut message, pending_reasoning_content);
    messages.push(message);
}

fn attach_pending_reasoning_content(message: &mut Value, pending_reasoning_content: &mut String) {
    if pending_reasoning_content.is_empty() {
        return;
    }
    message["reasoning_content"] = json!(std::mem::take(pending_reasoning_content));
}

fn append_message_text(output: &mut String, content: &Value) {
    let text = content
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| content.to_string());
    if text.is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(&text);
}

fn convert_message_content(content: &[ContentItem]) -> Option<Value> {
    let parts = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                Some(json!({ "type": "text", "text": text }))
            }
            ContentItem::InputImage { image_url, detail } => {
                Some(chat_image_content_part(image_url, *detail))
            }
        })
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [] => None,
        [single]
            if single
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "text") =>
        {
            single.get("text").cloned()
        }
        _ => Some(Value::Array(parts)),
    }
}

fn minimal_tool_output_content(output: &FunctionCallOutputPayload) -> Value {
    if let Some(text) = output.text_content() {
        return json!(text);
    }
    if let Some(content_items) = output.content_items() {
        let parts = content_items
            .iter()
            .map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => {
                    json!({ "type": "text", "text": text })
                }
                FunctionCallOutputContentItem::InputImage { image_url, detail } => {
                    chat_image_content_part(image_url, *detail)
                }
            })
            .collect::<Vec<_>>();
        return match parts.as_slice() {
            [] => json!(""),
            [single]
                if single
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "text") =>
            {
                single.get("text").cloned().unwrap_or_else(|| json!(""))
            }
            _ => Value::Array(parts),
        };
    }
    json!(output.to_string())
}

fn chat_image_content_part(image_url: &str, detail: Option<ImageDetail>) -> Value {
    let mut image_url_value = json!({ "url": image_url });
    if let Some(detail) = detail {
        image_url_value["detail"] = json!(detail);
    }
    json!({
        "type": "image_url",
        "image_url": image_url_value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputBody;
    use codex_protocol::models::FunctionCallOutputContentItem;
    use codex_protocol::models::ResponseItem;
    use pretty_assertions::assert_eq;

    fn test_prompt() -> Prompt {
        Prompt {
            input: vec![ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hello".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        }
    }

    fn thinking_toggle_model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "deepseek-v4-pro",
            "display_name": "DeepSeek V4 Pro",
            "description": "desc",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [],
            "reasoning_control": "thinking_toggle",
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "upgrade": null,
            "base_instructions": "ignored",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10000},
            "supports_parallel_tool_calls": true,
            "supports_image_detail_original": false,
            "context_window": 1000000,
            "auto_compact_token_limit": null,
            "experimental_supported_tools": []
        }))
        .expect("deserialize model info")
    }

    fn vision_thinking_toggle_model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "deepseek-vl-test",
            "display_name": "DeepSeek VL Test",
            "description": "desc",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [],
            "reasoning_control": "thinking_toggle",
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "upgrade": null,
            "base_instructions": "ignored",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10000},
            "supports_parallel_tool_calls": true,
            "supports_image_detail_original": false,
            "context_window": 1000000,
            "auto_compact_token_limit": null,
            "experimental_supported_tools": [],
            "input_modalities": ["text", "image"]
        }))
        .expect("deserialize model info")
    }

    #[test]
    fn thinking_toggle_model_sends_enabled_by_default() {
        let (request, _) = build_request(&test_prompt(), &thinking_toggle_model_info(), None)
            .expect("build request");

        assert_eq!(request.get("thinking"), Some(&json!({"type": "enabled"})));
    }

    #[test]
    fn thinking_toggle_model_sends_disabled_for_none_effort() {
        let (request, _) = build_request(
            &test_prompt(),
            &thinking_toggle_model_info(),
            Some(ReasoningEffortConfig::None),
        )
        .expect("build request");

        assert_eq!(request.get("thinking"), Some(&json!({"type": "disabled"})));
    }

    #[test]
    fn developer_messages_are_preserved_as_user_messages() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some("developer".to_string()),
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "<skills_instructions>\n- imagegen\n</skills_instructions>"
                            .to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: Some("user".to_string()),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "$imagegen what is this".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
            ],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) =
            build_request(&prompt, &thinking_toggle_model_info(), None).expect("build request");
        let messages = request
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages");

        assert_eq!(messages[1]["role"], json!("user"));
        assert_eq!(
            messages[1]["content"],
            json!("<skills_instructions>\n- imagegen\n</skills_instructions>")
        );
        assert_eq!(messages[2]["role"], json!("user"));
        assert_eq!(messages[2]["content"], json!("$imagegen what is this"));
    }

    #[test]
    fn reasoning_content_is_passed_back_on_assistant_tool_call_messages() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some("user".to_string()),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "run date".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Reasoning {
                    id: "reasoning".to_string(),
                    summary: vec![],
                    content: Some(vec![ReasoningItemContent::ReasoningText {
                        text: "I should inspect the clock.".to_string(),
                    }]),
                    encrypted_content: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "bash".to_string(),
                    namespace: None,
                    arguments: json!({"command": "date"}).to_string(),
                    call_id: "call-date".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "call-date".to_string(),
                    output: FunctionCallOutputPayload::from_text("Tue Apr 29".to_string()),
                },
                ResponseItem::Message {
                    id: Some("user2".to_string()),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "what did you run?".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
            ],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) =
            build_request(&prompt, &thinking_toggle_model_info(), None).expect("build request");
        let messages = request
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages");

        assert_eq!(messages[2]["role"], json!("assistant"));
        assert_eq!(
            messages[2]["reasoning_content"],
            json!("I should inspect the clock.")
        );
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["name"],
            json!("bash")
        );
    }

    #[test]
    fn image_content_is_preserved_for_vision_models() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![
                    ContentItem::InputText {
                        text: "describe this".to_string(),
                    },
                    ContentItem::InputImage {
                        image_url: "data:image/png;base64,AAA".to_string(),
                        detail: Some(ImageDetail::High),
                    },
                ],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(&prompt, &vision_thinking_toggle_model_info(), None)
            .expect("build request");
        let content = &request["messages"][1]["content"];

        assert_eq!(
            content,
            &json!([
                {"type": "text", "text": "describe this"},
                {
                    "type": "image_url",
                    "image_url": {
                        "url": "data:image/png;base64,AAA",
                        "detail": "high"
                    }
                }
            ])
        );
    }

    #[test]
    fn view_image_tool_output_preserves_image_payload() {
        let output = FunctionCallOutputPayload {
            body: FunctionCallOutputBody::ContentItems(vec![
                FunctionCallOutputContentItem::InputImage {
                    image_url: "data:image/png;base64,BBB".to_string(),
                    detail: Some(ImageDetail::High),
                },
            ]),
            success: Some(true),
        };

        assert_eq!(
            minimal_tool_output_content(&output),
            json!([{
                "type": "image_url",
                "image_url": {
                    "url": "data:image/png;base64,BBB",
                    "detail": "high"
                }
            }])
        );
    }
}
