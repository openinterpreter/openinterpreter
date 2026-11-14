use crate::client_common::Prompt;
use crate::event_mapping::is_contextual_user_message_content;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use serde_json::Value;
use serde_json::json;
use std::path::Path;

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let mut messages = vec![json!({
        "role": "system",
        "content": build_system_prompt(prompt),
    })];
    messages.extend(build_messages(prompt.get_formatted_input())?);
    let tools = build_tools();
    let tool_kinds = tools
        .iter()
        .filter_map(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(|name| (name.to_string(), ToolOutputKind::Function))
        })
        .collect();

    Ok((
        json!({
            "model": model_info.slug,
            "messages": messages,
            "stream": true,
            "stream_options": {
                "include_usage": true,
            },
            "store": false,
            "tools": tools,
            "thinking": {
                "type": "enabled",
            },
            "reasoning_effort": "high",
        }),
        tool_kinds,
    ))
}

fn build_system_prompt(prompt: &Prompt) -> String {
    let cwd = prompt.cwd.as_deref().unwrap_or_else(|| Path::new("."));
    let date = chrono::Local::now().format("%Y-%m-%d");
    let source_dir = std::env::var("OPEN_INTERPRETER_PI_SOURCE_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("PI_SOURCE_DIR")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "<pi>".to_string());
    format!(
        "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.\n\nAvailable tools:\n- read: Read file contents\n- bash: Execute bash commands (ls, grep, find, etc.)\n- edit: Make precise file edits with exact text replacement, including multiple disjoint edits in one call\n- write: Create or overwrite files\n\nIn addition to the tools above, you may have access to other custom tools depending on the project.\n\nGuidelines:\n- Use bash for file operations like ls, rg, find\n- Use read to examine files instead of cat or sed.\n- Use edit for precise changes (edits[].oldText must match exactly)\n- When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls\n- Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.\n- Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.\n- Use write only for new files or complete rewrites.\n- Be concise in your responses\n- Show file paths clearly when working with files\n\nPi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):\n- Main documentation: <pi>/packages/coding-agent/README.md\n- Additional docs: <pi>/packages/coding-agent/docs\n- Examples: <pi>/packages/coding-agent/examples (extensions, custom tools, SDK)\n- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory\n- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md)\n- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing\n- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)\nCurrent date: {date}\nCurrent working directory: {cwd}",
        cwd = cwd.display()
    )
    .replace("<pi>", &source_dir)
}

pub(crate) fn build_messages(items: &[ResponseItem]) -> Result<Vec<Value>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut pending_tool_calls = Vec::new();
    let mut awaiting_tool_call_ids = Vec::new();
    let mut pending_assistant_content: Option<String> = None;
    let mut pending_reasoning_content: Option<String> = None;

    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    if let Some(message_content) = message_content(content)
                        && !message_content.is_empty()
                    {
                        pending_assistant_content = Some(message_content);
                    }
                }
                "user" => {
                    if is_contextual_user_message_content(content) {
                        continue;
                    }
                    discard_unanswered_tool_calls(
                        &mut pending_tool_calls,
                        &mut awaiting_tool_call_ids,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                    flush_pending_assistant_message(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                    if let Some(message_content) = message_content(content) {
                        messages.push(json!({
                            "role": "user",
                            "content": [
                                {
                                    "type": "text",
                                    "text": message_content,
                                }
                            ],
                        }));
                    }
                }
                "developer" => {}
                _ => {}
            },
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                pending_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": compact_json_arguments(name, arguments),
                    }
                }));
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
                    .ok_or_else(|| {
                        serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "local_shell history item missing call id",
                        ))
                    })?;
                let arguments = match action {
                    LocalShellAction::Exec(exec) => json!({
                        "command": exec.command,
                        "timeout": exec.timeout_ms.map(|timeout| timeout / 1000),
                    })
                    .to_string(),
                };
                pending_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": "bash",
                        "arguments": compact_json_arguments("bash", &arguments),
                    }
                }));
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => {
                let Some(call_id) = call_id.as_ref() else {
                    continue;
                };
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut awaiting_tool_call_ids,
                    &mut pending_assistant_content,
                    &mut pending_reasoning_content,
                );
                if awaiting_tool_call_ids.iter().any(|id| id == call_id) {
                    messages.push(json!({
                        "role": "tool",
                        "content": tool_output_content(output),
                        "tool_call_id": call_id,
                    }));
                    awaiting_tool_call_ids.retain(|id| id != call_id);
                }
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut awaiting_tool_call_ids,
                    &mut pending_assistant_content,
                    &mut pending_reasoning_content,
                );
                if awaiting_tool_call_ids.iter().any(|id| id == call_id) {
                    messages.push(json!({
                        "role": "tool",
                        "content": tool_output_content(output),
                        "tool_call_id": call_id,
                    }));
                    awaiting_tool_call_ids.retain(|id| id != call_id);
                }
            }
            ResponseItem::Reasoning { content, .. } => {
                let text = content
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|part| match part {
                        codex_protocol::models::ReasoningItemContent::ReasoningText { text } => {
                            text.as_str()
                        }
                        codex_protocol::models::ReasoningItemContent::Text { text } => {
                            text.as_str()
                        }
                    })
                    .collect::<String>();
                if !text.is_empty() {
                    pending_reasoning_content = Some(text);
                }
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                let arguments =
                    compact_json_arguments(name, &json!({ "input": input }).to_string());
                pending_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments,
                    }
                }));
            }
            ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::AgentMessage { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::CompactionTrigger { .. }
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::AdditionalTools { .. }
            | ResponseItem::ConfigurationUpdate { .. }
            | ResponseItem::Other => {}
        }
    }
    flush_pending_tool_calls(
        &mut messages,
        &mut pending_tool_calls,
        &mut awaiting_tool_call_ids,
        &mut pending_assistant_content,
        &mut pending_reasoning_content,
    );
    flush_pending_assistant_message(
        &mut messages,
        &mut pending_assistant_content,
        &mut pending_reasoning_content,
    );
    Ok(messages)
}

