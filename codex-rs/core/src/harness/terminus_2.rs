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
use codex_utils_string::take_bytes_at_char_boundary;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

const TERMINUS_2_TEMPERATURE: f64 = 0.7;
const TERMINUS_2_COMMAND_TIMEOUT_MS: u64 = 180_000;
const TERMINUS_2_OUTPUT_LIMIT_BYTES: usize = 10_000;
const TERMINUS_2_PARSE_ERROR_PREFIX: &str = "Previous response had parsing errors:";
const TERMINUS_2_COMPLETION_CONFIRMATION_FRAGMENT: &str =
    "Are you sure you want to mark the task as complete?";
const TERMINUS_2_PREVIOUS_BUFFER_START: &str =
    "__OPEN_INTERPRETER_TERMINUS_2_PREVIOUS_BUFFER_START__";
const TERMINUS_2_PREVIOUS_BUFFER_END: &str = "__OPEN_INTERPRETER_TERMINUS_2_PREVIOUS_BUFFER_END__";
const TERMINUS_2_CURRENT_BUFFER_START: &str =
    "__OPEN_INTERPRETER_TERMINUS_2_CURRENT_BUFFER_START__";
const TERMINUS_2_CURRENT_BUFFER_END: &str = "__OPEN_INTERPRETER_TERMINUS_2_CURRENT_BUFFER_END__";
const TERMINUS_2_CURRENT_SCREEN_START: &str =
    "__OPEN_INTERPRETER_TERMINUS_2_CURRENT_SCREEN_START__";
const TERMINUS_2_CURRENT_SCREEN_END: &str = "__OPEN_INTERPRETER_TERMINUS_2_CURRENT_SCREEN_END__";
static TERMINUS_2_ACTION_CALL_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<(Value, ToolKinds, Terminus2RequestKind), serde_json::Error> {
    let kind = request_kind(prompt);
    Ok((
        json!({
            "messages": build_messages(prompt, &kind),
            "model": model_info.slug,
            "temperature": TERMINUS_2_TEMPERATURE,
        }),
        ToolKinds::new(),
        kind,
    ))
}

