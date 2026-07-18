use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::event_mapping::is_contextual_dev_fragment;
use crate::event_mapping::is_contextual_user_message_content;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use futures::StreamExt;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;

const MINI_SWE_AGENT_SYSTEM_PROMPT: &str =
    "You are a helpful assistant that can interact with a computer.";
const MINI_SWE_AGENT_NO_TOOL_CALL_ERROR: &str = "Tool call error:\n\n<error>\nNo tool calls found in the response. Every response MUST include at least one tool call.\n</error>\n\nHere is general guidance on how to submit correct toolcalls:\n\nEvery response needs to use the 'bash' tool at least once to execute commands.\n\nCall the bash tool with your command as the argument:\n- Tool: bash\n- Arguments: {\"command\": \"your_command_here\"}\n\nIf you want to end the task, please issue the following command: `echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT`\nwithout any other command.";

pub(crate) fn inject_no_tool_call_format_error(stream: ResponseStream) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel(1600);

    tokio::spawn(async move {
        let mut stream = stream;
        let mut saw_assistant_message = false;
        let mut saw_tool_call = false;
        let mut buffered_events: Vec<codex_protocol::error::Result<ResponseEvent>> = Vec::new();
        while let Some(event) = stream.next().await {
            match &event {
                Ok(ResponseEvent::OutputItemDone(ResponseItem::Message { role, .. }))
                    if role == "assistant" =>
                {
                    saw_assistant_message = true;
                }
                Ok(ResponseEvent::OutputItemDone(
                    ResponseItem::FunctionCall { .. }
                    | ResponseItem::CustomToolCall { .. }
                    | ResponseItem::LocalShellCall { .. },
                )) => {
                    saw_tool_call = true;
                }
                Ok(ResponseEvent::Completed { .. }) => {
                    if saw_assistant_message && !saw_tool_call {
                        if tx_event
                            .send(Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                                id: None,
                                role: "user".to_string(),
                                content: vec![ContentItem::InputText {
                                    text: MINI_SWE_AGENT_NO_TOOL_CALL_ERROR.to_string(),
                                }],
                                phase: None,
                                internal_chat_message_metadata_passthrough: None,
                            })))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    } else {
                        for buffered_event in buffered_events.drain(..) {
                            if tx_event.send(buffered_event).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                Ok(ResponseEvent::Created) => {
                    saw_assistant_message = false;
                    saw_tool_call = false;
                    buffered_events.clear();
                    if tx_event.send(event).await.is_err() {
                        return;
                    }
                    continue;
                }
                Ok(ResponseEvent::OutputItemAdded(_))
                | Ok(ResponseEvent::ServerModel(_))
                | Ok(ResponseEvent::ModelVerifications(_))
                | Ok(ResponseEvent::ServerReasoningIncluded(_))
                | Ok(ResponseEvent::OutputTextDelta(_))
                | Ok(ResponseEvent::ToolCallInputDelta { .. })
                | Ok(ResponseEvent::ReasoningSummaryDelta { .. })
                | Ok(ResponseEvent::ReasoningSummaryDone { .. })
                | Ok(ResponseEvent::ReasoningContentDelta { .. })
                | Ok(ResponseEvent::ReasoningSummaryPartAdded { .. })
                | Ok(ResponseEvent::RateLimits(_))
                | Ok(ResponseEvent::ModelsEtag(_))
                | Ok(ResponseEvent::TurnModerationMetadata(_))
                | Ok(ResponseEvent::SafetyBuffering(_))
                | Ok(ResponseEvent::OutputItemDone(_))
                | Err(_) => {}
            }

            if event.is_err() {
                let _ = tx_event.send(event).await;
                return;
            }
            let is_completed = matches!(event, Ok(ResponseEvent::Completed { .. }));
            if is_completed {
                if tx_event.send(event).await.is_err() {
                    return;
                }
                buffered_events.clear();
            } else {
                buffered_events.push(event);
            }
        }
    });

    ResponseStream {
        rx_event,
        consumer_dropped: tokio_util::sync::CancellationToken::new(),
    }
}

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let mut messages = vec![json!({
        "role": "system",
        "content": MINI_SWE_AGENT_SYSTEM_PROMPT,
    })];
    messages.extend(build_messages(prompt.get_formatted_input())?);
    let tools = build_tools(&prompt.tools)?;
    let tool_kinds = prompt
        .tools
        .iter()
        .map(|tool| (tool.name().to_string(), ToolOutputKind::Function))
        .collect();

    Ok((
        json!({
            "messages": messages,
            "model": model_info.slug,
            "tools": tools,
        }),
        tool_kinds,
    ))
}

