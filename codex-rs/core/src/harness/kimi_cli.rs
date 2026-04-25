use crate::client_common::Prompt;
use crate::event_mapping::is_contextual_user_message_content;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::function_call_output_content_items_to_text;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::Value;
use serde_json::json;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

const KIMI_CLI_DEFAULT_MAX_TOKENS: u32 = 32_000;
const KIMI_CLI_SYSTEM_PROMPT_TEMPLATE: &str = include_str!("kimi_cli_prompt.md");
const KIMI_CLI_YOLO_REMINDER: &str = "<system-reminder>\nYou are running in non-interactive mode. The user cannot answer questions or provide feedback during execution.\n- Do NOT call AskUserQuestion. If you need to make a decision, make your best judgment and proceed.\n- For EnterPlanMode / ExitPlanMode, they will be auto-approved. You can use them normally but expect no user feedback.\n</system-reminder>";
const KIMI_LIST_DIR_ROOT_WIDTH: usize = 30;
const KIMI_LIST_DIR_CHILD_WIDTH: usize = 10;
const KIMI_AGENTS_MD_START: &str = "# AGENTS.md instructions for ";
static KIMI_WORK_DIR_LS_CACHE: LazyLock<Mutex<std::collections::HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
    conversation_id: &str,
    session_source: Option<&SessionSource>,
    yolo_mode: bool,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let system_prompt = build_system_prompt(prompt, session_source, conversation_id);
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    messages.extend(build_messages(&prompt.get_formatted_input(), yolo_mode)?);
    let tools = build_tools(&prompt.tools)?;
    let tool_kinds = prompt
        .tools
        .iter()
        .map(|tool| (tool.name().to_string(), ToolOutputKind::Function))
        .collect();

    Ok((
        json!({
            "model": model_info.slug,
            "messages": messages,
            "max_tokens": KIMI_CLI_DEFAULT_MAX_TOKENS,
            "prompt_cache_key": std::env::var("OPEN_INTERPRETER_KIMI_PROMPT_CACHE_KEY_OVERRIDE")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| conversation_id.to_string()),
            "reasoning_effort": Value::Null,
            "stream": true,
            "stream_options": {
                "include_usage": true,
            },
            "thinking": {
                "type": "disabled",
            },
            "tools": tools,
        }),
        tool_kinds,
    ))
}

fn build_system_prompt(
    prompt: &Prompt,
    session_source: Option<&SessionSource>,
    conversation_id: &str,
) -> String {
    let work_dir = prompt
        .cwd
        .as_deref()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let kimi_os = current_kimi_os();
    let mut rendered = KIMI_CLI_SYSTEM_PROMPT_TEMPLATE.to_string();
    rendered = render_conditional_block(
        rendered,
        r#"{% if KIMI_OS == "Windows" %}"#,
        "{% endif %}",
        kimi_os == "Windows",
    );
    rendered = render_conditional_block(
        rendered,
        "{% if KIMI_ADDITIONAL_DIRS_INFO %}",
        "{% endif %}",
        /*include_block*/ false,
    );

    for (name, value) in [
        ("ROLE_ADDITIONAL", role_additional(session_source)),
        ("KIMI_OS", kimi_os),
        ("KIMI_SHELL", kimi_shell()),
        ("KIMI_NOW", current_kimi_now()),
        ("KIMI_WORK_DIR", work_dir.as_path().display().to_string()),
        (
            "KIMI_WORK_DIR_LS",
            cached_work_dir_listing(conversation_id, &work_dir),
        ),
        ("KIMI_AGENTS_MD", extract_agents_md(&prompt.input)),
        ("KIMI_SKILLS", discover_kimi_skills(&work_dir)),
        ("KIMI_ADDITIONAL_DIRS_INFO", String::new()),
    ] {
        rendered = rendered.replace(format!("${{{name}}}").as_str(), value.as_str());
    }

    rendered.trim_end_matches('\n').to_string()
}

fn cached_work_dir_listing(conversation_id: &str, work_dir: &Path) -> String {
    let key = format!("{conversation_id}:{}", work_dir.display());
    let mut cache = KIMI_WORK_DIR_LS_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(key)
        .or_insert_with(|| list_directory(work_dir))
        .clone()
}

