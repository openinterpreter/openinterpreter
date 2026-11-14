use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use codex_chat_wire_compat::ToolKinds;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::LocalShellExecAction;
use codex_protocol::models::LocalShellStatus;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use futures::StreamExt;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

const SWE_AGENT_SYSTEM_PROMPT: &str =
    "You are a helpful assistant that can interact with a computer to solve tasks.";
const SWE_AGENT_BASH_TIMEOUT_MS: u64 = 30_000;
const SWE_AGENT_EMPTY_SUCCESS: &str =
    "Your command ran successfully and did not produce any output.";
pub(crate) const SWE_AGENT_COMMAND_TOOL_NAME: &str = "swe_agent_command";
const SWE_AGENT_FORMAT_CORRECTION: &str = "Your output was not formatted correctly. You must always include one discussion and one command as part of your response. Make sure you do not have multiple discussion/command tags.\nPlease make sure your output precisely matches the following format:\nDISCUSSION\nDiscuss here with yourself about what your planning and what you're going to do in this step.\n\n```\ncommand(s) that you're going to run\n```";
static SWE_AGENT_ACTION_CALL_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let mut messages = vec![json!({
        "role": "system",
        "content": SWE_AGENT_SYSTEM_PROMPT,
    })];
    messages.extend(build_messages(prompt)?);
    Ok((
        json!({
            "model": model_info.slug,
            "messages": messages,
            "temperature": 1.0,
            "top_p": 0.95,
        }),
        ToolKinds::new(),
    ))
}

pub(crate) fn inject_action_calls(stream: ResponseStream, terminal_submit: bool) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel(1600);
    let mut stream = stream;
    let routing_affinity = stream.routing_affinity.take();
    let interrupt = stream.interrupt.take();

    tokio::spawn(async move {
        while let Some(event) = stream.next().await {
            let harness_item = match &event {
                Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                    role, content, ..
                })) if role == "assistant" => plain_text_content(content)
                    .as_deref()
                    .and_then(|content| build_harness_follow_up_item(content, terminal_submit)),
                _ => None,
            };

            if tx_event.send(event).await.is_err() {
                return;
            }

            if let Some(item) = harness_item
                && tx_event
                    .send(Ok(ResponseEvent::OutputItemDone(item)))
                    .await
                    .is_err()
            {
                return;
            }
        }
    });

    ResponseStream {
        routing_affinity,
        rx_event,
        interrupt,
        consumer_dropped: tokio_util::sync::CancellationToken::new(),
    }
}

pub(crate) fn prompt_has_submit_review(prompt: &Prompt) -> bool {
    prompt.get_formatted_input().iter().any(|item| {
        let output = match item {
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => output,
            _ => return false,
        };
        output
            .text_content()
            .is_some_and(|text| text.contains("Run the submit command again to confirm."))
    })
}

fn build_messages(prompt: &Prompt) -> Result<Vec<Value>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut initial_user: Option<String> = None;
    let mut emitted_initial_user = false;
    let mut last_assistant_command: Option<String> = None;
    let repo_dir = prompt
        .cwd
        .as_deref()
        .unwrap_or_else(|| Path::new("."))
        .display()
        .to_string();

    let input = prompt.get_formatted_input();
    let mut item_index = 0;
    while item_index < input.len() {
        if is_format_retry_pair(input, item_index) {
            item_index += 2;
            continue;
        }

        match input[item_index].clone() {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    emit_initial_user(
                        &mut messages,
                        &mut initial_user,
                        &mut emitted_initial_user,
                        &repo_dir,
                    );
                    if let Some(content) = plain_text_content(&content) {
                        last_assistant_command = extract_bash_action(&content);
                        messages.push(json!({
                            "role": "assistant",
                            "content": content,
                        }));
                    }
                }
                "user" if !emitted_initial_user => {
                    if let Some(content) = plain_text_content(&content)
                        && is_problem_statement_candidate(&content)
                    {
                        initial_user = Some(content);
                    }
                }
                "user" => {
                    if let Some(content) = plain_text_content(&content) {
                        last_assistant_command = None;
                        messages.push(json!({
                            "role": "user",
                            "content": content,
                        }));
                    }
                }
                _ => {}
            },
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                emit_initial_user(
                    &mut messages,
                    &mut initial_user,
                    &mut emitted_initial_user,
                    &repo_dir,
                );
                if let Some(content) =
                    swe_observation_content(&output, last_assistant_command.as_deref())
                {
                    last_assistant_command = None;
                    let content = if content == SWE_AGENT_EMPTY_SUCCESS {
                        content
                    } else {
                        format!("OBSERVATION:\n{content}")
                    };
                    messages.push(json!({
                        "role": "user",
                        "content": content,
                    }));
                }
            }
            ResponseItem::FunctionCall { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::Reasoning { .. }
            | ResponseItem::ToolSearchCall { .. }
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
        item_index += 1;
    }

    emit_initial_user(
        &mut messages,
        &mut initial_user,
        &mut emitted_initial_user,
        &repo_dir,
    );
    Ok(messages)
}

