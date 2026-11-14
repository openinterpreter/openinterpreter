use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::harness::minimal::build_request_with_system_prompt;
use codex_chat_wire_compat::ToolKinds;
use codex_protocol::ResponseItemId;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use futures::StreamExt;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;

const LOCAL_SYSTEM_PROMPT: &str = "You are Open Interpreter Local, a concise coding agent running through the bundled local inference engine.\n\nWork in a simple loop: understand the task, inspect relevant files, make focused edits, run the smallest useful verification, then report the result. Use tools only when they help. For simple questions or exact-output requests, answer directly without tools.\n\nYou have a smaller context window than hosted frontier models, so work piecemeal. Prefer narrow commands, specific grep patterns, and bounded file reads. When the task has separable parts, use subagents for small scoped jobs such as inspecting one area of the codebase, proposing a focused edit, checking a failure, or summarizing a file cluster. Give each subagent a precise objective and ask for concise findings. Do not delegate vague whole-project work, and do not spawn subagents when a direct tool call is enough.\n\nThe primary coding tools are lowercase only: bash, read, write, edit, glob, and grep. Sessions may also expose subagent tools and goal controls; use goal controls only when the user or system explicitly sets a goal, and mark an active goal complete only when it is actually done. Use exactly the JSON fields from each tool schema. For example, call bash with {\"command\":\"python3 -m unittest -v\"}, read with {\"path\":\"file.py\"}, write with {\"path\":\"result.txt\",\"content\":\"ok\\n\"}, and edit with {\"path\":\"file.py\",\"old_string\":\"old text\",\"new_string\":\"new text\"}. For simple file creation and verification, prefer one bash call such as {\"command\":\"printf 'ok\\n' > result.txt && cat result.txt\"}; use write only when you have both a path and exact content. Large command output is focused before you see it; trust the focused observation first. If a full output path is shown, use grep with a specific pattern or read with line_offset/n_lines for a small slice instead of reading the whole file or rerunning the command. After a verification command succeeds, stop using tools and answer the user with a short summary of what changed and what verification passed. Do not use bash echo or printf just to tell the user the task is done; final status belongs in your answer, not in a shell command. Do not repeat a successful command unless the user explicitly asked for repeated runs. Do not invent tool names. Do not expose internal reasoning, hidden tool markup, or raw model tool tokens. If the environment prevents an action, explain that plainly and choose the next useful step.";

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let system_prompt = local_system_prompt(prompt);
    let (mut request, tool_kinds) =
        build_request_with_system_prompt(&system_prompt, prompt, model_info, effort)?;
    request["temperature"] = json!(0);
    request["max_tokens"] = json!(2048);
    request["parallel_tool_calls"] = json!(false);
    request["stop"] = json!([
        "\n<|channel>thought",
        "\n<|channel>analysis",
        "\n<|channel>commentary",
        "\n<|channel>final",
        "\n<|channel>"
    ]);
    Ok((request, tool_kinds))
}

fn local_system_prompt(prompt: &Prompt) -> String {
    let base_instructions = prompt.base_instructions.text.trim();
    if base_instructions.is_empty() {
        LOCAL_SYSTEM_PROMPT.to_string()
    } else {
        format!("{LOCAL_SYSTEM_PROMPT}\n\n{base_instructions}")
    }
}

pub(crate) fn repair_text_tool_calls(stream: ResponseStream) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel(1600);

    tokio::spawn(async move {
        let mut stream = stream;
        while let Some(event) = stream.next().await {
            let event = repair_local_text_tool_call(event);
            let is_err = event.is_err();
            if tx_event.send(event).await.is_err() || is_err {
                return;
            }
        }
    });

    ResponseStream {
        rx_event,
        consumer_dropped: tokio_util::sync::CancellationToken::new(),
    }
}

