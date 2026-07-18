use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

const MAX_OUTPUT_CHARS: usize = 50_000;
const PREVIEW_CHARS: usize = 2_000;

pub(super) fn shape_large_outputs(messages: &mut [Value], cwd: &Path, conversation_id: &str) {
    let Ok(home) = crate::config::find_codex_home() else {
        return;
    };
    shape_large_outputs_at(messages, cwd, conversation_id, home.as_path());
}

fn shape_large_outputs_at(messages: &mut [Value], cwd: &Path, conversation_id: &str, home: &Path) {
    let tool_names = tool_names_by_call_id(messages);
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let Some(call_id) = message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(tool_name) = tool_names.get(&call_id) else {
            continue;
        };
        let Some(content) = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let (output, system_suffix) = split_system_suffix(&content);
        let output_chars = output.chars().count();
        if output_chars <= MAX_OUTPUT_CHARS {
            continue;
        }

        let output_path = tool_result_path(home, cwd, conversation_id, tool_name, &call_id);
        let Some(parent) = output_path.parent() else {
            continue;
        };
        if std::fs::create_dir_all(parent).is_err() || std::fs::write(&output_path, output).is_err()
        {
            continue;
        }

        let preview = output.chars().take(PREVIEW_CHARS).collect::<String>();
        let output_bytes = output.len();
        message["content"] = Value::String(format!(
            "Tool output exceeded {MAX_OUTPUT_CHARS} characters; showing a preview only.\n\
             tool_name: {tool_name}\n\
             tool_call_id: {call_id}\n\
             output_size_chars: {output_chars}\n\
             output_size_bytes: {output_bytes}\n\
             output_path: {}\n\
             next_step: Use Read with output_path to page through the full output.\n\n\
             [preview]\n{preview}{system_suffix}",
            output_path.display()
        ));
    }
}

fn tool_names_by_call_id(messages: &[Value]) -> HashMap<String, String> {
    let mut tool_names = HashMap::new();
    for message in messages {
        let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) else {
            continue;
        };
        for tool_call in tool_calls {
            let Some(call_id) = tool_call.get("id").and_then(Value::as_str) else {
                continue;
            };
            let Some(tool_name) = tool_call
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            tool_names.insert(call_id.to_string(), tool_name.to_string());
        }
    }
    tool_names
}

fn split_system_suffix(content: &str) -> (&str, &str) {
    let Some(index) = content.rfind("\n<system>") else {
        return (content, "");
    };
    if !content[index..].ends_with("</system>") {
        return (content, "");
    }
    content.split_at(index)
}

fn tool_result_path(
    home: &Path,
    cwd: &Path,
    conversation_id: &str,
    tool_name: &str,
    call_id: &str,
) -> std::path::PathBuf {
    let cwd_text = cwd.display().to_string();
    let cwd_hash = format!("{:x}", Sha256::digest(cwd_text.as_bytes()));
    let cwd_name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "workspace".to_string());
    let stable_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{conversation_id}:{call_id}").as_bytes(),
    );
    home.join("sessions")
        .join(format!("wd_{cwd_name}_{}", &cwd_hash[..12]))
        .join(format!("session_{}", sanitize(conversation_id)))
        .join("agents")
        .join("main")
        .join("tool-results")
        .join(format!(
            "{}-{}-{stable_id}.txt",
            sanitize(tool_name),
            sanitize(call_id)
        ))
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::shape_large_outputs_at;

    #[test]
    fn persists_and_previews_large_tool_output() {
        let home = tempfile::tempdir().expect("temp home");
        let workspace = tempfile::tempdir().expect("temp workspace");
        let output = format!(
            "{}\n<system>Read status.</system>",
            "R".repeat(/*n*/ 50_001)
        );
        let mut messages = vec![
            json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": "Read_12",
                    "function": {"name": "Read", "arguments": "{}"},
                }],
            }),
            json!({
                "role": "tool",
                "tool_call_id": "Read_12",
                "content": output,
            }),
        ];

        shape_large_outputs_at(
            &mut messages,
            workspace.path(),
            "conversation-id",
            home.path(),
        );

        let shaped = messages[1]["content"].as_str().expect("shaped output");
        assert!(shaped.starts_with(
            "Tool output exceeded 50000 characters; showing a preview only.\n\
             tool_name: Read\n\
             tool_call_id: Read_12\n\
             output_size_chars: 50001\n\
             output_size_bytes: 50001\n"
        ));
        assert!(shaped.ends_with(&format!(
            "[preview]\n{}\n<system>Read status.</system>",
            "R".repeat(/*n*/ 2_000)
        )));
        let output_path = shaped
            .lines()
            .find_map(|line| line.strip_prefix("output_path: "))
            .expect("output path");
        assert_eq!(
            std::fs::read_to_string(output_path).expect("persisted output"),
            "R".repeat(/*n*/ 50_001)
        );
    }
}