fn build_messages(
    items: &[ResponseItem],
    yolo_mode: bool,
) -> Result<impl Iterator<Item = Value>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut pending_tool_calls = Vec::new();
    let mut injected_yolo_reminder = false;

    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    flush_pending_tool_calls(&mut messages, &mut pending_tool_calls);
                    if let Some(message_content) = convert_message_content(content) {
                        if message_content.as_str().is_some_and(str::is_empty) {
                            continue;
                        }
                        messages.push(json!({
                            "role": "assistant",
                            "content": message_content,
                        }));
                    }
                }
                "user" => {
                    if is_contextual_user_message_content(content) {
                        continue;
                    }
                    flush_pending_tool_calls(&mut messages, &mut pending_tool_calls);
                    let mut parts = convert_message_parts(content);
                    if yolo_mode && !injected_yolo_reminder {
                        parts.push(json!({
                            "type": "text",
                            "text": KIMI_CLI_YOLO_REMINDER,
                        }));
                        injected_yolo_reminder = true;
                    }
                    if !parts.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": Value::Array(parts),
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
                        "name": "Shell",
                        "arguments": arguments,
                    }
                }));
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                flush_pending_tool_calls(&mut messages, &mut pending_tool_calls);
                messages.push(json!({
                    "role": "tool",
                    "content": kimi_tool_output_content(output),
                    "tool_call_id": call_id,
                }));
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                flush_pending_tool_calls(&mut messages, &mut pending_tool_calls);
                messages.push(json!({
                    "role": "tool",
                    "content": kimi_tool_output_content(output),
                    "tool_call_id": call_id,
                }));
            }
            ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::Reasoning { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::GhostSnapshot { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::Other => {}
        }
    }

    flush_pending_tool_calls(&mut messages, &mut pending_tool_calls);
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

fn flush_pending_tool_calls(messages: &mut Vec<Value>, pending_tool_calls: &mut Vec<Value>) {
    if pending_tool_calls.is_empty() {
        return;
    }
    messages.push(json!({
        "role": "assistant",
        "content": [],
        "tool_calls": std::mem::take(pending_tool_calls),
    }));
}

fn convert_message_content(content: &[ContentItem]) -> Option<Value> {
    collapse_message_parts(convert_message_parts(content))
}

fn convert_message_parts(content: &[ContentItem]) -> Vec<Value> {
    content
        .iter()
        .map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => json!({
                "type": "text",
                "text": text,
            }),
            ContentItem::InputImage { image_url, .. } => json!({
                "type": "image_url",
                "image_url": {
                    "url": image_url,
                }
            }),
        })
        .collect()
}

fn collapse_message_parts(parts: Vec<Value>) -> Option<Value> {
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

fn kimi_tool_output_content(output: &FunctionCallOutputPayload) -> Value {
    if output.success == Some(false) {
        let text = output
            .text_content()
            .map(str::to_string)
            .or_else(|| {
                output
                    .content_items()
                    .and_then(function_call_output_content_items_to_text)
            })
            .unwrap_or_else(|| output.to_string());
        return json!(format!("<system>ERROR: {text}</system>"));
    }

    match &output.body {
        codex_protocol::models::FunctionCallOutputBody::Text(text) => json!(text),
        codex_protocol::models::FunctionCallOutputBody::ContentItems(items) => {
            let content = items
                .iter()
                .map(kimi_output_content_item)
                .collect::<Vec<_>>();
            collapse_message_parts(content).unwrap_or_else(|| Value::Array(Vec::new()))
        }
    }
}

fn kimi_output_content_item(item: &FunctionCallOutputContentItem) -> Value {
    match item {
        FunctionCallOutputContentItem::InputText { text } => json!({
            "type": "text",
            "text": text,
        }),
        FunctionCallOutputContentItem::InputImage { image_url, .. } => json!({
            "type": "image_url",
            "image_url": {
                "url": image_url,
            }
        }),
    }
}

fn role_additional(session_source: Option<&SessionSource>) -> String {
    if matches!(
        session_source,
        Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. }))
    ) {
        "You are operating as a subagent instance spawned from a parent Kimi Code CLI conversation. Focus on the assigned subtask, keep your response concise, and do not assume direct access to the human user.".to_string()
    } else {
        String::new()
    }
}

fn current_kimi_os() -> String {
    match std::env::consts::OS {
        "macos" => "macOS".to_string(),
        "windows" => "Windows".to_string(),
        "linux" => "Linux".to_string(),
        other => other.to_string(),
    }
}

fn kimi_shell() -> String {
    if cfg!(windows) {
        "powershell (`powershell.exe`)".to_string()
    } else {
        "bash (`/bin/bash`)".to_string()
    }
}