pub(crate) fn inject_action_calls(
    stream: ResponseStream,
    request_kind: Terminus2RequestKind,
    pending_completion: bool,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel(1600);

    tokio::spawn(async move {
        let mut stream = stream;
        while let Some(event) = stream.next().await {
            let harness_items = match &event {
                Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                    role, content, ..
                })) if role == "assistant" => plain_text_content(content)
                    .as_deref()
                    .map(|content| {
                        build_harness_follow_up_items(content, &request_kind, pending_completion)
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };

            if tx_event.send(event).await.is_err() {
                return;
            }

            for item in harness_items {
                if tx_event
                    .send(Ok(ResponseEvent::OutputItemDone(item)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    });

    ResponseStream {
        rx_event,
        consumer_dropped: tokio_util::sync::CancellationToken::new(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Terminus2RequestKind {
    Action {
        current_workdir: String,
    },
    Summary {
        original_instruction: String,
        current_screen: String,
    },
    Questions,
    AnswerQuestions {
        questions: String,
    },
    Handoff {
        current_workdir: String,
    },
}

pub(crate) fn prompt_has_completion_confirmation(prompt: &Prompt) -> bool {
    prompt.get_formatted_input().iter().any(|item| {
        let ResponseItem::Message { role, content, .. } = item else {
            return false;
        };
        role == "user"
            && plain_text_content(content)
                .as_deref()
                .is_some_and(|text| text.contains(TERMINUS_2_COMPLETION_CONFIRMATION_FRAGMENT))
    })
}

fn build_messages(prompt: &Prompt, kind: &Terminus2RequestKind) -> Vec<Value> {
    let entries = prompt_entries(prompt);
    match kind {
        Terminus2RequestKind::Questions => {
            if let Some(messages) = build_questions_after_summary_response(&entries) {
                return messages;
            }
            return entries
                .iter()
                .rev()
                .find_map(|entry| match entry {
                    PromptEntry::Message { role, content }
                        if role == "user" && content.starts_with("You are picking up work") =>
                    {
                        Some(vec![json!({
                            "role": "user",
                            "content": content,
                        })])
                    }
                    _ => None,
                })
                .unwrap_or_default();
        }
        Terminus2RequestKind::AnswerQuestions { .. } => {
            return build_answer_question_messages(&entries);
        }
        Terminus2RequestKind::Handoff { .. } => {
            return build_handoff_messages(&entries);
        }
        Terminus2RequestKind::Action { .. } | Terminus2RequestKind::Summary { .. } => {}
    }

    let mut messages = Vec::new();
    let mut initial_user: Option<String> = None;
    let mut pending_commands: VecDeque<ParsedCommand> = VecDeque::new();
    let mut saw_initial_prompt = false;
    let mut pending_parse_error = false;

    for item in prompt.get_formatted_input() {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "user" if !saw_initial_prompt => {
                    if let Some(text) = plain_text_content(content)
                        && is_problem_statement_candidate(&text)
                    {
                        initial_user = Some(text);
                    }
                }
                "user" => {
                    if let Some(text) = plain_text_content(content) {
                        pending_parse_error = text.starts_with(TERMINUS_2_PARSE_ERROR_PREFIX);
                        messages.push(json!({
                            "role": "user",
                            "content": text,
                        }));
                    }
                }
                "assistant" => {
                    emit_initial_user(&mut messages, &mut initial_user, &mut saw_initial_prompt);
                    if let Some(text) = plain_text_content(content) {
                        messages.push(json!({
                            "role": "assistant",
                            "content": text,
                        }));
                        pending_commands = parse_response(&text).commands.into_iter().collect();
                    }
                }
                _ => {}
            },
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                emit_initial_user(&mut messages, &mut initial_user, &mut saw_initial_prompt);
                if pending_parse_error {
                    pending_parse_error = false;
                    continue;
                }
                let _command = pending_commands.pop_front();
                let terminal_output = terminus_terminal_output(output);
                messages.push(json!({
                    "role": "user",
                    "content": terminal_output,
                }));
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
    }

    if let Terminus2RequestKind::Summary {
        original_instruction,
        ..
    } = kind
    {
        if let Some(messages) = build_summary_after_handoff_messages(&entries, original_instruction)
        {
            return messages;
        }
        messages.retain(|message| {
            message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role != "user")
                || message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("You are an AI assistant tasked"))
        });
        messages.push(json!({
            "role": "user",
            "content": summary_prompt(original_instruction),
        }));
    }

    emit_initial_user(&mut messages, &mut initial_user, &mut saw_initial_prompt);
    messages
}

fn build_summary_after_handoff_messages(
    entries: &[PromptEntry],
    instruction: &str,
) -> Option<Vec<Value>> {
    let (question_prompt, questions) = latest_question_pair(entries)?;
    let handoff_index = entries.iter().rposition(|entry| {
        matches!(
            entry,
            PromptEntry::Message { role, content }
                if role == "user"
                    && content.starts_with("Here are the answers the other agent provided.")
        )
    })?;
    let handoff_prompt = match &entries[handoff_index] {
        PromptEntry::Message { content, .. } => content.as_str(),
        PromptEntry::ToolOutput { .. } => return None,
    };
    let handoff_assistant = entries[handoff_index + 1..]
        .iter()
        .find_map(|entry| match entry {
            PromptEntry::Message { role, content }
                if role == "assistant" && !parse_response(content).commands.is_empty() =>
            {
                Some(content.as_str())
            }
            _ => None,
        })?;
    Some(vec![
        json!({
            "role": "user",
            "content": initial_prompt(instruction, &initial_terminal_state()),
        }),
        json!({
            "role": "user",
            "content": question_prompt,
        }),
        json!({
            "role": "assistant",
            "content": questions,
        }),
        json!({
            "role": "user",
            "content": handoff_prompt,
        }),
        json!({
            "role": "assistant",
            "content": handoff_assistant,
        }),
        json!({
            "role": "user",
            "content": summary_prompt(instruction),
        }),
    ])
}

fn request_kind(prompt: &Prompt) -> Terminus2RequestKind {
    let entries = prompt_entries(prompt);
    if let Some(content) = last_user_content(&entries) {
        if content.starts_with("You are picking up work") {
            return Terminus2RequestKind::Questions;
        }
        if content.starts_with("The next agent has a few questions") {
            return Terminus2RequestKind::AnswerQuestions {
                questions: content
                    .strip_prefix(
                        "The next agent has a few questions for you, please answer each of them one by one in detail:\n\n",
                    )
                    .unwrap_or(content)
                    .to_string(),
            };
        }
        if content.starts_with("Here are the answers the other agent provided.") {
            if assistant_after_latest_user(&entries).is_some() {
                // The handoff agent has already responded. Let the summary/question
                // transition detection below decide the next provider request.
            } else {
                return Terminus2RequestKind::Handoff {
                    current_workdir: current_workdir(&entries),
                };
            }
        }
    }

    if summary_response_around_latest_summary_prompt(&entries)
        .or_else(|| latest_non_command_assistant_response(&entries))
        .is_some()
    {
        return Terminus2RequestKind::Questions;
    }
    if let Some(content) = last_user_content(&entries)
        && content.starts_with("You are about to hand off your work to another AI agent.")
        && let Some(original_instruction) =
            embedded_original_task(content).or_else(|| original_instruction(&entries))
    {
        return Terminus2RequestKind::Summary {
            original_instruction,
            current_screen: latest_terminal_screen(&entries).unwrap_or_else(initial_terminal_state),
        };
    }

    Terminus2RequestKind::Action {
        current_workdir: current_workdir(&entries),
    }
}

fn assistant_after_latest_user(entries: &[PromptEntry]) -> Option<&str> {
    let latest_user_index = entries.iter().rposition(|entry| {
        matches!(
            entry,
            PromptEntry::Message { role, .. } if role == "user"
        )
    })?;
    entries[latest_user_index + 1..]
        .iter()
        .find_map(|entry| match entry {
            PromptEntry::Message { role, content } if role == "assistant" => Some(content.as_str()),
            _ => None,
        })
}

#[derive(Debug)]
enum PromptEntry {
    Message { role: String, content: String },
    ToolOutput { output: FunctionCallOutputPayload },
}

fn prompt_entries(prompt: &Prompt) -> Vec<PromptEntry> {
    prompt
        .get_formatted_input()
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } => {
                plain_text_content(content).map(|content| PromptEntry::Message {
                    role: role.clone(),
                    content,
                })
            }
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => Some(PromptEntry::ToolOutput {
                output: output.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn last_user_content(entries: &[PromptEntry]) -> Option<&str> {
    entries.iter().rev().find_map(|entry| match entry {
        PromptEntry::Message { role, content } if role == "user" => Some(content.as_str()),
        _ => None,
    })
}

fn original_instruction(entries: &[PromptEntry]) -> Option<String> {
    entries.iter().find_map(|entry| match entry {
        PromptEntry::Message { role, content }
            if role == "user" && is_problem_statement_candidate(content) =>
        {
            embedded_original_task(content).or_else(|| Some(content.clone()))
        }
        _ => None,
    })
}

fn embedded_original_task(content: &str) -> Option<String> {
    let start = content.find("Original Task:")?;
    let after_marker = &content[start + "Original Task:".len()..];
    let end = after_marker
        .find("\n\n\n")
        .or_else(|| after_marker.find("\n\n**"))
        .unwrap_or(after_marker.len());
    let task = after_marker[..end].trim();
    (!task.is_empty()).then(|| task.to_string())
}

fn latest_terminal_screen(entries: &[PromptEntry]) -> Option<String> {
    latest_captured_terminal_screen(entries).or_else(|| render_terminal_screen(entries))
}

fn latest_captured_terminal_screen(entries: &[PromptEntry]) -> Option<String> {
    entries.iter().rev().find_map(|entry| match entry {
        PromptEntry::ToolOutput { output } => Some(
            parse_terminus_observation(&tool_output_text(output))
                .map(|observation| observation.current_screen)
                .unwrap_or_else(|| limit_terminal_screen_height(&tool_output_text(output))),
        ),
        PromptEntry::Message { .. } => None,
    })
}

fn build_questions_after_summary_response(entries: &[PromptEntry]) -> Option<Vec<Value>> {
    let summary_response = summary_response_around_latest_summary_prompt(entries)
        .or_else(|| latest_non_command_assistant_response(entries))?;
    let instruction = original_instruction(entries)?;
    Some(vec![json!({
        "role": "user",
        "content": question_prompt(
            &instruction,
            summary_response,
            &latest_terminal_screen(entries).unwrap_or_else(initial_terminal_state),
        ),
    })])
}

fn summary_response_around_latest_summary_prompt(entries: &[PromptEntry]) -> Option<&str> {
    let Some(summary_index) = entries.iter().rposition(|entry| {
        matches!(
            entry,
            PromptEntry::Message { role, content }
                if role == "user"
                    && content.starts_with("You are about to hand off your work to another AI agent.")
        )
    }) else {
        return summary_response_after_latest_handoff_prompt(entries);
    };
    entries[summary_index + 1..]
        .iter()
        .find_map(|entry| match entry {
            PromptEntry::Message { role, content } if role == "assistant" => Some(content.as_str()),
            _ => None,
        })
        .or_else(|| {
            entries[..summary_index]
                .iter()
                .rev()
                .find_map(|entry| match entry {
                    PromptEntry::Message { role, content }
                        if role == "assistant" && parse_response(content).commands.is_empty() =>
                    {
                        Some(content.as_str())
                    }
                    _ => None,
                })
        })
}

fn summary_response_after_latest_handoff_prompt(entries: &[PromptEntry]) -> Option<&str> {
    let handoff_index = entries.iter().rposition(|entry| {
        matches!(
            entry,
            PromptEntry::Message { role, content }
                if role == "user"
                    && content.starts_with("Here are the answers the other agent provided.")
        )
    })?;
    let latest_index = entries.len().checked_sub(1)?;
    if latest_index <= handoff_index {
        return None;
    }
    match &entries[latest_index] {
        PromptEntry::Message { role, content }
            if role == "assistant" && parse_response(content).commands.is_empty() =>
        {
            Some(content.as_str())
        }
        PromptEntry::Message { .. } | PromptEntry::ToolOutput { .. } => None,
    }
}

fn latest_non_command_assistant_response(entries: &[PromptEntry]) -> Option<&str> {
    match entries.last()? {
        PromptEntry::Message { role, content }
            if role == "assistant" && parse_response(content).commands.is_empty() =>
        {
            Some(content.as_str())
        }
        PromptEntry::Message { .. } | PromptEntry::ToolOutput { .. } => None,
    }
}

fn render_terminal_screen(entries: &[PromptEntry]) -> Option<String> {
    let mut pending_commands = VecDeque::<String>::new();
    let mut terminal_state = TerminalSessionState::default();
    let mut terminal = String::new();
    let mut saw_output = false;
    let mut pager_active = false;
    let mut help_active_at_batch_start = false;
    let mut pager_return_screen: Option<String> = None;
    let mut prompt_rendered = false;
    for entry in entries {
        match entry {
            PromptEntry::Message { role, content } if role == "assistant" => {
                help_active_at_batch_start =
                    terminal == less_help_screen() || terminal == less_help_screen_after_return();
                pending_commands = parse_response(content)
                    .commands
                    .into_iter()
                    .filter_map(|command| shell_command_from_keystrokes(&command.keystrokes))
                    .collect();
            }
            PromptEntry::ToolOutput { output } => {
                saw_output = true;
                let command = pending_commands.pop_front();
                if pager_active {
                    if let Some(command) = command.as_deref() {
                        if is_pager_quit_keystroke(command)
                            && (terminal == less_help_screen()
                                || terminal == less_help_screen_after_return())
                        {
                            if let Some(return_screen) = pager_return_screen.as_ref() {
                                terminal = return_screen.clone();
                            }
                        } else if is_pager_quit_keystroke(command) {
                            remove_less_status_line(&mut terminal);
                            if !terminal.ends_with('\n') {
                                terminal.push('\n');
                            }
                            terminal_state.push_prompted_command(&mut terminal, command.trim());
                            prompt_rendered = true;
                            pager_active = false;
                        } else if help_active_at_batch_start && terminal == less_help_screen() {
                            terminal = less_help_screen_after_return();
                        } else if pager_keystrokes_open_help(command) {
                            terminal = less_help_screen();
                        }
                    }
                } else {
                    if let Some(command) = command.as_deref()
                        && !command.trim().is_empty()
                    {
                        terminal_state.push_prompted_command(&mut terminal, command.trim());
                    }
                    let output_text = tool_output_text(output);
                    if command.as_deref().is_some_and(is_git_diff_command) {
                        terminal = pager_screen(&output_text);
                        pager_return_screen =
                            Some(pager_screen_without_synthetic_header(&output_text));
                        pager_active = true;
                    } else {
                        terminal.push_str(&output_text);
                    }
                    if !terminal.ends_with('\n') {
                        terminal.push('\n');
                    }
                    if let Some(command) = command {
                        terminal_state.apply_command(command.trim());
                    }
                }
            }
            PromptEntry::Message { .. } => {
                pending_commands.clear();
            }
        }
    }
    if !pager_active && !prompt_rendered {
        terminal_state.push_prompt(&mut terminal);
    }
    saw_output.then_some(limit_terminal_screen_height(&terminal))
}

fn remove_less_status_line(terminal: &mut String) {
    if terminal.ends_with(":\n") {
        terminal.truncate(terminal.len() - 2);
    }
}

fn current_workdir(entries: &[PromptEntry]) -> String {
    latest_captured_terminal_screen(entries)
        .and_then(|screen| cwd_from_terminal_screen(&screen))
        .unwrap_or_else(|| current_workdir_before_index(entries, entries.len()).cwd)
}

fn cwd_from_terminal_screen(screen: &str) -> Option<String> {
    let prompt_prefix = format!("root@{}:", terminus_hostname());
    screen.lines().rev().find_map(|line| {
        let after_host = line.strip_prefix(&prompt_prefix)?;
        let prompt_index = after_host.rfind('#')?;
        let cwd = after_host[..prompt_index].trim();
        (!cwd.is_empty()).then(|| cwd.to_string())
    })
}

fn current_workdir_before_index(entries: &[PromptEntry], end_index: usize) -> TerminalSessionState {
    let mut state = TerminalSessionState::default();
    let mut pending_commands = VecDeque::<String>::new();
    let mut pager_active = false;
    for entry in entries.iter().take(end_index) {
        match entry {
            PromptEntry::Message { role, content } if role == "assistant" => {
                pending_commands = parse_response(content)
                    .commands
                    .into_iter()
                    .filter_map(|command| shell_command_from_keystrokes(&command.keystrokes))
                    .collect();
            }
            PromptEntry::ToolOutput { .. } => {
                if let Some(command) = pending_commands.pop_front() {
                    if pager_active {
                        if is_pager_quit_keystroke(&command) {
                            pager_active = false;
                        }
                        continue;
                    }
                    if is_git_diff_command(&command) {
                        pager_active = true;
                    } else {
                        state.apply_command(&command);
                    }
                }
            }
            PromptEntry::Message { .. } => {}
        }
    }
    state
}

fn build_answer_question_messages(entries: &[PromptEntry]) -> Vec<Value> {
    let Some(answer_prompt) = last_user_content(entries) else {
        return Vec::new();
    };
    let Some(instruction) = original_instruction(entries) else {
        return Vec::new();
    };
    let mut messages = vec![json!({
        "role": "user",
        "content": initial_prompt(&instruction, &initial_terminal_state()),
    })];
    if question_prompt_count(entries) > 1
        && append_repeated_answer_question_messages(&mut messages, entries, &instruction)
    {
        return messages;
    }
    if let Some(first_assistant) = first_action_assistant(entries) {
        messages.push(json!({
            "role": "assistant",
            "content": first_assistant,
        }));
    }
    if let Some(summary_response) = assistant_before_last_question_prompt(entries) {
        messages.push(json!({
            "role": "user",
            "content": summary_prompt(&instruction),
        }));
        messages.push(json!({
            "role": "assistant",
            "content": summary_response,
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": answer_prompt,
    }));
    messages
}

fn question_prompt_count(entries: &[PromptEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| {
            matches!(entry, PromptEntry::Message { role, content } if role == "user" && content.starts_with("You are picking up work"))
        })
        .count()
}

fn append_repeated_answer_question_messages(
    messages: &mut Vec<Value>,
    entries: &[PromptEntry],
    instruction: &str,
) -> bool {
    let Some((handoff_index, handoff_prompt, handoff_assistant)) = latest_handoff_pair(entries)
    else {
        return false;
    };
    let Some((question_prompt, questions)) = latest_question_pair_before(entries, handoff_index)
    else {
        return false;
    };
    let Some(summary_response) = assistant_before_last_question_prompt(entries) else {
        return false;
    };
    let Some(answer_prompt) = last_user_content(entries) else {
        return false;
    };

    messages.push(json!({
        "role": "user",
        "content": question_prompt,
    }));
    messages.push(json!({
        "role": "assistant",
        "content": questions,
    }));
    messages.push(json!({
        "role": "user",
        "content": handoff_prompt,
    }));
    messages.push(json!({
        "role": "assistant",
        "content": handoff_assistant,
    }));
    messages.push(json!({
        "role": "user",
        "content": summary_prompt(instruction),
    }));
    messages.push(json!({
        "role": "assistant",
        "content": summary_response,
    }));
    messages.push(json!({
        "role": "user",
        "content": answer_prompt,
    }));
    true
}

fn build_handoff_messages(entries: &[PromptEntry]) -> Vec<Value> {
    let Some(handoff_prompt) = last_user_content(entries) else {
        return Vec::new();
    };
    let Some(instruction) = original_instruction(entries) else {
        return Vec::new();
    };
    let Some((question_prompt, questions)) = latest_question_pair(entries) else {
        return Vec::new();
    };
    vec![
        json!({
            "role": "user",
            "content": initial_prompt(&instruction, &initial_terminal_state()),
        }),
        json!({
            "role": "user",
            "content": question_prompt,
        }),
        json!({
            "role": "assistant",
            "content": questions,
        }),
        json!({
            "role": "user",
            "content": handoff_prompt,
        }),
    ]
}

fn first_action_assistant(entries: &[PromptEntry]) -> Option<&str> {
    entries.iter().find_map(|entry| match entry {
        PromptEntry::Message { role, content } if role == "assistant" => Some(content.as_str()),
        _ => None,
    })
}

fn assistant_before_last_question_prompt(entries: &[PromptEntry]) -> Option<&str> {
    entries
        .iter()
        .rposition(|entry| {
            matches!(entry, PromptEntry::Message { role, content } if role == "user" && content.starts_with("You are picking up work"))
        })
        .and_then(|index| {
            entries[..index].iter().rev().find_map(|entry| match entry {
                PromptEntry::Message { role, content } if role == "assistant" => {
                    Some(content.as_str())
                }
                _ => None,
            })
        })
}

fn latest_handoff_pair(entries: &[PromptEntry]) -> Option<(usize, &str, &str)> {
    let handoff_index = entries.iter().rposition(|entry| {
        matches!(
            entry,
            PromptEntry::Message { role, content }
                if role == "user"
                    && content.starts_with("Here are the answers the other agent provided.")
        )
    })?;
    let handoff_prompt = match &entries[handoff_index] {
        PromptEntry::Message { content, .. } => content.as_str(),
        PromptEntry::ToolOutput { .. } => return None,
    };
    let handoff_assistant = entries[handoff_index + 1..]
        .iter()
        .find_map(|entry| match entry {
            PromptEntry::Message { role, content }
                if role == "assistant" && !parse_response(content).commands.is_empty() =>
            {
                Some(content.as_str())
            }
            _ => None,
        })?;
    Some((handoff_index, handoff_prompt, handoff_assistant))
}

fn latest_question_pair_before(entries: &[PromptEntry], end_index: usize) -> Option<(&str, &str)> {
    let question_index = entries[..end_index].iter().rposition(|entry| {
        matches!(entry, PromptEntry::Message { role, content } if role == "user" && content.starts_with("You are picking up work"))
    })?;
    let question_prompt = match &entries[question_index] {
        PromptEntry::Message { content, .. } => content.as_str(),
        PromptEntry::ToolOutput { .. } => return None,
    };
    let questions =
        entries[question_index + 1..end_index]
            .iter()
            .find_map(|entry| match entry {
                PromptEntry::Message { role, content } if role == "assistant" => {
                    Some(content.as_str())
                }
                _ => None,
            })?;
    Some((question_prompt, questions))
}

fn latest_question_pair(entries: &[PromptEntry]) -> Option<(&str, &str)> {
    let question_index = entries.iter().rposition(|entry| {
        matches!(entry, PromptEntry::Message { role, content } if role == "user" && content.starts_with("You are picking up work"))
    })?;
    let question_prompt = match &entries[question_index] {
        PromptEntry::Message { content, .. } => content.as_str(),
        PromptEntry::ToolOutput { .. } => return None,
    };
    let questions = entries[question_index + 1..]
        .iter()
        .find_map(|entry| match entry {
            PromptEntry::Message { role, content } if role == "assistant" => Some(content.as_str()),
            _ => None,
        })?;
    Some((question_prompt, questions))
}

fn emit_initial_user(
    messages: &mut Vec<Value>,
    initial_user: &mut Option<String>,
    saw_initial_prompt: &mut bool,
) {
    if *saw_initial_prompt {
        return;
    }
    if let Some(instruction) = initial_user.take() {
        messages.push(json!({
            "role": "user",
            "content": initial_prompt(&instruction, &initial_terminal_state()),
        }));
        *saw_initial_prompt = true;
    }
}

fn build_harness_follow_up_items(
    content: &str,
    request_kind: &Terminus2RequestKind,
    pending_completion: bool,
) -> Vec<ResponseItem> {
    match request_kind {
        Terminus2RequestKind::Summary {
            original_instruction,
            current_screen,
        } => {
            return vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: question_prompt(original_instruction, content, current_screen),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }];
        }
        Terminus2RequestKind::Questions => {
            return vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: answer_questions_prompt(content),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }];
        }
        Terminus2RequestKind::AnswerQuestions { questions } => {
            return vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: handoff_prompt(content, questions),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            }];
        }
        Terminus2RequestKind::Handoff { .. } | Terminus2RequestKind::Action { .. } => {}
    }

    let parsed = parse_response(content);
    if !parsed.error.is_empty() {
        return vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: parse_error_prompt(&parsed.error, &parsed.warning),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }];
    }

    if parsed.task_complete {
        if pending_completion {
            return Vec::new();
        }
        return vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: completion_confirmation_prompt(&initial_terminal_state()),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }];
    }

    let working_directory = match request_kind {
        Terminus2RequestKind::Action { current_workdir }
        | Terminus2RequestKind::Handoff { current_workdir } => current_workdir.clone(),
        Terminus2RequestKind::Summary { .. }
        | Terminus2RequestKind::Questions
        | Terminus2RequestKind::AnswerQuestions { .. } => TerminalSessionState::default().cwd,
    };
    if parsed.commands.is_empty() {
        Vec::new()
    } else {
        vec![build_shell_call(parsed.commands, working_directory)]
    }
}

