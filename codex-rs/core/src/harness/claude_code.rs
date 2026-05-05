use crate::client_common::Prompt;
use crate::event_mapping::is_contextual_user_message_content;
use crate::harness::claude_code_prompt::build_child_agent_system_prompt;
use crate::harness::claude_code_prompt::build_system_prompt;
use chrono::Datelike;
use codex_api::AnthropicCacheControl;
use codex_api::AnthropicContentBlock;
use codex_api::AnthropicContextEdit;
use codex_api::AnthropicContextManagement;
use codex_api::AnthropicMessage;
use codex_api::AnthropicMessageContent;
use codex_api::AnthropicMessageRequest;
use codex_api::AnthropicOutputConfig;
use codex_api::AnthropicOutputFormat;
use codex_api::AnthropicRequestMetadata;
use codex_api::AnthropicTextBlock;
use codex_api::AnthropicThinkingConfig;
use codex_api::AnthropicTool;
use codex_api::anthropic::AnthropicToolResultContent;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_tools::ToolSpec;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

pub(crate) const CLAUDE_CODE_BETA_HEADER: &str = "claude-code-20250219,interleaved-thinking-2025-05-14,context-management-2025-06-27,prompt-caching-scope-2026-01-05,advisor-tool-2026-03-01,effort-2025-11-24";
pub(crate) const CLAUDE_CODE_TITLE_BETA_HEADER: &str = "interleaved-thinking-2025-05-14,context-management-2025-06-27,prompt-caching-scope-2026-01-05,advisor-tool-2026-03-01,structured-outputs-2025-12-15";
pub(crate) const CLAUDE_CODE_STARTUP_HEAD_USER_AGENT: &str = "Bun/1.3.14";
pub(crate) const CLAUDE_CODE_STARTUP_MODELS_USER_AGENT: &str = "claude-code/2.1.126";
pub(crate) const CLAUDE_CODE_USER_AGENT: &str = "claude-cli/2.1.126 (external, sdk-cli)";
pub(crate) const CLAUDE_CODE_APP_HEADER: &str = "cli";
const CLAUDE_CODE_DEFAULT_MAX_TOKENS: u32 = 32_000;
const CLAUDE_CODE_OPUS_4_6_PLUS_MAX_TOKENS: u32 = 64_000;
const CLAUDE_CODE_VERSION: &str = "2.1.126";
const CLAUDE_CODE_BILLING_VERSION_SALT: &str = "59cf53e54c78";
const CLAUDE_CODE_BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header: cc_version=";
const CLAUDE_CODE_BILLING_ENTRYPOINT: &str = "sdk-cli";
const CLAUDE_CODE_SYSTEM_PROMPT_HEADER: &str =
    "You are a Claude agent, built on Anthropic's Claude Agent SDK.";
const CLAUDE_CODE_METADATA_DEVICE_ID: &str =
    "5ac70074a85c7e515d6d6a5e5f442a6fe84d73ee6791b5b88d8c03e67dcfea6e";
const CLAUDE_CODE_TITLE_MODEL: &str = "claude-haiku-4-5-20251001";
const CLAUDE_CODE_TITLE_PROMPT: &str = "Generate a concise, sentence-case title (3-7 words) that captures the main topic or goal of this coding session. The title should be clear enough that the user recognizes the session in a list. Use sentence case: capitalize only the first word and proper nouns.\n\nReturn JSON with a single \"title\" field.\n\nGood examples:\n{\"title\": \"Fix login button on mobile\"}\n{\"title\": \"Add OAuth authentication\"}\n{\"title\": \"Debug failing CI tests\"}\n{\"title\": \"Refactor API client error handling\"}\n\nBad (too vague): {\"title\": \"Code changes\"}\nBad (too long): {\"title\": \"Investigate and fix the issue where the login button does not respond on mobile devices\"}\nBad (wrong case): {\"title\": \"Fix Login Button On Mobile\"}";
const CLAUDE_CODE_TODO_REMINDER_TOOL_GAP: usize = 8;
const CLAUDE_CODE_TODO_REMINDER_STALENESS_THRESHOLD: usize = 10;
const CLAUDE_CODE_TODO_REMINDER_PREFIX: &str = "<system-reminder>\nThe TodoWrite tool hasn't been used recently. If you're working on tasks that would benefit from tracking progress, consider using the TodoWrite tool to track progress. Also consider cleaning up the todo list if has become stale and no longer matches what you are working on. Only use it if it's relevant to the current work. This is just a gentle reminder - ignore if not applicable. Make sure that you NEVER mention this reminder to the user\n\n\nHere are the existing contents of your todo list:\n\n";
const CLAUDE_REFERENCE_SKILLS_REMINDER: &str = r#"- update-config: Use this skill to configure the Claude Code harness via settings.json. Automated behaviors ("from now on when X", "each time X", "whenever X", "before/after X") require hooks configured in settings.json - the harness executes these, not Claude, so memory/preferences cannot fulfill them. Also use for: permissions ("allow X", "add permission", "move permission to"), env vars ("set X=Y"), hook troubleshooting, or any changes to settings.json/settings.local.json files. Examples: "allow npm commands", "add bq permission to global settings", "move permission to user settings", "set DEBUG=true", "when claude stops show X". For simple settings like theme/model, suggest the /config command.
- keybindings-help: Use when the user wants to customize keyboard shortcuts, rebind keys, add chord bindings, or modify ~/.claude/keybindings.json. Examples: "rebind ctrl+s", "add a chord shortcut", "change the submit key", "customize keybindings".
- simplify: Review changed code for reuse, quality, and efficiency, then fix any issues found.
- fewer-permission-prompts: Scan your transcripts for common read-only Bash and MCP tool calls, then add a prioritized allowlist to project .claude/settings.json to reduce permission prompts.
- loop: Run a prompt or slash command on a recurring interval (e.g. /loop 5m /foo, defaults to 10m) - When the user wants to set up a recurring task, poll for status, or run something repeatedly on an interval (e.g. "check the deploy every 5 minutes", "keep running /babysit-prs"). Do NOT invoke for one-off tasks.
- schedule: Create, update, list, or run scheduled remote agents (routines) that execute on a cron schedule. - When the user wants to schedule a recurring remote agent, set up automated tasks, create a cron job for Claude Code, or manage their scheduled agents/routines. Also use when the user wants a one-time scheduled run ("run this once at 3pm", "remind me to check X tomorrow").
- claude-api: Build, debug, and optimize Claude API / Anthropic SDK apps. Apps built with this skill should include prompt caching. Also handles migrating existing Claude API code between Claude model versions (4.5 → 4.6, 4.6 → 4.7, retired-model replacements).
TRIGGER when: code imports `anthropic`/`@anthropic-ai/sdk`; user asks for the Claude API, Anthropic SDK, or Managed Agents; user adds/modifies/tunes a Claude feature (caching, thinking, compaction, tool use, batch, files, citations, memory) or model (Opus/Sonnet/Haiku) in a file; questions about prompt caching / cache hit rate in an Anthropic SDK project.
SKIP: file imports `openai`/other-provider SDK, filename like `*-openai.py`/`*-generic.py`, provider-neutral code, general programming/ML.
- init: Initialize a new CLAUDE.md file with codebase documentation
- review: Review a pull request
- security-review: Complete a security review of the pending changes on the current branch"#;

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    session_id: &str,
    session_source: Option<&SessionSource>,
) -> Result<AnthropicMessageRequest, serde_json::Error> {
    let billing_version_source = first_non_contextual_user_text(prompt)
        .unwrap_or_default()
        .to_string();
    let is_child_agent_request = matches!(
        session_source,
        Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. }))
    );
    let mut messages = build_messages(&prompt.input, !is_child_agent_request)?;
    if !is_child_agent_request {
        ensure_skills_reminder(&mut messages);
    }
    prepend_current_date_reminder(&mut messages);
    apply_message_cache_breakpoint(&mut messages);
    normalize_plain_text_messages(&mut messages);
    let max_tokens = claude_code_max_tokens(model_info.slug.as_str());
    let thinking = if is_child_agent_request || matches!(effort, Some(ReasoningEffortConfig::None))
    {
        None
    } else if claude_code_uses_adaptive_thinking(model_info.slug.as_str()) {
        Some(AnthropicThinkingConfig::adaptive())
    } else {
        Some(AnthropicThinkingConfig::enabled(max_tokens - 1))
    };
    let output_config = if is_child_agent_request {
        Some(AnthropicOutputConfig {
            effort: Some("high".to_string()),
            format: None,
        })
    } else {
        claude_code_output_config(model_info, effort)
    };

    let tools = build_tools(&prompt.tools, is_child_agent_request)?;
    let system_prompt = if is_child_agent_request {
        build_child_agent_system_prompt(prompt, model_info.slug.as_str())
    } else {
        build_system_prompt(prompt, model_info.slug.as_str())
    };
    let system = vec![
        AnthropicTextBlock::new(build_billing_header(
            "message",
            model_info.slug.as_str(),
            billing_version_source.as_str(),
            &messages,
            &tools,
        )),
        AnthropicTextBlock::ephemeral(CLAUDE_CODE_SYSTEM_PROMPT_HEADER.to_string()),
        AnthropicTextBlock::ephemeral(system_prompt),
    ];

    Ok(AnthropicMessageRequest {
        model: model_info.slug.clone(),
        messages,
        system,
        tools,
        thinking,
        context_management: (!is_child_agent_request).then_some(AnthropicContextManagement {
            edits: vec![AnthropicContextEdit {
                edit_type: "clear_thinking_20251015",
                keep: "all",
            }],
        }),
        output_config,
        metadata: Some(build_request_metadata(session_id)),
        temperature: is_child_agent_request.then_some(1),
        max_tokens,
        stream: true,
    })
}

pub(crate) fn build_title_request(
    prompt: &Prompt,
    session_id: &str,
) -> Result<Option<AnthropicMessageRequest>, serde_json::Error> {
    let Some(title_text) = first_non_contextual_user_text(prompt) else {
        return Ok(None);
    };
    let title_text = title_text.trim_end_matches('\n').to_string();
    let messages = vec![AnthropicMessage {
        role: "user".to_string(),
        content: AnthropicMessageContent::Blocks(vec![AnthropicContentBlock::Text {
            text: title_text.clone(),
            cache_control: None,
        }]),
    }];
    let tools = vec![];
    let system = vec![
        AnthropicTextBlock::new(build_billing_header(
            "title",
            CLAUDE_CODE_TITLE_MODEL,
            title_text.as_str(),
            &messages,
            &tools,
        )),
        AnthropicTextBlock::new(CLAUDE_CODE_SYSTEM_PROMPT_HEADER.to_string()),
        AnthropicTextBlock::new(CLAUDE_CODE_TITLE_PROMPT.to_string()),
    ];
    let output_config = Some(AnthropicOutputConfig {
        effort: None,
        format: Some(AnthropicOutputFormat::JsonSchema {
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string"
                    }
                },
                "required": ["title"],
                "additionalProperties": false
            }),
        }),
    });
    Ok(Some(AnthropicMessageRequest {
        model: CLAUDE_CODE_TITLE_MODEL.to_string(),
        messages,
        system,
        tools,
        thinking: None,
        context_management: None,
        output_config,
        metadata: Some(build_request_metadata(session_id)),
        temperature: Some(1),
        max_tokens: CLAUDE_CODE_DEFAULT_MAX_TOKENS,
        stream: true,
    }))
}

fn first_non_contextual_user_text(prompt: &Prompt) -> Option<&str> {
    prompt.input.iter().find_map(|item| match item {
        ResponseItem::Message { role, content, .. }
            if role == "user" && !is_contextual_user_message_content(content) =>
        {
            content.iter().find_map(|content_item| match content_item {
                ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                    Some(text.as_str())
                }
                ContentItem::InputImage { .. } => None,
            })
        }
        _ => None,
    })
}

fn claude_code_output_config(
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
) -> Option<AnthropicOutputConfig> {
    let output_effort = anthropic_output_effort(effort).or_else(|| {
        claude_code_uses_adaptive_thinking(model_info.slug.as_str())
            .then(|| claude_code_default_effort(model_info.slug.as_str()))
            .flatten()
    });
    if output_effort.is_none() && anthropic_output_effort_is_unsupported(model_info) {
        return None;
    }
    output_effort.map(|effort| AnthropicOutputConfig {
        effort: Some(effort.to_string()),
        format: None,
    })
}

