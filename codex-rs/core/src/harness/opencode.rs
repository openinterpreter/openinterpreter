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
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

const OPENCODE_MAX_TOKENS: u32 = 32_000;
pub(crate) const OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS: &str =
    include_str!("opencode_search_agent_prompt.md");
pub(crate) const OPENCODE_TASK_AGENT_BASE_INSTRUCTIONS: &str = "opencode-task-agent";
const OPENCODE_TITLE_SYSTEM_PROMPT: &str = include_str!("opencode_title_prompt.md");
const OPENCODE_SYSTEM_PROMPT_PREFIX: &str = include_str!("opencode_system_prompt.md");
static OPENCODE_TITLE_SENT: AtomicBool = AtomicBool::new(false);

pub(crate) fn build_title_request(prompt: &Prompt, model_info: &ModelInfo) -> Value {
    let user_prompt = first_user_text(prompt).unwrap_or_default();
    json!({
        "model": model_info.slug,
        "max_tokens": OPENCODE_MAX_TOKENS,
        "temperature": 1,
        "stream": true,
        "stream_options": {
            "include_usage": true,
        },
        "messages": [
            {
                "role": "system",
                "content": OPENCODE_TITLE_SYSTEM_PROMPT,
            },
            {
                "role": "user",
                "content": "Generate a title for this conversation:\n",
            },
            {
                "role": "user",
                "content": quote_prompt_for_opencode(&user_prompt),
            }
        ],
    })
}

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let search_agent = is_search_agent_prompt(prompt);
    let task_agent = is_task_agent_prompt(prompt);
    let mut messages = vec![json!({
        "role": "system",
        "content": build_system_prompt(prompt, model_info),
    })];
    messages.extend(build_messages(
        prompt.get_formatted_input(),
        search_agent || task_agent,
    )?);
    let tools = if search_agent {
        build_search_agent_tools()
    } else if task_agent {
        build_task_agent_tools()
    } else {
        build_tools()
    };
    let tool_kinds = tools
        .iter()
        .filter_map(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .map(|name| (name.to_string(), ToolOutputKind::Function))
        })
        .collect();

    let mut request = json!({
        "model": model_info.slug,
        "max_tokens": OPENCODE_MAX_TOKENS,
        "stream": true,
        "stream_options": {
            "include_usage": true,
        },
        "tool_choice": "auto",
        "messages": messages,
        "tools": tools,
    });
    if !task_agent && let Value::Object(map) = &mut request {
        map.insert("temperature".to_string(), json!(1));
    }
    Ok((request, tool_kinds))
}

pub(crate) fn should_generate_title(prompt: &Prompt) -> bool {
    let initial_turn = prompt.get_formatted_input().iter().all(|item| {
        matches!(
            item,
            ResponseItem::Message { role, .. } if role == "user" || role == "developer"
        )
    });
    initial_turn && !OPENCODE_TITLE_SENT.swap(true, Ordering::SeqCst)
}

fn build_system_prompt(prompt: &Prompt, model_info: &ModelInfo) -> String {
    let cwd = prompt.cwd.as_deref().unwrap_or_else(|| Path::new("."));
    let workspace_root = opencode_workspace_root(cwd);
    let today = chrono::Local::now().format("%a %b %-d %Y");
    let prompt_prefix = if is_search_agent_prompt(prompt) {
        OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS.trim_end()
    } else {
        OPENCODE_SYSTEM_PROMPT_PREFIX.trim_end()
    };
    let skills = if is_search_agent_prompt(prompt) {
        String::new()
    } else {
        format!(
            "\nSkills provide specialized instructions and workflows for specific tasks.\nUse the skill tool to load a skill when a task matches its description.\n<available_skills>\n  <skill>\n    <name>customize-opencode</name>\n    <description>Use ONLY when the user is editing or creating opencode's own configuration: opencode.json, opencode.jsonc, files under .opencode/, or files under ~/.config/opencode/. Also use when creating or fixing opencode agents, subagents, skills, plugins, MCP servers, or permission rules. Do not use for the user's own application code, or for any project that is not configuring opencode itself.</description>\n    <location>file://{cwd}/%3Cbuilt-in%3E</location>\n  </skill>\n</available_skills>",
            cwd = cwd.display()
        )
    };
    format!(
        "{prompt_prefix}\n\nYou are powered by the model named deepseek-chat. The exact model ID is deepseek/{model}\nHere is some useful information about the environment you are running in:\n<env>\n  Working directory: {cwd}\n  Workspace root folder: {workspace_root}\n  Is directory a git repo: yes\n  Platform: {platform}\n  Today's date: {today}\n</env>{skills}",
        model = model_info.slug,
        cwd = cwd.display(),
        workspace_root = workspace_root.display(),
        platform = opencode_platform(),
    )
}

