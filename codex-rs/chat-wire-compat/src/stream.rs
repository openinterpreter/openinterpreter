use crate::request::ToolKinds;
use crate::request::ToolOutputKind;
use codex_api::ApiError;
use codex_api::ResponseEvent;
use codex_api::ResponseStream;
use codex_api::SseTelemetry;
use codex_client::ByteStream;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ChatCompletionChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize, Default)]
struct Choice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    reasoning_content: Option<String>,
    // Ollama/OpenRouter send thinking as `reasoning`; DeepSeek as `reasoning_content`.
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    // Some OpenAI-compatible providers (e.g. Groq's gpt-oss) omit `index` on
    // tool-call deltas, where the OpenAI spec always includes it. Treat a
    // missing index as a continuation of the most recent tool call rather than
    // failing to parse the whole stream chunk.
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionCallDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionCallDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone, Copy)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
    #[serde(default)]
    completion_tokens: Option<i64>,
    #[serde(default)]
    total_tokens: Option<i64>,
}

#[derive(Debug, Default, Clone)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug)]
struct StreamState {
    response_id: String,
    message_item_id: String,
    reasoning_item_id: String,
    created_sent: bool,
    assistant_item_started: bool,
    assistant_text: String,
    reasoning_item_started: bool,
    reasoning_content: String,
    tool_calls: Vec<PartialToolCall>,
    finalized_tool_call_count: usize,
    usage: Option<ChatUsage>,
    server_model: Option<String>,
}

impl StreamState {
    fn new() -> Self {
        Self {
            response_id: "chatcmpl-compat".to_string(),
            message_item_id: "chat-message-1".to_string(),
            reasoning_item_id: "chat-reasoning-1".to_string(),
            created_sent: false,
            assistant_item_started: false,
            assistant_text: String::new(),
            reasoning_item_started: false,
            reasoning_content: String::new(),
            tool_calls: Vec::new(),
            finalized_tool_call_count: 0,
            usage: None,
            server_model: None,
        }
    }
}

pub(crate) fn spawn_chat_stream(
    stream: ByteStream,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    tool_kinds: ToolKinds,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(process_chat_sse(
        stream,
        tx_event,
        idle_timeout,
        telemetry,
        tool_kinds,
    ));
    ResponseStream { rx_event }
}

