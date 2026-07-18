use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::harness::kimi_cli;
use crate::harness::session_skills::SessionSkill;
use crate::harness::session_skills::parse_session_skills;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use futures::StreamExt;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::Mutex;
use tokio::sync::mpsc;

const KIMI_CODE_SYSTEM_PROMPT: &str = include_str!("kimi_code_system_prompt.md");
const KIMI_CODE_TOOLS: &str = include_str!("kimi_code_tools.json");
const KIMI_CODE_AUTO_PERMISSION_REMINDER: &str = "<system-reminder>\nAuto permission mode is active. Tool approvals will be handled automatically while this mode remains enabled.\n  - Continue normally without pausing for approval prompts.\n  - Do NOT call AskUserQuestion while auto mode is active. Make a reasonable decision and continue without asking the user.\n  - ExitPlanMode is also approved automatically, without the user reviewing the plan. An auto-approved plan is NOT a signal from the user to start executing — follow the user's original instructions on whether to proceed.\n</system-reminder>";
const KIMI_CODE_SKILLS_HEADER: &str =
    "DISREGARD any earlier skill listings. Current available skills:";
const KIMI_CODE_BUILTIN_SKILLS: &str = r#"### Built-in
- check-kimi-code-docs: Answer questions about the Kimi Code product using the official documentation — CLI usage, configuration, slash commands, features, membership and quota, API onboarding, third-party tool setup, and error codes. Use when the user asks how Kimi Code w…
  Path: builtin://check-kimi-code-docs
- update-config: Inspect or edit kimi-code's own config — `config.toml` (model, provider, permission, hooks) and `tui.toml` (theme, editor, notifications, auto-update). Use when the user asks what a setting does or wants to change one.
  Path: builtin://update-config
- write-goal: Help the user craft a well-specified `/goal` objective for goal mode — turn a rough intention into a completion contract with a clear finish line, proof, boundaries, and stop rule. Use when the user asks for help writing, refining, or improving a go…
  Path: builtin://write-goal"#;
static KIMI_CODE_SYSTEM_PROMPT_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
    conversation_id: &str,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let mut messages = vec![json!({
        "role": "system",
        "content": cached_system_prompt(prompt, conversation_id),
    })];
    messages.extend(add_auto_permission_reminders(
        kimi_cli::build_messages_with_options(
            prompt.get_formatted_input(),
            kimi_cli::MessageBuildOptions::kimi_code(),
        )?
        .collect(),
    ));
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
            "max_completion_tokens": 32768,
            "prompt_cache_key": kimi_code_prompt_cache_key(conversation_id),
            "stream": true,
            "stream_options": {
                "include_usage": true,
            },
            "tools": tools,
            "thinking": {
                "type": "enabled",
                "keep": "all",
            },
        }),
        tool_kinds,
    ))
}

/// Keeps Kimi reasoning available for the assistant tool-call message that is
/// sent on the next provider request.
pub(crate) fn preserve_reasoning_content(stream: ResponseStream) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel(1600);

    tokio::spawn(async move {
        let mut stream = stream;
        while let Some(mut event) = stream.next().await {
            if let Ok(ResponseEvent::OutputItemAdded(item) | ResponseEvent::OutputItemDone(item)) =
                &mut event
            {
                make_reasoning_content_durable(item);
            }
            if tx_event.send(event).await.is_err() {
                return;
            }
        }
    });

    ResponseStream {
        rx_event,
        consumer_dropped: tokio_util::sync::CancellationToken::new(),
    }
}

fn make_reasoning_content_durable(item: &mut ResponseItem) {
    let ResponseItem::Reasoning {
        content: Some(content),
        ..
    } = item
    else {
        return;
    };
    for part in content {
        if let ReasoningItemContent::ReasoningText { text } = part {
            *part = ReasoningItemContent::Text {
                text: std::mem::take(text),
            };
        }
    }
}

fn kimi_code_prompt_cache_key(conversation_id: &str) -> String {
    format!("session_{conversation_id}")
}