#[cfg(test)]
fn is_terminal_submit_call(item: &ResponseItem) -> bool {
    let ResponseItem::FunctionCall {
        name, arguments, ..
    } = item
    else {
        return false;
    };
    if name != "bash" {
        return false;
    }
    let Ok(arguments) = serde_json::from_str::<Value>(arguments) else {
        return false;
    };
    arguments
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.trim() == "echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT")
}

fn build_messages(items: &[ResponseItem]) -> Result<Vec<Value>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut first_user = true;
    let mut pending_assistant_content: Option<String> = None;
    let mut pending_tool_calls = Vec::new();
    let mut pending_tool_call_content = String::new();
    let mut pending_reasoning_content = String::new();

    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    if let Some(message_content) = plain_text_content(content) {
                        if !pending_tool_calls.is_empty() {
                            append_message_text(&mut pending_tool_call_content, &message_content);
                            continue;
                        }
                        pending_assistant_content = Some(message_content);
                    }
                }
                "user" => {
                    if is_contextual_user_message_content(content)
                        || content.iter().any(is_contextual_dev_fragment)
                    {
                        continue;
                    }
                    flush_pending_assistant_content(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                    flush_pending_tool_calls(
                        &mut messages,
                        &mut pending_tool_calls,
                        &mut pending_tool_call_content,
                        &mut pending_reasoning_content,
                    );
                    if let Some(message_content) = plain_text_content(content) {
                        let content = if first_user {
                            first_user = false;
                            initial_user_prompt(&message_content)
                        } else {
                            message_content
                        };
                        messages.push(json!({
                            "role": "user",
                            "content": content,
                        }));
                    }
                }
                "developer" => {
                    if content.iter().all(is_contextual_dev_fragment) {
                        continue;
                    }
                    flush_pending_assistant_content(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                    flush_pending_tool_calls(
                        &mut messages,
                        &mut pending_tool_calls,
                        &mut pending_tool_call_content,
                        &mut pending_reasoning_content,
                    );
                    if let Some(message_content) = plain_text_content(content) {
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
                let index = pending_tool_calls.len();
                move_pending_assistant_content_to_tool_call(
                    &mut pending_assistant_content,
                    &mut pending_tool_call_content,
                );
                pending_tool_calls.push(json!({
                    "index": index,
                    "function": {
                        "arguments": arguments,
                        "name": name,
                    },
                    "id": call_id,
                    "type": "function",
                }));
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                let index = pending_tool_calls.len();
                move_pending_assistant_content_to_tool_call(
                    &mut pending_assistant_content,
                    &mut pending_tool_call_content,
                );
                pending_tool_calls.push(json!({
                    "index": index,
                    "function": {
                        "arguments": json!({ "command": input }).to_string(),
                        "name": name,
                    },
                    "id": call_id,
                    "type": "function",
                }));
            }
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
                    })
                    .to_string(),
                };
                let index = pending_tool_calls.len();
                move_pending_assistant_content_to_tool_call(
                    &mut pending_assistant_content,
                    &mut pending_tool_call_content,
                );
                pending_tool_calls.push(json!({
                    "index": index,
                    "function": {
                        "arguments": arguments,
                        "name": "bash",
                    },
                    "id": call_id,
                    "type": "function",
                }));
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            }
            | ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                flush_pending_assistant_content(
                    &mut messages,
                    &mut pending_assistant_content,
                    &mut pending_reasoning_content,
                );
                flush_pending_tool_calls(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut pending_tool_call_content,
                    &mut pending_reasoning_content,
                );
                messages.push(json!({
                    "content": mini_swe_agent_tool_output_content(output),
                    "tool_call_id": call_id,
                    "role": "tool",
                }));
            }
            ResponseItem::Reasoning { content, .. } => {
                if let Some(content) = content {
                    for entry in content {
                        let text = match entry {
                            ReasoningItemContent::ReasoningText { text }
                            | ReasoningItemContent::Text { text } => text,
                        };
                        pending_reasoning_content.push_str(text);
                    }
                }
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
            | ResponseItem::Other => {}
        }
    }

    flush_pending_tool_calls(
        &mut messages,
        &mut pending_tool_calls,
        &mut pending_tool_call_content,
        &mut pending_reasoning_content,
    );
    flush_pending_assistant_content(
        &mut messages,
        &mut pending_assistant_content,
        &mut pending_reasoning_content,
    );
    Ok(messages)
}