fn anthropic_output_effort_is_unsupported(model_info: &ModelInfo) -> bool {
    model_info.default_reasoning_level.is_some() && model_info.supported_reasoning_levels.is_empty()
}

fn anthropic_output_effort(effort: Option<ReasoningEffortConfig>) -> Option<&'static str> {
    match effort {
        Some(ReasoningEffortConfig::Minimal | ReasoningEffortConfig::Low) => Some("low"),
        Some(ReasoningEffortConfig::Medium) => Some("medium"),
        Some(ReasoningEffortConfig::High) => Some("high"),
        Some(ReasoningEffortConfig::XHigh) => Some("xhigh"),
        Some(ReasoningEffortConfig::None) | None => None,
    }
}

fn claude_code_default_effort(model_slug: &str) -> Option<&'static str> {
    if is_claude_opus_4_7_or_newer(model_slug) {
        Some("xhigh")
    } else if claude_code_uses_adaptive_thinking(model_slug) {
        Some("high")
    } else {
        None
    }
}

fn claude_code_max_tokens(model_slug: &str) -> u32 {
    if is_claude_opus_4_6_or_newer(model_slug) {
        CLAUDE_CODE_OPUS_4_6_PLUS_MAX_TOKENS
    } else {
        CLAUDE_CODE_DEFAULT_MAX_TOKENS
    }
}

fn claude_code_uses_adaptive_thinking(model_slug: &str) -> bool {
    canonical_claude_model_slug(model_slug) == "claude-mythos-preview"
        || is_claude_opus_4_6_or_newer(model_slug)
        || is_claude_sonnet_4_6_or_newer(model_slug)
}

fn is_claude_opus_4_6_or_newer(model_slug: &str) -> bool {
    claude_model_minor_version(model_slug, "opus").is_some_and(|minor| minor >= 6)
}

fn is_claude_opus_4_7_or_newer(model_slug: &str) -> bool {
    claude_model_minor_version(model_slug, "opus").is_some_and(|minor| minor >= 7)
}

fn is_claude_sonnet_4_6_or_newer(model_slug: &str) -> bool {
    claude_model_minor_version(model_slug, "sonnet").is_some_and(|minor| minor >= 6)
}

fn claude_model_minor_version(model_slug: &str, family: &str) -> Option<u32> {
    let model_slug = canonical_claude_model_slug(model_slug);
    [
        format!("claude-{family}-4-"),
        format!("claude-{family}-4."),
        format!("claude-{family}4-"),
        format!("claude-{family}4."),
    ]
    .into_iter()
    .find_map(|prefix| {
        model_slug
            .strip_prefix(prefix.as_str())
            .and_then(parse_leading_minor_version)
    })
}

fn canonical_claude_model_slug(model_slug: &str) -> &str {
    model_slug
        .split(':')
        .next()
        .unwrap_or(model_slug)
        .rsplit('/')
        .next()
        .unwrap_or(model_slug)
}

