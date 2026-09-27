use codex_api::ApiError;
use codex_api::OpenAiVerbosity;
use codex_api::ResponsesApiRequest;
use codex_api::TextControls;
use codex_api::TextFormat;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ImageReference;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::SearchToolCallParams;
use schemars::JsonSchema;
use schemars::schema_for;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(JsonSchema)]
#[allow(dead_code)]
struct ShellToolCallParams {
    command: Vec<String>,
    timeout_ms: Option<u64>,
    working_directory: Option<String>,
    env: Option<HashMap<String, String>>,
    user: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutputKind {
    Function,
    NamespacedFunction { name: String, namespace: String },
    Custom,
}

pub type ToolKinds = HashMap<String, ToolOutputKind>;
type OriginalFunctionNames = HashMap<(Option<String>, String), String>;

#[cfg(test)]
pub(crate) fn responses_api_tools(tools: Vec<Value>) -> codex_api::ResponsesApiTools {
    let raw_tools =
        serde_json::value::to_raw_value(&tools).expect("tool definitions should serialize");
    codex_api::ResponsesApiTools::from(std::sync::Arc::from(raw_tools))
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatCompletionRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<ChatTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) verbosity: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatMessage {
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatTool {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) function: ChatFunction,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatFunction {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatToolCall {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) function: ChatFunctionCall,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatFunctionCall {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

pub(crate) fn convert_request(
    request: &ResponsesApiRequest,
) -> Result<(ChatCompletionRequest, ToolKinds), ApiError> {
    let response_tools: Vec<Value> = request
        .tools
        .as_ref()
        .map(|tools| serde_json::from_str(tools.as_raw_value().get()))
        .transpose()
        .map_err(|err| ApiError::InvalidRequest {
            message: format!("failed to parse serialized tool definitions: {err}"),
        })?
        .unwrap_or_default();
    let (tools, tool_kinds, original_function_names) = convert_tools(&response_tools)?;
    let mut messages = Vec::new();
    if !request.instructions.trim().is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(json!(request.instructions)),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    let mut pending_reasoning_content: Option<String> = None;
    let mut pending_assistant_content: Option<Value> = None;
    let mut pending_tool_calls: Vec<ChatToolCall> = Vec::new();
    // Responses histories can contain local context updates while tools are running. Chat
    // Completions requires every result for an assistant tool-call group to appear before the
    // next user message, so retain those context updates until the whole group is complete.
    let mut pending_tool_call_ids: Vec<String> = Vec::new();
    let mut deferred_messages: Vec<ChatMessage> = Vec::new();
    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let is_assistant = role == "assistant";
                let converted_content = convert_message_content(content);
                if is_assistant && chat_content_is_empty(&converted_content) {
                    continue;
                }
                if is_assistant {
                    merge_pending_assistant_content(
                        &mut pending_assistant_content,
                        converted_content,
                    );
                    continue;
                }
                let message = ChatMessage {
                    role: chat_message_role(role).to_string(),
                    content: converted_content,
                    reasoning_content: None,
                    tool_calls: None,
                    tool_call_id: None,
                };
                if pending_tool_call_ids.is_empty() {
                    flush_pending_assistant(
                        &mut messages,
                        &mut pending_reasoning_content,
                        &mut pending_assistant_content,
                        &mut pending_tool_calls,
                    );
                    messages.push(message);
                } else {
                    deferred_messages.push(message);
                }
            }
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => {
                pending_tool_call_ids.push(call_id.clone());
                pending_tool_calls.push(ChatToolCall {
                    id: call_id.clone(),
                    type_: "function".to_string(),
                    function: ChatFunctionCall {
                        name: chat_function_call_name(
                            namespace.as_deref(),
                            name,
                            &original_function_names,
                        ),
                        arguments: arguments.clone(),
                    },
                });
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                pending_tool_call_ids.push(call_id.clone());
                pending_tool_calls.push(ChatToolCall {
                    id: call_id.clone(),
                    type_: "function".to_string(),
                    function: ChatFunctionCall {
                        name: name.clone(),
                        arguments: json!({ "input": input }).to_string(),
                    },
                });
            }
            ResponseItem::LocalShellCall {
                id,
                call_id,
                action,
                ..
            } => {
                let call_id = call_id
                    .clone()
                    .or_else(|| id.as_ref().map(ToString::to_string))
                    .ok_or_else(|| ApiError::InvalidRequest {
                        message: "local_shell history item missing call id".to_string(),
                    })?;
                let arguments = match action {
                    LocalShellAction::Exec(exec) => json!({
                        "command": exec.command,
                        "workdir": exec.working_directory,
                        "timeout_ms": exec.timeout_ms,
                    })
                    .to_string(),
                };
                pending_tool_call_ids.push(call_id.clone());
                pending_tool_calls.push(ChatToolCall {
                    id: call_id,
                    type_: "function".to_string(),
                    function: ChatFunctionCall {
                        name: "local_shell".to_string(),
                        arguments,
                    },
                });
            }
            ResponseItem::ToolSearchCall {
                call_id,
                execution,
                arguments,
                ..
            } => {
                let call_id = call_id.clone().unwrap_or_else(|| "tool_search".to_string());
                pending_tool_call_ids.push(call_id.clone());
                pending_tool_calls.push(ChatToolCall {
                    id: call_id,
                    type_: "function".to_string(),
                    function: ChatFunctionCall {
                        name: "tool_search".to_string(),
                        arguments: json!({
                            "execution": execution,
                            "arguments": arguments,
                        })
                        .to_string(),
                    },
                });
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => {
                flush_pending_assistant(
                    &mut messages,
                    &mut pending_reasoning_content,
                    &mut pending_assistant_content,
                    &mut pending_tool_calls,
                );
                push_function_call_output(
                    &mut messages,
                    &mut deferred_messages,
                    &mut pending_tool_call_ids,
                    call_id.as_deref(),
                    tool_output_text(output),
                );
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                flush_pending_assistant(
                    &mut messages,
                    &mut pending_reasoning_content,
                    &mut pending_assistant_content,
                    &mut pending_tool_calls,
                );
                push_tool_output(
                    &mut messages,
                    &mut deferred_messages,
                    &mut pending_tool_call_ids,
                    call_id,
                    tool_output_text(output),
                );
            }
            ResponseItem::ToolSearchOutput {
                call_id,
                status,
                execution,
                tools,
                ..
            } => {
                flush_pending_assistant(
                    &mut messages,
                    &mut pending_reasoning_content,
                    &mut pending_assistant_content,
                    &mut pending_tool_calls,
                );
                let call_id = call_id.clone().unwrap_or_else(|| "tool_search".to_string());
                push_tool_output(
                    &mut messages,
                    &mut deferred_messages,
                    &mut pending_tool_call_ids,
                    &call_id,
                    tool_search_output_text(status, execution, tools),
                );
            }
            ResponseItem::Reasoning { content, .. } => {
                if let Some(text) = reasoning_content_text(content.as_deref()) {
                    pending_reasoning_content = Some(match pending_reasoning_content.take() {
                        Some(existing) => format!("{existing}{text}"),
                        None => text,
                    });
                }
            }
            ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::AgentMessage { .. }
            | ResponseItem::AdditionalTools { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::CompactionTrigger { .. }
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::ConfigurationUpdate { .. }
            | ResponseItem::Other => {}
        }
    }
    flush_pending_assistant(
        &mut messages,
        &mut pending_reasoning_content,
        &mut pending_assistant_content,
        &mut pending_tool_calls,
    );
    for call_id in pending_tool_call_ids {
        messages.push(ChatMessage {
            role: "tool".to_string(),
            content: Some(json!("aborted")),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(call_id),
        });
    }
    messages.append(&mut deferred_messages);
    if let Some(reasoning_content) = pending_reasoning_content.take() {
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: Some(json!("")),
            reasoning_content: Some(reasoning_content),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    let chat_request = ChatCompletionRequest {
        model: request.model.clone(),
        messages,
        stream: request.stream,
        tools,
        tool_choice: Some(request.tool_choice.clone()),
        parallel_tool_calls: Some(request.parallel_tool_calls),
        response_format: convert_response_format(request.text.as_ref()),
        service_tier: request.service_tier.clone(),
        store: Some(request.store),
        // Chat Completions support is the compatibility path. We intentionally avoid forwarding
        // Responses-specific reasoning controls here because real OpenAI chat-completions
        // endpoints reject tool-enabled requests that include reasoning_effort.
        reasoning_effort: None,
        verbosity: request
            .text
            .as_ref()
            .and_then(|text| text.verbosity.clone().map(verbosity_to_string)),
    };

    Ok((chat_request, tool_kinds))
}

fn push_tool_output(
    messages: &mut Vec<ChatMessage>,
    deferred_messages: &mut Vec<ChatMessage>,
    pending_tool_call_ids: &mut Vec<String>,
    call_id: &str,
    output: String,
) {
    messages.push(ChatMessage {
        role: "tool".to_string(),
        content: Some(json!(output)),
        reasoning_content: None,
        tool_calls: None,
        tool_call_id: Some(call_id.to_string()),
    });
    pending_tool_call_ids.retain(|pending_call_id| pending_call_id != call_id);
    if pending_tool_call_ids.is_empty() {
        messages.append(deferred_messages);
    }
}

fn push_function_call_output(
    messages: &mut Vec<ChatMessage>,
    deferred_messages: &mut Vec<ChatMessage>,
    pending_tool_call_ids: &mut Vec<String>,
    call_id: Option<&str>,
    output: String,
) {
    if let Some(call_id) = call_id {
        push_tool_output(
            messages,
            deferred_messages,
            pending_tool_call_ids,
            call_id,
            output,
        );
    } else {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: Some(json!(output)),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }
}

fn flush_pending_assistant(
    messages: &mut Vec<ChatMessage>,
    pending_reasoning_content: &mut Option<String>,
    pending_assistant_content: &mut Option<Value>,
    pending_tool_calls: &mut Vec<ChatToolCall>,
) {
    if pending_assistant_content.is_none() && pending_tool_calls.is_empty() {
        return;
    }

    messages.push(ChatMessage {
        role: "assistant".to_string(),
        content: pending_assistant_content.take(),
        reasoning_content: pending_reasoning_content.take(),
        tool_calls: (!pending_tool_calls.is_empty()).then(|| std::mem::take(pending_tool_calls)),
        tool_call_id: None,
    });
}

fn merge_pending_assistant_content(
    pending_assistant_content: &mut Option<Value>,
    converted_content: Option<Value>,
) {
    let Some(converted_content) = converted_content else {
        return;
    };

    match (pending_assistant_content.take(), converted_content) {
        (None, converted_content) => {
            *pending_assistant_content = Some(converted_content);
        }
        (Some(Value::String(mut existing)), Value::String(next)) => {
            existing.push_str(&next);
            *pending_assistant_content = Some(Value::String(existing));
        }
        (Some(Value::Array(mut existing)), Value::Array(next)) => {
            existing.extend(next);
            *pending_assistant_content = Some(Value::Array(existing));
        }
        (Some(existing), next) => {
            *pending_assistant_content = Some(json!([existing, next,]));
        }
    }
}

fn reasoning_content_text(content: Option<&[ReasoningItemContent]>) -> Option<String> {
    let text = content?
        .iter()
        .map(|item| match item {
            ReasoningItemContent::ReasoningText { text } | ReasoningItemContent::Text { text } => {
                text.as_str()
            }
        })
        .collect::<String>();
    if text.is_empty() { None } else { Some(text) }
}

fn chat_content_is_empty(content: &Option<Value>) -> bool {
    match content {
        None => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(object)) => object
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(str::is_empty),
        Some(Value::Bool(_) | Value::Null | Value::Number(_)) => false,
    }
}

fn chat_message_role(role: &str) -> &str {
    match role {
        "developer" => "user",
        role => role,
    }
}

fn convert_message_content(content: &[ContentItem]) -> Option<Value> {
    if content.is_empty() {
        return None;
    }

    if content.len() == 1 {
        match &content[0] {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                return Some(json!(text));
            }
            ContentItem::InputImage { image, .. } => {
                return Some(json!([convert_image_content(image)]));
            }
            ContentItem::InputAudio { audio_url } => {
                return Some(json!([
                    {
                        "type": "input_audio",
                        "audio_url": audio_url
                    }
                ]));
            }
        }
    }

    Some(Value::Array(
        content
            .iter()
            .map(|item| match item {
                ContentItem::InputText { text } | ContentItem::OutputText { text } => json!({
                    "type": "text",
                    "text": text,
                }),
                ContentItem::InputImage { image, .. } => convert_image_content(image),
                ContentItem::InputAudio { audio_url } => json!({
                    "type": "input_audio",
                    "audio_url": audio_url,
                }),
            })
            .collect(),
    ))
}

fn convert_image_content(image: &ImageReference) -> Value {
    match image {
        ImageReference::Inline { image_url } => json!({
            "type": "image_url",
            "image_url": { "url": image_url },
        }),
        ImageReference::File { file_id } => json!({
            "type": "file",
            "file": { "file_id": file_id },
        }),
    }
}

fn tool_output_text(output: &FunctionCallOutputPayload) -> String {
    output
        .text_content()
        .map(str::to_string)
        .or_else(|| output.content_items().map(|items| json!(items).to_string()))
        .unwrap_or_else(|| output.to_string())
}

fn tool_search_output_text(status: &str, execution: &str, tools: &[Value]) -> String {
    json!({
        "status": status,
        "execution": execution,
        "tools": tools,
    })
    .to_string()
}

fn convert_response_format(text: Option<&TextControls>) -> Option<Value> {
    let TextFormat {
        r#type,
        strict,
        schema,
        name,
    } = text?.format.as_ref()?.clone();
    Some(json!({
        "type": r#type,
        "json_schema": {
            "name": name,
            "schema": schema,
            "strict": strict,
        },
    }))
}

fn convert_tools(
    tools: &[Value],
) -> Result<(Option<Vec<ChatTool>>, ToolKinds, OriginalFunctionNames), ApiError> {
    let mut converted = Vec::new();
    let mut tool_kinds = ToolKinds::new();
    let mut original_function_names = OriginalFunctionNames::new();
    let mut reserved_chat_names = reserve_flat_chat_tool_names(tools);

    for tool in tools {
        let Some(tool_type) = tool.get("type").and_then(Value::as_str) else {
            return Err(ApiError::InvalidRequest {
                message: format!("tool is missing a type field: {tool}"),
            });
        };

        match tool_type {
            "function" => {
                let name = string_field(tool, "name")?;
                converted.push(ChatTool {
                    type_: "function".to_string(),
                    function: ChatFunction {
                        name: name.clone(),
                        description: string_field(tool, "description")
                            .unwrap_or_else(|_| name.clone()),
                        parameters: tool
                            .get("parameters")
                            .cloned()
                            .unwrap_or_else(empty_object_schema),
                    },
                });
                tool_kinds.insert(name.clone(), ToolOutputKind::Function);
                original_function_names.insert((None, name.clone()), name);
            }
            "tool_search" => {
                let name = "tool_search".to_string();
                converted.push(ChatTool {
                    type_: "function".to_string(),
                    function: ChatFunction {
                        name: name.clone(),
                        description: string_field(tool, "description")
                            .unwrap_or_else(|_| "Search available tools".to_string()),
                        parameters: tool
                            .get("parameters")
                            .cloned()
                            .unwrap_or_else(schema_value::<SearchToolCallParams>),
                    },
                });
                tool_kinds.insert(name, ToolOutputKind::Function);
            }
            "local_shell" => {
                let name = "local_shell".to_string();
                converted.push(ChatTool {
                    type_: "function".to_string(),
                    function: ChatFunction {
                        name: name.clone(),
                        description: "Run a shell command in the local environment".to_string(),
                        parameters: schema_value::<ShellToolCallParams>(),
                    },
                });
                tool_kinds.insert(name, ToolOutputKind::Function);
            }
            "custom" => {
                let name = string_field(tool, "name")?;
                let description = string_field(tool, "description")?;
                converted.push(ChatTool {
                    type_: "function".to_string(),
                    function: ChatFunction {
                        name: name.clone(),
                        description,
                        parameters: json!({
                            "type": "object",
                            "properties": {
                                "input": {
                                    "type": "string",
                                }
                            },
                            "required": ["input"],
                            "additionalProperties": false,
                        }),
                    },
                });
                tool_kinds.insert(name, ToolOutputKind::Custom);
            }
            "namespace" => {
                let namespace = string_field(tool, "name")?;
                let Some(namespace_tools) = tool.get("tools").and_then(Value::as_array) else {
                    return Err(ApiError::InvalidRequest {
                        message: format!("namespace tool is missing a tools array: {tool}"),
                    });
                };

                for namespace_tool in namespace_tools {
                    let Some(namespace_tool_type) =
                        namespace_tool.get("type").and_then(Value::as_str)
                    else {
                        return Err(ApiError::InvalidRequest {
                            message: format!(
                                "namespace tool is missing a type field: {namespace_tool}"
                            ),
                        });
                    };
                    match namespace_tool_type {
                        "function" => {
                            let name = string_field(namespace_tool, "name")?;
                            let chat_name = unique_chat_tool_name(
                                &namespaced_chat_tool_name(&namespace, &name),
                                &mut reserved_chat_names,
                            );
                            converted.push(ChatTool {
                                type_: "function".to_string(),
                                function: ChatFunction {
                                    name: chat_name.clone(),
                                    description: string_field(namespace_tool, "description")
                                        .unwrap_or_else(|_| name.clone()),
                                    parameters: namespace_tool
                                        .get("parameters")
                                        .cloned()
                                        .unwrap_or_else(empty_object_schema),
                                },
                            });
                            tool_kinds.insert(
                                chat_name.clone(),
                                ToolOutputKind::NamespacedFunction {
                                    name: name.clone(),
                                    namespace: namespace.clone(),
                                },
                            );
                            original_function_names
                                .insert((Some(namespace.clone()), name), chat_name);
                        }
                        other => {
                            return Err(ApiError::InvalidRequest {
                                message: format!(
                                    "unsupported chat wire namespace tool type: {other}"
                                ),
                            });
                        }
                    }
                }
            }
            "image_generation" => {
                let name = "image_generation".to_string();
                converted.push(ChatTool {
                    type_: "function".to_string(),
                    function: ChatFunction {
                        name: name.clone(),
                        description: "Generate an image from a text prompt".to_string(),
                        parameters: json!({
                            "type": "object",
                            "properties": {
                                "prompt": {
                                    "type": "string",
                                }
                            },
                            "required": ["prompt"],
                            "additionalProperties": false,
                        }),
                    },
                });
                tool_kinds.insert(name, ToolOutputKind::Function);
            }
            "web_search" => {
                let name = "web_search".to_string();
                converted.push(ChatTool {
                    type_: "function".to_string(),
                    function: ChatFunction {
                        name: name.clone(),
                        description: "Search the web for up-to-date information".to_string(),
                        parameters: json!({
                            "type": "object",
                            "properties": {
                                "query": {
                                    "type": "string",
                                }
                            },
                            "required": ["query"],
                            "additionalProperties": false,
                        }),
                    },
                });
                tool_kinds.insert(name, ToolOutputKind::Function);
            }
            other => {
                return Err(ApiError::InvalidRequest {
                    message: format!("unsupported chat wire tool type: {other}"),
                });
            }
        }
    }

    Ok((
        (!converted.is_empty()).then_some(converted),
        tool_kinds,
        original_function_names,
    ))
}

fn chat_function_call_name(
    namespace: Option<&str>,
    name: &str,
    original_function_names: &OriginalFunctionNames,
) -> String {
    original_function_names
        .get(&(namespace.map(str::to_string), name.to_string()))
        .cloned()
        .unwrap_or_else(|| match namespace {
            Some(namespace) => namespaced_chat_tool_name(namespace, name),
            None => name.to_string(),
        })
}

fn reserve_flat_chat_tool_names(tools: &[Value]) -> HashSet<String> {
    let mut reserved = HashSet::new();
    for tool in tools {
        match tool.get("type").and_then(Value::as_str) {
            Some("function") | Some("custom") => {
                if let Some(name) = tool.get("name").and_then(Value::as_str) {
                    reserved.insert(name.to_string());
                }
            }
            Some("tool_search") => {
                reserved.insert("tool_search".to_string());
            }
            Some("local_shell") => {
                reserved.insert("local_shell".to_string());
            }
            Some("image_generation") => {
                reserved.insert("image_generation".to_string());
            }
            Some("web_search") => {
                reserved.insert("web_search".to_string());
            }
            Some("namespace") | Some(_) | None => {}
        }
    }
    reserved
}

fn namespaced_chat_tool_name(namespace: &str, name: &str) -> String {
    let raw_name = if namespace.ends_with('_') || name.starts_with('_') {
        format!("{namespace}{name}")
    } else {
        format!("{namespace}_{name}")
    };
    sanitize_chat_tool_name(&raw_name)
}

fn sanitize_chat_tool_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' => character,
            _ => '_',
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "tool".to_string()
    } else {
        sanitized
    }
}