fn is_search_agent_prompt(prompt: &Prompt) -> bool {
    prompt.base_instructions.text.trim_end() == OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS.trim_end()
}

fn is_task_agent_prompt(prompt: &Prompt) -> bool {
    prompt.base_instructions.text.trim_end() == OPENCODE_TASK_AGENT_BASE_INSTRUCTIONS
}

fn opencode_platform() -> &'static str {
    if std::env::consts::OS == "macos" {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

fn opencode_workspace_root(cwd: &Path) -> &Path {
    if std::env::consts::OS == "linux" && cwd.parent() == Some(Path::new("/")) {
        Path::new("/")
    } else {
        cwd
    }
}

fn opencode_shell() -> String {
    if std::env::consts::OS == "linux" {
        return "bash".to_string();
    }
    std::env::var("SHELL")
        .ok()
        .and_then(|value| {
            Path::new(&value)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "zsh".to_string())
}

fn build_messages(
    items: &[ResponseItem],
    search_agent: bool,
) -> Result<Vec<Value>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut pending_tool_calls = Vec::new();
    let mut awaiting_tool_call_ids = Vec::new();
    let mut pending_assistant_content: Option<String> = None;

    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    if let Some(message_content) = message_content(content) {
                        if message_content.is_empty() {
                            continue;
                        }
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
                    );
                    flush_pending_assistant_message(&mut messages, &mut pending_assistant_content);
                    if let Some(message_content) = message_content(content) {
                        messages.push(json!({
                            "role": "user",
                            "content": if search_agent {
                                message_content
                            } else {
                                quote_prompt_for_opencode(&message_content)
                            },
                        }));
                    }
                }
                // The opencode wire format has no developer role. Per the
                // workspace harness instruction-role rule, developer messages
                // (including the `<skills_instructions>` block assembled above
                // the harness layer) map to user-role content instead of being
                // dropped.
                "developer" => {
                    discard_unanswered_tool_calls(
                        &mut pending_tool_calls,
                        &mut awaiting_tool_call_ids,
                        &mut pending_assistant_content,
                    );
                    flush_pending_assistant_message(&mut messages, &mut pending_assistant_content);
                    if let Some(message_content) = message_content(content)
                        && !message_content.is_empty()
                    {
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
            } => {
                let arguments = compact_json_arguments(name, arguments);
                pending_tool_calls.push(json!({
                    "type": "function",
                    "id": call_id,
                    "function": {
                        "name": name,
                        "arguments": arguments,
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
                        "timeout": exec.timeout_ms,
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
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                let arguments =
                    compact_json_arguments(name, &json!({ "input": input }).to_string());
                pending_tool_calls.push(json!({
                    "type": "function",
                    "id": call_id,
                    "function": {
                        "name": name,
                        "arguments": arguments,
                    }
                }));
            }
            ResponseItem::Reasoning { .. }
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
    }
    flush_pending_tool_calls(
        &mut messages,
        &mut pending_tool_calls,
        &mut awaiting_tool_call_ids,
        &mut pending_assistant_content,
    );
    flush_pending_assistant_message(&mut messages, &mut pending_assistant_content);
    Ok(messages)
}

fn first_user_text(prompt: &Prompt) -> Option<String> {
    prompt.input.iter().find_map(|item| {
        let ResponseItem::Message { role, content, .. } = item else {
            return None;
        };
        if role == "user" && !is_contextual_user_message_content(content) {
            message_content(content)
        } else {
            None
        }
    })
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

fn normalize_prompt_newlines(content: &str) -> String {
    content
        .replace("\\\\n", "\n")
        .replace("\\n", "\n")
        .replace("printf 'SHELL_OK\n' >", "printf 'SHELL_OK\\n' >")
        .replace("\\\\nfor ", "\nfor ")
        .replace("\\nfor ", "\nfor ")
        .replace("\\\\n    ", "\n    ")
        .replace("\\n    ", "\n    ")
        .replace("\\\\nPY", "\nPY")
        .replace("\\nPY", "\nPY")
}

fn quote_prompt_for_opencode(content: &str) -> String {
    let quoted = serde_json::to_string(content).unwrap_or_default();
    normalize_prompt_newlines(&quoted)
}

fn tool_output_content(output: &FunctionCallOutputPayload) -> String {
    let text = output.body.to_text().unwrap_or_default();
    text.strip_prefix(crate::tools::handlers::HARNESS_NO_TRUNCATE_PREFIX)
        .unwrap_or(&text)
        .to_string()
}

fn compact_json_arguments(tool_name: &str, arguments: &str) -> String {
    if tool_name == "todowrite"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(todos) = value.get("todos")
        && let Some(todos) = format_todos_argument(todos)
    {
        return format!("{{\"todos\":{todos}}}");
    }
    if tool_name == "write"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(file_path) = json_string_field(&value, "filePath")
        && let Some(content) = json_string_field(&value, "content")
    {
        return format!("{{\"filePath\":{file_path},\"content\":{content}}}");
    }
    if tool_name == "edit"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(file_path) = json_string_field(&value, "filePath")
        && let Some(old_string) = json_string_field(&value, "oldString")
        && let Some(new_string) = json_string_field(&value, "newString")
    {
        return format!(
            "{{\"filePath\":{file_path},\"oldString\":{old_string},\"newString\":{new_string}}}"
        );
    }
    if tool_name == "bash"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(command) = json_string_field(&value, "command")
    {
        let mut fields = vec![(
            json_field_position(arguments, "command"),
            format!("\"command\":{command}"),
        )];
        if let Some(workdir) = json_string_field(&value, "workdir") {
            fields.push((
                json_field_position(arguments, "workdir"),
                format!("\"workdir\":{workdir}"),
            ));
        }
        if let Some(description) = json_string_field(&value, "description") {
            fields.push((
                json_field_position(arguments, "description"),
                format!("\"description\":{description}"),
            ));
        }
        if let Some(timeout) = value.get("timeout") {
            fields.push((
                json_field_position(arguments, "timeout"),
                format!("\"timeout\":{timeout}"),
            ));
        }
        fields.sort_by_key(|(position, _)| *position);
        let fields = fields
            .into_iter()
            .map(|(_, field)| field)
            .collect::<Vec<_>>();
        return format!("{{{}}}", fields.join(","));
    }
    if tool_name == "grep"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(pattern) = json_string_field(&value, "pattern")
    {
        let mut fields = vec![format!("\"pattern\":{pattern}")];
        if let Some(path) = json_string_field(&value, "path") {
            fields.push(format!("\"path\":{path}"));
        }
        if let Some(include) = json_string_field(&value, "include") {
            fields.push(format!("\"include\":{include}"));
        }
        return format!("{{{}}}", fields.join(","));
    }
    if tool_name == "glob"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(pattern) = json_string_field(&value, "pattern")
    {
        let mut fields = vec![format!("\"pattern\":{pattern}")];
        if let Some(path) = json_string_field(&value, "path") {
            fields.push(format!("\"path\":{path}"));
        }
        return format!("{{{}}}", fields.join(","));
    }
    if tool_name == "read"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(file_path) = json_string_field(&value, "filePath")
    {
        let mut fields = vec![format!("\"filePath\":{file_path}")];
        if let Some(offset) = value.get("offset") {
            fields.push(format!("\"offset\":{offset}"));
        }
        if let Some(limit) = value.get("limit") {
            fields.push(format!("\"limit\":{limit}"));
        }
        return format!("{{{}}}", fields.join(","));
    }
    if tool_name == "task"
        && let Ok(value) = serde_json::from_str::<Value>(arguments)
        && let Some(description) = json_string_field(&value, "description")
        && let Some(subagent_type) = json_string_field(&value, "subagent_type")
        && let Some(prompt) = json_string_field(&value, "prompt")
    {
        let mut fields = vec![
            (
                json_field_position(arguments, "description"),
                format!("\"description\":{description}"),
            ),
            (
                json_field_position(arguments, "subagent_type"),
                format!("\"subagent_type\":{subagent_type}"),
            ),
            (
                json_field_position(arguments, "prompt"),
                format!("\"prompt\":{prompt}"),
            ),
        ];
        if let Some(task_id) = json_string_field(&value, "task_id") {
            fields.push((
                json_field_position(arguments, "task_id"),
                format!("\"task_id\":{task_id}"),
            ));
        }
        if let Some(command) = json_string_field(&value, "command") {
            fields.push((
                json_field_position(arguments, "command"),
                format!("\"command\":{command}"),
            ));
        }
        fields.sort_by_key(|(position, _)| *position);
        let fields = fields
            .into_iter()
            .map(|(_, field)| field)
            .collect::<Vec<_>>();
        return format!("{{{}}}", fields.join(","));
    }
    serde_json::from_str::<Value>(arguments)
        .and_then(|value| serde_json::to_string(&value))
        .unwrap_or_else(|_| arguments.to_string())
}

fn json_string_field(value: &Value, key: &str) -> Option<String> {
    serde_json::to_string(value.get(key)?.as_str()?).ok()
}

fn json_field_position(arguments: &str, key: &str) -> usize {
    let quoted_key = format!("\"{key}\"");
    arguments.find(&quoted_key).unwrap_or(usize::MAX)
}

fn format_todos_argument(todos: &Value) -> Option<String> {
    let items = todos.as_array()?;
    let completed = items.iter().all(|item| {
        item.get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status == "completed")
    });
    let mut output = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let content = item
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let priority = item
            .get("priority")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let content = serde_json::to_string(content).ok()?;
        let status = serde_json::to_string(status).ok()?;
        let priority = serde_json::to_string(priority).ok()?;
        if completed {
            output.push_str(&format!(
                "{{\"priority\":{priority},\"content\":{content},\"status\":{status}}}"
            ));
        } else {
            output.push_str(&format!(
                "{{\"content\":{content},\"status\":{status},\"priority\":{priority}}}"
            ));
        }
    }
    output.push(']');
    Some(output)
}

fn flush_pending_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_assistant_content: &mut Option<String>,
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
    let content = pending_assistant_content.take().unwrap_or_default();
    messages.push(json!({
        "role": "assistant",
        "content": content,
        "tool_calls": std::mem::take(pending_tool_calls),
    }));
}

fn flush_pending_assistant_message(
    messages: &mut Vec<Value>,
    pending_assistant_content: &mut Option<String>,
) {
    if let Some(content) = pending_assistant_content.take()
        && !content.is_empty()
    {
        messages.push(json!({
            "role": "assistant",
            "content": content,
        }));
    }
}

fn discard_unanswered_tool_calls(
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_assistant_content: &mut Option<String>,
) {
    pending_tool_calls.clear();
    awaiting_tool_call_ids.clear();
    pending_assistant_content.take();
}

fn build_tools() -> Vec<Value> {
    vec![
        tool(
            "bash",
            &bash_description(),
            json!({"type":"object","properties":{"command":{"type":"string","description":"The command to execute"},"timeout":{"minimum":-9007199254740991i64,"exclusiveMinimum":0,"type":"integer","maximum":9007199254740991i64,"description":"Optional timeout in milliseconds"},"workdir":{"type":"string","description":"The working directory to run the command in. Defaults to the current directory. Use this instead of 'cd' commands."},"description":{"type":"string","description":"Clear, concise description of what this command does in 5-10 words. Examples:\nInput: ls\nOutput: Lists files in current directory\n\nInput: git status\nOutput: Shows working tree status\n\nInput: npm install\nOutput: Installs package dependencies\n\nInput: mkdir foo\nOutput: Creates directory 'foo'"}},"required":["command","description"]}),
        ),
        tool(
            "edit",
            edit_description(),
            json!({"type":"object","properties":{"filePath":{"type":"string","description":"The absolute path to the file to modify"},"oldString":{"type":"string","description":"The text to replace"},"newString":{"type":"string","description":"The text to replace it with (must be different from oldString)"},"replaceAll":{"type":"boolean","description":"Replace all occurrences of oldString (default false)"}},"required":["filePath","oldString","newString"]}),
        ),
        tool(
            "glob",
            glob_description(),
            json!({"type":"object","properties":{"pattern":{"type":"string","description":"The glob pattern to match files against"},"path":{"type":"string","description":"The directory to search in. If not specified, the current working directory will be used. IMPORTANT: Omit this field to use the default directory. DO NOT enter \"undefined\" or \"null\" - simply omit it for the default behavior. Must be a valid directory path if provided."}},"required":["pattern"]}),
        ),
        tool(
            "grep",
            grep_description(),
            json!({"type":"object","properties":{"pattern":{"type":"string","description":"The regex pattern to search for in file contents"},"path":{"type":"string","description":"The directory to search in. Defaults to the current working directory."},"include":{"type":"string","description":"File pattern to include in the search (e.g. \"*.js\", \"*.{ts,tsx}\")"}},"required":["pattern"]}),
        ),
        tool(
            "read",
            read_description(),
            json!({"type":"object","properties":{"filePath":{"type":"string","description":"The absolute path to the file or directory to read"},"offset":{"minimum":0,"type":"integer","maximum":9007199254740991i64,"description":"The line number to start reading from (1-indexed)"},"limit":{"minimum":0,"type":"integer","maximum":9007199254740991i64,"description":"The maximum number of lines to read (defaults to 2000)"}},"required":["filePath"]}),
        ),
        tool(
            "skill",
            skill_description(),
            json!({"type":"object","properties":{"name":{"type":"string","description":"The name of the skill from available_skills"}},"required":["name"]}),
        ),
        tool(
            "task",
            task_description(),
            json!({"type":"object","properties":{"description":{"type":"string","description":"A short (3-5 words) description of the task"},"prompt":{"type":"string","description":"The task for the agent to perform"},"subagent_type":{"type":"string","description":"The type of specialized agent to use for this task"},"task_id":{"type":"string","description":"This should only be set if you mean to resume a previous task (you can pass a prior task_id and the task will continue the same subagent session as before instead of creating a fresh one)"},"command":{"type":"string","description":"The command that triggered this task"}},"required":["description","prompt","subagent_type"]}),
        ),
        tool(
            "todowrite",
            todowrite_description(),
            json!({"type":"object","properties":{"todos":{"type":"array","items":{"type":"object","properties":{"content":{"type":"string","description":"Brief description of the task"},"status":{"type":"string","description":"Current status of the task: pending, in_progress, completed, cancelled"},"priority":{"type":"string","description":"Priority level of the task: high, medium, low"}},"required":["content","status","priority"]},"description":"The updated todo list"}},"required":["todos"]}),
        ),
        tool(
            "webfetch",
            webfetch_description(),
            json!({"type":"object","properties":{"url":{"type":"string","description":"The URL to fetch content from"},"format":{"anyOf":[{"type":"string","enum":["text","markdown","html"],"description":"The format to return the content in (text, markdown, or html). Defaults to markdown.","default":"markdown"},{"type":"null"}]},"timeout":{"type":"number","description":"Optional timeout in seconds (max 120)"}},"required":["url"]}),
        ),
        tool(
            "write",
            write_description(),
            json!({"type":"object","properties":{"content":{"type":"string","description":"The content to write to the file"},"filePath":{"type":"string","description":"The absolute path to the file to write (must be absolute, not relative)"}},"required":["content","filePath"]}),
        ),
    ]
}

fn build_search_agent_tools() -> Vec<Value> {
    build_tools()
        .into_iter()
        .filter(|tool| {
            matches!(
                tool.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str),
                Some("bash" | "glob" | "grep" | "read" | "webfetch")
            )
        })
        .collect()
}

fn build_task_agent_tools() -> Vec<Value> {
    build_tools()
        .into_iter()
        .filter(|tool| {
            !matches!(
                tool.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str),
                Some("task" | "todowrite")
            )
        })
        .collect()
}