fn parse_leading_minor_version(suffix: &str) -> Option<u32> {
    let digits = suffix
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

fn current_local_date() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

fn build_messages(
    items: &[ResponseItem],
    include_skills_reminder: bool,
) -> Result<Vec<AnthropicMessage>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut tool_names_by_call_id = HashMap::new();
    let mut todo_reminder = ClaudeTodoReminderState::default();
    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    let blocks = content
                        .iter()
                        .filter_map(map_message_content_item)
                        .collect::<Vec<_>>();
                    if blocks
                        .iter()
                        .any(|block| matches!(block, AnthropicContentBlock::Text { .. }))
                    {
                        todo_reminder.record_assistant_text_message();
                    }
                    push_message(&mut messages, "assistant", blocks);
                }
                "user" => {
                    if is_contextual_user_message_content(content) {
                        continue;
                    }
                    let blocks = content
                        .iter()
                        .filter_map(map_message_content_item)
                        .collect::<Vec<_>>();
                    push_message(&mut messages, "user", blocks);
                }
                "developer" => {
                    let blocks = content
                        .iter()
                        .filter_map(|item| {
                            include_skills_reminder
                                .then(|| map_claude_code_developer_content_item(item))
                                .flatten()
                        })
                        .collect::<Vec<_>>();
                    push_message(&mut messages, "user", blocks);
                }
                "system" => {}
                _ => {
                    let blocks = content
                        .iter()
                        .filter_map(map_message_content_item)
                        .collect::<Vec<_>>();
                    push_message(&mut messages, "user", blocks);
                }
            },
            ResponseItem::Reasoning {
                content,
                summary,
                encrypted_content,
                ..
            } => {
                let thinking = content
                    .iter()
                    .flatten()
                    .map(|entry| match entry {
                        ReasoningItemContent::ReasoningText { text }
                        | ReasoningItemContent::Text { text } => text.as_str(),
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let thinking = if thinking.is_empty() {
                    summary
                        .iter()
                        .map(|entry| {
                            match entry {
                            codex_protocol::models::ReasoningItemReasoningSummary::SummaryText {
                                text,
                            } => text.as_str(),
                        }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    thinking
                };
                push_message(
                    &mut messages,
                    "assistant",
                    vec![AnthropicContentBlock::Thinking {
                        thinking,
                        signature: encrypted_content.clone(),
                    }],
                );
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                tool_names_by_call_id.insert(call_id.clone(), name.clone());
                let input: Value = serde_json::from_str(arguments)?;
                todo_reminder.record_tool_call(name, &input);
                push_message(
                    &mut messages,
                    "assistant",
                    vec![AnthropicContentBlock::ToolUse {
                        id: call_id.clone(),
                        name: name.clone(),
                        input,
                    }],
                );
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                let tool_name = tool_names_by_call_id.get(call_id).map(String::as_str);
                let todo_reminder_text = todo_reminder.reminder_for_tool_result(tool_name);
                let is_error = if output.success == Some(false) {
                    Some(true)
                } else if tool_name.is_some_and(|name| name == "Bash") {
                    Some(false)
                } else {
                    None
                };
                push_message(
                    &mut messages,
                    "user",
                    vec![AnthropicContentBlock::ToolResult {
                        tool_use_id: call_id.clone(),
                        content: build_claude_tool_result_content(
                            tool_name,
                            &output.body,
                            todo_reminder_text.as_deref(),
                        ),
                        is_error,
                        cache_control: None,
                    }],
                );
            }
            ResponseItem::CustomToolCall { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::GhostSnapshot { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::Other => {}
        }
    }
    Ok(messages)
}

fn build_claude_tool_result_content(
    tool_name: Option<&str>,
    body: &FunctionCallOutputBody,
    todo_reminder_text: Option<&str>,
) -> AnthropicToolResultContent {
    match body {
        FunctionCallOutputBody::Text(content) => {
            normalize_claude_tool_result_text(tool_name, content.clone(), todo_reminder_text).into()
        }
        FunctionCallOutputBody::ContentItems(items) => {
            let blocks = items
                .iter()
                .filter_map(map_tool_result_content_item)
                .collect::<Vec<_>>();
            if blocks.len() == 1 {
                normalize_claude_tool_result_text(
                    tool_name,
                    blocks[0].text.clone(),
                    todo_reminder_text,
                )
                .into()
            } else {
                blocks.into()
            }
        }
    }
}

fn normalize_claude_tool_result_text(
    tool_name: Option<&str>,
    content: String,
    todo_reminder_text: Option<&str>,
) -> String {
    let normalized = if matches!(tool_name, Some("Bash" | "TodoWrite")) {
        trim_single_trailing_newline(content)
    } else {
        content
    };

    if let Some(todo_reminder_text) = todo_reminder_text {
        format!("{normalized}\n\n{todo_reminder_text}")
    } else {
        normalized
    }
}

#[derive(Default)]
struct ClaudeTodoReminderState {
    latest_todo_snapshot: Option<String>,
    non_todo_tool_outputs_since_last_todo: usize,
    assistant_text_messages_since_last_todo: usize,
    reminder_emitted_for_current_list: bool,
}

impl ClaudeTodoReminderState {
    fn record_assistant_text_message(&mut self) {
        self.assistant_text_messages_since_last_todo += 1;
    }

    fn record_tool_call(&mut self, tool_name: &str, input: &Value) {
        if tool_name == "TodoWrite" {
            self.latest_todo_snapshot = todo_snapshot_from_tool_input(input);
        }
    }

    fn reminder_for_tool_result(&mut self, tool_name: Option<&str>) -> Option<String> {
        match tool_name {
            Some("TodoWrite") => {
                self.non_todo_tool_outputs_since_last_todo = 0;
                self.assistant_text_messages_since_last_todo = 0;
                self.reminder_emitted_for_current_list = false;
                None
            }
            Some(_) => {
                self.non_todo_tool_outputs_since_last_todo += 1;
                if self.reminder_emitted_for_current_list
                    || self.non_todo_tool_outputs_since_last_todo
                        < CLAUDE_CODE_TODO_REMINDER_TOOL_GAP
                    || self.non_todo_tool_outputs_since_last_todo
                        + self.assistant_text_messages_since_last_todo
                        < CLAUDE_CODE_TODO_REMINDER_STALENESS_THRESHOLD
                {
                    return None;
                }
                self.reminder_emitted_for_current_list = true;
                self.latest_todo_snapshot
                    .as_deref()
                    .map(build_todo_reminder_text)
            }
            None => None,
        }
    }
}

fn todo_snapshot_from_tool_input(input: &Value) -> Option<String> {
    let todos = input.get("todos")?.as_array()?;
    let lines = todos
        .iter()
        .enumerate()
        .map(|(index, todo)| {
            let status = todo.get("status")?.as_str()?;
            let content = todo.get("content")?.as_str()?;
            Some(format!("{}. [{}] {content}", index + 1, status))
        })
        .collect::<Option<Vec<_>>>()?;

    (!lines.is_empty()).then(|| format!("[{}]", lines.join("\n")))
}

fn build_todo_reminder_text(todo_snapshot: &str) -> String {
    format!("{CLAUDE_CODE_TODO_REMINDER_PREFIX}{todo_snapshot}\n</system-reminder>")
}

fn trim_single_trailing_newline(mut content: String) -> String {
    if content.ends_with("\r\n") {
        content.truncate(content.len() - 2);
    } else if content.ends_with('\n') {
        content.pop();
    }
    content
}

fn map_message_content_item(item: &ContentItem) -> Option<AnthropicContentBlock> {
    match item {
        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
            Some(AnthropicContentBlock::Text {
                text: text.clone(),
                cache_control: None,
            })
        }
        ContentItem::InputImage { .. } => Some(AnthropicContentBlock::Text {
            text: "[image omitted by claude-code harness]".to_string(),
            cache_control: None,
        }),
    }
}

fn map_tool_result_content_item(
    item: &FunctionCallOutputContentItem,
) -> Option<AnthropicTextBlock> {
    match item {
        FunctionCallOutputContentItem::InputText { text } => {
            Some(AnthropicTextBlock::new(text.clone()))
        }
        FunctionCallOutputContentItem::InputImage { .. } => Some(AnthropicTextBlock::new(
            "[image omitted by claude-code harness]".to_string(),
        )),
    }
}

fn map_claude_code_developer_content_item(item: &ContentItem) -> Option<AnthropicContentBlock> {
    let ContentItem::InputText { text } = item else {
        return None;
    };
    extract_claude_code_skills_reminder(text).map(|text| AnthropicContentBlock::Text {
        text,
        cache_control: None,
    })
}

fn extract_claude_code_skills_reminder(text: &str) -> Option<String> {
    let body = text
        .strip_prefix(SKILLS_INSTRUCTIONS_OPEN_TAG)?
        .strip_suffix(SKILLS_INSTRUCTIONS_CLOSE_TAG)?
        .trim();
    let available_skills = body
        .split_once("### Available skills\n")?
        .1
        .split_once("\n### How to use skills")
        .map(|(skills, _)| skills.trim_end())
        .unwrap_or_default();
    if available_skills.is_empty() {
        return None;
    }
    Some(format!(
        "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n{CLAUDE_REFERENCE_SKILLS_REMINDER}\n</system-reminder>\n"
    ))
}

fn is_claude_code_skills_reminder_block(block: &AnthropicContentBlock) -> bool {
    matches!(
        block,
        AnthropicContentBlock::Text { text, .. }
            if text.starts_with(
                "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n"
            )
    )
}

fn ensure_skills_reminder(messages: &mut Vec<AnthropicMessage>) {
    let reminder = AnthropicContentBlock::Text {
        text: format!(
            "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n{CLAUDE_REFERENCE_SKILLS_REMINDER}\n</system-reminder>\n"
        ),
        cache_control: None,
    };
    if let Some(first_user_message) = messages.iter_mut().find(|message| message.role == "user") {
        let Some(blocks) = first_user_message.content.blocks_mut() else {
            return;
        };
        if !blocks.iter().any(is_claude_code_skills_reminder_block) {
            blocks.insert(0, reminder);
        }
    } else {
        messages.insert(
            0,
            AnthropicMessage {
                role: "user".to_string(),
                content: vec![reminder].into(),
            },
        );
    }
}

fn apply_message_cache_breakpoint(messages: &mut [AnthropicMessage]) {
    let mut last_cacheable_block = None;
    for (message_idx, message) in messages.iter_mut().enumerate() {
        let Some(blocks) = message.content.blocks_mut() else {
            continue;
        };
        for (content_idx, block) in blocks.iter_mut().enumerate() {
            match block {
                AnthropicContentBlock::Text {
                    cache_control,
                    text,
                } => {
                    *cache_control = None;
                    if !text.is_empty() {
                        last_cacheable_block = Some((message_idx, content_idx));
                    }
                }
                AnthropicContentBlock::ToolResult { cache_control, .. } => {
                    *cache_control = None;
                    last_cacheable_block = Some((message_idx, content_idx));
                }
                AnthropicContentBlock::Thinking { .. } | AnthropicContentBlock::ToolUse { .. } => {}
            }
        }
    }

    if let Some((message_idx, content_idx)) = last_cacheable_block {
        let Some(blocks) = messages[message_idx].content.blocks_mut() else {
            return;
        };
        match &mut blocks[content_idx] {
            AnthropicContentBlock::Text { cache_control, .. }
            | AnthropicContentBlock::ToolResult { cache_control, .. } => {
                *cache_control = Some(AnthropicCacheControl::ephemeral());
            }
            AnthropicContentBlock::Thinking { .. } | AnthropicContentBlock::ToolUse { .. } => {}
        }
    }
}

fn push_message(
    messages: &mut Vec<AnthropicMessage>,
    role: &str,
    blocks: Vec<AnthropicContentBlock>,
) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut()
        && last.role == role
        && let Some(last_blocks) = last.content.blocks_mut()
    {
        last_blocks.extend(blocks);
    } else {
        messages.push(AnthropicMessage {
            role: role.to_string(),
            content: blocks.into(),
        });
    }
}

fn prepend_current_date_reminder(messages: &mut Vec<AnthropicMessage>) {
    let reminder = AnthropicContentBlock::Text {
        text: format!(
            "<system-reminder>\nAs you answer the user's questions, you can use the following context:\n# currentDate\nToday's date is {}.\n\n      IMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n\n",
            current_local_date()
        ),
        cache_control: None,
    };
    if let Some(first_user_message) = messages.iter_mut().find(|message| message.role == "user") {
        if let Some(blocks) = first_user_message.content.blocks_mut() {
            let insert_idx = blocks
                .first()
                .map(is_claude_code_skills_reminder_block)
                .map(usize::from)
                .unwrap_or(0);
            blocks.insert(insert_idx, reminder);
        }
    } else {
        messages.insert(
            0,
            AnthropicMessage {
                role: "user".to_string(),
                content: vec![reminder].into(),
            },
        );
    }
}

fn normalize_plain_text_messages(messages: &mut [AnthropicMessage]) {
    for message in messages {
        if message.role != "user" {
            continue;
        }
        let Some(blocks) = message.content.blocks() else {
            continue;
        };
        if let [
            AnthropicContentBlock::Text {
                text,
                cache_control: None,
            },
        ] = blocks
        {
            message.content = AnthropicMessageContent::Text(text.clone());
        }
    }
}

fn build_tools(
    tools: &[ToolSpec],
    is_child_agent_request: bool,
) -> Result<Vec<AnthropicTool>, serde_json::Error> {
    let mut tools = tools
        .iter()
        .filter_map(|tool| match tool {
            ToolSpec::Function(tool) => {
                if is_child_agent_request
                    && matches!(
                        tool.name.as_str(),
                        "AskUserQuestion" | "EnterPlanMode" | "ExitPlanMode" | "TaskOutput"
                    )
                {
                    return None;
                }
                match tool.name.as_str() {
                "LSP" => None,
                "Bash" => Some(Ok(AnthropicTool {
                    name: "Bash".to_string(),
                    description: tool.description.clone(),
                    input_schema: serde_json::json!({
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "properties": {
                            "command": {
                                "description": "The command to execute",
                                "type": "string"
                            },
                            "timeout": {
                                "description": "Optional timeout in milliseconds (max 600000)",
                                "type": "number"
                            },
                            "description": {
                                "description": "Clear, concise description of what this command does in active voice. Never use words like \"complex\" or \"risk\" in the description - just describe what it does.\n\nFor simple commands (git, npm, standard CLI tools), keep it brief (5-10 words):\n- ls → \"List files in current directory\"\n- git status → \"Show working tree status\"\n- npm install → \"Install package dependencies\"\n\nFor commands that are harder to parse at a glance (piped commands, obscure flags, etc.), add enough context to clarify what it does:\n- find . -name \"*.tmp\" -exec rm {} \\; → \"Find and delete all .tmp files recursively\"\n- git reset --hard origin/main → \"Discard all local changes and match remote main\"\n- curl -s url | jq '.data[]' → \"Fetch JSON from URL and extract data array elements\"",
                                "type": "string"
                            },
                            "run_in_background": {
                                "description": "Set to true to run this command in the background. Use Read to read the output later.",
                                "type": "boolean"
                            },
                            "dangerouslyDisableSandbox": {
                                "description": "Set this to true to dangerously override sandbox mode and run commands without sandboxing.",
                                "type": "boolean"
                            }
                        },
                        "required": ["command"],
                        "additionalProperties": false
                    }),
                })),
                "Edit" => Some(Ok(AnthropicTool {
                    name: "Edit".to_string(),
                    description: tool.description.clone(),
                    input_schema: serde_json::json!({
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "description": "The absolute path to the file to modify",
                                "type": "string"
                            },
                            "old_string": {
                                "description": "The text to replace",
                                "type": "string"
                            },
                            "new_string": {
                                "description": "The text to replace it with (must be different from old_string)",
                                "type": "string"
                            },
                            "replace_all": {
                                "description": "Replace all occurrences of old_string (default false)",
                                "default": false,
                                "type": "boolean"
                            }
                        },
                        "required": ["file_path", "old_string", "new_string"],
                        "additionalProperties": false
                    }),
                })),
                "Read" => Some(Ok(AnthropicTool {
                    name: "Read".to_string(),
                    description: tool.description.clone(),
                    input_schema: serde_json::json!({
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "description": "The absolute path to the file to read",
                                "type": "string"
                            },
                            "offset": {
                                "description": "The line number to start reading from. Only provide if the file is too large to read at once",
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 9007199254740991_i64
                            },
                            "limit": {
                                "description": "The number of lines to read. Only provide if the file is too large to read at once.",
                                "type": "integer",
                                "exclusiveMinimum": 0,
                                "maximum": 9007199254740991_i64
                            },
                            "pages": {
                                "description": "Page range for PDF files (e.g., \"1-5\", \"3\", \"10-20\"). Only applicable to PDF files. Maximum 20 pages per request.",
                                "type": "string"
                            }
                        },
                        "required": ["file_path"],
                        "additionalProperties": false
                    }),
                })),
                "TodoWrite" => Some(Ok(AnthropicTool {
                    name: "TodoWrite".to_string(),
                    description: tool.description.clone(),
                    input_schema: serde_json::json!({
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "properties": {
                            "todos": {
                                "description": "The updated todo list",
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "content": {
                                            "type": "string",
                                            "minLength": 1
                                        },
                                        "status": {
                                            "type": "string",
                                            "enum": ["pending", "in_progress", "completed"]
                                        },
                                        "activeForm": {
                                            "type": "string",
                                            "minLength": 1
                                        }
                                    },
                                    "required": ["content", "status", "activeForm"],
                                    "additionalProperties": false
                                }
                            }
                        },
                        "required": ["todos"],
                        "additionalProperties": false
                    }),
                })),
                _ => Some(serde_json::to_value(&tool.parameters).map(|input_schema| {
                    AnthropicTool {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        input_schema: add_json_schema_draft(input_schema),
                    }
                })),
            }
            }
            ToolSpec::Namespace(_) => None,
            ToolSpec::ToolSearch {
                description,
                parameters,
                ..
            } => Some(serde_json::to_value(parameters).map(|input_schema| AnthropicTool {
                name: "ToolSearch".to_string(),
                description: description.clone(),
                input_schema: add_json_schema_draft(input_schema),
            })),
            ToolSpec::LocalShell {}
            | ToolSpec::ImageGeneration { .. }
            | ToolSpec::WebSearch { .. }
            | ToolSpec::Freeform(_) => None,
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    let existing_names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    tools.extend(reference_claude_supplemental_tools(
        &existing_names,
        is_child_agent_request,
    )?);
    normalize_reference_claude_tool_descriptions(&mut tools);
    tools.sort_by_key(|tool| match tool.name.as_str() {
        "Agent" => 0,
        "AskUserQuestion" => 1,
        "Bash" => 2,
        "CronCreate" => 3,
        "CronDelete" => 4,
        "CronList" => 5,
        "Edit" => 6,
        "EnterPlanMode" => 7,
        "EnterWorktree" => 8,
        "ExitPlanMode" => 9,
        "ExitWorktree" => 10,
        "Glob" => 11,
        "Grep" => 12,
        "Monitor" => 13,
        "NotebookEdit" => 14,
        "PushNotification" => 15,
        "Read" => 16,
        "RemoteTrigger" => 17,
        "ScheduleWakeup" => 18,
        "Skill" => 19,
        "TaskOutput" => 20,
        "TaskStop" => 21,
        "TodoWrite" => 22,
        "ToolSearch" => 23,
        "WebFetch" => 24,
        "WebSearch" => 25,
        "Write" => 26,
        _ => 27,
    });
    Ok(tools)
}