fn initial_user_prompt(task: &str) -> String {
    let system_information = system_information();
    let sed_note = if system_information.starts_with("Darwin ") {
        "<important>\nYou are on MacOS. For all the below examples, you need to use `sed -i ''` instead of `sed -i`.\n</important>"
    } else {
        ""
    };
    format!(
        "Please solve this issue: {task}\n\n\
You can execute bash commands and edit files to implement the necessary changes.\n\n\
## Recommended Workflow\n\n\
This workflow should be done step-by-step so that you can iterate on your changes and any possible problems.\n\n\
1. Analyze the codebase by finding and reading relevant files\n\
2. Create a script to reproduce the issue\n\
3. Edit the source code to resolve the issue\n\
4. Verify your fix works by running your script again\n\
5. Test edge cases to ensure your fix is robust\n\
6. Submit your changes and finish your work by issuing the following command: `echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT`.\n   \
Do not combine it with any other command. <important>After this command, you cannot continue working on this task.</important>\n\n\
## Command Execution Rules\n\n\
You are operating in an environment where\n\n\
1. You issue at least one command\n\
2. The system executes the command(s) in a subshell\n\
3. You see the result(s)\n\
4. You write your next command(s)\n\n\
Each response should include:\n\n\
1. **Reasoning text** where you explain your analysis and plan\n\
2. At least one tool call with your command\n\n\
**CRITICAL REQUIREMENTS:**\n\n\
- Your response SHOULD include reasoning text explaining what you're doing\n\
- Your response MUST include AT LEAST ONE bash tool call\n\
- Directory or environment variable changes are not persistent. Every action is executed in a new subshell.\n\
- However, you can prefix any action with `MY_ENV_VAR=MY_VALUE cd /path/to/working/dir && ...` or write/load environment variables from files\n\
- Submit your changes and finish your work by issuing the following command: `echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT`.\n  \
Do not combine it with any other command. <important>After this command, you cannot continue working on this task.</important>\n\n\
Example of a CORRECT response:\n\
<example_response>\n\
I need to understand the structure of the repository first. Let me check what files are in the current directory to get a better understanding of the codebase.\n\n\
[Makes bash tool call with {{\"command\": \"ls -la\"}} as arguments]\n\
</example_response>\n\n\
<system_information>\n\
{system_information}\n\
</system_information>\n\n\
## Useful command examples\n\n\
### Create a new file:\n\n\
```bash\n\
cat <<'EOF' > newfile.py\n\
import numpy as np\n\
hello = \"world\"\n\
print(hello)\n\
EOF\n\
```\n\n\
### Edit files with sed:{sed_note}\
```bash\n\
# Replace all occurrences\n\
sed -i 's/old_string/new_string/g' filename.py\n\n\
# Replace only first occurrence\n\
sed -i 's/old_string/new_string/' filename.py\n\n\
# Replace first occurrence on line 1\n\
sed -i '1s/old_string/new_string/' filename.py\n\n\
# Replace all occurrences in lines 1-10\n\
sed -i '1,10s/old_string/new_string/g' filename.py\n\
```\n\n\
### View file content:\n\n\
```bash\n\
# View specific lines with numbers\n\
nl -ba filename.py | sed -n '10,20p'\n\
```\n\n\
### Any other command you want to run\n\n\
```bash\n\
anything\n\
```"
    )
}

fn system_information() -> String {
    match std::process::Command::new("uname").arg("-srvm").output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    }
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

fn flush_pending_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_tool_call_content: &mut String,
    pending_reasoning_content: &mut String,
) {
    if pending_tool_calls.is_empty() {
        return;
    }
    let mut message = json!({
        "content": std::mem::take(pending_tool_call_content),
        "role": "assistant",
        "tool_calls": std::mem::take(pending_tool_calls),
    });
    attach_pending_reasoning_content(&mut message, pending_reasoning_content);
    messages.push(message);
}