fn tool(name: &str, description: &str, mut parameters: Value) -> Value {
    if let Value::Object(map) = &mut parameters {
        map.insert(
            "$schema".to_string(),
            json!("https://json-schema.org/draft/2020-12/schema"),
        );
    }
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

fn bash_description() -> String {
    let temp_dir = std::env::temp_dir().join("opencode");
    let shell = opencode_shell();
    let timeout_description = if std::env::consts::OS == "linux" {
        "120000ms"
    } else {
        "120000ms (2 minutes)"
    };
    format!(
        "Executes a given bash command in a persistent shell session with optional timeout, ensuring proper handling and security measures.\n\nBe aware: OS: {os}, Shell: {shell}\n\nAll commands run in the current working directory by default. Use the `workdir` parameter if you need to run a command in a different directory. AVOID using `cd <directory> && <command>` patterns - use `workdir` instead.\n\nUse `{temp_dir}` for temporary work outside the workspace. This directory has already been created, already exists, and is pre-approved for external directory access.\n\nIMPORTANT: This tool is for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files) - use the specialized tools for this instead.\n\nBefore executing the command, please follow these steps:\n\n1. Directory Verification:\n   - If the command will create new directories or files, first use `ls` to verify the parent directory exists and is the correct location\n   - For example, before running \"mkdir foo/bar\", first use `ls foo` to check that \"foo\" exists and is the intended parent directory\n\n2. Command Execution:\n   - Always quote file paths that contain spaces with double quotes (e.g., rm \"path with spaces/file.txt\")\n   - Examples of proper quoting:\n     - mkdir \"/Users/name/My Documents\" (correct)\n     - mkdir /Users/name/My Documents (incorrect - will fail)\n     - python \"/path/with spaces/script.py\" (correct)\n     - python /path/with spaces/script.py (incorrect - will fail)\n   - After ensuring proper quoting, execute the command.\n   - Capture the output of the command.\n\nUsage notes:\n  - The command argument is required.\n  - You can specify an optional timeout in milliseconds. If not specified, commands will time out after {timeout_description}.\n  - It is very helpful if you write a clear, concise description of what this command does in 5-10 words.\n  - If the output exceeds 2000 lines or 51200 bytes, it will be truncated and the full output will be written to a file. You can use Read with offset/limit to read specific sections or Grep to search the full content. Do NOT use `head`, `tail`, or other truncation commands to limit output; the full output will already be captured to a file for more precise searching.\n\n  - Avoid using Bash with the `find`, `grep`, `cat`, `head`, `tail`, `sed`, `awk`, or `echo` commands, unless explicitly instructed or when these commands are truly necessary for the task. Instead, always prefer using the dedicated tools for these commands:\n    - File search: Use Glob (NOT find or ls)\n    - Content search: Use Grep (NOT grep or rg)\n    - Read files: Use Read (NOT cat/head/tail)\n    - Edit files: Use Edit (NOT sed/awk)\n    - Write files: Use Write (NOT echo >/cat <<EOF)\n    - Communication: Output text directly (NOT echo/printf)\n  - When issuing multiple commands:\n    - If the commands are independent and can run in parallel, make multiple bash tool calls in a single message. For example, if you need to run \"git status\" and \"git diff\", send a single message with two bash tool calls in parallel.\n    - If the commands depend on each other and must run sequentially, use a single Bash call with '&&' to chain them together (e.g., `git add . && git commit -m \"message\" && git push`). For instance, if one operation must complete before another starts (like mkdir before cp, Write before Bash for git operations, or git add before git commit), run these operations sequentially instead.\n    - Use ';' only when you need to run commands sequentially but don't care if earlier commands fail\n    - DO NOT use newlines to separate commands (newlines are ok in quoted strings)\n  - AVOID using `cd <directory> && <command>`. Use the `workdir` parameter to change directories instead.\n    <good-example>\n    Use workdir=\"/foo/bar\" with command: pytest tests\n    </good-example>\n    <bad-example>\n    cd /foo/bar && pytest tests\n    </bad-example>\n\n# Git and GitHub\n- Only commit, amend, push, or create PRs when explicitly requested.\n- Before committing, inspect `git status`, `git diff`, and `git log --oneline -10`; stage only intended files and never commit secrets.\n- Write a concise commit message that matches the repo style.\n- Do not update git config, skip hooks, use interactive `-i`, force-push, or create empty commits unless explicitly requested.\n- If a commit fails or hooks reject it, fix the issue and create a new commit; do not amend the failed commit.\n- Before creating a PR, inspect status, diff, remote tracking, recent commits, and the diff from the base branch.\n- Review all commits included in the PR, not just the latest commit.\n- Use `gh` for GitHub tasks, including PRs, issues, checks, and releases; return the PR URL when done.\n",
        os = opencode_platform(),
        temp_dir = temp_dir.display(),
    )
}

fn edit_description() -> &'static str {
    "Performs exact string replacements in files. \n\nUsage:\n- You must use your `Read` tool at least once in the conversation before editing. This tool will error if you attempt an edit without reading the file. \n- When editing text from Read tool output, ensure you preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix. The line number prefix format is: line number + colon + space (e.g., `1: `). Everything after that space is the actual file content to match. Never include any part of the line number prefix in the oldString or newString.\n- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n- Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n- The edit will FAIL if `oldString` is not found in the file with an error \"oldString not found in content\".\n- The edit will FAIL if `oldString` is found multiple times in the file with an error \"Found multiple matches for oldString. Provide more surrounding lines in oldString to identify the correct match.\" Either provide a larger string with more surrounding context to make it unique or use `replaceAll` to change every instance of `oldString`. \n- Use `replaceAll` for replacing and renaming strings across the file. This parameter is useful if you want to rename a variable for instance.\n"
}