fn normalize_reference_claude_tool_descriptions(tools: &mut [AnthropicTool]) {
    for tool in tools {
        match tool.name.as_str() {
            "Agent" => {
                tool.description = tool.description.replace(
                    "- Explore: Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. \"src/components/**/*.tsx\"), search code for keywords (eg. \"API endpoints\"), or answer questions about the codebase (eg. \"how do API endpoints work?\"). When calling this agent, specify the desired thoroughness level: \"quick\" for basic searches, \"medium\" for moderate exploration, or \"very thorough\" for comprehensive analysis across multiple locations and naming conventions. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)",
                    "- Explore: Fast read-only search agent for locating code. Use it to find files by pattern (eg. \"src/components/**/*.tsx\"), grep for symbols or keywords (eg. \"API endpoints\"), or answer \"where is X defined / which files reference Y.\" Do NOT use it for code review, design-doc auditing, cross-file consistency checks, or open-ended analysis — it reads excerpts rather than whole files and will miss content past its read window. When calling, specify search breadth: \"quick\" for a single targeted lookup, \"medium\" for moderate exploration, or \"very thorough\" to search across multiple locations and naming conventions. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)",
                );
            }
            "Bash" => {
                tool.description = tool.description.replace(
                    "  - If your command is long running and you would like to be notified when it finishes — use `run_in_background`. No sleep needed.\n  - Do not retry failing commands in a sleep loop — diagnose the root cause.\n  - If waiting for a background task you started with `run_in_background`, you will be notified when it completes — do not poll.\n  - If you must poll an external process, use a check command (e.g. `gh run view`) rather than sleeping first.\n  - If you must sleep, keep the duration short to avoid blocking the user.",
                    "  - Use the Monitor tool to stream events from a background process (each stdout line is a notification). For one-shot \"wait until done,\" use Bash with run_in_background instead.\n  - If your command is long running and you would like to be notified when it finishes — use `run_in_background`. No sleep needed.\n  - Do not retry failing commands in a sleep loop — diagnose the root cause.\n  - If waiting for a background task you started with `run_in_background`, you will be notified when it completes — do not poll.\n  - Long leading `sleep` commands are blocked. To poll until a condition is met, use Monitor with an until-loop (e.g. `until <check>; do sleep 2; done`) — you get a notification when the loop exits. Do not chain shorter sleeps to work around the block.",
                );
            }
            "Read" => {
                tool.description = tool
                    .description
                    .replace(
                        "- You can optionally specify a line offset and limit (especially handy for long files), but it's recommended to read the whole file by not providing these parameters",
                        "- When you already know which part of the file you need, only read that part. This can be important for larger files.",
                    )
                    .replace(
                        "- This tool can only read files, not directories. To read a directory, use an ls command via the Bash tool.",
                        "- This tool can only read files, not directories. To list files in a directory, use the registered shell tool.",
                    );
            }
            "WebSearch" => {
                let date = current_local_date();
                let current_month = format!("{} {}", month_name(date.month()), date.year());
                tool.description = tool.description.replace(
                    "The current month is April 2026.",
                    format!("The current month is {current_month}.").as_str(),
                );
            }
            _ => {}
        }
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "January",
    }
}