async fn process_chat_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    tool_kinds: ToolKinds,
) {
    let mut stream = stream.eventsource();
    let mut state = StreamState::new();

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(telemetry) = telemetry.as_ref() {
            telemetry.on_sse_poll(&response, start.elapsed());
        }

        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(error))) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(error.to_string())))
                    .await;
                return;
            }
            Ok(None) => {
                let _ = finalize_and_complete(&tx_event, &mut state, &tool_kinds).await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "idle timeout waiting for chat completions SSE".to_string(),
                    )))
                    .await;
                return;
            }
        };

        if sse.data.trim() == "[DONE]" {
            let _ = finalize_and_complete(&tx_event, &mut state, &tool_kinds).await;
            return;
        }

        let chunk: ChatCompletionChunk = match serde_json::from_str(&sse.data) {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(format!(
                        "failed to parse chat completions chunk: {error}"
                    ))))
                    .await;
                return;
            }
        };

        if let Some(id) = chunk.id.clone() {
            state.response_id = id;
        }
        if let Some(model) = chunk.model.clone()
            && state.server_model.as_deref() != Some(model.as_str())
        {
            state.server_model = Some(model.clone());
            if tx_event
                .send(Ok(ResponseEvent::ServerModel(model)))
                .await
                .is_err()
            {
                return;
            }
        }
        if !state.created_sent {
            state.created_sent = true;
            if tx_event.send(Ok(ResponseEvent::Created)).await.is_err() {
                return;
            }
        }
        if let Some(usage) = chunk.usage {
            state.usage = Some(usage);
        }

        for choice in chunk.choices {
            if let Some(delta) = choice.delta {
                if let Some(reasoning_content) = delta.reasoning_content.or(delta.reasoning)
                    && !reasoning_content.is_empty()
                {
                    if !state.reasoning_item_started {
                        state.reasoning_item_started = true;
                        if tx_event
                            .send(Ok(ResponseEvent::OutputItemAdded(
                                ResponseItem::Reasoning {
                                    id: state.reasoning_item_id.clone(),
                                    summary: vec![],
                                    content: Some(vec![ReasoningItemContent::ReasoningText {
                                        text: String::new(),
                                    }]),
                                    encrypted_content: None,
                                },
                            )))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    state.reasoning_content.push_str(&reasoning_content);
                    if tx_event
                        .send(Ok(ResponseEvent::ReasoningContentDelta {
                            delta: reasoning_content,
                            content_index: 0,
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }

                if let Some(content) = delta.content {
                    let deltas = extract_text_deltas(&content);
                    if !deltas.is_empty() {
                        if !state.assistant_item_started {
                            state.assistant_item_started = true;
                            if tx_event
                                .send(Ok(ResponseEvent::OutputItemAdded(ResponseItem::Message {
                                    id: Some(state.message_item_id.clone()),
                                    role: "assistant".to_string(),
                                    content: vec![ContentItem::OutputText {
                                        text: String::new(),
                                    }],
                                    end_turn: None,
                                    phase: None,
                                })))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        for delta in deltas {
                            state.assistant_text.push_str(&delta);
                            if tx_event
                                .send(Ok(ResponseEvent::OutputTextDelta(delta)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }

                if let Some(tool_calls) = delta.tool_calls {
                    for tool_call in tool_calls {
                        // Providers that omit `index` (e.g. Groq's gpt-oss) are
                        // streaming a continuation of the most recent tool call.
                        let index = tool_call
                            .index
                            .unwrap_or_else(|| state.tool_calls.len().saturating_sub(1));
                        let starts_new_tool_call = index > state.finalized_tool_call_count
                            && tool_call.id.is_some()
                            && tool_call
                                .function
                                .as_ref()
                                .is_some_and(|function| function.name.is_some());
                        if starts_new_tool_call
                            && finalize_tool_calls_until(&tx_event, &mut state, &tool_kinds, index)
                                .await
                                .is_err()
                        {
                            return;
                        }
                        let partial = ensure_partial_tool_call(&mut state.tool_calls, index);
                        if let Some(id) = tool_call.id.filter(|id| !id.is_empty()) {
                            partial.id = Some(id);
                        }
                        if let Some(function) = tool_call.function {
                            if let Some(name) = function.name.filter(|name| !name.is_empty()) {
                                partial.name = Some(name);
                            }
                            if let Some(arguments) = function.arguments {
                                partial.arguments.push_str(&arguments);
                            }
                        }
                    }
                }
            }

            if let Some(finish_reason) = choice.finish_reason {
                match finish_reason.as_str() {
                    "tool_calls" => {
                        if finalize_reasoning(&tx_event, &mut state).await.is_err() {
                            return;
                        }
                        if finalize_assistant_message(&tx_event, &mut state)
                            .await
                            .is_err()
                        {
                            return;
                        }
                        if finalize_tool_calls(&tx_event, &mut state, &tool_kinds)
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    "stop" | "length" | "content_filter" => {}
                    _ => {}
                }
            }
        }
    }
}

fn extract_text_deltas(content: &Value) -> Vec<String> {
    let mut texts = match content {
        Value::String(text) => vec![text.clone()],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str).map(str::to_string))
            .collect(),
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .into_iter()
            .collect(),
        Value::Bool(_) | Value::Null | Value::Number(_) => Vec::new(),
    };
    // Ollama pairs every `reasoning` delta with an empty-string `content`;
    // empty fragments must not start an assistant message item.
    texts.retain(|text| !text.is_empty());
    texts
}

fn ensure_partial_tool_call(
    tool_calls: &mut Vec<PartialToolCall>,
    index: usize,
) -> &mut PartialToolCall {
    while tool_calls.len() <= index {
        tool_calls.push(PartialToolCall::default());
    }
    &mut tool_calls[index]
}

async fn finalize_and_complete(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    state: &mut StreamState,
    tool_kinds: &ToolKinds,
) -> Result<(), ApiError> {
    finalize_reasoning(tx_event, state).await?;

    finalize_assistant_message(tx_event, state).await?;

    finalize_tool_calls(tx_event, state, tool_kinds).await?;

    tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id: state.response_id.clone(),
            token_usage: state.usage.map(|usage| TokenUsage {
                input_tokens: usage.prompt_tokens.unwrap_or(0),
                cached_input_tokens: 0,
                output_tokens: usage.completion_tokens.unwrap_or(0),
                reasoning_output_tokens: 0,
                total_tokens: usage.total_tokens.unwrap_or_else(|| {
                    usage.prompt_tokens.unwrap_or(0) + usage.completion_tokens.unwrap_or(0)
                }),
            }),
        }))
        .await
        .map_err(|_| ApiError::Stream("chat stream channel closed".to_string()))?;
    Ok(())
}

async fn finalize_assistant_message(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    state: &mut StreamState,
) -> Result<(), ApiError> {
    if !state.assistant_item_started {
        return Ok(());
    }
    tx_event
        .send(Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
            id: Some(state.message_item_id.clone()),
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: state.assistant_text.clone(),
            }],
            end_turn: None,
            phase: None,
        })))
        .await
        .map_err(|_| ApiError::Stream("chat stream channel closed".to_string()))?;
    state.assistant_item_started = false;
    Ok(())
}