fn build_shell_call(commands: Vec<ParsedCommand>, working_directory: String) -> ResponseItem {
    let id = TERMINUS_2_ACTION_CALL_ID.fetch_add(1, Ordering::Relaxed);
    let command_duration_seconds = commands
        .iter()
        .map(|command| command.duration.unwrap_or(1.0).clamp(0.0, 60.0))
        .sum::<f64>();
    let execution_command = tmux_execution_command(&commands, &working_directory);
    let prompt_wait_budget_ms = commands.len() as f64 * 8_000.0;
    let timeout_ms = command_duration_seconds
        .max(0.0)
        .mul_add(1000.0, 10_000.0 + prompt_wait_budget_ms)
        .ceil()
        .min(TERMINUS_2_COMMAND_TIMEOUT_MS as f64) as u64;
    let timeout_ms = timeout_ms.max(1_000);
    let timeout_seconds = timeout_ms.saturating_add(999) / 1000;
    ResponseItem::LocalShellCall {
        id: None,
        call_id: Some(format!("terminus-2-shell-{id}")),
        status: LocalShellStatus::InProgress,
        action: LocalShellAction::Exec(LocalShellExecAction {
            command: vec![
                "timeout".to_string(),
                "--kill-after=2s".to_string(),
                format!("{timeout_seconds}s"),
                "bash".to_string(),
                "-lc".to_string(),
                execution_command,
            ],
            timeout_ms: Some(timeout_ms.saturating_add(3_000)),
            working_directory: Some(working_directory),
            env: Some(HashMap::from([
                ("PAGER".to_string(), "less".to_string()),
                ("GIT_PAGER".to_string(), "less".to_string()),
                ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
                ("GIT_CONFIG_KEY_0".to_string(), "log.decorate".to_string()),
                ("GIT_CONFIG_VALUE_0".to_string(), "short".to_string()),
            ])),
            user: None,
        }),
        internal_chat_message_metadata_passthrough: None,
    }
}