fn reference_claude_supplemental_tools(
    existing_names: &HashSet<&str>,
    is_child_agent_request: bool,
) -> Result<Vec<AnthropicTool>, serde_json::Error> {
    [
        (
            "CronCreate",
            r###"{"name":"CronCreate","description":"Schedule a prompt to be enqueued at a future time. Use for both recurring schedules and one-shot reminders.\n\nUses standard 5-field cron in the user's local timezone: minute hour day-of-month month day-of-week. \"0 9 * * *\" means 9am local — no timezone conversion needed.\n\n## One-shot tasks (recurring: false)\n\nFor \"remind me at X\" or \"at <time>, do Y\" requests — fire once then auto-delete.\nPin minute/hour/day-of-month/month to specific values:\n  \"remind me at 2:30pm today to check the deploy\" → cron: \"30 14 <today_dom> <today_month> *\", recurring: false\n  \"tomorrow morning, run the smoke test\" → cron: \"57 8 <tomorrow_dom> <tomorrow_month> *\", recurring: false\n\n## Recurring jobs (recurring: true, the default)\n\nFor \"every N minutes\" / \"every hour\" / \"weekdays at 9am\" requests:\n  \"*/5 * * * *\" (every 5 min), \"0 * * * *\" (hourly), \"0 9 * * 1-5\" (weekdays at 9am local)\n\n## Avoid the :00 and :30 minute marks when the task allows it\n\nEvery user who asks for \"9am\" gets `0 9`, and every user who asks for \"hourly\" gets `0 *` — which means requests from across the planet land on the API at the same instant. When the user's request is approximate, pick a minute that is NOT 0 or 30:\n  \"every morning around 9\" → \"57 8 * * *\" or \"3 9 * * *\" (not \"0 9 * * *\")\n  \"hourly\" → \"7 * * * *\" (not \"0 * * * *\")\n  \"in an hour or so, remind me to...\" → pick whatever minute you land on, don't round\n\nOnly use minute 0 or 30 when the user names that exact time and clearly means it (\"at 9:00 sharp\", \"at half past\", coordinating with a meeting). When in doubt, nudge a few minutes early or late — the user will not notice, and the fleet will.\n\n## Session-only\n\nJobs live only in this Claude session — nothing is written to disk, and the job is gone when Claude exits.\n\n## Runtime behavior\n\nJobs only fire while the REPL is idle (not mid-query). The scheduler adds a small deterministic jitter on top of whatever you pick: recurring tasks fire up to 10% of their period late (max 15 min); one-shot tasks landing on :00 or :30 fire up to 90 s early. Picking an off-minute is still the bigger lever.\n\nRecurring tasks auto-expire after 7 days — they fire one final time, then are deleted. This bounds session lifetime. Tell the user about the 7-day limit when scheduling recurring jobs.\n\nReturns a job ID you can pass to CronDelete.","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"cron":{"description":"Standard 5-field cron expression in local time: \"M H DoM Mon DoW\" (e.g. \"*/5 * * * *\" = every 5 minutes, \"30 14 28 2 *\" = Feb 28 at 2:30pm local once).","type":"string"},"prompt":{"description":"The prompt to enqueue at each fire time.","type":"string"},"recurring":{"description":"true (default) = fire on every cron match until deleted or auto-expired after 7 days. false = fire once at the next match, then auto-delete. Use false for \"remind me at X\" one-shot requests with pinned minute/hour/dom/month.","type":"boolean"},"durable":{"description":"true = persist to .claude/scheduled_tasks.json and survive restarts. false (default) = in-memory only, dies when this Claude session ends. Use true only when the user asks the task to survive across sessions.","type":"boolean"}},"required":["cron","prompt"],"additionalProperties":false}}"###,
        ),
        (
            "CronDelete",
            r###"{"name":"CronDelete","description":"Cancel a cron job previously scheduled with CronCreate. Removes it from the in-memory session store.","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"id":{"description":"Job ID returned by CronCreate.","type":"string"}},"required":["id"],"additionalProperties":false}}"###,
        ),
        (
            "CronList",
            r###"{"name":"CronList","description":"List all cron jobs scheduled via CronCreate in this session.","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{},"additionalProperties":false}}"###,
        ),
        (
            "EnterPlanMode",
            r###"{"name":"EnterPlanMode","description":"Use this tool proactively when you're about to start a non-trivial implementation task. Getting user sign-off on your approach before writing code prevents wasted effort and ensures alignment. This tool transitions you into plan mode where you can explore the codebase and design an implementation approach for user approval.\n\n## When to Use This Tool\n\n**Prefer using EnterPlanMode** for implementation tasks unless they're simple. Use it when ANY of these conditions apply:\n\n1. **New Feature Implementation**: Adding meaningful new functionality\n   - Example: \"Add a logout button\" - where should it go? What should happen on click?\n   - Example: \"Add form validation\" - what rules? What error messages?\n\n2. **Multiple Valid Approaches**: The task can be solved in several different ways\n   - Example: \"Add caching to the API\" - could use Redis, in-memory, file-based, etc.\n   - Example: \"Improve performance\" - many optimization strategies possible\n\n3. **Code Modifications**: Changes that affect existing behavior or structure\n   - Example: \"Update the login flow\" - what exactly should change?\n   - Example: \"Refactor this component\" - what's the target architecture?\n\n4. **Architectural Decisions**: The task requires choosing between patterns or technologies\n   - Example: \"Add real-time updates\" - WebSockets vs SSE vs polling\n   - Example: \"Implement state management\" - Redux vs Context vs custom solution\n\n5. **Multi-File Changes**: The task will likely touch more than 2-3 files\n   - Example: \"Refactor the authentication system\"\n   - Example: \"Add a new API endpoint with tests\"\n\n6. **Unclear Requirements**: You need to explore before understanding the full scope\n   - Example: \"Make the app faster\" - need to profile and identify bottlenecks\n   - Example: \"Fix the bug in checkout\" - need to investigate root cause\n\n7. **User Preferences Matter**: The implementation could reasonably go multiple ways\n   - If you would use AskUserQuestion to clarify the approach, use EnterPlanMode instead\n   - Plan mode lets you explore first, then present options with context\n\n## When NOT to Use This Tool\n\nOnly skip EnterPlanMode for simple tasks:\n- Single-line or few-line fixes (typos, obvious bugs, small tweaks)\n- Adding a single function with clear requirements\n- Tasks where the user has given very specific, detailed instructions\n- Pure research/exploration tasks (use the Agent tool with explore agent instead)\n\n## What Happens in Plan Mode\n\nIn plan mode, you'll:\n1. Thoroughly explore the codebase using Glob, Grep, and Read tools\n2. Understand existing patterns and architecture\n3. Design an implementation approach\n4. Present your plan to the user for approval\n5. Use AskUserQuestion if you need to clarify approaches\n6. Exit plan mode with ExitPlanMode when ready to implement\n\n## Examples\n\n### GOOD - Use EnterPlanMode:\nUser: \"Add user authentication to the app\"\n- Requires architectural decisions (session vs JWT, where to store tokens, middleware structure)\n\nUser: \"Optimize the database queries\"\n- Multiple approaches possible, need to profile first, significant impact\n\nUser: \"Implement dark mode\"\n- Architectural decision on theme system, affects many components\n\nUser: \"Add a delete button to the user profile\"\n- Seems simple but involves: where to place it, confirmation dialog, API call, error handling, state updates\n\nUser: \"Update the error handling in the API\"\n- Affects multiple files, user should approve the approach\n\n### BAD - Don't use EnterPlanMode:\nUser: \"Fix the typo in the README\"\n- Straightforward, no planning needed\n\nUser: \"Add a console.log to debug this function\"\n- Simple, obvious implementation\n\nUser: \"What files handle routing?\"\n- Research task, not implementation planning\n\n## Important Notes\n\n- This tool REQUIRES user approval - they must consent to entering plan mode\n- If unsure whether to use it, err on the side of planning - it's better to get alignment upfront than to redo work\n- Users appreciate being consulted before significant changes are made to their codebase\n","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{},"additionalProperties":false}}"###,
        ),
        (
            "EnterWorktree",
            r###"{"name":"EnterWorktree","description":"Use this tool ONLY when explicitly instructed to work in a worktree — either by the user directly, or by project instructions (CLAUDE.md / memory). This tool creates an isolated git worktree and switches the current session into it.\n\n## When to Use\n\n- The user explicitly says \"worktree\" (e.g., \"start a worktree\", \"work in a worktree\", \"create a worktree\", \"use a worktree\")\n- CLAUDE.md or memory instructions direct you to work in a worktree for the current task\n\n## When NOT to Use\n\n- The user asks to create a branch, switch branches, or work on a different branch — use git commands instead\n- The user asks to fix a bug or work on a feature — use normal git workflow unless worktrees are explicitly requested by the user or project instructions\n- Never use this tool unless \"worktree\" is explicitly mentioned by the user or in CLAUDE.md / memory instructions\n\n## Requirements\n\n- Must be in a git repository, OR have WorktreeCreate/WorktreeRemove hooks configured in settings.json\n- Must not already be in a worktree\n\n## Behavior\n\n- In a git repository: creates a new git worktree inside `.claude/worktrees/` with a new branch based on HEAD\n- Outside a git repository: delegates to WorktreeCreate/WorktreeRemove hooks for VCS-agnostic isolation\n- Switches the session's working directory to the new worktree\n- Use ExitWorktree to leave the worktree mid-session (keep or remove). On session exit, if still in the worktree, the user will be prompted to keep or remove it\n\n## Entering an existing worktree\n\nPass `path` instead of `name` to switch the session into a worktree that already exists (e.g., one you just created with `git worktree add`). The path must appear in `git worktree list` for the current repository — paths that are not registered worktrees of this repo are rejected. ExitWorktree will not remove a worktree entered this way; use `action: \"keep\"` to return to the original directory.\n\n## Parameters\n\n- `name` (optional): A name for a new worktree. If neither `name` nor `path` is provided, a random name is generated.\n- `path` (optional): Path to an existing worktree of the current repository to enter instead of creating one. Mutually exclusive with `name`.\n","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"name":{"description":"Optional name for a new worktree. Each \"/\"-separated segment may contain only letters, digits, dots, underscores, and dashes; max 64 chars total. A random name is generated if not provided. Mutually exclusive with `path`.","type":"string"},"path":{"description":"Path to an existing worktree of the current repository to switch into instead of creating a new one. Must appear in `git worktree list` for the current repo. Mutually exclusive with `name`.","type":"string"}},"additionalProperties":false}}"###,
        ),
        (
            "ExitPlanMode",
            r###"{"name":"ExitPlanMode","description":"Use this tool when you are in plan mode and have finished writing your plan to the plan file and are ready for user approval.\n\n## How This Tool Works\n- You should have already written your plan to the plan file specified in the plan mode system message\n- This tool does NOT take the plan content as a parameter - it will read the plan from the file you wrote\n- This tool simply signals that you're done planning and ready for the user to review and approve\n- The user will see the contents of your plan file when they review it\n\n## When to Use This Tool\nIMPORTANT: Only use this tool when the task requires planning the implementation steps of a task that requires writing code. For research tasks where you're gathering information, searching files, reading files or in general trying to understand the codebase - do NOT use this tool.\n\n## Before Using This Tool\nEnsure your plan is complete and unambiguous:\n- If you have unresolved questions about requirements or approach, use AskUserQuestion first (in earlier phases)\n- Once your plan is finalized, use THIS tool to request approval\n\n**Important:** Do NOT use AskUserQuestion to ask \"Is this plan okay?\" or \"Should I proceed?\" - that's exactly what THIS tool does. ExitPlanMode inherently requests user approval of your plan.\n\n## Examples\n\n1. Initial task: \"Search for and understand the implementation of vim mode in the codebase\" - Do not use the exit plan mode tool because you are not planning the implementation steps of a task.\n2. Initial task: \"Help me implement yank mode for vim\" - Use the exit plan mode tool after you have finished planning the implementation steps of the task.\n3. Initial task: \"Add a new feature to handle user authentication\" - If unsure about auth method (OAuth, JWT, etc.), use AskUserQuestion first, then use exit plan mode tool after clarifying the approach.\n","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"allowedPrompts":{"description":"Prompt-based permissions needed to implement the plan. These describe categories of actions rather than specific commands.","type":"array","items":{"type":"object","properties":{"tool":{"description":"The tool this prompt applies to","type":"string","enum":["Bash"]},"prompt":{"description":"Semantic description of the action, e.g. \"run tests\", \"install dependencies\"","type":"string"}},"required":["tool","prompt"],"additionalProperties":false}}},"additionalProperties":{}}}"###,
        ),
        (
            "ExitWorktree",
            r###"{"name":"ExitWorktree","description":"Exit a worktree session created by EnterWorktree and return the session to the original working directory.\n\n## Scope\n\nThis tool ONLY operates on worktrees created by EnterWorktree in this session. It will NOT touch:\n- Worktrees you created manually with `git worktree add`\n- Worktrees from a previous session (even if created by EnterWorktree then)\n- The directory you're in if EnterWorktree was never called\n\nIf called outside an EnterWorktree session, the tool is a **no-op**: it reports that no worktree session is active and takes no action. Filesystem state is unchanged.\n\n## When to Use\n\n- The user explicitly asks to \"exit the worktree\", \"leave the worktree\", \"go back\", or otherwise end the worktree session\n- Do NOT call this proactively — only when the user asks\n\n## Parameters\n\n- `action` (required): `\"keep\"` or `\"remove\"`\n  - `\"keep\"` — leave the worktree directory and branch intact on disk. Use this if the user wants to come back to the work later, or if there are changes to preserve.\n  - `\"remove\"` — delete the worktree directory and its branch. Use this for a clean exit when the work is done or abandoned.\n- `discard_changes` (optional, default false): only meaningful with `action: \"remove\"`. If the worktree has uncommitted files or commits not on the original branch, the tool will REFUSE to remove it unless this is set to `true`. If the tool returns an error listing changes, confirm with the user before re-invoking with `discard_changes: true`.\n\n## Behavior\n\n- Restores the session's working directory to where it was before EnterWorktree\n- Clears CWD-dependent caches (system prompt sections, memory files, plans directory) so the session state reflects the original directory\n- If a tmux session was attached to the worktree: killed on `remove`, left running on `keep` (its name is returned so the user can reattach)\n- Once exited, EnterWorktree can be called again to create a fresh worktree\n","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"action":{"description":"\"keep\" leaves the worktree and branch on disk; \"remove\" deletes both.","type":"string","enum":["keep","remove"]},"discard_changes":{"description":"Required true when action is \"remove\" and the worktree has uncommitted files or unmerged commits. The tool will refuse and list them otherwise.","type":"boolean"}},"required":["action"],"additionalProperties":false}}"###,
        ),
        (
            "Monitor",
            r###"{"name":"Monitor","description":"Start a background monitor that streams events from a long-running script. Each stdout line is an event — you keep working and notifications arrive in the chat. Events arrive on their own schedule and are not replies from the user, even if one lands while you're waiting for the user to answer a question.\n\nPick by how many notifications you need:\n- **One** (\"tell me when the server is ready / the build finishes\") → use **Bash with `run_in_background`** and a command that exits when the condition is true, e.g. `until grep -q \"Ready in\" dev.log; do sleep 0.5; done`. You get a single completion notification when it exits.\n- **One per occurrence, indefinitely** (\"tell me every time an ERROR line appears\") → Monitor with an unbounded command (`tail -f`, `inotifywait -m`, `while true`).\n- **One per occurrence, until a known end** (\"emit each CI step result, stop when the run completes\") → Monitor with a command that emits lines and then exits.\n\nYour script's stdout is the event stream. Each line becomes a notification. Exit ends the watch.\n\n  # Each matching log line is an event\n  tail -f /var/log/app.log | grep --line-buffered \"ERROR\"\n\n  # Each file change is an event\n  inotifywait -m --format '%e %f' /watched/dir\n\n  # Poll GitHub for new PR comments and emit one line per new comment\n  last=$(date -u +%Y-%m-%dT%H:%M:%SZ)\n  while true; do\n    now=$(date -u +%Y-%m-%dT%H:%M:%SZ)\n    gh api \"repos/owner/repo/issues/123/comments?since=$last\" --jq '.[] | \"\\(.user.login): \\(.body)\"'\n    last=$now; sleep 30\n  done\n\n  # Node script that emits events as they arrive (e.g. WebSocket listener)\n  node watch-for-events.js\n\n  # Per-occurrence with a natural end: emit each CI check as it lands, exit when the run completes\n  prev=\"\"\n  while true; do\n    s=$(gh pr checks 123 --json name,bucket)\n    cur=$(jq -r '.[] | select(.bucket!=\"pending\") | \"\\(.name): \\(.bucket)\"' <<<\"$s\" | sort)\n    comm -13 <(echo \"$prev\") <(echo \"$cur\")\n    prev=$cur\n    jq -e 'all(.bucket!=\"pending\")' <<<\"$s\" >/dev/null && break\n    sleep 30\n  done\n\n**Don't use an unbounded command for a single notification.** `tail -f`, `inotifywait -m`, and `while true` never exit on their own, so the monitor stays armed until timeout even after the event has fired. For \"tell me when X is ready,\" use Bash `run_in_background` with an `until` loop instead (one notification, ends in seconds). Note that `tail -f log | grep -m 1 ...` does *not* fix this: if the log goes quiet after the match, `tail` never receives SIGPIPE and the pipeline hangs anyway.\n\n**Script quality:**\n- Always use `grep --line-buffered` in pipes — without it, pipe buffering delays events by minutes.\n- In poll loops, handle transient failures (`curl ... || true`) — one failed request shouldn't kill the monitor.\n- Poll intervals: 30s+ for remote APIs (rate limits), 0.5-1s for local checks.\n- Write a specific `description` — it appears in every notification (\"errors in deploy.log\" not \"watching logs\").\n- Only stdout is the event stream. Stderr goes to the output file (readable via Read) but does not trigger notifications — for a command you run directly (e.g. `python train.py 2>&1 | grep --line-buffered ...`), merge stderr with `2>&1` so its failures reach your filter. (No effect on `tail -f` of an existing log — that file only contains what its writer redirected.)\n\n**Coverage — silence is not success.** When watching a job or process for an outcome, your filter must match every terminal state, not just the happy path. A monitor that greps only for the success marker stays silent through a crashloop, a hung process, or an unexpected exit — and silence looks identical to \"still running.\" Before arming, ask: *if this process crashed right now, would my filter emit anything?* If not, widen it.\n\n  # Wrong — silent on crash, hang, or any non-success exit\n  tail -f run.log | grep --line-buffered \"elapsed_steps=\"\n\n  # Right — one alternation covering progress + the failure signatures you'd act on\n  tail -f run.log | grep -E --line-buffered \"elapsed_steps=|Traceback|Error|FAILED|assert|Killed|OOM\"\n\nFor poll loops checking job state, emit on every terminal status (`succeeded|failed|cancelled|timeout`), not just success. If you cannot confidently enumerate the failure signatures, broaden the grep alternation rather than narrow it — some extra noise is better than missing a crashloop.\n\n**Output volume**: Every stdout line is a conversation message, so the filter should be selective — but selective means \"the lines you'd act on,\" not \"only good news.\" Never pipe raw logs; use `grep --line-buffered`, `awk`, or a wrapper that emits exactly the success and failure signals you care about. Monitors that produce too many events are automatically stopped; restart with a tighter filter if this happens.\n\nStdout lines within 200ms are batched into a single notification, so multiline output from a single event groups naturally.\n\nThe script runs in the same shell environment as Bash. Exit ends the watch (exit code is reported). Timeout → killed. Set `persistent: true` for session-length watches (PR monitoring, log tails) — the monitor runs until you call TaskStop or the session ends. Use TaskStop to cancel early.","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"description":{"description":"Short human-readable description of what you are monitoring (shown in notifications).","type":"string"},"timeout_ms":{"description":"Kill the monitor after this deadline. Default 300000ms, max 3600000ms. Ignored when persistent is true.","default":300000,"type":"number","minimum":1000},"persistent":{"description":"Run for the lifetime of the session (no timeout). Use for session-length watches like PR monitoring or log tails. Stop with TaskStop.","default":false,"type":"boolean"},"command":{"description":"Shell command or script. Each stdout line is an event; exit ends the watch.","type":"string"}},"required":["description","timeout_ms","persistent","command"],"additionalProperties":false}}"###,
        ),
        (
            "NotebookEdit",
            r###"{"name":"NotebookEdit","description":"Completely replaces the contents of a specific cell in a Jupyter notebook (.ipynb file) with new source. Jupyter notebooks are interactive documents that combine code, text, and visualizations, commonly used for data analysis and scientific computing. The notebook_path parameter must be an absolute path, not a relative path. The cell_number is 0-indexed. Use edit_mode=insert to add a new cell at the index specified by cell_number. Use edit_mode=delete to delete the cell at the index specified by cell_number.","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"notebook_path":{"description":"The absolute path to the Jupyter notebook file to edit (must be absolute, not relative)","type":"string"},"cell_id":{"description":"The ID of the cell to edit. When inserting a new cell, the new cell will be inserted after the cell with this ID, or at the beginning if not specified.","type":"string"},"new_source":{"description":"The new source for the cell","type":"string"},"cell_type":{"description":"The type of the cell (code or markdown). If not specified, it defaults to the current cell type. If using edit_mode=insert, this is required.","type":"string","enum":["code","markdown"]},"edit_mode":{"description":"The type of edit to make (replace, insert, delete). Defaults to replace.","type":"string","enum":["replace","insert","delete"]}},"required":["notebook_path","new_source"],"additionalProperties":false}}"###,
        ),
        (
            "PushNotification",
            r###"{"name":"PushNotification","description":"This tool sends a desktop notification in the user's terminal. If Remote Control is connected, it also pushes to their phone. Either way, it pulls their attention from whatever they're doing — a meeting, another task, dinner — to this session. That's the cost. The benefit is they learn something now that they'd want to know now: a long task finished while they were away, a build is ready, you've hit something that needs their decision before you can continue.\n\nBecause a notification they didn't need is annoying in a way that accumulates, err toward not sending one. Don't notify for routine progress, or to announce you've answered something they asked seconds ago and are clearly still watching, or when a quick task completes. Notify when there's a real chance they've walked away and there's something worth coming back for — or when they've explicitly asked you to notify them.\n\nKeep the message under 200 characters, one line, no markdown. Lead with what they'd act on — \"build failed: 2 auth tests\" tells them more than \"task done\" and more than a status dump.\n\nIf the result says the push wasn't sent, that's expected — no action needed.","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"message":{"description":"The notification body. Keep it under 200 characters; mobile OSes truncate.","type":"string","minLength":1},"status":{"type":"string","const":"proactive"}},"required":["message","status"],"additionalProperties":false}}"###,
        ),
        (
            "RemoteTrigger",
            r###"{"name":"RemoteTrigger","description":"Call the claude.ai remote-trigger API. Use this instead of curl — the OAuth token is added automatically in-process and never exposed.\n\nActions:\n- list: GET /v1/code/triggers\n- get: GET /v1/code/triggers/{trigger_id}\n- create: POST /v1/code/triggers (requires body)\n- update: POST /v1/code/triggers/{trigger_id} (requires body, partial update)\n- run: POST /v1/code/triggers/{trigger_id}/run (optional body)\n\nThe response is the raw JSON from the API.","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"action":{"type":"string","enum":["list","get","create","update","run"]},"trigger_id":{"description":"Required for get, update, and run","type":"string","pattern":"^[\\w-]+$"},"body":{"description":"Required for create and update; optional for run","type":"object","propertyNames":{"type":"string"},"additionalProperties":{}}},"required":["action"],"additionalProperties":false}}"###,
        ),
        (
            "ScheduleWakeup",
            r###"{"name":"ScheduleWakeup","description":"Schedule when to resume work in /loop dynamic mode — the user invoked /loop without an interval, asking you to self-pace iterations of a specific task.\n\nPass the same /loop prompt back via `prompt` each turn so the next firing repeats the task. For an autonomous /loop (no user prompt), pass the literal sentinel `<<autonomous-loop-dynamic>>` as `prompt` instead — the runtime resolves it back to the autonomous-loop instructions at fire time. (There is a similar `<<autonomous-loop>>` sentinel for CronCreate-based autonomous loops; do not confuse the two — ScheduleWakeup always uses the `-dynamic` variant.) Omit the call to end the loop.\n\n## Picking delaySeconds\n\nThe Anthropic prompt cache has a 5-minute TTL. Sleeping past 300 seconds means the next wake-up reads your full conversation context uncached — slower and more expensive. So the natural breakpoints:\n\n- **Under 5 minutes (60s–270s)**: cache stays warm. Right for active work — checking a build, polling for state that's about to change, watching a process you just started.\n- **5 minutes to 1 hour (300s–3600s)**: pay the cache miss. Right when there's no point checking sooner — waiting on something that takes minutes to change, or genuinely idle.\n\n**Don't pick 300s.** It's the worst-of-both: you pay the cache miss without amortizing it. If you're tempted to \"wait 5 minutes,\" either drop to 270s (stay in cache) or commit to 1200s+ (one cache miss buys a much longer wait). Don't think in round-number minutes — think in cache windows.\n\nFor idle ticks with no specific signal to watch, default to **1200s–1800s** (20–30 min). The loop checks back, you don't burn cache 12× per hour for nothing, and the user can always interrupt if they need you sooner.\n\nThink about what you're actually waiting for, not just \"how long should I sleep.\" If you kicked off an 8-minute build, sleeping 60s burns the cache 8 times before it finishes — sleep ~270s twice instead.\n\nThe runtime clamps to [60, 3600], so you don't need to clamp yourself.\n\n## The reason field\n\nOne short sentence on what you chose and why. Goes to telemetry and is shown back to the user. \"checking long bun build\" beats \"waiting.\" The user reads this to understand what you're doing without having to predict your cadence in advance — make it specific.\n","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"delaySeconds":{"description":"Seconds from now to wake up. Clamped to [60, 3600] by the runtime.","type":"number"},"reason":{"description":"One short sentence explaining the chosen delay. Goes to telemetry and is shown to the user. Be specific.","type":"string"},"prompt":{"description":"The /loop input to fire on wake-up. Pass the same /loop input verbatim each turn so the next firing re-enters the skill and continues the loop. For autonomous /loop (no user prompt), pass the literal sentinel `<<autonomous-loop-dynamic>>` instead (the dynamic-pacing variant, not the CronCreate-mode `<<autonomous-loop>>`).","type":"string"}},"required":["delaySeconds","reason","prompt"],"additionalProperties":false}}"###,
        ),
        (
            "Skill",
            r###"{"name":"Skill","description":"Execute a skill within the main conversation\n\nWhen users ask you to perform tasks, check if any of the available skills match. Skills provide specialized capabilities and domain knowledge.\n\nWhen users reference a \"slash command\" or \"/<something>\", they are referring to a skill. Use this tool to invoke it.\n\nHow to invoke:\n- Set `skill` to the exact name of an available skill (no leading slash). For plugin-namespaced skills use the fully qualified `plugin:skill` form.\n- Set `args` to pass optional arguments.\n\nImportant:\n- Available skills are listed in system-reminder messages in the conversation\n- Only invoke a skill that appears in that list, or one the user explicitly typed as `/<name>` in their message. Never guess or invent a skill name from training data; otherwise do not call this tool\n- When a skill matches the user's request, this is a BLOCKING REQUIREMENT: invoke the relevant Skill tool BEFORE generating any other response about the task\n- NEVER mention a skill without actually calling this tool\n- Do not invoke a skill that is already running\n- Do not use this tool for built-in CLI commands (like /help, /clear, etc.)\n- If you see a <command-name> tag in the current conversation turn, the skill has ALREADY been loaded - follow the instructions directly instead of calling this tool again\n","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"skill":{"description":"The name of a skill from the available-skills list. Do not guess names.","type":"string"},"args":{"description":"Optional arguments for the skill","type":"string"}},"required":["skill"],"additionalProperties":false}}"###,
        ),
        (
            "TaskOutput",
            r###"{"name":"TaskOutput","description":"DEPRECATED: Background tasks return their output file path in the tool result, and you receive a <task-notification> with the same path when the task completes.\n- For bash tasks: prefer using the Read tool on that output file path — it contains stdout/stderr.\n- For local_agent tasks: use the Agent tool result directly. Do NOT Read the .output file — it is a symlink to the full sub-agent conversation transcript (JSONL) and will overflow your context window.\n- For remote_agent tasks: prefer using the Read tool on the output file path — it contains the streamed remote session output (same as bash).\n\n- Retrieves output from a running or completed task (background shell, agent, or remote session)\n- Takes a task_id parameter identifying the task\n- Returns the task output along with status information\n- Use block=true (default) to wait for task completion\n- Use block=false for non-blocking check of current status\n- Task IDs can be found using the /tasks command\n- Works with all task types: background shells, async agents, and remote sessions","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"task_id":{"description":"The task ID to get output from","type":"string"},"block":{"description":"Whether to wait for completion","default":true,"type":"boolean"},"timeout":{"description":"Max wait time in ms","default":30000,"type":"number","minimum":0,"maximum":600000}},"required":["task_id","block","timeout"],"additionalProperties":false}}"###,
        ),
        (
            "TaskStop",
            r###"{"name":"TaskStop","description":"\n- Stops a running background task by its ID\n- Takes a task_id parameter identifying the task to stop\n- Returns a success or failure status\n- Use this tool when you need to terminate a long-running task\n","input_schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"task_id":{"description":"The ID of the background task to stop","type":"string"},"shell_id":{"description":"Deprecated: use task_id instead","type":"string"}},"additionalProperties":false}}"###,
        ),
    ]
    .into_iter()
    .filter(|(name, _)| {
        !is_child_agent_request
            || !matches!(*name, "EnterPlanMode" | "ExitPlanMode" | "TaskOutput")
    })
    .filter(|(name, _)| !existing_names.contains(name))
    .map(|(_, tool_json)| {
        let tool = serde_json::from_str::<Value>(tool_json)?;
        Ok(AnthropicTool {
            name: tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            description: tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input_schema: tool.get("input_schema").cloned().unwrap_or(Value::Null),
        })
    })
    .collect()
}