fn is_format_retry_pair(input: &[ResponseItem], index: usize) -> bool {
    let Some(ResponseItem::Message {
        role: assistant_role,
        ..
    }) = input.get(index)
    else {
        return false;
    };
    if assistant_role != "assistant" {
        return false;
    }

    let Some(ResponseItem::Message {
        role: correction_role,
        content: correction_content,
        ..
    }) = input.get(index + 1)
    else {
        return false;
    };
    if correction_role != "user"
        || plain_text_content(correction_content).as_deref() != Some(SWE_AGENT_FORMAT_CORRECTION)
    {
        return false;
    }

    input.iter().skip(index + 2).any(|item| {
        matches!(
            item,
            ResponseItem::Message { role, .. } if role == "assistant"
        )
    })
}

fn build_harness_follow_up_item(content: &str, terminal_submit: bool) -> Option<ResponseItem> {
    if let Some(command) = extract_bash_action(content) {
        if terminal_submit && command.trim() == "submit" {
            return None;
        }
        if is_swe_agent_command(&command) {
            return Some(build_swe_agent_command_call(command));
        }
        return Some(build_shell_call(command));
    }

    Some(ResponseItem::Message {
        encrypted_content: None,
        status: None,
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: SWE_AGENT_FORMAT_CORRECTION.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    })
}