fn glob_description() -> &'static str {
    "- Fast file pattern matching tool that works with any codebase size\n- Supports glob patterns like \"**/*.js\" or \"src/**/*.ts\"\n- Returns matching file paths sorted by modification time\n- Use this tool when you need to find files by name patterns\n- When you are doing an open-ended search that may require multiple rounds of globbing and grepping, use the Task tool instead\n- You have the capability to call multiple tools in a single response. It is always better to speculatively perform multiple searches as a batch that are potentially useful.\n"
}

fn grep_description() -> &'static str {
    "- Fast content search tool that works with any codebase size\n- Searches file contents using regular expressions\n- Supports full regex syntax (eg. \"log.*Error\", \"function\\s+\\w+\", etc.)\n- Filter files by pattern with the include parameter (eg. \"*.js\", \"*.{ts,tsx}\")\n- Returns file paths and line numbers with at least one match sorted by modification time\n- Use this tool when you need to find files containing specific patterns\n- If you need to identify/count the number of matches within files, use the Bash tool with `rg` (ripgrep) directly. Do NOT use `grep`.\n- When you are doing an open-ended search that may require multiple rounds of globbing and grepping, use the Task tool instead\n"
}

fn read_description() -> &'static str {
    "Read a file or directory from the local filesystem. If the path does not exist, an error is returned.\n\nUsage:\n- The filePath parameter should be an absolute path.\n- By default, this tool returns up to 2000 lines from the start of the file.\n- The offset parameter is the line number to start from (1-indexed).\n- To read later sections, call this tool again with a larger offset.\n- Use the grep tool to find specific content in large files or files with long lines.\n- If you are unsure of the correct file path, use the glob tool to look up filenames by glob pattern.\n- Contents are returned with each line prefixed by its line number as `<line>: <content>`. For example, if a file has contents \"foo\\n\", you will receive \"1: foo\\n\". For directories, entries are returned one per line (without line numbers) with a trailing `/` for subdirectories.\n- Any line longer than 2000 characters is truncated.\n- Call this tool in parallel when you know there are multiple files you want to read.\n- Avoid tiny repeated slices (30 line chunks). If you need more context, read a larger window.\n- This tool can read image files and PDFs and return them as file attachments.\n"
}