fn flush_pending_assistant_content(
    messages: &mut Vec<Value>,
    pending_assistant_content: &mut Option<String>,
    pending_reasoning_content: &mut String,
) {
    let Some(content) = pending_assistant_content.take() else {
        return;
    };
    let mut message = json!({
        "content": content,
        "role": "assistant",
    });
    attach_pending_reasoning_content(&mut message, pending_reasoning_content);
    messages.push(message);
}

fn move_pending_assistant_content_to_tool_call(
    pending_assistant_content: &mut Option<String>,
    pending_tool_call_content: &mut String,
) {
    let Some(content) = pending_assistant_content.take() else {
        return;
    };
    append_message_text(pending_tool_call_content, &content);
}

fn attach_pending_reasoning_content(message: &mut Value, pending_reasoning_content: &mut String) {
    if pending_reasoning_content.is_empty() {
        return;
    }
    let reasoning_content = std::mem::take(pending_reasoning_content);
    message["reasoning_content"] = json!(reasoning_content);
    message["provider_specific_fields"] = json!({
        "refusal": null,
        "reasoning_content": message["reasoning_content"].clone(),
    });
}

fn append_message_text(output: &mut String, content: &str) {
    if content.is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(content);
}

fn plain_text_content(content: &[ContentItem]) -> Option<String> {
    let mut text = String::new();
    for item in content {
        match item {
            ContentItem::InputText { text: item_text }
            | ContentItem::OutputText { text: item_text } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(item_text);
            }
            ContentItem::InputImage { .. } => {}
        }
    }
    (!text.is_empty()).then_some(text)
}