fn extract_agents_md(items: &[ResponseItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } if text.starts_with(KIMI_AGENTS_MD_START) => {
                Some(text.as_str())
            }
            ContentItem::InputText { .. }
            | ContentItem::OutputText { .. }
            | ContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn discover_kimi_skills(work_dir: &Path) -> String {
    let mut seen = HashSet::new();
    let mut skills = kimi_skill_roots(
        work_dir,
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from),
        std::env::var_os("KIMI_CLI_SOURCE_DIR").map(PathBuf::from),
    )
    .into_iter()
    .flat_map(|root| discover_skills_in_root(&root))
    .filter(|skill| seen.insert(skill.name.to_ascii_lowercase()))
    .collect::<Vec<_>>();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    if skills.is_empty() {
        "No skills found.".to_string()
    } else {
        skills
            .into_iter()
            .map(|skill| {
                format!(
                    "- {}\n  - Path: {}\n  - Description: {}",
                    skill.name,
                    skill.path.display(),
                    skill.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn kimi_skill_roots(
    work_dir: &Path,
    home_dir: Option<PathBuf>,
    kimi_source_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = builtin_kimi_skills_root(kimi_source_dir.as_deref()) {
        roots.push(root);
    }
    if let Some(home) = home_dir {
        if let Some(root) = first_existing_dir([
            home.join(".kimi/skills"),
            home.join(".claude/skills"),
            home.join(".codex/skills"),
        ]) {
            roots.push(root);
        }
        if let Some(root) = first_existing_dir([
            home.join(".config/agents/skills"),
            home.join(".agents/skills"),
        ]) {
            roots.push(root);
        }
    }
    if let Some(root) = first_existing_dir([
        work_dir.join(".kimi/skills"),
        work_dir.join(".claude/skills"),
        work_dir.join(".codex/skills"),
    ]) {
        roots.push(root);
    }
    if let Some(root) = first_existing_dir([work_dir.join(".agents/skills")]) {
        roots.push(root);
    }
    roots
}

fn builtin_kimi_skills_root(kimi_source_dir: Option<&Path>) -> Option<PathBuf> {
    let path = kimi_source_dir?.join("src/kimi_cli/skills");
    path.is_dir().then_some(path)
}

fn first_existing_dir<I>(candidates: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    candidates.into_iter().find(|candidate| candidate.is_dir())
}

fn discover_skills_in_root(root: &Path) -> Vec<KimiSkill> {
    let mut skills = fs::read_dir(root)
        .ok()
        .into_iter()
        .flat_map(|iter| iter.filter_map(Result::ok))
        .filter_map(|entry| {
            let skill_md = entry.path().join("SKILL.md");
            parse_kimi_skill(&skill_md)
        })
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    skills
}

fn parse_kimi_skill(skill_md: &Path) -> Option<KimiSkill> {
    let text = fs::read_to_string(skill_md).ok()?;
    let (name, description) = parse_kimi_skill_frontmatter(text.as_str())?;
    Some(KimiSkill {
        name,
        description,
        path: skill_md.to_path_buf(),
    })
}

fn parse_kimi_skill_frontmatter(text: &str) -> Option<(String, String)> {
    let frontmatter = text
        .strip_prefix("---\n")
        .and_then(|remaining| remaining.split_once("\n---\n"))
        .map(|(frontmatter, _)| frontmatter)?;
    let mut name = None;
    let mut description = None;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            description = Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    Some((name?, description?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KimiSkill {
    name: String,
    description: String,
    path: PathBuf,
}

fn render_conditional_block(
    mut template: String,
    start_marker: &str,
    end_marker: &str,
    include_block: bool,
) -> String {
    while let Some(start_index) = template.find(start_marker) {
        let block_with_start = &template[start_index + start_marker.len()..];
        let Some(end_offset) = block_with_start.find(end_marker) else {
            break;
        };
        let end_index = start_index + start_marker.len() + end_offset;
        let replacement = if include_block {
            block_with_start[..end_offset]
                .trim_matches('\n')
                .to_string()
        } else {
            String::new()
        };
        let mut replace_end = end_index + end_marker.len();
        if !include_block
            && template
                .as_bytes()
                .get(replace_end)
                .is_some_and(|byte| *byte == b'\n')
        {
            replace_end += 1;
        }
        template.replace_range(start_index..replace_end, replacement.as_str());
    }
    template
}

fn current_kimi_now() -> String {
    env::var("OPEN_INTERPRETER_KIMI_NOW_OVERRIDE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
        })
}

fn list_directory(work_dir: &Path) -> String {
    let (entries, total) = collect_entries(work_dir, KIMI_LIST_DIR_ROOT_WIDTH);
    let mut lines = Vec::new();
    let remaining = total.saturating_sub(entries.len());

    for (index, (name, is_dir)) in entries.iter().enumerate() {
        let is_last = index + 1 == entries.len() && remaining == 0;
        let connector = if is_last { "└── " } else { "├── " };
        if *is_dir {
            lines.push(format!("{connector}{name}/"));
            let child_prefix = if is_last { "    " } else { "│   " };
            let child_path = work_dir.join(name);
            let (children, child_total) = collect_entries(&child_path, KIMI_LIST_DIR_CHILD_WIDTH);
            let child_remaining = child_total.saturating_sub(children.len());
            for (child_index, (child_name, child_is_dir)) in children.iter().enumerate() {
                let child_is_last = child_index + 1 == children.len() && child_remaining == 0;
                let child_connector = if child_is_last {
                    "└── "
                } else {
                    "├── "
                };
                let suffix = if *child_is_dir { "/" } else { "" };
                lines.push(format!(
                    "{child_prefix}{child_connector}{child_name}{suffix}"
                ));
            }
            if child_remaining > 0 {
                lines.push(format!("{child_prefix}└── ... and {child_remaining} more"));
            }
        } else {
            lines.push(format!("{connector}{name}"));
        }
    }

    if remaining > 0 {
        lines.push(format!("└── ... and {remaining} more entries"));
    }

    if lines.is_empty() {
        "(empty directory)".to_string()
    } else {
        lines.join("\n")
    }
}

fn collect_entries(dir: &Path, max_width: usize) -> (Vec<(String, bool)>, usize) {
    let mut entries = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|iter| iter.filter_map(Result::ok))
        .map(|entry| {
            let path = entry.path();
            let is_dir = entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            (name, is_dir, path)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.1.cmp(&right.1).reverse().then(left.0.cmp(&right.0)));
    let total = entries.len();
    let collected = entries
        .into_iter()
        .take(max_width)
        .map(|(name, is_dir, _path)| (name, is_dir))
        .collect();
    (collected, total)
}

#[cfg(test)]
mod tests {
    use super::build_messages;
    use super::build_request;
    use super::discover_skills_in_root;
    use super::kimi_skill_roots;
    use super::parse_kimi_skill_frontmatter;
    use crate::client_common::Prompt;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::openai_models::ModelInfo;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::fs;

    #[test]
    fn kimi_user_messages_stay_as_typed_content_blocks() {
        let items = vec![ResponseItem::Message {
            id: Some("user".to_string()),
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "hello".to_string(),
            }],
            end_turn: None,
            phase: None,
        }];

        let messages = build_messages(&items, /*yolo_mode*/ false)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "hello",
                    }
                ],
            })]
        );
    }

    #[test]
    fn kimi_request_omits_openai_specific_chat_fields_but_keeps_kimi_fields() {
        let prompt = Prompt {
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
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            "conversation-id",
            None,
            /*yolo_mode*/ false,
        )
        .expect("build request");

        assert_eq!(
            request.get("prompt_cache_key"),
            Some(&json!("conversation-id"))
        );
        assert_eq!(request.get("tool_choice"), None);
        assert_eq!(request.get("parallel_tool_calls"), None);
        assert_eq!(request.get("store"), None);
    }

    #[test]
    fn kimi_skill_parser_reads_frontmatter_name_and_description() {
        let text = r#"---
name: demo-guide
description: Read the demo creation workflow guide
---

Body
"#;

        assert_eq!(
            parse_kimi_skill_frontmatter(text),
            Some((
                "demo-guide".to_string(),
                "Read the demo creation workflow guide".to_string(),
            ))
        );
    }

    #[test]
    fn kimi_skill_discovery_uses_brand_then_generic_roots() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work_dir = temp.path().join("workspace");
        fs::create_dir_all(home.join(".claude/skills/demo-guide")).expect("claude skills dir");
        fs::create_dir_all(home.join(".agents/skills/generic-guide")).expect("generic skills dir");
        fs::create_dir_all(&work_dir).expect("workspace dir");
        fs::write(
            home.join(".claude/skills/demo-guide/SKILL.md"),
            "---\nname: demo-guide\ndescription: Read the demo creation workflow guide\n---\n",
        )
        .expect("write claude skill");
        fs::write(
            home.join(".agents/skills/generic-guide/SKILL.md"),
            "---\nname: generic-guide\ndescription: Read the generic workflow guide\n---\n",
        )
        .expect("write generic skill");

        let skills = kimi_skill_roots(&work_dir, Some(home.clone()), None)
            .into_iter()
            .flat_map(|root| discover_skills_in_root(&root))
            .map(|skill| {
                format!(
                    "- {}\n  - Path: {}\n  - Description: {}",
                    skill.name,
                    skill.path.display(),
                    skill.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            skills,
            format!(
                "- demo-guide\n  - Path: {}\n  - Description: Read the demo creation workflow guide\n- generic-guide\n  - Path: {}\n  - Description: Read the generic workflow guide",
                home.join(".claude/skills/demo-guide/SKILL.md").display(),
                home.join(".agents/skills/generic-guide/SKILL.md").display(),
            )
        );
    }

    fn test_model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "kimi-k2.5",
            "display_name": "Kimi K2.5",
            "description": null,
            "supported_reasoning_levels": [],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "availability_nux": null,
            "upgrade": null,
            "base_instructions": "base",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "default_reasoning_summary": "auto",
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": "freeform",
            "truncation_policy": {
                "mode": "bytes",
                "limit": 10000
            },
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": null,
            "auto_compact_token_limit": null,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "input_modalities": ["text", "image"],
            "supports_search_tool": false
        }))
        .expect("deserialize test model")
    }
}