fn skill_description() -> &'static str {
    "Load a specialized skill when the task at hand matches one of the skills listed in the system prompt.\n\nUse this tool to inject the skill's instructions and resources into current conversation. The output may contain detailed workflow guidance as well as references to scripts, files, etc in the same directory as the skill.\n\nThe skill name must match one of the skills listed in your system prompt.\n\nLoad a specialized skill that provides domain-specific instructions and workflows.\n\nWhen you recognize that a task matches one of the available skills listed below, use this tool to load the full skill instructions.\n\nThe skill will inject detailed instructions, workflows, and access to bundled resources (scripts, references, templates) into the conversation context.\n\nTool output includes a `<skill_content name=\"...\">` block with the loaded content.\n\nThe following skills provide specialized sets of instructions for particular tasks\nInvoke this tool to load a skill when a task matches one of the available skills listed below:\n\n## Available Skills\n- **customize-opencode**: Use ONLY when the user is editing or creating opencode's own configuration: opencode.json, opencode.jsonc, files under .opencode/, or files under ~/.config/opencode/. Also use when creating or fixing opencode agents, subagents, skills, plugins, MCP servers, or permission rules. Do not use for the user's own application code, or for any project that is not configuring opencode itself."
}

fn task_description() -> &'static str {
    "Launch a new agent to handle complex, multistep tasks autonomously.\n\nWhen using the Task tool, you must specify a subagent_type parameter to select which agent type to use.\n\nWhen NOT to use the Task tool:\n- If you want to read a specific file path, use the Read or Glob tool instead of the Task tool, to find the match more quickly\n- If you are searching for a specific class definition like \"class Foo\", use the Grep tool instead, to find the match more quickly\n- If you are searching for code within a specific file or set of 2-3 files, use the Read tool instead of the Task tool, to find the match more quickly\n- If no available agent is a good fit for the task, use other tools directly\n\n\nUsage notes:\n1. Launch multiple agents concurrently whenever possible, to maximize performance; to do that, use a single message with multiple tool uses\n2. When the agent is done, it will return a single message back to you. The result returned by the agent is not visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result. The output includes a task_id you can reuse later to continue the same subagent session.\n3. Each agent invocation starts with a fresh context unless you provide task_id to resume the same subagent session (which continues with its previous messages and tool outputs). When starting fresh, your prompt should contain a highly detailed task description for the agent to perform autonomously and you should specify exactly what information the agent should return back to you in its final and only message to you.\n4. The agent's outputs should generally be trusted\n5. Clearly tell the agent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent. Tell it how to verify its work if possible (e.g., relevant test commands).\n6. If the agent description mentions that it should be used proactively, then you should try your best to use it without the user having to ask for it first. Use your judgement.\n\nAvailable agent types and the tools they have access to:\n- explore: Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. \"src/components/**/*.tsx\"), search code for keywords (eg. \"API endpoints\"), or answer questions about the codebase (eg. \"how do API endpoints work?\"). When calling this agent, specify the desired thoroughness level: \"quick\" for basic searches, \"medium\" for moderate exploration, or \"very thorough\" for comprehensive analysis across multiple locations and naming conventions.\n- general: General-purpose agent for researching complex questions and executing multi-step tasks. Use this agent to execute multiple units of work in parallel."
}