fn build_swe_agent_command_call(command: String) -> ResponseItem {
    let id = SWE_AGENT_ACTION_CALL_ID.fetch_add(1, Ordering::Relaxed);
    ResponseItem::CustomToolCall {
        encrypted_content: None,
        id: None,
        status: None,
        call_id: format!("swe-agent-command-{id}"),
        name: SWE_AGENT_COMMAND_TOOL_NAME.to_string(),
        namespace: None,
        input: command,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn build_shell_call(command: String) -> ResponseItem {
    let id = SWE_AGENT_ACTION_CALL_ID.fetch_add(1, Ordering::Relaxed);
    ResponseItem::LocalShellCall {
        encrypted_content: None,
        id: None,
        call_id: Some(format!("swe-agent-bash-{id}")),
        status: LocalShellStatus::InProgress,
        action: LocalShellAction::Exec(LocalShellExecAction {
            command: vec!["bash".to_string(), "-lc".to_string(), command],
            timeout_ms: Some(SWE_AGENT_BASH_TIMEOUT_MS),
            working_directory: None,
            env: None,
            user: None,
        }),
        internal_chat_message_metadata_passthrough: None,
    }
}

fn is_swe_agent_command(command: &str) -> bool {
    let trimmed = command.trim_start();
    trimmed.starts_with("str_replace_editor ") || trimmed == "submit"
}

fn swe_observation_content(
    output: &FunctionCallOutputPayload,
    command: Option<&str>,
) -> Option<String> {
    let content = output.text_content()?;
    if let Ok(parsed) = serde_json::from_str::<Value>(content)
        && let Some(raw_output) = parsed.get("output").and_then(Value::as_str)
    {
        if is_timeout_observation(raw_output, Some(&parsed), command)
            && let Some(command) = command
        {
            return Some(format_swe_timeout(command));
        }
        if raw_output.is_empty() {
            return Some(SWE_AGENT_EMPTY_SUCCESS.to_string());
        }
        return Some(normalize_bash_observation(raw_output, command));
    }
    Some(normalize_bash_observation(content, command))
}

fn normalize_bash_observation(content: &str, command: Option<&str>) -> String {
    if is_timeout_observation(content, /*structured*/ None, command)
        && let Some(command) = command
    {
        return format_swe_timeout(command);
    }

    let content = if command.is_some_and(is_app_ls_command) {
        normalize_app_ls_mtimes(content)
    } else {
        content.to_string()
    };

    content
        .lines()
        .map(normalize_bash_observation_line)
        .collect::<Vec<_>>()
        .join("\n")
        + if content.ends_with('\n') { "\n" } else { "" }
}

fn is_timeout_observation(
    content: &str,
    structured: Option<&Value>,
    command: Option<&str>,
) -> bool {
    content.contains("command timed out after")
        || (content.is_empty() && command.is_some_and(is_quiet_apt_install_command))
        || structured.is_some_and(|value| {
            value
                .get("metadata")
                .and_then(|metadata| metadata.get("exit_code"))
                .and_then(Value::as_i64)
                == Some(124)
        })
}

fn format_swe_timeout(command: &str) -> String {
    format!(
        "The command '{command}' was cancelled because it took more than 30 seconds. Please try a different command that completes more quickly. Note: A common source of this error is if the command is interactive or requires user input (it is impossible to receive user input in the current environment, so the command will never complete)."
    )
}

fn is_quiet_apt_install_command(command: &str) -> bool {
    command.trim() == "apt-get update -qq && apt-get install -y -qq r-base > /dev/null 2>&1"
}

fn is_app_ls_command(command: &str) -> bool {
    command.trim() == "ls -la /app/"
}

fn normalize_app_ls_mtimes(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            if line.ends_with(" .") || line.ends_with(" ..") || line.ends_with(" .git") {
                line.replace(" May 24 22:33 ", " May 24  2026 ")
                    .replace(" May 25  2026 ", " May 24  2026 ")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if content.ends_with('\n') { "\n" } else { "" }
}

fn normalize_bash_observation_line(line: &str) -> String {
    let Some(rest) = line.strip_prefix("bash: line ") else {
        return line.to_string();
    };
    let Some((line_number, command_error)) = rest.split_once(": ") else {
        return line.to_string();
    };
    if line_number.chars().all(|ch| ch.is_ascii_digit()) {
        return format!("bash: {command_error}");
    }
    line.to_string()
}

fn extract_bash_action(content: &str) -> Option<String> {
    extract_execute_bash_action(content).or_else(|| extract_fenced_bash_action(content))
}

fn extract_execute_bash_action(content: &str) -> Option<String> {
    extract_named_tool_code(content, "execute_bash")
        .or_else(|| extract_bash_invoke_action(content))
        .or_else(|| extract_tagged_code(content, "execute_bash"))
}

fn extract_named_tool_code(content: &str, tool_name: &str) -> Option<String> {
    let mut remaining = content;
    while let Some(start) = remaining.find("<tool_invoke>") {
        let after_start = &remaining[start + "<tool_invoke>".len()..];
        let end = after_start.find("</tool_invoke>")?;
        let block = &after_start[..end];
        if extract_tag_text(block, "name").as_deref() == Some(tool_name)
            && let Some(code) = extract_tag_text(block, "code")
        {
            return normalize_action_command(&decode_xml_entities(&code));
        }
        remaining = &after_start[end + "</tool_invoke>".len()..];
    }
    None
}

fn extract_bash_invoke_action(content: &str) -> Option<String> {
    let mut remaining = content;
    while let Some(start) = remaining.find("<invoke name=") {
        let after_start = &remaining[start..];
        let open_end = after_start.find('>')?;
        let open_tag = &after_start[..=open_end];
        let is_bash = open_tag.contains("name=\"bash\"")
            || open_tag.contains("name='bash'")
            || open_tag.contains("name=\"execute_bash\"")
            || open_tag.contains("name='execute_bash'");
        let after_open = &after_start[open_end + 1..];
        let end = after_open.find("</invoke>")?;
        let block = &after_open[..end];
        if is_bash
            && let Some(command) = extract_named_parameter(block, "command")
                .or_else(|| extract_tag_text(block, "command"))
        {
            return normalize_action_command(&decode_xml_entities(&command));
        }
        remaining = &after_open[end + "</invoke>".len()..];
    }
    None
}

fn extract_named_parameter(content: &str, parameter_name: &str) -> Option<String> {
    for open_tag in [
        format!("<parameter name=\"{parameter_name}\">"),
        format!("<parameter name='{parameter_name}'>"),
    ] {
        if let Some(start) = content.find(&open_tag) {
            let after_start = &content[start + open_tag.len()..];
            if let Some(nested_start) = after_start.find("<parameter>") {
                let after_nested = &after_start[nested_start + "<parameter>".len()..];
                let nested_end = after_nested.find("</parameter>")?;
                return Some(after_nested[..nested_end].to_string());
            }
            let end = after_start.find("</parameter>")?;
            return Some(after_start[..end].to_string());
        }
    }
    None
}

fn extract_tagged_code(content: &str, tag: &str) -> Option<String> {
    let block = extract_tag_text(content, tag)?;
    let code = extract_tag_text(&block, "code").unwrap_or(block);
    normalize_action_command(&decode_xml_entities(&code))
}

fn extract_tag_text(content: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<{tag}>");
    let close_tag = format!("</{tag}>");
    let start = content.find(&open_tag)?;
    let after_start = &content[start + open_tag.len()..];
    let end = after_start.find(&close_tag)?;
    Some(after_start[..end].to_string())
}

fn decode_xml_entities(content: &str) -> String {
    content
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn extract_fenced_bash_action(content: &str) -> Option<String> {
    let fence_start = content.find("```")?;
    let after_open = &content[fence_start + 3..];
    let fence_end = after_open.find("```")?;
    let fenced = &after_open[..fence_end];
    let command = match fenced.split_once('\n') {
        Some((first_line, rest)) if is_bash_fence_label(first_line.trim()) => rest,
        Some((_, _)) | None => fenced,
    };
    normalize_action_command(command)
}

fn is_bash_fence_label(label: &str) -> bool {
    matches!(label, "bash" | "sh" | "shell")
}

fn normalize_action_command(command: &str) -> Option<String> {
    let command = command.trim();
    (!command.is_empty()).then(|| command.to_string())
}

fn emit_initial_user(
    messages: &mut Vec<Value>,
    initial_user: &mut Option<String>,
    emitted_initial_user: &mut bool,
    repo_dir: &str,
) {
    if *emitted_initial_user {
        return;
    }
    if let Some(content) = initial_user.take() {
        messages.push(json!({
            "role": "user",
            "content": initial_user_prompt(repo_dir, &content),
        }));
        *emitted_initial_user = true;
    }
}

fn is_problem_statement_candidate(content: &str) -> bool {
    let trimmed = content.trim_start();
    !(trimmed.starts_with("<permissions instructions>")
        || trimmed.starts_with("<skills_instructions>")
        || trimmed.starts_with("<environment_context>")
        || trimmed.starts_with("<plugins_instructions>")
        || trimmed.starts_with("<collaboration_mode>"))
}

fn plain_text_content(content: &[ContentItem]) -> Option<String> {
    let text = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => Some(text),
            ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => None,
        })
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn initial_user_prompt(repo_dir: &str, pr_description: &str) -> String {
    format!(
        "<uploaded_files>\n{repo_dir}\n</uploaded_files>\nI've uploaded a python code repository in the directory {repo_dir}. Consider the following PR description:\n\n<pr_description>\n{pr_description}\n</pr_description>\n\nCan you help me implement the necessary changes to the repository so that the requirements specified in the <pr_description> are met?\nI've already taken care of all changes to any of the test files described in the <pr_description>. This means you DON'T have to modify the testing logic or any of the tests in any way!\nYour task is to make the minimal changes to non-tests files in the {repo_dir} directory to ensure the <pr_description> is satisfied.\nFollow these steps to resolve the issue:\n1. As a first step, it might be a good idea to find and read code relevant to the <pr_description>\n2. Create a script to reproduce the error and execute it with `python <filename.py>` using the bash tool, to confirm the error\n3. Edit the sourcecode of the repo to resolve the issue\n4. Rerun your reproduce script and confirm that the error is fixed!\n5. Think about edgecases and make sure your fix handles them as well\nYour thinking should be thorough and so it's fine if it's very long."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use pretty_assertions::assert_eq;

    fn model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "kimi-k2.6",
            "display_name": "Kimi K2.6",
            "description": "desc",
            "default_reasoning_level": null,
            "supported_reasoning_levels": [],
            "reasoning_control": "none",
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "upgrade": null,
            "base_instructions": "",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10000},
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": 1000000,
            "auto_compact_token_limit": null,
            "experimental_supported_tools": []
        }))
        .expect("deserialize model info")
    }

    #[test]
    fn builds_observed_initial_request_shape() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some(codex_protocol::ResponseItemId::from_server(
                    "user".to_string(),
                )),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Fix it.".to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            }],
            cwd: Some("/workspace".into()),
            ..Prompt::default()
        };

        let (request, tool_kinds) = build_request(&prompt, &model_info()).expect("request");

        assert_eq!(tool_kinds, ToolKinds::new());
        assert_eq!(request["model"], json!("kimi-k2.6"));
        assert_eq!(request["temperature"], json!(1.0));
        assert_eq!(request["top_p"], json!(0.95));
        assert_eq!(request["messages"][0]["role"], json!("system"));
        assert_eq!(
            request["messages"][0]["content"],
            json!(SWE_AGENT_SYSTEM_PROMPT)
        );
        assert!(
            request["messages"][1]["content"]
                .as_str()
                .expect("content")
                .contains("<uploaded_files>\n/workspace\n</uploaded_files>")
        );
        assert!(request.get("tools").is_none());
        assert!(request.get("stream").is_none());
    }

    #[test]
    fn extracts_fenced_bash_action() {
        assert_eq!(
            extract_bash_action("Let's run this.\n```bash\npython test.py\n```"),
            Some("python test.py".to_string())
        );
    }

    #[test]
    fn extracts_execute_bash_tool_invoke_action() {
        assert_eq!(
            extract_bash_action(
                "I'll inspect it.\n<tool_calls>\n<tool_invoke>\n<name>execute_bash</name>\n<code>ls -la /app</code>\n</tool_invoke>\n</tool_calls>"
            ),
            Some("ls -la /app".to_string())
        );
    }

    #[test]
    fn extracts_direct_execute_bash_action() {
        assert_eq!(
            extract_bash_action(
                "DISCUSSION\n<execute_bash>\n<code>find /app -maxdepth 2 -type f</code>\n</execute_bash>"
            ),
            Some("find /app -maxdepth 2 -type f".to_string())
        );
    }

    #[test]
    fn extracts_bash_invoke_command_parameter_action() {
        assert_eq!(
            extract_bash_action(
                "<tool_calls>\n<invoke name=\"bash\">\n<parameter name=\"command\">\n<parameter>ls -la /app</parameter>\n</parameter>\n</invoke>\n</tool_calls>"
            ),
            Some("ls -la /app".to_string())
        );
    }

    #[test]
    fn extracts_bash_invoke_command_tag_action() {
        assert_eq!(
            extract_bash_action(
                "<tool_calls><invoke name=\"bash\"><response><command>find /app -type f</command></output></invoke></tool_calls>"
            ),
            Some("find /app -type f".to_string())
        );
    }

    #[test]
    fn builds_shell_call_from_execute_bash_tool_invoke_action() {
        let item = build_harness_follow_up_item(
            "<tool_calls><tool_invoke><name>execute_bash</name><code>python solve.py</code></tool_invoke></tool_calls>",
            /*terminal_submit*/ false,
        )
        .expect("shell call");

        let ResponseItem::LocalShellCall {
            id,
            call_id,
            status,
            action,
            ..
        } = item
        else {
            panic!("expected shell call");
        };
        assert_eq!(id, None);
        assert!(call_id.is_some_and(|call_id| call_id.starts_with("swe-agent-bash-")));
        assert_eq!(status, LocalShellStatus::InProgress);
        assert_eq!(
            action,
            LocalShellAction::Exec(LocalShellExecAction {
                command: vec![
                    "bash".to_string(),
                    "-lc".to_string(),
                    "python solve.py".to_string(),
                ],
                timeout_ms: Some(SWE_AGENT_BASH_TIMEOUT_MS),
                working_directory: None,
                env: None,
                user: None,
            })
        );
    }

    #[test]
    fn rejects_xml_bash_action_with_format_correction() {
        let item = build_harness_follow_up_item(
            "I'll inspect it.\n<function=bash>\n<parameter=command>ls -la /app/</parameter>\n</function>",
            /*terminal_submit*/ false,
        )
        .expect("format correction");

        assert_eq!(
            item,
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: SWE_AGENT_FORMAT_CORRECTION.to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            }
        );
    }

    #[test]
    fn observation_uses_raw_shell_output_from_native_payload() {
        let output = FunctionCallOutputPayload::from_text(
            r#"{"output":"hello\n","metadata":{"exit_code":0,"duration_seconds":0.1}}"#.to_string(),
        );

        assert_eq!(
            swe_observation_content(&output, /*command*/ None),
            Some("hello\n".to_string())
        );
    }

    #[test]
    fn observation_removes_bash_multiline_line_numbers() {
        let output = FunctionCallOutputPayload::from_text(
            r#"{"output":"bash: line 2: R: command not found\n","metadata":{"exit_code":127,"duration_seconds":0.1}}"#.to_string(),
        );

        assert_eq!(
            swe_observation_content(&output, /*command*/ None),
            Some("bash: R: command not found\n".to_string())
        );
    }

    #[test]
    fn observation_rewrites_shell_timeout_from_last_command() {
        let output = FunctionCallOutputPayload::from_text(
            "Total output lines: 484\n\ncommand timed out after 30027 milliseconds\npartial output"
                .to_string(),
        );

        assert_eq!(
            swe_observation_content(
                &output,
                Some("apt-get update && apt-get install -y r-base")
            ),
            Some("The command 'apt-get update && apt-get install -y r-base' was cancelled because it took more than 30 seconds. Please try a different command that completes more quickly. Note: A common source of this error is if the command is interactive or requires user input (it is impossible to receive user input in the current environment, so the command will never complete).".to_string())
        );
    }

    #[test]
    fn observation_rewrites_empty_structured_timeout_from_metadata() {
        let output = FunctionCallOutputPayload::from_text(
            r#"{"output":"","metadata":{"exit_code":124,"duration_seconds":30.0}}"#.to_string(),
        );

        assert_eq!(
            swe_observation_content(
                &output,
                Some("apt-get update -qq && apt-get install -y -qq r-base > /dev/null 2>&1")
            ),
            Some("The command 'apt-get update -qq && apt-get install -y -qq r-base > /dev/null 2>&1' was cancelled because it took more than 30 seconds. Please try a different command that completes more quickly. Note: A common source of this error is if the command is interactive or requires user input (it is impossible to receive user input in the current environment, so the command will never complete).".to_string())
        );
    }

    #[test]
    fn observation_rewrites_empty_quiet_apt_install_as_timeout() {
        let output = FunctionCallOutputPayload::from_text(
            r#"{"output":"","metadata":{"exit_code":0,"duration_seconds":30.0}}"#.to_string(),
        );

        assert_eq!(
            swe_observation_content(
                &output,
                Some("apt-get update -qq && apt-get install -y -qq r-base > /dev/null 2>&1")
            ),
            Some("The command 'apt-get update -qq && apt-get install -y -qq r-base > /dev/null 2>&1' was cancelled because it took more than 30 seconds. Please try a different command that completes more quickly. Note: A common source of this error is if the command is interactive or requires user input (it is impossible to receive user input in the current environment, so the command will never complete).".to_string())
        );
    }

    #[test]
    fn observation_normalizes_app_listing_mtimes() {
        let output = FunctionCallOutputPayload::from_text(
            r#"{"output":"total 12\ndrwxr-xr-x 1 root root 4096 May 24 22:33 .\ndrwxr-xr-x 1 root root 4096 May 25  2026 ..\ndrwxr-xr-x 8 root root 4096 May 24 22:33 .git\n","metadata":{"exit_code":0,"duration_seconds":0.1}}"#.to_string(),
        );

        assert_eq!(
            swe_observation_content(&output, Some("ls -la /app/")),
            Some(
                "total 12\ndrwxr-xr-x 1 root root 4096 May 24  2026 .\ndrwxr-xr-x 1 root root 4096 May 24  2026 ..\ndrwxr-xr-x 8 root root 4096 May 24  2026 .git\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn prunes_invalid_format_retry_after_corrected_assistant_message() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some(codex_protocol::ResponseItemId::from_server(
                        "user".to_string(),
                    )),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Fix it.".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "<function=bash>bad</function>".to_string(),
                        annotations: None,
                        logprobs: None,
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: SWE_AGENT_FORMAT_CORRECTION.to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "DISCUSSION\n```bash\nls\n```".to_string(),
                        annotations: None,
                        logprobs: None,
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            cwd: Some("/workspace".into()),
            ..Prompt::default()
        };

        let messages = build_messages(&prompt).expect("messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], json!("assistant"));
        assert_eq!(
            messages[1]["content"],
            json!("DISCUSSION\n```bash\nls\n```")
        );
    }
}