fn cached_system_prompt(prompt: &Prompt, conversation_id: &str) -> String {
    let cwd = prompt.cwd.as_deref().unwrap_or_else(|| Path::new("."));
    let skills = session_skills_listing(prompt);
    let mut skills_hasher = DefaultHasher::new();
    skills.hash(&mut skills_hasher);
    let key = format!(
        "{conversation_id}:{}:{:016x}",
        cwd.display(),
        skills_hasher.finish()
    );
    let mut cache = KIMI_CODE_SYSTEM_PROMPT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(key)
        .or_insert_with(|| build_system_prompt(prompt, &skills))
        .clone()
}

/// Renders Kimi Code's source-backed model-facing skill listing.
fn session_skills_listing(prompt: &Prompt) -> String {
    let session_skills = parse_session_skills(&prompt.input);
    let mut listing = KIMI_CODE_SKILLS_HEADER.to_string();
    if !session_skills.is_empty() {
        let mut skills_by_name: BTreeMap<String, SessionSkill> = BTreeMap::new();
        for skill in session_skills {
            let prefer_skill = skills_by_name.get(&skill.name).is_none_or(|current| {
                !current.path.contains("/.agents/skills/")
                    && skill.path.contains("/.agents/skills/")
            });
            if prefer_skill {
                skills_by_name.insert(skill.name.clone(), skill);
            }
        }
        listing.push_str("\n### Project");
        for skill in skills_by_name.into_values() {
            let _ = write!(
                listing,
                "\n- {}: {}\n  Path: {}",
                skill.name,
                kimi_code_skill_description(&skill.description),
                skill.path
            );
        }
    }
    listing.push('\n');
    listing.push_str(KIMI_CODE_BUILTIN_SKILLS);
    listing
}

fn kimi_code_skill_description(description: &str) -> String {
    if description.chars().count() <= 250 {
        return description.to_string();
    }
    let mut truncated = description.chars().take(249).collect::<String>();
    truncated.push('…');
    truncated
}

fn add_auto_permission_reminders(messages: Vec<Value>) -> Vec<Value> {
    let mut with_reminders = Vec::with_capacity(messages.len() + 1);
    for message in messages {
        let is_user = message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| role == "user");
        let already_reminder = message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| content == KIMI_CODE_AUTO_PERMISSION_REMINDER);
        with_reminders.push(message);
        if is_user && !already_reminder {
            with_reminders.push(json!({
                "role": "user",
                "content": KIMI_CODE_AUTO_PERMISSION_REMINDER,
            }));
        }
    }
    with_reminders
}

fn build_system_prompt(prompt: &Prompt, skills: &str) -> String {
    let cwd = prompt.cwd.as_deref().unwrap_or_else(|| Path::new("."));
    let listing = kimi_work_dir_listing(cwd);
    KIMI_CODE_SYSTEM_PROMPT
        .replace("\r\n", "\n")
        .replace(
            "{% if KIMI_OS == \"Windows\" %}\n\nIMPORTANT: You are on Windows. The Bash tool runs through Git Bash, so use Unix shell syntax inside Bash commands — `/dev/null` not `NUL`, and forward slashes in paths. For file operations, always prefer the built-in tools (Read, Write, Edit, Glob, Grep) over Bash commands — they work reliably across all platforms.\n{% endif %}",
            "",
        )
        .replace(
            "{% if KIMI_ADDITIONAL_DIRS_INFO %}\n\n## Additional Directories\n\nThe following directories have been added to the workspace. You can read, write, search, and glob files in these directories as part of your workspace scope.\n\n{{ KIMI_ADDITIONAL_DIRS_INFO }}\n{% endif %}",
            "",
        )
        .replace("{{ ROLE_ADDITIONAL }}", "")
        .replace("{{ KIMI_OS }}", kimi_os_label())
        .replace("{{ KIMI_SHELL }}", "bash (`/bin/bash`)")
        .replace("{{ KIMI_NOW }}", &kimi_now())
        .replace("{{ KIMI_WORK_DIR }}", cwd.display().to_string().as_str())
        .replace("{{ KIMI_WORK_DIR_LS }}", &listing)
        .replace("{{ KIMI_ADDITIONAL_DIRS_INFO }}", "")
        .replace("{{ KIMI_AGENTS_MD }}", "")
        .replace("{{ KIMI_SKILLS }}", skills)
        .replace("{% if KIMI_SKILLS %}\n", "\n")
        .replace("{% endif %}\n", "\n")
}