fn todowrite_description() -> &'static str {
    "Create and maintain a structured task list for the current coding session. Tracks progress, organizes multi-step work, and surfaces status to the user.\n\n## When to use\nUse proactively when:\n- The task requires 3+ distinct steps or actions (not just 3 tool calls for a single conceptual step)\n- The work is non-trivial and benefits from planning\n- The user provides multiple tasks (numbered or comma-separated) or explicitly asks for a todo list\n- New instructions arrive - capture them as todos\n- You start a task - mark it `in_progress` (only one at a time) before working\n- You finish a task - mark it `completed` and add any follow-ups discovered during the work\n\n## When NOT to use\nSkip when:\n- The work is a single, straightforward task (or <3 trivial steps)\n- The request is purely informational or conversational\n- Tracking adds no organizational value\n\n## States\n- `pending` - not started\n- `in_progress` - actively working (exactly ONE at a time)\n- `completed` - finished successfully\n- `cancelled` - no longer needed\n\n## Rules\n- Update status in real time; don't batch completions\n- Mark `completed` only after the required work is actually done, including any required verification. Never based on intent.\n- Keep exactly one `in_progress` while work remains\n- If blocked or partial, keep it `in_progress` and add a follow-up todo describing the blocker\n- Preserve user-provided commands verbatim (flags, args, order)\n- Items should be specific and actionable; break large work into smaller steps\n\n## Examples\n\nUse it:\n- \"Add a dark mode toggle and run the tests\" -> multi-step feature + explicit verification\n- \"Rename getCwd -> getCurrentWorkingDirectory across the repo\" -> grep reveals 15 occurrences in 8 files\n- \"Implement registration, catalog, cart, checkout\" -> multiple complex features\n\nSkip it:\n- \"How do I print Hello World in Python?\" -> informational\n- \"Add a comment to calculateTotal\" -> single edit\n- \"Run npm install and tell me what happened\" -> one command\n\nWhen in doubt, use it.\n"
}