fn tmux_execution_command(commands: &[ParsedCommand], working_directory: &str) -> String {
    let quoted_working_directory = shell_quote(working_directory);
    let hostname = terminus_hostname();
    let prompt = shell_quote(&format!("root@{hostname}:\\w# "));
    let prompt_prefix = shell_quote(&format!("root@{hostname}:"));
    let previous_buffer_start = shell_quote(TERMINUS_2_PREVIOUS_BUFFER_START);
    let previous_buffer_end = shell_quote(TERMINUS_2_PREVIOUS_BUFFER_END);
    let current_buffer_start = shell_quote(TERMINUS_2_CURRENT_BUFFER_START);
    let current_buffer_end = shell_quote(TERMINUS_2_CURRENT_BUFFER_END);
    let current_screen_start = shell_quote(TERMINUS_2_CURRENT_SCREEN_START);
    let current_screen_end = shell_quote(TERMINUS_2_CURRENT_SCREEN_END);
    let command_script = commands
        .iter()
        .map(|command| {
            let duration = command.duration.unwrap_or(1.0).clamp(0.0, 60.0);
            let quoted_keystrokes = shell_quote(&command.keystrokes);
            format!(
                r#"keys={quoted_keystrokes}
send_terminus_keys "$keys"
sleep {duration}
case "$keys" in
  *$'\n'|"C-c") wait_for_terminus_prompt ;;
esac
"#
            )
        })
        .collect::<String>();
    format!(
        r##"set -e
session="${{INTERPRETER_TERMINUS_2_TMUX_SESSION:-oi-terminus-2-$(printf %s "${{INTERPRETER_HOME:-${{CODEX_HOME:-$PWD}}}}" | cksum | awk '{{print $1}}')}}"
target="$session:0.0"
state_dir="${{INTERPRETER_HOME:-${{CODEX_HOME:-$PWD}}}}/.terminus-2"
mkdir -p "$state_dir"
previous_buffer_path="$state_dir/$session.previous-buffer"
current_buffer_path="$state_dir/$session.current-buffer"
current_screen_path="$state_dir/$session.current-screen"
if ! tmux has-session -t "$session" 2>/dev/null; then
  tmux new-session -x 160 -y 40 -d -s "$session" -c {quoted_working_directory} "env PAGER=less GIT_PAGER=less GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=log.decorate GIT_CONFIG_VALUE_0=short PS1={prompt} bash --noprofile --norc -i"
  tmux set-option -t "$session" history-limit 50000 >/dev/null
  sleep 0.1
fi
tmux resize-pane -t "$target" -x 160 -y 40 2>/dev/null || true
if [ ! -f "$previous_buffer_path" ]; then
  tmux capture-pane -p -t "$target" -S - > "$previous_buffer_path"
fi
prompt_prefix={prompt_prefix}
wait_for_terminus_prompt() {{
local last_line
for _ in {{1..50}}; do
  last_line="$(tmux capture-pane -p -t "$target" | awk 'NF {{ line=$0 }} END {{ sub(/[[:space:]]+$/, "", line); print line }}')"
  case "$last_line" in
    "$prompt_prefix"*"#") return 0 ;;
  esac
  sleep 0.1
done
}}
send_terminus_keys() {{
local keys="$1"
case "$keys" in
  "C-c") tmux send-keys -t "$target" C-c ;;
  "C-d") tmux send-keys -t "$target" C-d ;;
  "")
    ;;
  *)
    printf '%s' "$keys" | tmux load-buffer -
    tmux paste-buffer -t "$target"
    ;;