fn unique_chat_tool_name(base_name: &str, reserved: &mut HashSet<String>) -> String {
    const CHAT_TOOL_NAME_MAX_LEN: usize = 64;
    let base_name = truncate_chat_tool_name(base_name, CHAT_TOOL_NAME_MAX_LEN);
    if reserved.insert(base_name.clone()) {
        return base_name;
    }

    for suffix_number in 2.. {
        let suffix = format!("_{suffix_number}");
        let prefix_len = CHAT_TOOL_NAME_MAX_LEN.saturating_sub(suffix.len());
        let candidate = format!(
            "{}{}",
            truncate_chat_tool_name(base_name.as_str(), prefix_len),
            suffix
        );
        if reserved.insert(candidate.clone()) {
            return candidate;
        }
    }

    unreachable!("unbounded suffix search should find a unique chat tool name")
}

fn truncate_chat_tool_name(name: &str, max_len: usize) -> String {
    name.chars().take(max_len).collect()
}

fn string_field(value: &Value, field: &str) -> Result<String, ApiError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ApiError::InvalidRequest {
            message: format!("tool is missing a string `{field}` field: {value}"),
        })
}

fn empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

fn schema_value<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).unwrap_or_else(|_| empty_object_schema())
}

fn verbosity_to_string(verbosity: OpenAiVerbosity) -> String {
    match verbosity {
        OpenAiVerbosity::Low => "low".to_string(),
        OpenAiVerbosity::Medium => "medium".to_string(),
        OpenAiVerbosity::High => "high".to_string(),
    }
}

