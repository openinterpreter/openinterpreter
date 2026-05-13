use crate::client_common::Prompt;
use chrono::Local;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::openai_models::ModelInfo;
use serde_json::Value;
use serde_json::json;
use std::fs;
use std::path::Path;

const QWEN_CODE_DEFAULT_MAX_TOKENS: u32 = 8_000;
const QWEN_CODE_SYSTEM_PROMPT: &str = include_str!("qwen_code_prompt.md");

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
    _reasoning_effort: Option<codex_protocol::openai_models::ReasoningEffort>,
    _conversation_id: &str,
    yolo_mode: bool,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let mut messages = vec![
        json!({
            "role": "system",
            "content": build_system_prompt(prompt),
        }),
        build_startup_context_message(prompt),
        json!({
            "role": "assistant",
            "content": "Got it. Thanks for the context!",
        }),
    ];
    messages.extend(build_qwen_messages(&prompt.get_formatted_input())?);
    let tools = super::kimi_cli::build_tools(&prompt.tools, yolo_mode)?;
    let tool_kinds = prompt
        .tools
        .iter()
        .map(|tool| (tool.name().to_string(), ToolOutputKind::Function))
        .collect();

    let request = json!({
        "model": model_info.slug,
        "messages": messages,
        "max_tokens": QWEN_CODE_DEFAULT_MAX_TOKENS,
        "stream": true,
        "stream_options": {
            "include_usage": true,
        },
        "tools": tools,
    });
    Ok((request, tool_kinds))
}

fn build_startup_context_message(prompt: &Prompt) -> Value {
    let cwd = prompt
        .cwd
        .as_deref()
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .unwrap_or_else(|_| {
            prompt
                .cwd
                .as_deref()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        });
    let cwd_display = cwd.display();
    let folder_listing = render_folder_listing(&cwd);
    let today = Local::now().format("%A, %B %-d, %Y");
    let operating_system = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };

    json!({
        "role": "user",
        "content": [
            {
                "type": "text",
                "text": format!(
                    "This is the Qwen Code. We are setting up the context for our chat.\nToday's date is {today} (formatted according to the user's locale).\nMy operating system is: {operating_system}\nI'm currently working in the directory: {cwd_display}\nHere is the folder structure of the current working directories:\n\nShowing up to 20 items:\n\n{folder_listing}"
                ),
            }
        ],
    })
}

fn render_folder_listing(cwd: &Path) -> String {
    let mut lines = Vec::new();
    lines.push(format!("{}/", cwd.display()));
    if let Ok(entries) = fs::read_dir(cwd) {
        let mut names = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
                Some(if is_dir { format!("{name}/") } else { name })
            })
            .collect::<Vec<_>>();
        names.sort();
        let visible_names = names.into_iter().take(20).collect::<Vec<_>>();
        let last_index = visible_names.len().saturating_sub(1);
        lines.extend(visible_names.into_iter().enumerate().map(|(index, name)| {
            let prefix = if index == last_index {
                "└───"
            } else {
                "├───"
            };
            format!("{prefix}{name}")
        }));
    }
    lines.join("\n")
}

fn build_qwen_messages(
    items: &[codex_protocol::models::ResponseItem],
) -> Result<Vec<Value>, serde_json::Error> {
    let mut messages = Vec::new();
    for mut message in super::kimi_cli::build_messages(items)? {
        normalize_qwen_message(&mut message);
        messages.push(message);
    }
    Ok(messages)
}

fn normalize_qwen_message(message: &mut Value) {
    let Some(message_object) = message.as_object_mut() else {
        return;
    };
    if message_object
        .get("role")
        .and_then(Value::as_str)
        .is_some_and(|role| role == "assistant")
        && message_object.contains_key("tool_calls")
        && message_object
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    {
        message_object.insert("content".to_string(), Value::String(String::new()));
    }
    if message_object
        .get("role")
        .and_then(Value::as_str)
        .is_some_and(|role| role == "tool")
        && let Some(text) = message_object.get("content").and_then(Value::as_str)
    {
        message_object.insert(
            "content".to_string(),
            json!([{
                "type": "text",
                "text": text,
            }]),
        );
    }
    if let Some(reasoning_content) = message_object
        .get("reasoning_content")
        .and_then(Value::as_str)
        .map(|text| text.trim_end_matches('\n').to_string())
    {
        message_object.insert(
            "reasoning_content".to_string(),
            Value::String(reasoning_content),
        );
    }
    let Some(tool_calls) = message_object
        .get_mut("tool_calls")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for tool_call in tool_calls {
        let Some(function) = tool_call.get_mut("function").and_then(Value::as_object_mut) else {
            continue;
        };
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let Some(arguments) = function.get_mut("arguments") else {
            continue;
        };
        let Some(arguments_text) = arguments.as_str() else {
            continue;
        };
        let Ok(parsed_arguments) = serde_json::from_str::<Value>(arguments_text) else {
            continue;
        };
        if let Some(compact_arguments) = ordered_qwen_arguments(&name, &parsed_arguments) {
            *arguments = Value::String(compact_arguments);
        } else if let Ok(compact_arguments) = serde_json::to_string(&parsed_arguments) {
            *arguments = Value::String(compact_arguments);
        }
    }
}

fn ordered_qwen_arguments(tool_name: &str, arguments: &Value) -> Option<String> {
    let Some(arguments_object) = arguments.as_object() else {
        return None;
    };
    let ordered_keys = match tool_name {
        "read_file" => &["file_path", "offset", "limit", "pages"][..],
        "edit" => &["file_path", "old_string", "new_string", "replace_all"],
        "run_shell_command" => &[
            "command",
            "description",
            "directory",
            "is_background",
            "timeout",
        ],
        _ => return None,
    };
    let mut parts = Vec::new();
    for key in ordered_keys {
        if let Some(value) = arguments_object.get(*key) {
            let key_json = serde_json::to_string(key).ok()?;
            let value_json = serde_json::to_string(value).ok()?;
            parts.push(format!("{key_json}:{value_json}"));
        }
    }
    Some(format!("{{{}}}", parts.join(",")))
}

fn build_system_prompt(prompt: &Prompt) -> String {
    let mut rendered = QWEN_CODE_SYSTEM_PROMPT.trim_end_matches('\n').to_string();
    if std::env::var("INTERPRETER_DISABLE_SYSTEM_IMPORT").as_deref() == Ok("1") {
        return rendered;
    }

    let developer_instructions = prompt
        .input
        .iter()
        .filter_map(|item| match item {
            codex_protocol::models::ResponseItem::Message { role, content, .. }
                if role == "developer" =>
            {
                Some(
                    content
                        .iter()
                        .filter_map(|item| match item {
                            codex_protocol::models::ContentItem::InputText { text }
                            | codex_protocol::models::ContentItem::OutputText { text } => {
                                Some(text.as_str())
                            }
                            codex_protocol::models::ContentItem::InputImage { .. } => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            _ => None,
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !developer_instructions.is_empty() {
        rendered.push_str("\n\n# Additional Developer Instructions\n\n");
        rendered.push_str(&developer_instructions);
    }
    if !prompt.base_instructions.text.trim().is_empty() {
        rendered.push_str("\n\n# Session Instructions\n\n");
        rendered.push_str(prompt.base_instructions.text.trim());
    }
    rendered
}