fn add_json_schema_draft(input_schema: Value) -> Value {
    match input_schema {
        Value::Object(mut map) => {
            map.entry("$schema".to_string()).or_insert_with(|| {
                Value::String("https://json-schema.org/draft/2020-12/schema".to_string())
            });
            Value::Object(map)
        }
        other => other,
    }
}

fn build_request_metadata(session_id: &str) -> AnthropicRequestMetadata {
    let device_id = std::env::var("OPEN_INTERPRETER_CLAUDE_CODE_DEVICE_ID_OVERRIDE")
        .unwrap_or_else(|_| CLAUDE_CODE_METADATA_DEVICE_ID.to_string());
    AnthropicRequestMetadata {
        user_id: serde_json::json!({
            "device_id": device_id,
            "account_uuid": "",
            "session_id": session_id,
        })
        .to_string(),
    }
}

fn build_billing_header(
    request_kind: &str,
    model: &str,
    billing_version_source: &str,
    messages: &[AnthropicMessage],
    tools: &[AnthropicTool],
) -> String {
    let billing_version = build_billing_header_version(billing_version_source);
    let mut hasher = DefaultHasher::new();
    request_kind.hash(&mut hasher);
    model.hash(&mut hasher);
    serde_json::to_string(messages)
        .unwrap_or_default()
        .hash(&mut hasher);
    serde_json::to_string(tools)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!(
        "{CLAUDE_CODE_BILLING_HEADER_PREFIX}{billing_version}; cc_entrypoint={CLAUDE_CODE_BILLING_ENTRYPOINT}; cch={:05x};",
        hasher.finish() & 0xFFFFF
    )
}