esac
}}
{command_script}
tmux capture-pane -p -t "$target" -S - > "$current_buffer_path"
tmux capture-pane -p -t "$target" > "$current_screen_path"
printf '%s\n' {previous_buffer_start}
cat "$previous_buffer_path"
printf '\n%s\n' {previous_buffer_end}
printf '%s\n' {current_buffer_start}
cat "$current_buffer_path"
printf '\n%s\n' {current_buffer_end}
printf '%s\n' {current_screen_start}
cat "$current_screen_path"
printf '%s\n' {current_screen_end}
cp "$current_buffer_path" "$previous_buffer_path"
"##
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn shell_command_from_keystrokes(keystrokes: &str) -> Option<String> {
    let trimmed = keystrokes.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed {
        "C-c" => None,
        command => Some(command.to_string()),
    }
}

#[derive(Debug, Clone)]
struct TerminalSessionState {
    hostname: String,
    cwd: String,
    previous_cwd: Option<String>,
}

impl Default for TerminalSessionState {
    fn default() -> Self {
        Self::new("/app".to_string())
    }
}

impl TerminalSessionState {
    fn new(cwd: String) -> Self {
        Self {
            hostname: terminus_hostname(),
            cwd,
            previous_cwd: None,
        }
    }

    fn push_prompted_command(&self, output: &mut String, command: &str) {
        output.push_str("root@");
        output.push_str(&self.hostname);
        output.push(':');
        output.push_str(&self.cwd);
        output.push_str("# ");
        output.push_str(command);
        output.push('\n');
    }

    fn push_prompt(&self, output: &mut String) {
        output.push_str("root@");
        output.push_str(&self.hostname);
        output.push(':');
        output.push_str(&self.cwd);
        output.push_str("#\n");
    }

    fn apply_command(&mut self, command: &str) {
        let Some(target) = cd_target(command) else {
            return;
        };
        let next = match target.as_str() {
            "-" => self
                .previous_cwd
                .clone()
                .unwrap_or_else(|| self.cwd.clone()),
            "" | "~" => "/root".to_string(),
            _ => resolve_workdir(&self.cwd, &target),
        };
        if next != self.cwd {
            self.previous_cwd = Some(std::mem::replace(&mut self.cwd, next));
        }
    }
}

fn terminus_hostname() -> String {
    std::env::var("OPEN_INTERPRETER_TERMINUS_2_HOSTNAME_OVERRIDE")
        .ok()
        .filter(|hostname| !hostname.trim().is_empty())
        .unwrap_or_else(|| "<docker-host>".to_string())
}

fn cd_target(command: &str) -> Option<String> {
    let words = shlex::split(command.trim())?;
    match words.as_slice() {
        [cd] if cd == "cd" => Some(String::new()),
        [cd, target] if cd == "cd" => Some(target.clone()),
        _ => None,
    }
}

fn resolve_workdir(current: &str, target: &str) -> String {
    let path = if target.starts_with('/') {
        PathBuf::from(target)
    } else {
        Path::new(current).join(target)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::Prefix(_) => {}
        }
    }
    let text = normalized.to_string_lossy().to_string();
    if text.is_empty() {
        "/".to_string()
    } else {
        text
    }
}

fn terminus_terminal_output(output: &FunctionCallOutputPayload) -> String {
    let text = tool_output_text(output);
    if let Some(observation) = parse_terminus_observation(&text) {
        return limit_output_length(&observation.incremental_output());
    }
    limit_output_length(&limit_terminal_screen_height(&text))
}

fn tool_output_text(output: &FunctionCallOutputPayload) -> String {
    let mut text = output.text_content().unwrap_or_default().to_string();
    if let Ok(parsed) = serde_json::from_str::<Value>(&text)
        && let Some(raw_output) = parsed.get("output").and_then(Value::as_str)
    {
        text = raw_output.to_string();
    }
    text
}

struct TerminusObservation {
    previous_buffer: String,
    current_buffer: String,
    current_screen: String,
}

impl TerminusObservation {
    fn incremental_output(&self) -> String {
        if let Some(new_content) =
            find_terminus_new_content(&self.previous_buffer, &self.current_buffer)
            && !new_content.trim().is_empty()
        {
            return format!("New Terminal Output:\n{new_content}");
        }
        format!("Current Terminal Screen:\n{}", self.current_screen)
    }
}

fn parse_terminus_observation(text: &str) -> Option<TerminusObservation> {
    Some(TerminusObservation {
        previous_buffer: marker_section(
            text,
            TERMINUS_2_PREVIOUS_BUFFER_START,
            TERMINUS_2_PREVIOUS_BUFFER_END,
        )?
        .to_string(),
        current_buffer: marker_section(
            text,
            TERMINUS_2_CURRENT_BUFFER_START,
            TERMINUS_2_CURRENT_BUFFER_END,
        )?
        .to_string(),
        current_screen: marker_section(
            text,
            TERMINUS_2_CURRENT_SCREEN_START,
            TERMINUS_2_CURRENT_SCREEN_END,
        )?
        .to_string(),
    })
}

fn marker_section<'a>(text: &'a str, start_marker: &str, end_marker: &str) -> Option<&'a str> {
    let start_index = text.find(start_marker)? + start_marker.len();
    let after_start = text[start_index..]
        .strip_prefix('\n')
        .unwrap_or(&text[start_index..]);
    let end_index = after_start.find(end_marker)?;
    Some(&after_start[..end_index])
}

fn find_terminus_new_content(previous_buffer: &str, current_buffer: &str) -> Option<String> {
    let previous = previous_buffer.trim();
    if !current_buffer.contains(previous) {
        return None;
    }
    if previous.contains('\n') {
        let index = previous.rfind('\n')?;
        return Some(current_buffer[index..].to_string());
    }
    let index = current_buffer.find(previous)?;
    Some(current_buffer[index..].to_string())
}

fn limit_output_length(output: &str) -> String {
    if output.len() <= TERMINUS_2_OUTPUT_LIMIT_BYTES {
        return output.to_string();
    }
    let portion_size = TERMINUS_2_OUTPUT_LIMIT_BYTES / 2;
    let first = take_bytes_at_char_boundary(output, portion_size);
    let mut last_start = output.len() - portion_size;
    while !output.is_char_boundary(last_start) {
        last_start += 1;
    }
    let last = &output[last_start..];
    let omitted = output.len() - first.len() - last.len();
    format!(
        "{first}\n[... output limited to {TERMINUS_2_OUTPUT_LIMIT_BYTES} bytes; {omitted} interior bytes omitted ...]\n{last}"
    )
}