fn kimi_os_label() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    }
}

fn kimi_now() -> String {
    if let Ok(fake_time) = std::env::var("OPENINTERPRETER_TEST_TIME") {
        if let Some((date, time)) = fake_time.split_once(' ') {
            return format!("{date}T{time}.000Z");
        }
        return fake_time;
    }
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn kimi_work_dir_listing(cwd: &Path) -> String {
    let entries = match sorted_dir_entries(cwd) {
        Ok(entries) => entries,
        Err(_) => return String::new(),
    };
    let mut lines = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let last = index + 1 == entries.len();
        let branch = if last { "└── " } else { "├── " };
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().is_dir() {
            lines.push(format!("{branch}{name}/"));
            if !name.starts_with('.') {
                append_child_listing(&mut lines, &entry.path(), last);
            }
        } else {
            lines.push(format!("{branch}{name}"));
        }
    }
    lines.join("\n")
}

fn append_child_listing(lines: &mut Vec<String>, dir: &Path, parent_last: bool) {
    let Ok(children) = sorted_dir_entries(dir) else {
        return;
    };
    let prefix = if parent_last { "    " } else { "│   " };
    for (index, child) in children.iter().take(20).enumerate() {
        let child_last = index + 1 == children.len().min(20);
        let branch = if child_last {
            "└── "
        } else {
            "├── "
        };
        let mut name = child.file_name().to_string_lossy().to_string();
        if child.path().is_dir() {
            name.push('/');
        }
        lines.push(format!("{prefix}{branch}{name}"));
    }
    if children.len() > 20 {
        lines.push(format!(
            "{prefix}└── ... and {} more",
            children.len().saturating_sub(20)
        ));
    }
}

fn sorted_dir_entries(cwd: &Path) -> std::io::Result<Vec<std::fs::DirEntry>> {
    let entries = std::fs::read_dir(cwd)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries {
        if entry.path().is_dir() {
            dirs.push(entry);
        } else {
            files.push(entry);
        }
    }
    sort_kimi_entries(&mut dirs);
    sort_kimi_entries(&mut files);
    dirs.extend(files);
    Ok(dirs)
}

fn sort_kimi_entries(entries: &mut [std::fs::DirEntry]) {
    entries.sort_by(|left, right| {
        let left_name = left.file_name().to_string_lossy().to_string();
        let right_name = right.file_name().to_string_lossy().to_string();
        left_name
            .to_lowercase()
            .cmp(&right_name.to_lowercase())
            .then_with(|| left_name.cmp(&right_name))
    });
}