fn mini_swe_agent_tool_output_content(output: &FunctionCallOutputPayload) -> String {
    if let Some(text) = output.text_content() {
        return text.to_string();
    }
    if let Some(content_items) = output.content_items() {
        return content_items
            .iter()
            .filter_map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => Some(text.as_str()),
                FunctionCallOutputContentItem::InputImage { .. }
                | FunctionCallOutputContentItem::InputVideo { .. }
                | FunctionCallOutputContentItem::EncryptedContent { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    output.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use futures::StreamExt;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use tokio::sync::mpsc;

    fn test_bash_tool() -> ToolSpec {
        ToolSpec::Function(ResponsesApiTool {
            name: "bash".to_string(),
            description: "bash".to_string(),
            strict: false,
            defer_loading: None,
            parameters: codex_tools::JsonSchema::object(
                BTreeMap::new(),
                /*required*/ None,
                /*additional_properties*/ None,
            ),
            output_schema: None,
        })
    }

    fn test_model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "kimi-k2.6",
            "display_name": "Kimi K2.6",
            "description": "desc",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [],
            "reasoning_control": "none",
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
            "context_window": 1000000,
            "auto_compact_token_limit": null,
            "experimental_supported_tools": []
        }))
        .expect("model info")
    }

    #[test]
    fn first_user_message_is_wrapped_with_mini_prompt() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some(std::convert::identity("user".to_string())),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "do the task".to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            }],
            tools: vec![test_bash_tool()],
            ..Prompt::default()
        };

        let (request, _) = build_request(&prompt, &test_model_info()).expect("request");

        assert_eq!(request["model"], json!("kimi-k2.6"));
        assert_eq!(
            request["messages"][0],
            json!({
                "role": "system",
                "content": MINI_SWE_AGENT_SYSTEM_PROMPT,
            })
        );
        assert!(
            request["messages"][1]["content"]
                .as_str()
                .expect("content")
                .starts_with("Please solve this issue: do the task\n\n")
        );
        assert_eq!(request["tools"][0]["function"]["name"], json!("bash"));
    }

    #[test]
    fn assistant_text_and_following_tool_call_are_grouped() {
        let prompt = Prompt {
            input: vec![
                ResponseItem::Message {
                    id: Some(std::convert::identity("user".to_string())),
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "do the task".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Message {
                    id: Some(std::convert::identity("assistant".to_string())),
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "I will run pwd.\n\n```bash\npwd\n```".to_string(),
                    }],
                    phase: None,

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: Some(std::convert::identity("call".to_string())),
                    name: "bash".to_string(),
                    namespace: None,
                    arguments: "{\"command\":\"pwd\"}".to_string(),
                    call_id: "bash:0".to_string(),

                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: "bash:0".to_string(),
                    output: FunctionCallOutputPayload::from_text(
                        "{\n  \"returncode\": 0,\n  \"output\": \"/workspace\\n\"\n}".to_string(),
                    ),

                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: vec![test_bash_tool()],
            ..Prompt::default()
        };

        let (request, _) = build_request(&prompt, &test_model_info()).expect("request");

        assert_eq!(request["messages"][2]["role"], json!("assistant"));
        assert_eq!(
            request["messages"][2]["content"],
            json!("I will run pwd.\n\n```bash\npwd\n```")
        );
        assert_eq!(
            request["messages"][2]["tool_calls"][0]["function"]["name"],
            json!("bash")
        );
        assert_eq!(request["messages"][3]["role"], json!("tool"));
        assert_eq!(request["messages"][3]["tool_call_id"], json!("bash:0"));
    }

    #[test]
    fn detects_terminal_submit_call() {
        let item = ResponseItem::FunctionCall {
            id: Some(std::convert::identity("call".to_string())),
            name: "bash".to_string(),
            namespace: None,
            arguments: "{\"command\":\" echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT\\n\"}"
                .to_string(),
            call_id: "bash:0".to_string(),

            internal_chat_message_metadata_passthrough: None,
        };

        assert!(is_terminal_submit_call(&item));
    }

    #[tokio::test]
    async fn text_only_response_is_replaced_with_retry_error() {
        let (tx_event, rx_event) = mpsc::channel(8);
        tx_event
            .send(Ok(ResponseEvent::Created))
            .await
            .expect("send created");
        tx_event
            .send(Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                id: Some(std::convert::identity("assistant".to_string())),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "I forgot the tool.".to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            })))
            .await
            .expect("send message");
        tx_event
            .send(Ok(ResponseEvent::Completed {
                response_id: "response".to_string(),
                token_usage: None,
                end_turn: None,
            }))
            .await
            .expect("send completed");
        drop(tx_event);

        let mut stream = inject_no_tool_call_format_error(ResponseStream {
            rx_event,
            consumer_dropped: tokio_util::sync::CancellationToken::new(),
        });
        assert!(matches!(
            stream.next().await,
            Some(Ok(ResponseEvent::Created))
        ));
        let Some(Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
            role, content, ..
        }))) = stream.next().await
        else {
            panic!("expected injected user message");
        };
        assert_eq!(role, "user");
        assert_eq!(
            content,
            vec![ContentItem::InputText {
                text: MINI_SWE_AGENT_NO_TOOL_CALL_ERROR.to_string(),
            }]
        );
        assert!(matches!(
            stream.next().await,
            Some(Ok(ResponseEvent::Completed { .. }))
        ));
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn tool_response_preserves_assistant_and_tool_call() {
        let (tx_event, rx_event) = mpsc::channel(8);
        tx_event
            .send(Ok(ResponseEvent::Created))
            .await
            .expect("send created");
        tx_event
            .send(Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                id: Some(std::convert::identity("assistant".to_string())),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "Running pwd.".to_string(),
                }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            })))
            .await
            .expect("send message");
        tx_event
            .send(Ok(ResponseEvent::OutputItemDone(
                ResponseItem::FunctionCall {
                    id: Some(std::convert::identity("call".to_string())),
                    name: "bash".to_string(),
                    namespace: None,
                    arguments: "{\"command\":\"pwd\"}".to_string(),
                    call_id: "bash:0".to_string(),

                    internal_chat_message_metadata_passthrough: None,
                },
            )))
            .await
            .expect("send tool call");
        tx_event
            .send(Ok(ResponseEvent::Completed {
                response_id: "response".to_string(),
                token_usage: None,
                end_turn: None,
            }))
            .await
            .expect("send completed");
        drop(tx_event);

        let mut stream = inject_no_tool_call_format_error(ResponseStream {
            rx_event,
            consumer_dropped: tokio_util::sync::CancellationToken::new(),
        });
        assert!(matches!(
            stream.next().await,
            Some(Ok(ResponseEvent::Created))
        ));
        assert!(matches!(
            stream.next().await,
            Some(Ok(ResponseEvent::OutputItemDone(
                ResponseItem::Message { .. }
            )))
        ));
        assert!(matches!(
            stream.next().await,
            Some(Ok(ResponseEvent::OutputItemDone(
                ResponseItem::FunctionCall { .. }
            )))
        ));
        assert!(matches!(
            stream.next().await,
            Some(Ok(ResponseEvent::Completed { .. }))
        ));
        assert!(stream.next().await.is_none());
    }
}