fn repair_local_text_tool_call(event: CodexResult<ResponseEvent>) -> CodexResult<ResponseEvent> {
    let Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
        id,
        role,
        content,
        internal_chat_message_metadata_passthrough: metadata,
        ..
    })) = &event
    else {
        return event;
    };

    if role != "assistant" {
        return event;
    }

    let Some((name, arguments)) = local_repaired_tool_call_from_content(content.as_slice()) else {
        return event;
    };

    let call_item_id = id.clone().unwrap_or_else(|| ResponseItemId::new("call"));
    Ok(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
        id: Some(call_item_id.clone()),
        name: name.clone(),
        namespace: None,
        arguments,
        encrypted_function_args: None,
        call_id: call_item_id.to_string(),
        internal_chat_message_metadata_passthrough: metadata.clone(),
    }))
}

fn local_repaired_tool_call_from_content(content: &[ContentItem]) -> Option<(String, String)> {
    let text = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::OutputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    let text = strip_json_code_fence(text.trim())?;
    let value = serde_json::from_str::<Value>(text).ok()?;
    let object = value.as_object()?;
    let explicit_name = object
        .get("name")
        .or_else(|| object.get("tool"))
        .or_else(|| object.get("tool_name"))
        .and_then(Value::as_str)
        .map(str::trim);
    let name = if let Some(name) = explicit_name {
        name.to_string()
    } else if object.get("command").is_some() {
        "bash".to_string()
    } else {
        return None;
    };
    if !is_local_repairable_tool_name(&name) {
        return None;
    }
    let arguments = if explicit_name.is_some() {
        object
            .get("arguments")
            .or_else(|| object.get("args"))
            .or_else(|| object.get("parameters"))
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()))
    } else {
        Value::Object(object.clone())
    };
    let arguments = match arguments {
        Value::String(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(value) => value,
            Err(_) if name == "bash" => json!({ "command": raw }),
            Err(_) => return None,
        },
        value => value,
    };
    let mut arguments = arguments;
    if name == "bash"
        && let Some(object) = arguments.as_object_mut()
        && !object.contains_key("command")
        && let Some(input) = object.remove("input")
    {
        object.insert("command".to_string(), input);
    }
    let arguments = serde_json::to_string(&arguments).ok()?;
    Some((name, arguments))
}

fn strip_json_code_fence(text: &str) -> Option<&str> {
    let text = text.trim();
    if !text.starts_with("```") {
        return Some(text);
    }
    let without_opening = text.strip_prefix("```")?;
    let first_newline = without_opening.find('\n')?;
    let body = &without_opening[first_newline + 1..];
    body.trim().strip_suffix("```").map(str::trim)
}

fn is_local_repairable_tool_name(name: &str) -> bool {
    matches!(name, "bash" | "read" | "write" | "edit" | "glob" | "grep")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(text: &str) -> Vec<ContentItem> {
        vec![ContentItem::OutputText {
            text: text.to_string(),
        }]
    }

    #[test]
    fn repairs_fenced_json_tool_call_text() {
        let (name, arguments) = local_repaired_tool_call_from_content(&content(
            "```json\n{\"name\":\"bash\",\"arguments\":{\"input\":\"echo ok\"}}\n```",
        ))
        .expect("repairable tool call");

        assert_eq!(name, "bash");
        assert_eq!(arguments, r#"{"command":"echo ok"}"#);
    }

    #[test]
    fn does_not_repair_unknown_tool_name_as_write() {
        let repaired = local_repaired_tool_call_from_content(&content(
            "```json\n{\"name\":\"result.txt\",\"arguments\":{\"input\":\"ok\"}}\n```",
        ));

        assert_eq!(repaired, None);
    }

    #[test]
    fn repairs_bare_bash_arguments() {
        let (name, arguments) = local_repaired_tool_call_from_content(&content(
            "```json\n{\"command\":\"printf ok > result.txt && cat result.txt\"}\n```",
        ))
        .expect("repairable bare bash arguments");

        assert_eq!(name, "bash");
        assert_eq!(
            arguments,
            r#"{"command":"printf ok > result.txt && cat result.txt"}"#
        );
    }
}