fn limit_terminal_screen_height(output: &str) -> String {
    let mut lines = output.lines().map(str::trim_end).collect::<Vec<_>>();
    if lines.len() > 40 {
        lines = lines[lines.len() - 40..].to_vec();
    }
    let missing = 40usize.saturating_sub(lines.len());
    let mut limited = lines.join("\n");
    limited.push('\n');
    limited.push_str(&"\n".repeat(missing));
    limited
}

fn is_git_diff_command(command: &str) -> bool {
    let trimmed = command.trim_start();
    trimmed == "git diff" || trimmed.starts_with("git diff ")
}

fn pager_screen(output: &str) -> String {
    let mut raw_lines = output
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>();
    if raw_lines
        .first()
        .is_some_and(|line| line.starts_with("index "))
        && let (Some(old_path), Some(new_path)) = (raw_lines.get(1), raw_lines.get(2))
        && old_path.starts_with("--- ")
        && new_path.starts_with("+++ ")
    {
        let old_path = old_path.trim_start_matches("--- ");
        let new_path = new_path.trim_start_matches("+++ ");
        let header = format!("diff --git {old_path} {new_path}");
        raw_lines.insert(0, header);
    }
    pager_screen_from_raw_lines(&raw_lines)
}

fn pager_screen_without_synthetic_header(output: &str) -> String {
    let mut raw_lines = output
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>();
    if raw_lines
        .first()
        .is_some_and(|line| line.starts_with("diff --git "))
    {
        raw_lines.remove(0);
    }
    pager_screen_from_raw_lines(&raw_lines)
}

fn pager_screen_from_raw_lines(raw_lines: &[String]) -> String {
    let mut wrapped = Vec::new();
    for line in raw_lines {
        if line.is_empty() {
            wrapped.push(line.as_str());
            continue;
        }
        let mut start = 0;
        while start < line.len() {
            let mut end = (start + 160).min(line.len());
            while end < line.len() && !line.is_char_boundary(end) {
                end -= 1;
            }
            wrapped.push(&line[start..end]);
            start = end;
        }
    }
    let mut lines = wrapped.into_iter().take(39).collect::<Vec<_>>();
    lines.push(":");
    let mut screen = lines.join("\n");
    screen.push('\n');
    screen
}

fn pager_keystrokes_open_help(command: &str) -> bool {
    command.contains('h')
}

fn is_pager_quit_keystroke(command: &str) -> bool {
    command.trim() == "q"
}

fn less_help_screen() -> String {
    [
        "  N                 *  Repeat previous search in reverse direction.",
        "  ESC-n             *  Repeat previous search, spanning files.",
        "  ESC-N             *  Repeat previous search, reverse dir. & spanning files.",
        "  ESC-u                Undo (toggle) search highlighting.",
        "  ESC-U                Clear search highlighting.",
        "  &pattern          *  Display only matching lines.",
        "        ---------------------------------------------------",
        "        A search pattern may begin with one or more of:",
        "        ^N or !  Search for NON-matching lines.",
        "        ^E or *  Search multiple files (pass thru END OF FILE).",
        "        ^F or @  Start search at FIRST file (for /) or last file (for ?).",
        "        ^K       Highlight matches, but don't move (KEEP position).",
        "        ^R       Don't use REGULAR EXPRESSIONS.",
        "        ^W       WRAP search if no match found.",
        " ---------------------------------------------------------------------------",
        "",
        "                           JUMPING",
        "",
        "  g  <  ESC-<       *  Go to first line in file (or line N).",
        "  G  >  ESC->       *  Go to last line in file (or line N).",
        "  p  %              *  Go to beginning of file (or N percent into file).",
        "  t                 *  Go to the (N-th) next tag.",
        "  T                 *  Go to the (N-th) previous tag.",
        "  {  (  [           *  Find close bracket } ) ].",
        "  }  )  ]           *  Find open bracket { ( [.",
        "  ESC-^F <c1> <c2>  *  Find close bracket <c2>.",
        "  ESC-^B <c1> <c2>  *  Find open bracket <c1>.",
        "        ---------------------------------------------------",
        "        Each \"find close bracket\" command goes forward to the close bracket",
        "          matching the (N-th) open bracket in the top line.",
        "        Each \"find open bracket\" command goes backward to the open bracket",
        "          matching the (N-th) close bracket in the bottom line.",
        "",
        "  m<letter>            Mark the current top line with <letter>.",
        "  M<letter>            Mark the current bottom line with <letter>.",
        "  '<letter>            Go to a previously marked position.",
        "  ''                   Go to the previous position.",
        "  ^X^X                 Same as '.",
        "  ESC-M<letter>        Clear a mark.",
        "HELP -- Press RETURN for more, or q when done",
    ]
    .join("\n")
        + "\n"
}

fn less_help_screen_after_return() -> String {
    [
        "  n                 *  Repeat previous search (for N-th occurrence).",
        "  N                 *  Repeat previous search in reverse direction.",
        "  ESC-n             *  Repeat previous search, spanning files.",
        "  ESC-N             *  Repeat previous search, reverse dir. & spanning files.",
        "  ESC-u                Undo (toggle) search highlighting.",
        "  ESC-U                Clear search highlighting.",
        "  &pattern          *  Display only matching lines.",
        "        ---------------------------------------------------",
        "        A search pattern may begin with one or more of:",
        "        ^N or !  Search for NON-matching lines.",
        "        ^E or *  Search multiple files (pass thru END OF FILE).",
        "        ^F or @  Start search at FIRST file (for /) or last file (for ?).",
        "        ^K       Highlight matches, but don't move (KEEP position).",
        "        ^R       Don't use REGULAR EXPRESSIONS.",
        "        ^W       WRAP search if no match found.",
        " ---------------------------------------------------------------------------",
        "",
        "                           JUMPING",
        "",
        "  g  <  ESC-<       *  Go to first line in file (or line N).",
        "  G  >  ESC->       *  Go to last line in file (or line N).",
        "  p  %              *  Go to beginning of file (or N percent into file).",
        "  t                 *  Go to the (N-th) next tag.",
        "  T                 *  Go to the (N-th) previous tag.",
        "  {  (  [           *  Find close bracket } ) ].",
        "  }  )  ]           *  Find open bracket { ( [.",
        "  ESC-^F <c1> <c2>  *  Find close bracket <c2>.",
        "  ESC-^B <c1> <c2>  *  Find open bracket <c1>.",
        "        ---------------------------------------------------",
        "        Each \"find close bracket\" command goes forward to the close bracket",
        "          matching the (N-th) open bracket in the top line.",
        "        Each \"find open bracket\" command goes backward to the open bracket",
        "          matching the (N-th) close bracket in the bottom line.",
        "",
        "  m<letter>            Mark the current top line with <letter>.",
        "  M<letter>            Mark the current bottom line with <letter>.",
        "  '<letter>            Go to a previously marked position.",
        "  ''                   Go to the previous position.",
        "  ^X^X                 Same as '.",
        "Use new bottom of screen behavior  (press RETURN)",
    ]
    .join("\n")
        + "\n"
}

#[derive(Debug, Default)]
struct ParsedTerminusResponse {
    commands: Vec<ParsedCommand>,
    task_complete: bool,
    error: String,
    warning: String,
}

#[derive(Debug, Deserialize)]
struct ParsedCommand {
    keystrokes: String,
    #[allow(dead_code)]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawTerminusResponse {
    analysis: Option<Value>,
    plan: Option<Value>,
    commands: Option<Value>,
    task_complete: Option<Value>,
}