#[cfg(test)]
#[path = "request_adjacency_tests.rs"]
mod request_adjacency_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use codex_api::TextFormatType;
    use codex_protocol::ResponseItemId;
    use pretty_assertions::assert_eq;

    #[test]
    fn missing_function_call_id_is_preserved_as_user_context() {
        let mut messages = Vec::new();
        let mut deferred_messages = Vec::new();
        let mut pending_tool_call_ids = vec!["call-1".to_string()];

        push_function_call_output(
            &mut messages,
            &mut deferred_messages,
            &mut pending_tool_call_ids,
            None,
            "untracked external context".to_string(),
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(
            messages[0].content,
            Some(json!("untracked external context"))
        );
        assert_eq!(messages[0].tool_call_id, None);
        assert_eq!(pending_tool_call_ids, vec!["call-1"]);
    }

    #[test]
    fn convert_request_maps_messages_and_tools() {
        let request = ResponsesApiRequest {
            model: "gpt-5.2-codex".to_string(),
            instructions: "be terse".to_string(),
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hello".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }],
            tools: Some(responses_api_tools(vec![json!({
                "type": "function",
                "name": "shell_command",
                "description": "Run a shell command",
                "parameters": { "type": "object" }
            })])),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, tool_kinds) = convert_request(&request).expect("request should convert");

        assert_eq!(chat.model, "gpt-5.2-codex");
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, "system");
        assert_eq!(chat.messages[1].role, "user");
        assert_eq!(chat.messages[1].content, Some(json!("hello")));
        assert_eq!(
            tool_kinds.get("shell_command"),
            Some(&ToolOutputKind::Function)
        );
    }

    #[test]
    fn convert_message_content_preserves_inline_urls_and_file_ids() {
        let inline = ContentItem::InputImage {
            image: ImageReference::Inline {
                image_url: "data:image/png;base64,abc".to_string(),
            },
            detail: None,
        };
        let file = ContentItem::InputImage {
            image: ImageReference::File {
                file_id: "file_123".to_string(),
            },
            detail: None,
        };

        assert_eq!(
            convert_message_content(std::slice::from_ref(&inline)),
            Some(json!([{
                "type": "image_url",
                "image_url": { "url": "data:image/png;base64,abc" },
            }]))
        );
        assert_eq!(
            convert_message_content(&[inline, file]),
            Some(json!([
                {
                    "type": "image_url",
                    "image_url": { "url": "data:image/png;base64,abc" },
                },
                {
                    "type": "file",
                    "file": { "file_id": "file_123" },
                }
            ]))
        );
    }

    #[test]
    fn convert_request_maps_developer_messages_to_user_for_chat() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-flash".to_string(),
            instructions: String::new(),
            input: vec![ResponseItem::Message {
                id: None,
                role: "developer".to_string(),
                content: vec![ContentItem::InputText {
                    text: "keep going".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }],
            tools: Some(responses_api_tools(Vec::new())),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, _) = convert_request(&request).expect("request should convert");

        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
        assert_eq!(chat.messages[0].content, Some(json!("keep going")));
    }

    #[test]
    fn convert_request_replays_reasoning_content_on_next_assistant_tool_call() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-flash".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "list files".to_string(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Reasoning {
                    id: Some(ResponseItemId::with_suffix("rs", "1")),
                    summary: Vec::new(),
                    content: Some(vec![ReasoningItemContent::ReasoningText {
                        text: "I need to inspect the directory.".to_string(),
                    }]),
                    encrypted_content: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "shell".to_string(),
                    namespace: None,
                    arguments: json!({ "command": "ls" }).to_string(),
                    call_id: "call-1".to_string(),
                    encrypted_function_args: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("call-1".to_string()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text("file.txt".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: Some(responses_api_tools(vec![json!({
                "type": "function",
                "name": "shell",
                "description": "Run a command",
                "parameters": { "type": "object" }
            })])),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, _) = convert_request(&request).expect("request should convert");

        let assistant_tool_call = chat
            .messages
            .iter()
            .find(|message| message.tool_calls.is_some())
            .expect("assistant tool call message should be present");
        assert_eq!(assistant_tool_call.role, "assistant");
        assert_eq!(
            assistant_tool_call.reasoning_content.as_deref(),
            Some("I need to inspect the directory.")
        );
    }

    #[test]
    fn convert_request_flattens_namespace_tools_and_preserves_output_mapping() {
        let request = ResponsesApiRequest {
            model: "gpt-5.2-codex".to_string(),
            instructions: String::new(),
            input: vec![ResponseItem::FunctionCall {
                id: None,
                name: "lookup_order".to_string(),
                namespace: Some("mcp__demo__".to_string()),
                arguments: json!({ "order_id": "ord_123" }).to_string(),
                call_id: "call-lookup".to_string(),
                encrypted_function_args: None,
                internal_chat_message_metadata_passthrough: None,
            }],
            tools: Some(responses_api_tools(vec![json!({
                "type": "namespace",
                "name": "mcp__demo__",
                "description": "Demo tools",
                "tools": [
                    {
                        "type": "function",
                        "name": "lookup_order",
                        "description": "Look up an order",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "order_id": { "type": "string" }
                            },
                            "required": ["order_id"],
                            "additionalProperties": false
                        }
                    }
                ]
            })])),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, tool_kinds) = convert_request(&request).expect("request should convert");

        let tool = &chat.tools.as_ref().expect("tools should be converted")[0];
        assert_eq!(tool.function.name, "mcp__demo__lookup_order");
        assert_eq!(tool.function.description, "Look up an order");
        assert_eq!(
            tool.function.parameters,
            json!({
                "type": "object",
                "properties": {
                    "order_id": { "type": "string" }
                },
                "required": ["order_id"],
                "additionalProperties": false
            })
        );
        assert_eq!(
            tool_kinds.get("mcp__demo__lookup_order"),
            Some(&ToolOutputKind::NamespacedFunction {
                name: "lookup_order".to_string(),
                namespace: "mcp__demo__".to_string(),
            })
        );
        assert!(matches!(
            &chat.messages[0],
            ChatMessage {
                tool_calls: Some(tool_calls),
                ..
            } if tool_calls[0].function.name == "mcp__demo__lookup_order"
        ));
    }

    #[test]
    fn convert_tools_sanitizes_namespaced_function_names_for_chat_completions() {
        let tools = vec![json!({
            "type": "namespace",
            "name": "mcp.demo",
            "tools": [
                { "type": "function", "name": "lookup.order" }
            ]
        })];

        let (chat_tools, tool_kinds, _) = convert_tools(&tools).expect("tools should convert");

        let chat_tools = chat_tools.expect("namespace tool should produce a chat tool");
        assert_eq!(chat_tools[0].function.name, "mcp_demo_lookup_order");
        assert_eq!(
            tool_kinds.get("mcp_demo_lookup_order"),
            Some(&ToolOutputKind::NamespacedFunction {
                name: "lookup.order".to_string(),
                namespace: "mcp.demo".to_string(),
            })
        );
    }

    #[test]
    fn convert_tools_dedupes_flattened_name_colliding_with_flat_function() {
        let tools = vec![
            json!({ "type": "function", "name": "codex_app_demo" }),
            json!({
                "type": "namespace",
                "name": "codex_app",
                "tools": [
                    { "type": "function", "name": "demo" }
                ]
            }),
        ];

        let (chat_tools, tool_kinds, _) = convert_tools(&tools).expect("tools should convert");

        let chat_tools = chat_tools.expect("tools should convert");
        let names: Vec<&str> = chat_tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect();
        assert_eq!(names, vec!["codex_app_demo", "codex_app_demo_2"]);
        assert_eq!(
            tool_kinds.get("codex_app_demo"),
            Some(&ToolOutputKind::Function)
        );
        assert_eq!(
            tool_kinds.get("codex_app_demo_2"),
            Some(&ToolOutputKind::NamespacedFunction {
                name: "demo".to_string(),
                namespace: "codex_app".to_string(),
            })
        );
    }

    #[test]
    fn convert_request_replays_reasoning_content_past_empty_assistant_message() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-flash".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::Reasoning {
                    id: Some(ResponseItemId::with_suffix("rs", "1")),
                    summary: Vec::new(),
                    content: Some(vec![ReasoningItemContent::ReasoningText {
                        text: "Need to inspect files.".to_string(),
                    }]),
                    encrypted_content: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: String::new(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Read".to_string(),
                    namespace: None,
                    arguments: json!({ "file_path": "/app/file.txt" }).to_string(),
                    call_id: "call-1".to_string(),
                    encrypted_function_args: None,
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: Some(responses_api_tools(vec![json!({
                "type": "function",
                "name": "Read",
                "description": "Read a file",
                "parameters": { "type": "object" }
            })])),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, _) = convert_request(&request).expect("request should convert");

        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, "assistant");
        assert_eq!(
            chat.messages[0].reasoning_content.as_deref(),
            Some("Need to inspect files.")
        );
        assert!(chat.messages[0].tool_calls.is_some());
        assert_eq!(chat.messages[1].role, "tool");
        assert_eq!(chat.messages[1].content, Some(json!("aborted")));
        assert_eq!(chat.messages[1].tool_call_id.as_deref(), Some("call-1"));
    }

    #[test]
    fn convert_request_groups_parallel_tool_calls_with_reasoning_content() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-flash".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Read".to_string(),
                    namespace: None,
                    arguments: json!({ "file_path": "/app/legacy.py" }).to_string(),
                    call_id: "call-1".to_string(),
                    encrypted_function_args: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Read".to_string(),
                    namespace: None,
                    arguments: json!({ "file_path": "/app/data.csv" }).to_string(),
                    call_id: "call-2".to_string(),
                    encrypted_function_args: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: String::new(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Reasoning {
                    id: Some(ResponseItemId::with_suffix("rs", "1")),
                    summary: Vec::new(),
                    content: Some(vec![ReasoningItemContent::ReasoningText {
                        text: "Need to inspect both files.".to_string(),
                    }]),
                    encrypted_content: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Read".to_string(),
                    namespace: None,
                    arguments: json!({ "file_path": "/app/config.ini" }).to_string(),
                    call_id: "call-3".to_string(),
                    encrypted_function_args: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("call-1".to_string()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text("legacy".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("call-2".to_string()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text("data".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("call-3".to_string()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text("config".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: Some(responses_api_tools(vec![json!({
                "type": "function",
                "name": "Read",
                "description": "Read a file",
                "parameters": { "type": "object" }
            })])),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, _) = convert_request(&request).expect("request should convert");

        assert_eq!(chat.messages.len(), 4);
        assert_eq!(chat.messages[0].role, "assistant");
        assert_eq!(
            chat.messages[0].reasoning_content.as_deref(),
            Some("Need to inspect both files.")
        );
        assert_eq!(chat.messages[0].tool_calls.as_ref().map(Vec::len), Some(3));
        assert_eq!(chat.messages[1].role, "tool");
        assert_eq!(chat.messages[2].role, "tool");
        assert_eq!(chat.messages[3].role, "tool");
    }

    #[test]
    fn convert_request_groups_assistant_text_with_following_tool_call() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-flash".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::Reasoning {
                    id: Some(ResponseItemId::with_suffix("rs", "1")),
                    summary: Vec::new(),
                    content: Some(vec![ReasoningItemContent::ReasoningText {
                        text: "Need one more directory listing.".to_string(),
                    }]),
                    encrypted_content: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "I will inspect the directory.".to_string(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Bash".to_string(),
                    namespace: None,
                    arguments: json!({ "command": "ls" }).to_string(),
                    call_id: "call-1".to_string(),
                    encrypted_function_args: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: " Then I will inspect hidden files.".to_string(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Bash".to_string(),
                    namespace: None,
                    arguments: json!({ "command": "ls -a" }).to_string(),
                    call_id: "call-2".to_string(),
                    encrypted_function_args: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("call-1".to_string()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text("file.txt".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("call-2".to_string()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text(".git".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: Some(responses_api_tools(vec![json!({
                "type": "function",
                "name": "Bash",
                "description": "Run a command",
                "parameters": { "type": "object" }
            })])),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, _) = convert_request(&request).expect("request should convert");

        assert_eq!(chat.messages.len(), 3);
        assert_eq!(chat.messages[0].role, "assistant");
        assert_eq!(
            chat.messages[0].content,
            Some(json!(
                "I will inspect the directory. Then I will inspect hidden files."
            ))
        );
        assert_eq!(
            chat.messages[0].reasoning_content.as_deref(),
            Some("Need one more directory listing.")
        );
        assert_eq!(chat.messages[0].tool_calls.as_ref().map(Vec::len), Some(2));
        assert_eq!(chat.messages[1].role, "tool");
        assert_eq!(chat.messages[2].role, "tool");
    }

    #[test]
    fn convert_request_serializes_tool_search_outputs_into_tool_messages() {
        let request = ResponsesApiRequest {
            model: "gpt-5.2-codex".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::ToolSearchCall {
                    id: None,
                    call_id: Some("search-1".to_string()),
                    status: Some("completed".to_string()),
                    execution: "client".to_string(),
                    arguments: json!({ "query": "search tools" }),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::ToolSearchOutput {
                    id: None,
                    call_id: Some("search-1".to_string()),
                    status: "completed".to_string(),
                    execution: "client".to_string(),
                    tools: vec![json!({ "name": "shell", "type": "function" })],
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: Some(responses_api_tools(vec![json!({
                "type": "tool_search",
                "description": "Search available tools"
            })])),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: None,
        };

        let (chat, _) = convert_request(&request).expect("request should convert");

        let tool_message = chat
            .messages
            .last()
            .expect("tool result message should be present");
        assert_eq!(tool_message.role, "tool");
        assert_eq!(tool_message.tool_call_id.as_deref(), Some("search-1"));
        let content = tool_message
            .content
            .as_ref()
            .and_then(Value::as_str)
            .expect("tool message content should be a string");
        let payload: Value =
            serde_json::from_str(content).expect("tool message content should be valid json");
        assert_eq!(
            payload,
            json!({
                "status": "completed",
                "execution": "client",
                "tools": [{ "name": "shell", "type": "function" }],
            })
        );
    }

    #[test]
    fn convert_request_rebuilds_chat_completions_response_format() {
        let request = ResponsesApiRequest {
            model: "gpt-5.2-codex".to_string(),
            instructions: String::new(),
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "return structured output".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }],
            tools: Some(responses_api_tools(Vec::new())),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            stream_options: None,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
            access_programs: None,
            text: Some(TextControls {
                verbosity: None,
                format: Some(TextFormat {
                    r#type: TextFormatType::JsonSchema,
                    strict: true,
                    schema: json!({
                        "type": "object",
                        "properties": {
                            "answer": { "type": "string" }
                        },
                        "required": ["answer"],
                        "additionalProperties": false,
                    }),
                    name: "codex_output_schema".to_string(),
                }),
            }),
        };

        let (chat, _) = convert_request(&request).expect("request should convert");

        assert_eq!(
            chat.response_format,
            Some(json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "codex_output_schema",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "answer": { "type": "string" }
                        },
                        "required": ["answer"],
                        "additionalProperties": false,
                    },
                    "strict": true,
                },
            }))
        );
    }
}
