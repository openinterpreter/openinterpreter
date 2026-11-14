use super::*;
use pretty_assertions::assert_eq;

fn request_with_input(input: Vec<ResponseItem>) -> ResponsesApiRequest {
    ResponsesApiRequest {
        model: "gpt-5.6".to_string(),
        input,
        tools: Some(responses_api_tools(vec![json!({
            "type": "function",
            "name": "Bash",
            "description": "Run a command",
            "parameters": { "type": "object" }
        })])),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: false,
        stream: true,
        stream_options: None,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        prompt_cache_options: None,
        client_metadata: None,
        access_programs: None,
        request_country: Default::default(),
        text: None,
    }
}

fn function_call(call_id: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: "Bash".to_string(),
        namespace: None,
        arguments: json!({ "command": "pwd" }).to_string(),
        call_id: call_id.to_string(),
        encrypted_function_args: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn function_output(call_id: &str, output: &str) -> ResponseItem {
    ResponseItem::FunctionCallOutput {
        id: None,
        call_id: Some(call_id.to_string()),
        name: None,
        namespace: None,
        output: FunctionCallOutputPayload::from_text(output.to_string()),
        internal_chat_message_metadata_passthrough: None,
    }
}

fn text_message(role: &str, text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        status: None,
        encrypted_content: None,
        role: role.to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn serialized_messages(messages: &[ChatMessage]) -> Value {
    serde_json::to_value(messages).expect("chat messages should serialize")
}

#[test]
fn defers_approval_notice_until_after_tool_output() {
    let request = request_with_input(vec![
        function_call("call-1"),
        text_message(
            "developer",
            "Approved command prefix saved:\n[\"git\", \"status\"]",
        ),
        function_output("call-1", "done"),
    ]);

    let (chat, _) = convert_request(&request).expect("request should convert");

    assert_eq!(
        serialized_messages(&chat.messages),
        json!([
            {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "Bash",
                        "arguments": "{\"command\":\"pwd\"}"
                    }
                }]
            },
            {
                "role": "tool",
                "content": "done",
                "tool_call_id": "call-1"
            },
            {
                "role": "user",
                "content": "Approved command prefix saved:\n[\"git\", \"status\"]"
            }
        ])
    );
}

#[test]
fn keeps_parallel_tool_outputs_adjacent_before_interleaved_messages() {
    let request = request_with_input(vec![
        function_call("call-1"),
        function_call("call-2"),
        text_message("developer", "approval updated"),
        function_output("call-1", "first"),
        text_message("user", "also check the hidden files"),
        function_output("call-2", "second"),
    ]);

    let (chat, _) = convert_request(&request).expect("request should convert");

    assert_eq!(
        serialized_messages(&chat.messages),
        json!([
            {
                "role": "assistant",
                "tool_calls": [
                    {
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "Bash",
                            "arguments": "{\"command\":\"pwd\"}"
                        }
                    },
                    {
                        "id": "call-2",
                        "type": "function",
                        "function": {
                            "name": "Bash",
                            "arguments": "{\"command\":\"pwd\"}"
                        }
                    }
                ]
            },
            {
                "role": "tool",
                "content": "first",
                "tool_call_id": "call-1"
            },
            {
                "role": "tool",
                "content": "second",
                "tool_call_id": "call-2"
            },
            {
                "role": "user",
                "content": "approval updated"
            },
            {
                "role": "user",
                "content": "also check the hidden files"
            }
        ])
    );
}

#[test]
fn synthesizes_aborted_output_before_deferred_messages() {
    let request = request_with_input(vec![
        function_call("call-1"),
        text_message("user", "continue without it"),
    ]);

    let (chat, _) = convert_request(&request).expect("request should convert");

    assert_eq!(
        serialized_messages(&chat.messages),
        json!([
            {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "Bash",
                        "arguments": "{\"command\":\"pwd\"}"
                    }
                }]
            },
            {
                "role": "tool",
                "content": "aborted",
                "tool_call_id": "call-1"
            },
            {
                "role": "user",
                "content": "continue without it"
            }
        ])
    );
}