fn webfetch_description() -> &'static str {
    "- Fetches content from a specified URL\n- Takes a URL and optional format as input\n- Fetches the URL content, converts to requested format (markdown by default)\n- Returns the content in the specified format\n- Use this tool when you need to retrieve and analyze web content\n\nUsage notes:\n  - IMPORTANT: if another tool is present that offers better web fetching capabilities, is more targeted to the task, or has fewer restrictions, prefer using that tool instead of this one.\n  - The URL must be a fully-formed valid URL\n  - HTTP URLs will be automatically upgraded to HTTPS\n  - Format options: \"markdown\" (default), \"text\", or \"html\"\n  - This tool is read-only and does not modify any files\n  - Results may be summarized if the content is very large\n"
}

fn write_description() -> &'static str {
    "Writes a file to the local filesystem.\n\nUsage:\n- This tool will overwrite the existing file if there is one at the provided path.\n- If this is an existing file, you MUST use the Read tool first to read the file's contents. This tool will fail if you did not read the file first.\n- ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n- NEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested by the User.\n- Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked.\n"
}

#[cfg(test)]
mod tests {
    use super::build_request;
    use crate::client_common::Prompt;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::openai_models::ModelInfo;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    const QA_TESTING_SKILLS_INSTRUCTIONS: &str = "<skills_instructions>\n## Skills\nA skill is a set of local instructions to follow that is stored in a `SKILL.md` file.\n### Available skills\n- qa-testing: Run the project's QA test plan against a live build (file: /home/user/skills/.system/qa-testing/SKILL.md)\n### How to use skills\n- Discovery: ...\n</skills_instructions>";

    #[test]
    fn opencode_request_maps_developer_skills_block_to_user_message() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some(codex_protocol::ResponseItemId::from_server(
                        "developer".to_string(),
                    )),
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: QA_TESTING_SKILLS_INSTRUCTIONS.to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: Some(codex_protocol::ResponseItemId::from_server(
                        "user".to_string(),
                    )),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Run the QA pass".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(&prompt, &test_model_info()).expect("build request");

        let messages = request["messages"].as_array().expect("messages array");
        assert_eq!(messages[0]["role"], "system");
        // The developer skills block maps to user-role content rather than
        // being dropped, per the workspace harness instruction-role rule.
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(
            messages[1]["content"].as_str().expect("skills content"),
            QA_TESTING_SKILLS_INSTRUCTIONS
        );
        let request_json = serde_json::to_string(&request).expect("serialize request");
        assert!(request_json.contains("qa-testing"));
        assert!(request_json.contains("Run the project's QA test plan against a live build"));
    }

    fn test_model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "deepseek-chat",
            "display_name": "DeepSeek Chat",
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
}