async fn finalize_reasoning(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    state: &mut StreamState,
) -> Result<(), ApiError> {
    if !state.reasoning_item_started {
        return Ok(());
    }

    tx_event
        .send(Ok(ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
            id: state.reasoning_item_id.clone(),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText {
                text: std::mem::take(&mut state.reasoning_content),
            }]),
            encrypted_content: None,
        })))
        .await
        .map_err(|_| ApiError::Stream("chat stream channel closed".to_string()))?;
    state.reasoning_item_started = false;
    Ok(())
}

async fn finalize_tool_calls(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    state: &mut StreamState,
    tool_kinds: &ToolKinds,
) -> Result<(), ApiError> {
    finalize_tool_calls_until(tx_event, state, tool_kinds, state.tool_calls.len()).await?;
    state.tool_calls.clear();
    state.finalized_tool_call_count = 0;
    Ok(())
}

async fn finalize_tool_calls_until(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    state: &mut StreamState,
    tool_kinds: &ToolKinds,
    end_index: usize,
) -> Result<(), ApiError> {
    while state.finalized_tool_call_count < end_index {
        let Some(slot) = state.tool_calls.get_mut(state.finalized_tool_call_count) else {
            return Err(ApiError::Stream(
                "tool call missing streamed index".to_string(),
            ));
        };
        let tool_call = std::mem::take(slot);
        let name = tool_call
            .name
            .ok_or_else(|| ApiError::Stream("tool call missing name".to_string()))?;
        let call_id = tool_call
            .id
            .unwrap_or_else(|| format!("call_{}", name.replace('.', "_")));
        let item = match tool_kinds.get(&name) {
            Some(ToolOutputKind::Function) => ResponseItem::FunctionCall {
                id: None,
                name,
                namespace: None,
                arguments: tool_call.arguments,
                call_id,
            },
            Some(ToolOutputKind::NamespacedFunction {
                name: output_name,
                namespace,
            }) => ResponseItem::FunctionCall {
                id: None,
                name: output_name.clone(),
                namespace: Some(namespace.clone()),
                arguments: tool_call.arguments,
                call_id,
            },
            Some(ToolOutputKind::Custom) => {
                let input = match serde_json::from_str::<Value>(&tool_call.arguments) {
                    Ok(value) => value
                        .get("input")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| tool_call.arguments.clone()),
                    Err(_) => tool_call.arguments.clone(),
                };
                ResponseItem::CustomToolCall {
                    id: None,
                    status: None,
                    call_id,
                    name,
                    input,
                }
            }
            None => ResponseItem::FunctionCall {
                id: None,
                name,
                namespace: None,
                arguments: tool_call.arguments,
                call_id,
            },
        };
        tx_event
            .send(Ok(ResponseEvent::OutputItemDone(item)))
            .await
            .map_err(|_| ApiError::Stream("chat stream channel closed".to_string()))?;
        state.finalized_tool_call_count += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn extract_text_deltas_supports_string_and_array_shapes() {
        assert_eq!(
            extract_text_deltas(&Value::String("hello".to_string())),
            vec!["hello".to_string()]
        );
        assert_eq!(
            extract_text_deltas(&serde_json::json!([
                { "text": "a" },
                { "text": "b" }
            ])),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[tokio::test]
    async fn spawn_chat_stream_reconstructs_fragmented_tool_calls() {
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-tool-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.2-codex\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,",
            "\"id\":\"call-shell-1\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":[\\\"/bin/echo\\\"\"}}]},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-tool-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.2-codex\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,",
            "\"function\":{\"arguments\":\",\\\"chat wire\\\"],\\\"timeout_ms\\\":1000}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-tool-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.2-codex\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut tool_kinds = HashMap::new();
        tool_kinds.insert("shell".to_string(), ToolOutputKind::Function);

        let mut stream = spawn_chat_stream(
            Box::pin(futures::stream::once(async move { Ok(sse.into()) })),
            Duration::from_secs(1),
            /*telemetry*/ None,
            tool_kinds,
        );

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("chat stream event"));
        }

        assert!(matches!(
            &events[0],
            ResponseEvent::ServerModel(model) if model == "gpt-5.2-codex"
        ));
        assert!(matches!(&events[1], ResponseEvent::Created));
        assert!(matches!(
            &events[2],
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                id: None,
                name,
                namespace: None,
                arguments,
                call_id,
            }) if name == "shell"
                && call_id == "call-shell-1"
                && arguments
                    == &json!({
                        "command": ["/bin/echo", "chat wire"],
                        "timeout_ms": 1_000,
                    })
                    .to_string()
        ));
        assert!(matches!(
            events[3],
            ResponseEvent::Completed {
                response_id: _,
                token_usage: None,
            }
        ));
    }

    #[tokio::test]
    async fn spawn_chat_stream_reconstructs_tool_calls_without_index() {
        // Regression for Groq's gpt-oss models, which omit `index` on tool-call
        // deltas. Before the fix this failed the whole stream with
        // "failed to parse chat completions chunk: missing field `index`".
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-groq-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"openai/gpt-oss-120b\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{",
            "\"id\":\"call-shell-1\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":[\\\"/bin/echo\\\"\"}}]},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-groq-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"openai/gpt-oss-120b\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{",
            "\"function\":{\"arguments\":\",\\\"groq\\\"],\\\"timeout_ms\\\":1000}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-groq-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"openai/gpt-oss-120b\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut tool_kinds = HashMap::new();
        tool_kinds.insert("shell".to_string(), ToolOutputKind::Function);

        let mut stream = spawn_chat_stream(
            Box::pin(futures::stream::once(async move { Ok(sse.into()) })),
            Duration::from_secs(1),
            /*telemetry*/ None,
            tool_kinds,
        );

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("chat stream event (must not error on missing index)"));
        }

        assert!(
            events.iter().any(|event| matches!(
                event,
                ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                    name,
                    arguments,
                    call_id,
                    ..
                }) if name == "shell"
                    && call_id == "call-shell-1"
                    && arguments
                        == &json!({
                            "command": ["/bin/echo", "groq"],
                            "timeout_ms": 1_000,
                        })
                        .to_string()
            )),
            "expected a reconstructed shell tool call, got: {events:?}"
        );
    }

    #[tokio::test]
    async fn spawn_chat_stream_maps_flat_namespace_tool_name_back_to_response_namespace() {
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-tool-namespace\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.2-codex\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,",
            "\"id\":\"call-lookup-1\",\"function\":{\"name\":\"mcp__demo__lookup_order\",",
            "\"arguments\":\"{\\\"order_id\\\":\\\"ord_123\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-tool-namespace\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-5.2-codex\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut tool_kinds = HashMap::new();
        tool_kinds.insert(
            "mcp__demo__lookup_order".to_string(),
            ToolOutputKind::NamespacedFunction {
                name: "lookup_order".to_string(),
                namespace: "mcp__demo__".to_string(),
            },
        );

        let mut stream = spawn_chat_stream(
            Box::pin(futures::stream::once(async move { Ok(sse.into()) })),
            Duration::from_secs(1),
            /*telemetry*/ None,
            tool_kinds,
        );

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("chat stream event"));
        }

        assert!(matches!(
            &events[2],
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                name,
                namespace: Some(namespace),
                arguments,
                call_id,
                ..
            }) if name == "lookup_order"
                && namespace == "mcp__demo__"
                && arguments == "{\"order_id\":\"ord_123\"}"
                && call_id == "call-lookup-1"
        ));
    }

    #[tokio::test]
    async fn spawn_chat_stream_finalizes_previous_tool_call_when_next_starts() {
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-tool-2\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"kimi-k2.5\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,",
            "\"id\":\"call-shell-1\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":[\\\"/bin/echo\\\",\\\"one\\\"]}\"}}]},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-tool-2\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"kimi-k2.5\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,",
            "\"id\":\"call-shell-2\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":[\\\"/bin/echo\\\",\\\"two\\\"]}\"}}]},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-tool-2\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"kimi-k2.5\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut tool_kinds = HashMap::new();
        tool_kinds.insert("shell".to_string(), ToolOutputKind::Function);

        let mut stream = spawn_chat_stream(
            Box::pin(futures::stream::once(async move { Ok(sse.into()) })),
            Duration::from_secs(1),
            /*telemetry*/ None,
            tool_kinds,
        );

        assert!(matches!(
            stream.next().await.expect("server model").expect("event"),
            ResponseEvent::ServerModel(_)
        ));
        assert!(matches!(
            stream.next().await.expect("created").expect("event"),
            ResponseEvent::Created
        ));
        assert!(matches!(
            stream
                .next()
                .await
                .expect("first tool should finalize before finish_reason")
                .expect("event"),
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { call_id, .. })
                if call_id == "call-shell-1"
        ));
        assert!(matches!(
            stream
                .next()
                .await
                .expect("second tool should finalize at finish_reason")
                .expect("event"),
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { call_id, .. })
                if call_id == "call-shell-2"
        ));
    }

    #[tokio::test]
    async fn spawn_chat_stream_preserves_reasoning_content() {
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-reasoning-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"deepseek-v4-pro\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"think \"},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-reasoning-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"deepseek-v4-pro\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"again\"},",
            "\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-reasoning-1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"deepseek-v4-pro\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},",
            "\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );

        let mut stream = spawn_chat_stream(
            Box::pin(futures::stream::once(async move { Ok(sse.into()) })),
            Duration::from_secs(1),
            /*telemetry*/ None,
            HashMap::new(),
        );

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("chat stream event"));
        }

        assert!(matches!(
            &events[2],
            ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. })
        ));
        assert!(matches!(
            &events[3],
            ResponseEvent::ReasoningContentDelta { delta, .. } if delta == "think "
        ));
        assert!(matches!(
            &events[4],
            ResponseEvent::ReasoningContentDelta { delta, .. } if delta == "again"
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                content: Some(content),
                ..
            }) if content == &vec![ReasoningItemContent::ReasoningText {
                text: "think again".to_string(),
            }]
        )));
    }

    #[tokio::test]
    async fn spawn_chat_stream_parses_ollama_reasoning_deltas() {
        // Ollama's OpenAI-compat endpoint streams parsed thinking as `reasoning`
        // and pairs it with an empty-string `content` on the same chunks
        // (observed live on Ollama 0.30.7 with qwen3.5). The empty content
        // fragments must not start an assistant message item.
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-323\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"qwen3.5:0.8b\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",",
            "\"content\":\"\",\"reasoning\":\"think \"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-323\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"qwen3.5:0.8b\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",",
            "\"content\":\"\",\"reasoning\":\"again\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-323\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"qwen3.5:0.8b\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",",
            "\"content\":\"hello\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-323\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"qwen3.5:0.8b\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",",
            "\"content\":\"\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );

        let mut stream = spawn_chat_stream(
            Box::pin(futures::stream::once(async move { Ok(sse.into()) })),
            Duration::from_secs(1),
            /*telemetry*/ None,
            HashMap::new(),
        );

        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("chat stream event"));
        }

        assert!(matches!(
            &events[2],
            ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. })
        ));
        assert!(matches!(
            &events[3],
            ResponseEvent::ReasoningContentDelta { delta, .. } if delta == "think "
        ));
        assert!(matches!(
            &events[4],
            ResponseEvent::ReasoningContentDelta { delta, .. } if delta == "again"
        ));
        assert!(matches!(
            &events[5],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(
            &events[6],
            ResponseEvent::OutputTextDelta(delta) if delta == "hello"
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. })
                if content == &vec![ContentItem::OutputText { text: "hello".to_string() }]
        )));
    }
}