fn message_content(content: &[ContentItem]) -> Option<String> {
    let mut text = String::new();
    for item in content {
        match item {
            ContentItem::InputText { text: value }
            | ContentItem::OutputText { text: value, .. } => {
                text.push_str(value);
            }
            ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => {}
        }
    }
    Some(text)
}

fn tool_output_content(output: &FunctionCallOutputPayload) -> String {
    let text = output.body.to_text().unwrap_or_default();
    text.strip_prefix(crate::tools::handlers::HARNESS_NO_TRUNCATE_PREFIX)
        .unwrap_or(&text)
        .to_string()
}

fn compact_json_arguments(tool_name: &str, arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(arguments) else {
        return arguments.to_string();
    };
    match tool_name {
        "read" => compact_fields(&value, &["path", "offset", "limit"]),
        "bash" => compact_fields(&value, &["command", "timeout"]),
        "edit" => compact_edit_fields(&value),
        "write" => compact_fields(&value, &["path", "content"]),
        _ => serde_json::to_string(&value).unwrap_or_else(|_| arguments.to_string()),
    }
}

fn compact_edit_fields(value: &Value) -> String {
    let mut fields = Vec::new();
    if let Some(path) = value
        .get("path")
        .and_then(|field| serde_json::to_string(field).ok())
    {
        fields.push(format!("\"path\":{path}"));
    }
    if let Some(edits) = value.get("edits").and_then(Value::as_array) {
        let edits = edits
            .iter()
            .map(|edit| compact_fields(edit, &["oldText", "newText"]))
            .collect::<Vec<_>>();
        fields.push(format!("\"edits\":[{}]", edits.join(",")));
    }
    format!("{{{}}}", fields.join(","))
}

fn compact_fields(value: &Value, keys: &[&str]) -> String {
    let mut fields = Vec::new();
    for key in keys {
        if let Some(field) = value.get(*key) {
            let Ok(field) = serde_json::to_string(field) else {
                continue;
            };
            let Ok(key) = serde_json::to_string(key) else {
                continue;
            };
            fields.push(format!("{key}:{field}"));
        }
    }
    format!("{{{}}}", fields.join(","))
}

fn flush_pending_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_assistant_content: &mut Option<String>,
    pending_reasoning_content: &mut Option<String>,
) {
    if pending_tool_calls.is_empty() {
        return;
    }
    awaiting_tool_call_ids.extend(
        pending_tool_calls
            .iter()
            .filter_map(|tool_call| tool_call.get("id").and_then(Value::as_str))
            .map(str::to_string),
    );
    messages.push(json!({
        "role": "assistant",
        "content": pending_assistant_content.take(),
        "reasoning_content": pending_reasoning_content.take().unwrap_or_default(),
        "tool_calls": std::mem::take(pending_tool_calls),
    }));
}

fn flush_pending_assistant_message(
    messages: &mut Vec<Value>,
    pending_assistant_content: &mut Option<String>,
    pending_reasoning_content: &mut Option<String>,
) {
    if pending_assistant_content.is_none() && pending_reasoning_content.is_none() {
        return;
    }
    messages.push(json!({
        "role": "assistant",
        "content": pending_assistant_content.take().unwrap_or_default(),
        "reasoning_content": pending_reasoning_content.take().unwrap_or_default(),
    }));
}

fn discard_unanswered_tool_calls(
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_assistant_content: &mut Option<String>,
    pending_reasoning_content: &mut Option<String>,
) {
    pending_tool_calls.clear();
    awaiting_tool_call_ids.clear();
    pending_assistant_content.take();
    pending_reasoning_content.take();
}

fn build_tools() -> Vec<Value> {
    vec![
        tool(
            "read",
            "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.",
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string","description":"Path to the file to read (relative or absolute)"},"offset":{"type":"number","description":"Line number to start reading from (1-indexed)"},"limit":{"type":"number","description":"Maximum number of lines to read"}}}),
        ),
        tool(
            "bash",
            "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.",
            json!({"type":"object","required":["command"],"properties":{"command":{"type":"string","description":"Bash command to execute"},"timeout":{"type":"number","description":"Timeout in seconds (optional, no default timeout)"}}}),
        ),
        tool(
            "edit",
            "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.",
            json!({"type":"object","required":["path","edits"],"properties":{"path":{"type":"string","description":"Path to the file to edit (relative or absolute)"},"edits":{"type":"array","items":{"type":"object","required":["oldText","newText"],"properties":{"oldText":{"type":"string","description":"Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."},"newText":{"type":"string","description":"Replacement text for this targeted edit."}},"additionalProperties":false},"description":"One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead."}},"additionalProperties":false}),
        ),
        tool(
            "write",
            "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.",
            json!({"type":"object","required":["path","content"],"properties":{"path":{"type":"string","description":"Path to the file to write (relative or absolute)"},"content":{"type":"string","description":"Content to write to the file"}}}),
        ),
    ]
}

pub(crate) fn tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
            "strict": false,
        }
    })
}
