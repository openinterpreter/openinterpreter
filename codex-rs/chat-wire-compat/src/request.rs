use codex_api::ApiError;
use codex_api::ResponsesApiRequest;
use codex_api::common::OpenAiVerbosity;
use codex_api::common::TextControls;
use codex_api::common::TextFormat;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::SearchToolCallParams;
use codex_protocol::models::ShellToolCallParams;
use schemars::JsonSchema;
use schemars::schema_for;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutputKind {
    Function,
    NamespacedFunction { name: String, namespace: String },
    Custom,
}

pub type ToolKinds = HashMap<String, ToolOutputKind>;
type OriginalFunctionNames = HashMap<(Option<String>, String), String>;

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
    let (tools, tool_kinds, original_function_names) = convert_tools(&request.tools)?;
    let mut messages = Vec::new();
    if !request.instructions.trim().is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(json!(request.instructions)),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                messages.push(ChatMessage {
                    role: role.clone(),
                    content: convert_message_content(content),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![ChatToolCall {
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
                    }]),
                    tool_call_id: None,
                });
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: call_id.clone(),
                        type_: "function".to_string(),
                        function: ChatFunctionCall {
                            name: name.clone(),
                            arguments: json!({ "input": input }).to_string(),
                        },
                    }]),
                    tool_call_id: None,
                });
            }
            ResponseItem::LocalShellCall {
                id,
                call_id,
                action,
                ..
            } => {
                let call_id = call_id.clone().or_else(|| id.clone()).ok_or_else(|| {
                    ApiError::InvalidRequest {
                        message: "local_shell history item missing call id".to_string(),
                    }
                })?;
                let arguments = match action {
                    LocalShellAction::Exec(exec) => json!({
                        "command": exec.command,
                        "workdir": exec.working_directory,
                        "timeout_ms": exec.timeout_ms,
                    })
                    .to_string(),
                };
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: call_id,
                        type_: "function".to_string(),
                        function: ChatFunctionCall {
                            name: "local_shell".to_string(),
                            arguments,
                        },
                    }]),
                    tool_call_id: None,
                });
            }
            ResponseItem::ToolSearchCall {
                call_id,
                execution,
                arguments,
                ..
            } => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: call_id.clone().unwrap_or_else(|| "tool_search".to_string()),
                        type_: "function".to_string(),
                        function: ChatFunctionCall {
                            name: "tool_search".to_string(),
                            arguments: json!({
                                "execution": execution,
                                "arguments": arguments,
                            })
                            .to_string(),
                        },
                    }]),
                    tool_call_id: None,
                });
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(json!(tool_output_text(output))),
                    tool_calls: None,
                    tool_call_id: Some(call_id.clone()),
                });
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(json!(tool_output_text(output))),
                    tool_calls: None,
                    tool_call_id: Some(call_id.clone()),
                });
            }
            ResponseItem::ToolSearchOutput {
                call_id,
                status,
                execution,
                tools,
            } => {
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(json!(tool_search_output_text(status, execution, tools))),
                    tool_calls: None,
                    tool_call_id: Some(
                        call_id.clone().unwrap_or_else(|| "tool_search".to_string()),
                    ),
                });
            }
            ResponseItem::Reasoning { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::GhostSnapshot { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::Other => {}
        }
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

fn convert_message_content(content: &[ContentItem]) -> Option<Value> {
    if content.is_empty() {
        return None;
    }

    if content.len() == 1 {
        match &content[0] {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                return Some(json!(text));
            }
            ContentItem::InputImage { image_url, .. } => {
                return Some(json!([
                    {
                        "type": "image_url",
                        "image_url": { "url": image_url }
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
                ContentItem::InputImage { image_url, .. } => json!({
                    "type": "image_url",
                    "image_url": { "url": image_url },
                }),
            })
            .collect(),
    ))
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
mod tests {
    use super::*;
    use codex_api::common::TextFormatType;
    use pretty_assertions::assert_eq;

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
                end_turn: None,
                phase: None,
            }],
            tools: vec![json!({
                "type": "function",
                "name": "shell_command",
                "description": "Run a shell command",
                "parameters": { "type": "object" }
            })],
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
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
            }],
            tools: vec![json!({
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
            })],
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
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
                },
                ResponseItem::ToolSearchOutput {
                    call_id: Some("search-1".to_string()),
                    status: "completed".to_string(),
                    execution: "client".to_string(),
                    tools: vec![json!({ "name": "shell", "type": "function" })],
                },
            ],
            tools: vec![json!({
                "type": "tool_search",
                "description": "Search available tools"
            })],
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
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
                end_turn: None,
                phase: None,
            }],
            tools: Vec::new(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            client_metadata: None,
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