fn parse_response(response: &str) -> ParsedTerminusResponse {
    let Some((json_content, warning)) = extract_json_content(response) else {
        return ParsedTerminusResponse {
            error: "No valid JSON found in response".to_string(),
            warning: "- No valid JSON object found".to_string(),
            ..Default::default()
        };
    };
    let Ok(raw) = serde_json::from_str::<RawTerminusResponse>(json_content) else {
        return ParsedTerminusResponse {
            error: "Invalid JSON".to_string(),
            warning,
            ..Default::default()
        };
    };
    for field in ["analysis", "plan", "commands"] {
        if match field {
            "analysis" => raw.analysis.is_none(),
            "plan" => raw.plan.is_none(),
            "commands" => raw.commands.is_none(),
            _ => false,
        } {
            return ParsedTerminusResponse {
                error: format!("Missing required fields: {field}"),
                warning,
                ..Default::default()
            };
        }
    }
    let Some(commands_value) = raw.commands else {
        return ParsedTerminusResponse {
            error: "Missing required fields: commands".to_string(),
            warning,
            ..Default::default()
        };
    };
    let Ok(commands) = serde_json::from_value::<Vec<ParsedCommand>>(commands_value) else {
        return ParsedTerminusResponse {
            error: "Field 'commands' must be an array".to_string(),
            warning,
            ..Default::default()
        };
    };
    ParsedTerminusResponse {
        commands,
        task_complete: task_complete_value(raw.task_complete.as_ref()),
        error: String::new(),
        warning,
    }
}

fn extract_json_content(response: &str) -> Option<(&str, String)> {
    let mut json_start = None;
    let mut brace_count = 0i64;
    let mut in_string = false;
    let mut escape_next = false;
    for (index, ch) in response.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if ch == '{' {
            if brace_count == 0 {
                json_start = Some(index);
            }
            brace_count += 1;
        } else if ch == '}' {
            brace_count -= 1;
            if brace_count == 0
                && let Some(start) = json_start
            {
                let end = index + ch.len_utf8();
                let warning = extra_text_warning(&response[..start], &response[end..]);
                return Some((&response[start..end], warning));
            }
        }
    }
    None
}

fn extra_text_warning(before: &str, after: &str) -> String {
    let mut warnings = Vec::new();
    if !before.trim().is_empty() {
        warnings.push("Extra text detected before JSON object");
    }
    if !after.trim().is_empty() {
        warnings.push("Extra text detected after JSON object");
    }
    if warnings.is_empty() {
        String::new()
    } else {
        format!("- {}", warnings.join("\n- "))
    }
}

fn task_complete_value(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => {
            matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes")
        }
        _ => false,
    }
}

fn parse_error_prompt(error: &str, warning: &str) -> String {
    let feedback = if warning.is_empty() {
        format!("ERROR: {error}")
    } else {
        format!("ERROR: {error}\nWARNINGS: {warning}")
    };
    format!(
        "{TERMINUS_2_PARSE_ERROR_PREFIX}\n{feedback}\n\nPlease fix these issues and provide a proper JSON response."
    )
}

fn completion_confirmation_prompt(terminal_output: &str) -> String {
    format!(
        "Current terminal state:\n{terminal_output}\n\nAre you sure you want to mark the task as complete? This will trigger your solution to be graded and you won't be able to make any further corrections. If so, include \"task_complete\": true in your JSON response again."
    )
}

fn initial_prompt(instruction: &str, terminal_state: &str) -> String {
    format!(
        "You are an AI assistant tasked with solving command-line tasks in a Linux environment. You will be given a task description and the output from previously executed commands. Your goal is to solve the task by providing batches of shell commands.\n\nFormat your response as JSON with the following structure:\n\n{{\n  \"analysis\": \"Analyze the current state based on the terminal output provided. What do you see? What has been accomplished? What still needs to be done?\",\n  \"plan\": \"Describe your plan for the next steps. What commands will you run and why? Be specific about what you expect each command to accomplish.\",\n  \"commands\": [\n    {{\n      \"keystrokes\": \"ls -la\\n\",\n      \"duration\": 0.1\n    }},\n    {{\n      \"keystrokes\": \"cd project\\n\",\n      \"duration\": 0.1\n    }}\n  ],\n  \"task_complete\": true\n}}\n\nRequired fields:\n- \"analysis\": Your analysis of the current situation\n- \"plan\": Your plan for the next steps\n- \"commands\": Array of command objects to execute\n\nOptional fields:\n- \"task_complete\": Boolean indicating if the task is complete (defaults to false if not present)\n\nCommand object structure:\n- \"keystrokes\": String containing the exact keystrokes to send to the terminal (required)\n- \"duration\": Number of seconds to wait for the command to complete before the next command will be executed (defaults to 1.0 if not present)\n\nIMPORTANT: The text inside \"keystrokes\" will be used completely verbatim as keystrokes. Write commands exactly as you want them sent to the terminal:\n- Most bash commands should end with a newline (\\n) to cause them to execute\n- For special key sequences, use tmux-style escape sequences:\n  - C-c for Ctrl+C\n  - C-d for Ctrl+D\n\nThe \"duration\" attribute specifies the number of seconds to wait for the command to complete (default: 1.0) before the next command will be executed. On immediate tasks (e.g., cd, ls, echo, cat) set a duration of 0.1 seconds. On commands (e.g., gcc, find, rustc) set a duration of 1.0 seconds. On slow commands (e.g., make, python3 [long running script], wget [file]) set an appropriate duration as you determine necessary.\n\nIt is better to set a smaller duration than a longer duration. It is always possible to wait again if the prior output has not finished, by running {{\"keystrokes\": \"\", \"duration\": 10.0}} on subsequent requests to wait longer. Never wait longer than 60 seconds; prefer to poll to see intermediate result status.\n\nImportant notes:\n- Each command's keystrokes are sent exactly as written to the terminal\n- Do not include extra whitespace before or after the keystrokes unless it's part of the intended command\n- Extra text before or after the JSON will generate warnings but be tolerated\n- The JSON must be valid - use proper escaping for quotes and special characters within strings\n- Commands array can be empty if you want to wait without taking action\n\nTask Description:\n{instruction}\n\nCurrent terminal state:\n{terminal_state}"
    )
}

fn summary_prompt(original_instruction: &str) -> String {
    format!(
        "You are about to hand off your work to another AI agent. \"\n            f\"Please provide a comprehensive summary of what you have \"\n            f\"accomplished so far on this task:\n\nOriginal Task: {original_instruction}\n\nBased on the conversation history, please provide a detailed summary covering:\n1. **Major Actions Completed** - List each significant command you executed \"\n            f\"and what you learned from it.\n2. **Important Information Learned** - A summary of crucial findings, file \"\n            f\"locations, configurations, error messages, or system state discovered.\n3. **Challenging Problems Addressed** - Any significant issues you \"\n            f\"encountered and how you resolved them.\n4. **Current Status** - Exactly where you are in the task completion process.\n\nBe comprehensive and detailed. The next agent needs to understand everything \"\n            f\"that has happened so far in order to continue."
    )
}

fn question_prompt(
    original_instruction: &str,
    summary_response: &str,
    current_screen: &str,
) -> String {
    format!(
        "You are picking up work from a previous AI agent on this task:\n\n**Original Task:** \n{original_instruction}\n\n**Summary from Previous Agent:**\n{summary_response}\n\n**Current Terminal Screen:**\n{current_screen}\n\nPlease begin by asking several questions (at least five, more if necessary)\nabout the current state of the solution that are not answered in the\nsummary from the prior agent. After you ask these questions you will\nbe on your own, so ask everything you need to know.\n"
    )
}

fn answer_questions_prompt(model_questions: &str) -> String {
    format!(
        "The next agent has a few questions for you, please answer each of them one by one in detail:\n\n{model_questions}"
    )
}

fn handoff_prompt(model_answers: &str, _questions: &str) -> String {
    "Here are the answers the other agent provided.\n\n".to_string()
        + model_answers
        + "\n\n"
        + "Continue working on this task from where the previous agent left off."
        + " You can no longer ask questions. Please follow the spec to interact with "
        + "the terminal."
}