fn build_billing_header_version(first_user_text: &str) -> String {
    let suffix_input = [4usize, 7, 20]
        .into_iter()
        .map(|index| first_user_text.chars().nth(index).unwrap_or('0'))
        .collect::<String>();
    let digest = Sha256::digest(
        format!("{CLAUDE_CODE_BILLING_VERSION_SALT}{suffix_input}{CLAUDE_CODE_VERSION}").as_bytes(),
    );
    let digest_hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{CLAUDE_CODE_VERSION}.{}", &digest_hex[..3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextualUserFragment;
    use crate::context::UserShellCommand;
    use codex_protocol::models::BaseInstructions;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::ResponseItem;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    const CLAUDE_TODO_WRITE_SUCCESS_MESSAGE: &str = "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable";
    const USER_SHELL_COMMAND_OPEN_TAG: &str =
        <UserShellCommand as ContextualUserFragment>::START_MARKER;
    const USER_SHELL_COMMAND_CLOSE_TAG: &str =
        <UserShellCommand as ContextualUserFragment>::END_MARKER;

    fn test_model_info(slug: &str) -> ModelInfo {
        serde_json::from_value(serde_json::json!({
            "slug": slug,
            "display_name": slug,
            "description": "desc",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                {"effort": "medium", "description": "medium"}
            ],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "upgrade": null,
            "base_instructions": "ignored",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10000},
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": 200000,
            "auto_compact_token_limit": null,
            "experimental_supported_tools": []
        }))
        .expect("deserialize model info")
    }

    fn thinking_only_model_info(slug: &str) -> ModelInfo {
        serde_json::from_value(serde_json::json!({
            "slug": slug,
            "display_name": slug,
            "description": "desc",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "upgrade": null,
            "base_instructions": "ignored",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10000},
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": 200000,
            "auto_compact_token_limit": null,
            "experimental_supported_tools": []
        }))
        .expect("deserialize toggle model info")
    }

    #[test]
    fn builds_claude_code_request_with_tool_history() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Update files".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Bash".to_string(),
                    namespace: None,
                    arguments: "{\"command\":\"printf 'WRITE_OK\\\\n' > /tmp/output.txt\"}"
                        .to_string(),
                    call_id: "toolu_1".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "toolu_1".to_string(),
                    output: FunctionCallOutputPayload::from_text(
                        "(Bash completed with no output)".to_string(),
                    ),
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Read".to_string(),
                    namespace: None,
                    arguments: "{\"file_path\":\"/tmp/input.txt\"}".to_string(),
                    call_id: "toolu_2".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "toolu_2".to_string(),
                    output: FunctionCallOutputPayload::from_text("1\tREAD_OK".to_string()),
                },
            ],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "base instructions".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");
        assert_eq!(request.model, "claude-sonnet-4-6");
        assert!(
            request.system[0]
                .text
                .starts_with(CLAUDE_CODE_BILLING_HEADER_PREFIX)
        );
        assert_eq!(
            request.system[1],
            AnthropicTextBlock::ephemeral(CLAUDE_CODE_SYSTEM_PROMPT_HEADER.to_string())
        );
        assert_eq!(
            request.system[2].cache_control,
            Some(AnthropicCacheControl::ephemeral())
        );
        assert_eq!(
            request.system[2].text.contains(
                "You are an interactive agent that helps users with software engineering tasks."
            ),
            true
        );
        assert_eq!(
            request
                .system[2]
                .text
                .contains("Users may configure 'hooks', shell commands that execute in response to events like tool calls, in settings."),
            true
        );
        assert_eq!(
            request.system[2]
                .text
                .contains("# Executing actions with care"),
            true
        );
        assert_eq!(request.system[2].text.contains("# auto memory"), true);
        assert_eq!(
            request.system[2]
                .text
                .contains("Primary working directory: /tmp/workspace"),
            true
        );
        assert_eq!(
            request.system[2]
                .text
                .contains("The exact model ID is claude-sonnet-4-6."),
            true
        );
        assert_eq!(
            request.system[2].text.contains(
                "When the user types `/<skill-name>`, invoke it via Skill. Only use skills listed in the user-invocable skills section — don't guess."
            ),
            true
        );
        assert_eq!(
            request.metadata,
            Some(build_request_metadata("session-123"))
        );
        assert_eq!(request.thinking, Some(AnthropicThinkingConfig::adaptive()));
        assert_eq!(
            request.output_config,
            Some(AnthropicOutputConfig {
                effort: Some("high".to_string()),
                format: None,
            })
        );
        assert_eq!(request.max_tokens, 32_000);
        assert_eq!(request.messages.len(), 3);
        assert_eq!(request.messages[1].role, "assistant");
        assert_eq!(
            request.messages[1].content,
            AnthropicMessageContent::Blocks(vec![
                AnthropicContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({"command": "printf 'WRITE_OK\\n' > /tmp/output.txt"}),
                },
                AnthropicContentBlock::ToolUse {
                    id: "toolu_2".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/tmp/input.txt"}),
                },
            ])
        );
        assert_eq!(
            request.messages[2].content,
            AnthropicMessageContent::Blocks(vec![
                AnthropicContentBlock::ToolResult {
                    tool_use_id: "toolu_1".to_string(),
                    content: "(Bash completed with no output)".to_string().into(),
                    is_error: Some(false),
                    cache_control: None,
                },
                AnthropicContentBlock::ToolResult {
                    tool_use_id: "toolu_2".to_string(),
                    content: "1\tREAD_OK".to_string().into(),
                    is_error: None,
                    cache_control: Some(AnthropicCacheControl::ephemeral()),
                },
            ])
        );
    }

    #[test]
    fn developer_skills_section_becomes_first_user_skills_reminder() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "<skills_instructions>\n## Skills\nA skill is a set of local instructions to follow that is stored in a `SKILL.md` file.\n### Available skills\n- alpha: first skill (file: /tmp/alpha/SKILL.md)\n- beta: second skill (file: /tmp/beta/SKILL.md)\n### How to use skills\n- Discovery: ...\n</skills_instructions>"
                            .to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Audit the environment".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
            ],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(
            request.messages[0].content,
            AnthropicMessageContent::Blocks(vec![
                AnthropicContentBlock::Text {
                    text: format!(
                        "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n{}\n</system-reminder>\n",
                        CLAUDE_REFERENCE_SKILLS_REMINDER
                    ),
                    cache_control: None,
                },
                AnthropicContentBlock::Text {
                    text: format!(
                        "<system-reminder>\nAs you answer the user's questions, you can use the following context:\n# currentDate\nToday's date is {}.\n\n      IMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n\n",
                        current_local_date()
                    ),
                    cache_control: None,
                },
                AnthropicContentBlock::Text {
                    text: "Audit the environment".to_string(),
                    cache_control: Some(AnthropicCacheControl::ephemeral()),
                },
            ])
        );
    }

    #[test]
    fn build_request_marks_git_repositories_and_model_display_name_in_environment() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp_dir.path().join(".git")).expect("create git dir");
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hi".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(temp_dir.path().to_path_buf()),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-opus-4-7"),
            None,
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(
            request.system[2].text.contains("Is a git repository: true"),
            true
        );
        assert_eq!(
            request
                .system[2]
                .text
                .contains("You are powered by the model named Opus 4.7. The exact model ID is claude-opus-4-7."),
            true
        );
    }

    #[test]
    fn spawned_subagent_request_uses_claude_child_agent_profile() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Create child-proof.txt and reply with CHILD_DONE.".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            Some(ReasoningEffortConfig::Medium),
            "session-123",
            Some(&SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: codex_protocol::ThreadId::new(),
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            })),
        )
        .expect("build request");

        assert_eq!(request.thinking, None);
        assert_eq!(request.context_management, None);
        assert_eq!(request.temperature, Some(1));
        assert_eq!(
            request.output_config,
            Some(AnthropicOutputConfig {
                effort: Some("high".to_string()),
                format: None,
            })
        );
        assert_eq!(
            request.system[2].text,
            "You are an agent for Claude Code, Anthropic's official CLI for Claude. Given the user's message, you should use the tools available to complete the task. Complete the task fully—don't gold-plate, but don't leave it half-done. When you complete the task, respond with a concise report covering what was done and any key findings — the caller will relay this to the user, so it only needs the essentials.\n\nYour strengths:\n- Searching for code, configurations, and patterns across large codebases\n- Analyzing multiple files to understand system architecture\n- Investigating complex questions that require exploring many files\n- Performing multi-step research tasks\n\nGuidelines:\n- For file searches: search broadly when you don't know where something lives. Use Read when you know the specific file path.\n- For analysis: Start broad and narrow down. Use multiple search strategies if the first doesn't yield results.\n- Be thorough: Check multiple locations, consider different naming conventions, look for related files.\n- NEVER create files unless they're absolutely necessary for achieving your goal. ALWAYS prefer editing an existing file to creating a new one.\n- NEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested.\n\nNotes:\n- Agent threads always have their cwd reset between bash calls, as a result please only use absolute file paths.\n- In your final response, share file paths (always absolute, never relative) that are relevant to the task. Include code snippets only when the exact text is load-bearing (e.g., a bug you found, a function signature the caller asked for) — do not recap code you merely read.\n- For clear communication with the user the assistant MUST avoid using emojis.\n- Do not use a colon before tool calls. Text like \"Let me read the file:\" followed by a read tool call should just be \"Let me read the file.\" with a period.\n\nHere is useful information about the environment you are running in:\n<env>\nWorking directory: /tmp/workspace\nIs directory a git repo: No\nPlatform: darwin\nShell: zsh\nOS Version: Darwin 25.2.0\n</env>\nYou are powered by the model named Sonnet 4.6. The exact model ID is claude-sonnet-4-6.\n\nAssistant knowledge cutoff is August 2025."
        );
    }

    #[test]
    fn trims_bash_trailing_newline_from_tool_result() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Make a todo list".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Bash".to_string(),
                    namespace: None,
                    arguments: "{\"command\":\"echo \\\"TodoWrite done\\\"\"}".to_string(),
                    call_id: "toolu_bash".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "toolu_bash".to_string(),
                    output: FunctionCallOutputPayload::from_text("TodoWrite done\n".to_string()),
                },
            ],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "base instructions".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(
            request.messages[2].content,
            AnthropicMessageContent::Blocks(vec![AnthropicContentBlock::ToolResult {
                tool_use_id: "toolu_bash".to_string(),
                content: "TodoWrite done".to_string().into(),
                is_error: Some(false),
                cache_control: Some(AnthropicCacheControl::ephemeral()),
            }])
        );
    }

    #[test]
    fn normalizes_plain_user_followups_but_keeps_assistant_text_as_blocks() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Use the Write tool once".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "TURN1_DONE".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Now use Bash once".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
            ],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(
            request.messages[1].content,
            AnthropicMessageContent::Blocks(vec![AnthropicContentBlock::Text {
                text: "TURN1_DONE".to_string(),
                cache_control: None,
            }])
        );
        assert_eq!(
            request.messages[2].content,
            AnthropicMessageContent::Text("Now use Bash once".to_string())
        );
    }

    #[test]
    fn appends_todo_reminder_after_ten_stale_steps_without_progress_text() {
        let todos = serde_json::json!({
            "todos": [
                {"content": "Generate dossier", "activeForm": "Generating dossier", "status": "in_progress"},
                {"content": "Review dossier chunks", "activeForm": "Reviewing dossier chunks", "status": "pending"},
                {"content": "Write report", "activeForm": "Writing report", "status": "pending"},
                {"content": "Run child verification", "activeForm": "Running child verification", "status": "pending"},
                {"content": "Finalize", "activeForm": "Finalizing", "status": "pending"}
            ]
        })
        .to_string();
        let mut input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Process the dossier".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "TodoWrite".to_string(),
                namespace: None,
                arguments: todos,
                call_id: "todo_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "todo_1".to_string(),
                output: FunctionCallOutputPayload::from_text(
                    CLAUDE_TODO_WRITE_SUCCESS_MESSAGE.to_string(),
                ),
            },
        ];
        for index in 1..=CLAUDE_CODE_TODO_REMINDER_STALENESS_THRESHOLD {
            input.push(ResponseItem::FunctionCall {
                id: None,
                name: "Read".to_string(),
                namespace: None,
                arguments: serde_json::json!({
                    "file_path": format!("/tmp/dossier-{index}.txt"),
                    "offset": 1,
                    "limit": 180
                })
                .to_string(),
                call_id: format!("read_{index}"),
            });
            input.push(ResponseItem::FunctionCallOutput {
                call_id: format!("read_{index}"),
                output: FunctionCallOutputPayload::from_text(format!(
                    "{index}\tCHECKPOINT_{index:02}"
                )),
            });
        }
        let prompt = Prompt {
            input,
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");
        let serialized = serde_json::to_string(&request).expect("serialize request");

        assert!(
            serialized.contains("The TodoWrite tool hasn't been used recently."),
            "expected stale todo reminder in request: {serialized}"
        );
        assert!(
            serialized.contains("[1. [in_progress] Generate dossier\\n2. [pending] Review dossier chunks\\n3. [pending] Write report\\n4. [pending] Run child verification\\n5. [pending] Finalize]"),
            "expected todo snapshot in reminder: {serialized}"
        );
    }

    #[test]
    fn does_not_append_todo_reminder_before_ten_stale_steps_without_progress_text() {
        let todos = serde_json::json!({
            "todos": [
                {"content": "Generate dossier", "activeForm": "Generating dossier", "status": "in_progress"}
            ]
        })
        .to_string();
        let mut input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Process the dossier".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "TodoWrite".to_string(),
                namespace: None,
                arguments: todos,
                call_id: "todo_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "todo_1".to_string(),
                output: FunctionCallOutputPayload::from_text(
                    CLAUDE_TODO_WRITE_SUCCESS_MESSAGE.to_string(),
                ),
            },
        ];
        for index in 1..CLAUDE_CODE_TODO_REMINDER_STALENESS_THRESHOLD {
            input.push(ResponseItem::FunctionCall {
                id: None,
                name: "Read".to_string(),
                namespace: None,
                arguments: serde_json::json!({
                    "file_path": format!("/tmp/dossier-{index}.txt"),
                })
                .to_string(),
                call_id: format!("read_{index}"),
            });
            input.push(ResponseItem::FunctionCallOutput {
                call_id: format!("read_{index}"),
                output: FunctionCallOutputPayload::from_text(format!(
                    "{index}\tCHECKPOINT_{index:02}"
                )),
            });
        }
        let prompt = Prompt {
            input,
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");
        let serialized = serde_json::to_string(&request).expect("serialize request");

        assert!(
            !serialized.contains("The TodoWrite tool hasn't been used recently."),
            "unexpected stale todo reminder in request: {serialized}"
        );
    }

    #[test]
    fn progress_text_turns_advance_todo_reminder_staleness() {
        let todos = serde_json::json!({
            "todos": [
                {"content": "Generate dossier", "activeForm": "Generating dossier", "status": "in_progress"}
            ]
        })
        .to_string();
        let mut input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Process the dossier".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "TodoWrite".to_string(),
                namespace: None,
                arguments: todos,
                call_id: "todo_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "todo_1".to_string(),
                output: FunctionCallOutputPayload::from_text(
                    CLAUDE_TODO_WRITE_SUCCESS_MESSAGE.to_string(),
                ),
            },
        ];
        for index in 1..=7 {
            if index != 1 {
                input.push(ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: format!("Found CHECKPOINT_{:02}. Continuing.", index - 1),
                    }],
                    end_turn: None,
                    phase: None,
                });
            }
            input.push(ResponseItem::FunctionCall {
                id: None,
                name: "Read".to_string(),
                namespace: None,
                arguments: serde_json::json!({
                    "file_path": format!("/tmp/dossier-{index}.txt"),
                })
                .to_string(),
                call_id: format!("read_{index}"),
            });
            input.push(ResponseItem::FunctionCallOutput {
                call_id: format!("read_{index}"),
                output: FunctionCallOutputPayload::from_text(format!(
                    "{index}\tCHECKPOINT_{index:02}"
                )),
            });
        }
        let prompt = Prompt {
            input,
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");
        let serialized = serde_json::to_string(&request).expect("serialize request");

        assert!(
            serialized.contains("The TodoWrite tool hasn't been used recently."),
            "expected stale todo reminder in request: {serialized}"
        );
    }

    #[test]
    fn does_not_append_todo_reminder_before_threshold() {
        let todos = serde_json::json!({
            "todos": [
                {"content": "Generate dossier", "activeForm": "Generating dossier", "status": "in_progress"}
            ]
        })
        .to_string();
        let mut input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Process the dossier".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "TodoWrite".to_string(),
                namespace: None,
                arguments: todos,
                call_id: "todo_1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "todo_1".to_string(),
                output: FunctionCallOutputPayload::from_text(
                    CLAUDE_TODO_WRITE_SUCCESS_MESSAGE.to_string(),
                ),
            },
        ];
        for index in 1..CLAUDE_CODE_TODO_REMINDER_TOOL_GAP {
            input.push(ResponseItem::FunctionCall {
                id: None,
                name: "Read".to_string(),
                namespace: None,
                arguments: serde_json::json!({
                    "file_path": format!("/tmp/dossier-{index}.txt"),
                })
                .to_string(),
                call_id: format!("read_{index}"),
            });
            input.push(ResponseItem::FunctionCallOutput {
                call_id: format!("read_{index}"),
                output: FunctionCallOutputPayload::from_text(format!(
                    "{index}\tCHECKPOINT_{index:02}"
                )),
            });
        }
        let prompt = Prompt {
            input,
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");
        let serialized = serde_json::to_string(&request).expect("serialize request");

        assert!(
            !serialized.contains("The TodoWrite tool hasn't been used recently."),
            "unexpected stale todo reminder in request: {serialized}"
        );
    }

    #[test]
    fn skips_contextual_and_developer_messages() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "<permissions instructions>context</permissions instructions>"
                            .to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: format!(
                            "{USER_SHELL_COMMAND_OPEN_TAG}pwd{USER_SHELL_COMMAND_CLOSE_TAG}"
                        ),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Write output.txt".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
            ],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");
        let date = current_local_date();

        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "user");
        assert_eq!(
            request.messages[0].content,
            AnthropicMessageContent::Blocks(vec![
                AnthropicContentBlock::Text {
                    text: format!(
                        "<system-reminder>\nAs you answer the user's questions, you can use the following context:\n# currentDate\nToday's date is {date}.\n\n      IMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n\n"
                    ),
                    cache_control: None,
                },
                AnthropicContentBlock::Text {
                    text: "Write output.txt".to_string(),
                    cache_control: Some(AnthropicCacheControl::ephemeral()),
                },
            ])
        );
    }

    #[test]
    fn build_tools_matches_reference_claude_core_surface() {
        let parameters: codex_tools::JsonSchema = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }))
        .expect("tool schema");
        let tools = vec![
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "Agent".to_string(),
                description: "agent".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "LSP".to_string(),
                description: "lsp".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "TodoWrite".to_string(),
                description: "todo".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "Write".to_string(),
                description: "write".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "WebFetch".to_string(),
                description: "web fetch".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "WebSearch".to_string(),
                description: "web search".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "Read".to_string(),
                description: "read".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "AskUserQuestion".to_string(),
                description: "ask".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "Glob".to_string(),
                description: "glob".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "Grep".to_string(),
                description: "grep".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "Edit".to_string(),
                description: "edit".to_string(),
                strict: false,
                defer_loading: None,
                parameters: parameters.clone(),
                output_schema: None,
            }),
            ToolSpec::Function(codex_tools::ResponsesApiTool {
                name: "Bash".to_string(),
                description: "bash".to_string(),
                strict: false,
                defer_loading: None,
                parameters,
                output_schema: None,
            }),
            ToolSpec::ToolSearch {
                execution: "deferred".to_string(),
                description: "search".to_string(),
                parameters: serde_json::from_value(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string"
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }))
                .expect("tool search schema"),
            },
        ];

        let tools = build_tools(&tools, false).expect("build tools");

        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Agent",
                "AskUserQuestion",
                "Bash",
                "CronCreate",
                "CronDelete",
                "CronList",
                "Edit",
                "EnterPlanMode",
                "EnterWorktree",
                "ExitPlanMode",
                "ExitWorktree",
                "Glob",
                "Grep",
                "LSP",
                "NotebookEdit",
                "Read",
                "ScheduleWakeup",
                "Skill",
                "TaskOutput",
                "TaskStop",
                "TodoWrite",
                "ToolSearch",
                "WebFetch",
                "WebSearch",
                "Write",
            ]
        );
        assert_eq!(tools[0].description, "agent");
        assert_eq!(tools[1].description, "ask");
        assert_eq!(tools[2].description, "bash");
        assert_eq!(tools[6].description, "edit");
        assert_eq!(tools[11].description, "glob");
        assert_eq!(tools[12].description, "grep");
        assert_eq!(tools[13].description, "lsp");
        assert_eq!(tools[15].description, "read");
        assert_eq!(tools[20].description, "todo");
        assert_eq!(tools[21].description, "search");
        assert_eq!(tools[22].description, "web fetch");
        assert_eq!(tools[23].description, "web search");
        assert_eq!(tools[24].description, "write");
        assert_eq!(
            tools[0].input_schema["$schema"],
            serde_json::json!("https://json-schema.org/draft/2020-12/schema")
        );
        assert_eq!(
            tools[6].input_schema["$schema"],
            serde_json::json!("https://json-schema.org/draft/2020-12/schema")
        );
    }

    #[test]
    fn multi_turn_request_only_marks_latest_message_cache_breakpoint() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Create output.txt".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "Bash".to_string(),
                    namespace: None,
                    arguments: "{\"command\":\"printf 'DOT_OK\\\\n' > /tmp/output.txt\"}"
                        .to_string(),
                    call_id: "toolu_1".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "toolu_1".to_string(),
                    output: FunctionCallOutputPayload::from_text(
                        "(Bash completed with no output)".to_string(),
                    ),
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "DONE".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Edit output.txt".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
            ],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-sonnet-4-6"),
            None,
            "session-123",
            None,
        )
        .expect("build request");

        let message_cache_breakpoints = request
            .messages
            .iter()
            .filter_map(|message| message.content.blocks())
            .flat_map(|blocks| blocks.iter())
            .filter(|block| {
                matches!(
                    block,
                    AnthropicContentBlock::Text {
                        cache_control: Some(_),
                        ..
                    } | AnthropicContentBlock::ToolResult {
                        cache_control: Some(_),
                        ..
                    }
                )
            })
            .count();

        assert_eq!(message_cache_breakpoints, 1);
        assert_eq!(
            request.messages.last().expect("last message").content,
            AnthropicMessageContent::Blocks(vec![AnthropicContentBlock::Text {
                text: "Edit output.txt".to_string(),
                cache_control: Some(AnthropicCacheControl::ephemeral()),
            }])
        );
    }

    #[test]
    fn does_not_build_title_request_for_first_turn() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Use the Write tool to create output.txt".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_title_request(&prompt, "session-123").expect("build title request");

        assert_eq!(request, None);
    }

    #[test]
    fn does_not_build_title_request_when_first_turn_contains_internal_non_user_items() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "<permissions instructions>context</permissions instructions>"
                            .to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Other,
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: format!(
                            "{USER_SHELL_COMMAND_OPEN_TAG}pwd{USER_SHELL_COMMAND_CLOSE_TAG}"
                        ),
                    }],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Use the Read tool exactly once".to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
            ],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_title_request(&prompt, "session-123").expect("build title request");

        assert_eq!(request, None);
    }

    #[test]
    fn billing_header_version_matches_claude_code_suffix() {
        assert_eq!(
            build_billing_header_version("Use the Write tool exactly once"),
            "2.1.116.c99"
        );
    }

    #[test]
    fn billing_header_version_matches_claude_code_child_suffix() {
        assert_eq!(
            build_billing_header_version(
                "Create /tmp/child-proof.txt with exactly CHILD_OK followed by a newline, then reply with exactly the UTF-8 string whose hex bytes are 4348494c445f444f4e45 and nothing else."
            ),
            "2.1.116.e8e"
        );
    }

    #[test]
    fn opus_47_defaults_to_adaptive_xhigh_with_opus_token_budget() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hi".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-opus-4-7"),
            None,
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(request.thinking, Some(AnthropicThinkingConfig::adaptive()));
        assert_eq!(
            request.output_config,
            Some(AnthropicOutputConfig {
                effort: Some("xhigh".to_string()),
                format: None,
            })
        );
        assert_eq!(request.max_tokens, 64_000);
    }

    #[test]
    fn opus_47_respects_explicit_medium_effort() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hi".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("claude-opus-4-7"),
            Some(ReasoningEffortConfig::Medium),
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(request.thinking, Some(AnthropicThinkingConfig::adaptive()));
        assert_eq!(
            request.output_config,
            Some(AnthropicOutputConfig {
                effort: Some("medium".to_string()),
                format: None,
            })
        );
        assert_eq!(request.max_tokens, 64_000);
    }

    #[test]
    fn provider_prefixed_dotted_opus_47_uses_adaptive_thinking() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hi".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("anthropic/claude-opus-4.7"),
            Some(ReasoningEffortConfig::XHigh),
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(request.thinking, Some(AnthropicThinkingConfig::adaptive()));
        assert_eq!(
            request.output_config,
            Some(AnthropicOutputConfig {
                effort: Some("xhigh".to_string()),
                format: None,
            })
        );
        assert_eq!(request.max_tokens, 64_000);
    }

    #[test]
    fn provider_prefixed_dotted_sonnet_46_uses_adaptive_thinking() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hi".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &test_model_info("anthropic/claude-sonnet-4.6"),
            Some(ReasoningEffortConfig::Medium),
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(request.thinking, Some(AnthropicThinkingConfig::adaptive()));
        assert_eq!(
            request.output_config,
            Some(AnthropicOutputConfig {
                effort: Some("medium".to_string()),
                format: None,
            })
        );
        assert_eq!(request.max_tokens, 32_000);
    }

    #[test]
    fn thinking_only_models_do_not_send_output_effort() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hi".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(PathBuf::from("/tmp/workspace")),
            tools: vec![],
            parallel_tool_calls: false,
            base_instructions: BaseInstructions {
                text: "ignored".to_string(),
            },
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        };

        let request = build_request(
            &prompt,
            &thinking_only_model_info("claude-haiku-4-5-20251001"),
            Some(ReasoningEffortConfig::High),
            "session-123",
            None,
        )
        .expect("build request");

        assert_eq!(
            request.thinking,
            Some(AnthropicThinkingConfig::enabled(31_999))
        );
        assert_eq!(request.output_config, None);
    }
}