fn build_tools() -> Vec<Value> {
    serde_json::from_str(KIMI_CODE_TOOLS).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::build_request;
    use super::make_reasoning_content_durable;
    use super::session_skills_listing;
    use crate::client_common::Prompt;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::ReasoningItemContent;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::openai_models::ModelInfo;
    use serde_json::json;

    #[test]
    fn kimi_code_reasoning_content_remains_serializable_for_follow_up_requests() {
        let mut item = ResponseItem::Reasoning {
            id: Some("reasoning-1".to_string()),
            summary: Vec::new(),
            content: Some(vec![ReasoningItemContent::ReasoningText {
                text: "Use the requested tool.".to_string(),
            }]),
            encrypted_content: None,
            internal_chat_message_metadata_passthrough: None,
        };

        make_reasoning_content_durable(&mut item);

        assert_eq!(
            item,
            ResponseItem::Reasoning {
                id: Some("reasoning-1".to_string()),
                summary: Vec::new(),
                content: Some(vec![ReasoningItemContent::Text {
                    text: "Use the requested tool.".to_string(),
                }]),
                encrypted_content: None,
                internal_chat_message_metadata_passthrough: None,
            }
        );
        assert_eq!(
            serde_json::to_value(&item).expect("serialize reasoning"),
            json!({
                "type": "reasoning",
                "id": "reasoning-1",
                "summary": [],
                "content": [{
                    "type": "text",
                    "text": "Use the requested tool.",
                }],
                "encrypted_content": null,
            })
        );
    }

    #[test]
    fn kimi_code_request_renders_kimi_code_builtin_skills() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some(std::convert::identity("user".to_string())),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Run the QA pass".to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            "kimi-code-session-skills-conversation",
        )
        .expect("build request");

        let system = request["messages"][0]["content"]
            .as_str()
            .expect("system content");
        assert!(system.contains("DISREGARD any earlier skill listings"));
        assert!(system.contains("- check-kimi-code-docs:"));
        assert!(system.contains("- update-config: Inspect or edit kimi-code's own config"));
        assert!(system.contains("Path: builtin://update-config"));
        assert!(system.contains("- write-goal:"));
        assert!(!system.contains('\r'));
        assert!(!system.contains("{{ KIMI_SKILLS }}"));
        assert!(!system.contains("{% if KIMI_SKILLS %}"));
        assert!(!system.contains("<skills_instructions>"));
    }

    #[test]
    fn kimi_code_request_matches_current_kimi_k3_transport_options_and_tools() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some(std::convert::identity("user".to_string())),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Inspect the project".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            "kimi-code-current-request-conversation",
        )
        .expect("build request");

        assert_eq!(
            request["thinking"],
            json!({
                "type": "enabled",
                "keep": "all",
            })
        );
        assert_eq!(request["max_completion_tokens"], json!(32768));
        assert!(request.get("reasoning_effort").is_none());

        let tool_names = request["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| {
                tool["function"]["name"]
                    .as_str()
                    .expect("tool name")
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tool_names,
            [
                "Agent",
                "AgentSwarm",
                "AskUserQuestion",
                "Bash",
                "CreateGoal",
                "CronCreate",
                "CronDelete",
                "CronList",
                "Edit",
                "EnterPlanMode",
                "ExitPlanMode",
                "FetchURL",
                "GetGoal",
                "Glob",
                "Grep",
                "Read",
                "ReadMediaFile",
                "SetGoalBudget",
                "Skill",
                "TaskList",
                "TaskOutput",
                "TaskStop",
                "TodoList",
                "UpdateGoal",
                "Write",
            ]
        );

        let messages = request["messages"].as_array().expect("messages array");
        assert!(messages.iter().any(|message| {
            message["content"].as_str().is_some_and(|content| {
                content.contains("ExitPlanMode is also approved automatically")
            })
        }));
    }

    #[test]
    fn kimi_code_request_renders_open_interpreter_session_skills() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some(std::convert::identity(
                        "developer".to_string(),
                    )),
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "<skills_instructions>\n## Skills\nA skill is a set of local instructions to follow that is stored in a `SKILL.md` file.\n### Available skills\n- qa-testing: Run the project's QA test plan against a live build (file: /home/user/skills/.system/qa-testing/SKILL.md)\n### How to use skills\n- Discovery: ...\n</skills_instructions>"
                            .to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: Some(std::convert::identity(
                        "user".to_string(),
                    )),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "hello".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
            ],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            "kimi-code-no-skills-conversation",
        )
        .expect("build request");

        let system = request["messages"][0]["content"]
            .as_str()
            .expect("system content");
        assert!(!system.contains("{{ KIMI_SKILLS }}"));
        assert!(system.contains("### Project"));
        assert!(
            system.contains("- qa-testing: Run the project's QA test plan against a live build")
        );
        assert!(system.contains("Path: /home/user/skills/.system/qa-testing/SKILL.md"));
        assert!(!system.contains("<skills_instructions>"));
    }

    #[test]
    fn kimi_code_request_deduplicates_and_truncates_project_skills() {
        let description = "x".repeat(251);
        let skills = format!(
            "<skills_instructions>\n## Skills\n### Available skills\n- qa-testing: {description} (file: /home/user/.openinterpreter/skills/.system/qa-testing/SKILL.md)\n- qa-testing: {description} (file: /workspace/.agents/skills/qa-testing/SKILL.md)\n### How to use skills\n- Discovery: ...\n</skills_instructions>"
        );
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some("developer".to_string()),
                role: "developer".to_string(),
                content: vec![ContentItem::InputText { text: skills }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }],
            ..Prompt::default()
        };

        let listing = session_skills_listing(&prompt);

        assert_eq!(listing.matches("- qa-testing:").count(), 1);
        assert!(listing.contains(&format!("- qa-testing: {}…", "x".repeat(249))));
        assert!(listing.contains("Path: /workspace/.agents/skills/qa-testing/SKILL.md"));
        assert!(!listing.contains(".openinterpreter/skills/.system"));
        assert!(
            listing.find("### Project").expect("project section")
                < listing.find("### Built-in").expect("built-in section")
        );
    }

    #[test]
    fn kimi_code_request_renders_tool_call_ids_with_underscores() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some(std::convert::identity("user".to_string())),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Find Python files".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Glob".to_string(),
                    namespace: None,
                    arguments: r#"{"pattern":"*.py"}"#.to_string(),
                    call_id: "Glob:0".to_string(),

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: "Glob:0".to_string(),
                    output: FunctionCallOutputPayload::from_text("module.py".to_string()),

                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            "kimi-code-tool-call-id-conversation",
        )
        .expect("build request");
        let messages = request["messages"].as_array().expect("messages array");
        let assistant_tool_call = messages
            .iter()
            .find_map(|message| message.get("tool_calls"))
            .expect("assistant tool call");
        let tool_message = messages
            .iter()
            .find(|message| message.get("role").and_then(serde_json::Value::as_str) == Some("tool"))
            .expect("tool message");

        assert_eq!(assistant_tool_call[0]["id"], json!("Glob_0"));
        assert_eq!(tool_message["tool_call_id"], json!("Glob_0"));
    }

    #[test]
    fn kimi_code_request_preserves_image_content() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some(std::convert::identity("user".to_string())),
                role: "user".to_string(),
                content: vec![
                    ContentItem::InputText {
                        text: "Describe this image.".to_string(),
                    },
                    ContentItem::InputImage {
                        image_url: "data:image/png;base64,AAAB".to_string(),
                        detail: None,
                    },
                ],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            "kimi-code-image-content-conversation",
        )
        .expect("build request");

        assert_eq!(
            request["messages"][1]["content"],
            json!([
                {
                    "type": "text",
                    "text": "Describe this image."
                },
                {
                    "type": "image_url",
                    "image_url": {
                        "url": "data:image/png;base64,AAAB",
                        "id": null
                    }
                }
            ])
        );
    }

    #[test]
    fn kimi_code_request_preserves_video_tool_content() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some("user".to_string()),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Read the video.".to_string(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "ReadMediaFile".to_string(),
                    namespace: None,
                    arguments: r#"{"path":"probe.mp4"}"#.to_string(),
                    call_id: "ReadMediaFile:1".to_string(),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: "ReadMediaFile:1".to_string(),
                    output: FunctionCallOutputPayload::from_content_items(vec![
                        FunctionCallOutputContentItem::InputText {
                            text: "<video path=\"/workspace/probe.mp4\">".to_string(),
                        },
                        FunctionCallOutputContentItem::InputVideo {
                            video_url: "ms://file-123".to_string(),
                            id: Some("file-123".to_string()),
                        },
                        FunctionCallOutputContentItem::InputText {
                            text: "</video>".to_string(),
                        },
                    ]),
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            "kimi-code-video-content-conversation",
        )
        .expect("build request");
        let tool_message = request["messages"]
            .as_array()
            .expect("messages array")
            .iter()
            .find(|message| message["role"] == json!("tool"))
            .expect("tool message");

        assert_eq!(
            tool_message["content"],
            json!([
                {
                    "type": "text",
                    "text": "<video path=\"/workspace/probe.mp4\">"
                },
                {
                    "type": "video_url",
                    "video_url": {
                        "url": "ms://file-123",
                        "id": "file-123"
                    }
                },
                {
                    "type": "text",
                    "text": "</video>"
                }
            ])
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