fn initial_terminal_state() -> String {
    format!("root@{}:/app#{}", terminus_hostname(), "\n".repeat(41))
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
            ContentItem::InputText { text } | ContentItem::OutputText { text } => Some(text),
            ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => None,
        })
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use pretty_assertions::assert_eq;

    fn model_info() -> ModelInfo {
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
        .expect("model info")
    }

    #[test]
    fn first_request_matches_terminus_shape() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Fix it".to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            }],
            ..Default::default()
        };
        let (request, tools, kind) = build_request(&prompt, &model_info()).expect("request");
        assert_eq!(tools, ToolKinds::new());
        assert_eq!(
            kind,
            Terminus2RequestKind::Action {
                current_workdir: "/app".to_string()
            }
        );
        assert_eq!(
            request,
            json!({
                "messages": [{
                    "role": "user",
                    "content": initial_prompt("Fix it", &initial_terminal_state()),
                }],
                "model": "deepseek-chat",
                "temperature": 0.7,
            })
        );
    }

    #[test]
    fn parses_fenced_json_command() {
        let parsed = parse_response(
            "Here:\n```json\n{\"analysis\":\"a\",\"plan\":\"p\",\"commands\":[{\"keystrokes\":\"ls -la\\n\",\"duration\":0.1}]}\n```",
        );
        assert_eq!(parsed.error, "");
        assert_eq!(parsed.commands[0].keystrokes, "ls -la\n");
        assert!(
            parsed
                .warning
                .contains("Extra text detected before JSON object")
        );
    }

    #[test]
    fn limit_output_length_splits_multibyte_output_on_char_boundaries() {
        let output = "\u{65e5}".repeat(4_000);
        let portion = "\u{65e5}".repeat(1_666);
        let omitted = output.len() - 2 * portion.len();
        assert_eq!(
            limit_output_length(&output),
            format!(
                "{portion}\n[... output limited to {TERMINUS_2_OUTPUT_LIMIT_BYTES} bytes; {omitted} interior bytes omitted ...]\n{portion}"
            )
        );
    }

    #[test]
    fn pager_screen_wraps_multibyte_lines_on_char_boundaries() {
        // A line made entirely of 3-byte characters (日 = U+65E5) that exceeds
        // the 160-byte wrap width.  Before the fix, the byte-based slice
        // `&line[start..end]` would land inside a character and panic.
        let line = "\u{65e5}".repeat(80); // 240 bytes, 80 chars
        let raw_lines = vec![line];
        // Must not panic:
        let screen = pager_screen_from_raw_lines(&raw_lines);
        // Every wrapped segment must be valid UTF-8 (guaranteed by returning
        // a String) and the original content must survive.
        for segment in screen.lines() {
            if segment == ":" {
                continue;
            }
            assert!(
                segment.chars().all(|c| c == '\u{65e5}'),
                "unexpected character in segment: {segment:?}"
            );
        }
    }

    #[test]
    fn long_wait_command_has_capture_budget_after_sleep() {
        let call = build_shell_call(
            vec![ParsedCommand {
                keystrokes: String::new(),
                duration: Some(60.0),
            }],
            "/app".to_string(),
        );
        let ResponseItem::LocalShellCall {
            action:
                LocalShellAction::Exec(LocalShellExecAction {
                    command,
                    timeout_ms,
                    ..
                }),
            ..
        } = call
        else {
            panic!("expected local shell call");
        };
        assert_eq!(command[0], "timeout");
        assert_eq!(command[2], "78s");
        assert_eq!(timeout_ms, Some(81_000));
    }

    #[test]
    fn completion_requires_confirmation() {
        let items = build_harness_follow_up_items(
            "{\"analysis\":\"a\",\"plan\":\"p\",\"commands\":[],\"task_complete\":true}",
            &Terminus2RequestKind::Action {
                current_workdir: "/app".to_string(),
            },
            /*pending_completion*/ false,
        );
        assert!(
            matches!(items.first(), Some(ResponseItem::Message { role, .. }) if role == "user")
        );
        assert!(
            build_harness_follow_up_items(
                "{\"analysis\":\"a\",\"plan\":\"p\",\"commands\":[],\"task_complete\":true}",
                &Terminus2RequestKind::Action {
                    current_workdir: "/app".to_string(),
                },
                /*pending_completion*/ true,
            )
            .is_empty()
        );
    }

    #[test]
    fn tool_output_keeps_same_interactive_agent_in_action_loop() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Fix the repo".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "{\"analysis\":\"Inspect\",\"plan\":\"pwd\",\"commands\":[{\"keystrokes\":\"pwd\\n\",\"duration\":0.1}]}".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("terminus-2-shell-0".to_string()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text(
                        "root@host:/app# pwd\n/app\nroot@host:/app#\n".to_string(),
                    ),

                    internal_chat_message_metadata_passthrough: None,},
            ],
            ..Default::default()
        };
        let (request, _, kind) = build_request(&prompt, &model_info()).expect("request");
        assert_eq!(
            kind,
            Terminus2RequestKind::Action {
                current_workdir: "/app".to_string()
            }
        );
        let messages = request
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages");
        assert_eq!(
            messages.last().and_then(|message| message.get("role")),
            Some(&json!("user"))
        );
        assert!(
            messages
                .last()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("root@host:/app#"))
        );
    }

    #[test]
    fn handoff_summary_prompt_is_summary_request() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "You are about to hand off your work to another AI agent.\n\nOriginal Task: Fix the repo\n\n\nBased on the conversation history, please provide a detailed summary."
                        .to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,}],
            ..Default::default()
        };
        let (_, _, kind) = build_request(&prompt, &model_info()).expect("request");
        assert_eq!(
            kind,
            Terminus2RequestKind::Summary {
                original_instruction: "Fix the repo".to_string(),
                current_screen: initial_terminal_state(),
            }
        );
    }

    #[test]
    fn summary_response_turn_becomes_questions_request() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Original Task: Fix the repo".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "You are about to hand off your work to another AI agent.\n\nOriginal Task: Fix the repo\n\n\nBased on the conversation history, please provide a detailed summary."
                            .to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "Summary so far".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
            ],
            ..Default::default()
        };
        let (request, _, kind) = build_request(&prompt, &model_info()).expect("request");
        assert_eq!(kind, Terminus2RequestKind::Questions);
        let messages = request
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages");
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0]
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.starts_with("You are picking up work"))
        );
    }

    #[test]
    fn command_shaped_summary_response_still_becomes_questions_request() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Original Task: Fix the repo".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "You are about to hand off your work to another AI agent.\n\nOriginal Task: Fix the repo\n\n\nBased on the conversation history, please provide a detailed summary."
                            .to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "```json\n{\"analysis\":\"Summary\",\"plan\":\"Ask\",\"commands\":[{\"keystrokes\":\"git status\\n\",\"duration\":0.1}]}\n```"
                            .to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
            ],
            ..Default::default()
        };
        let (request, _, kind) = build_request(&prompt, &model_info()).expect("request");
        assert_eq!(kind, Terminus2RequestKind::Questions);
        let messages = request
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages");
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0]
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("\"keystrokes\":\"git status\\n\""))
        );
    }

    #[test]
    fn summary_response_before_summary_prompt_becomes_questions_request() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Original Task: Fix the repo".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "Summary so far".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "You are about to hand off your work to another AI agent.\n\nOriginal Task: Fix the repo\n\n\nBased on the conversation history, please provide a detailed summary."
                            .to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,},
            ],
            ..Default::default()
        };
        let (request, _, kind) = build_request(&prompt, &model_info()).expect("request");
        assert_eq!(kind, Terminus2RequestKind::Questions);
        let messages = request
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages");
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0]
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("Summary so far"))
        );
    }

    #[test]
    fn summary_response_after_handoff_becomes_questions_request() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Original Task: Fix the repo".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Here are the answers the other agent provided.\n\nAnswers"
                            .to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "```json\n{\"analysis\":\"Run tests\",\"commands\":[]}\n```"
                            .to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            ..Default::default()
        };
        let (request, _, kind) = build_request(&prompt, &model_info()).expect("request");
        assert_eq!(kind, Terminus2RequestKind::Questions);
        let messages = request
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages");
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0]
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("Run tests"))
        );
    }
}
