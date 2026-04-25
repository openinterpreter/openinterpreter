use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

pub fn spawn_anthropic_response_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(process_sse(
        stream_response.bytes,
        tx_event,
        idle_timeout,
        telemetry,
    ));
    ResponseStream { rx_event }
}

#[derive(Debug, Default)]
struct AnthropicStreamState {
    message_id: Option<String>,
    server_model: Option<String>,
    input_tokens: i64,
    cache_creation_input_tokens: i64,
    cache_read_input_tokens: i64,
    output_tokens: i64,
    pending_blocks: BTreeMap<i64, PendingBlock>,
}

#[derive(Debug)]
enum PendingBlock {
    Text {
        text: String,
    },
    Thinking {
        id: String,
        text: String,
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: Option<AnthropicMessageEnvelope>,
    #[serde(default)]
    index: Option<i64>,
    #[serde(default)]
    content_block: Option<AnthropicContentBlockStart>,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    #[serde(default)]
    error: Option<AnthropicError>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageEnvelope {
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<i64>,
    #[serde(default)]
    cache_read_input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStart {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct AnthropicDelta {
    #[serde(rename = "type")]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
    #[serde(default)]
    signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    #[serde(default)]
    message: Option<String>,
}

pub async fn process_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut state = AnthropicStreamState::default();

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(err))) => {
                debug!("Anthropic SSE error: {err:#}");
                let _ = tx_event.send(Err(ApiError::Stream(err.to_string()))).await;
                return;
            }
            Ok(None) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "stream closed before message_stop".to_string(),
                    )))
                    .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "idle timeout waiting for Anthropic SSE".to_string(),
                    )))
                    .await;
                return;
            }
        };

        if sse.data.is_empty() {
            continue;
        }
        trace!("Anthropic SSE event: {}", &sse.data);

        let event: AnthropicStreamEvent = match serde_json::from_str(&sse.data) {
            Ok(event) => event,
            Err(err) => {
                debug!(
                    "Failed to parse Anthropic SSE event: {err}, data: {}",
                    &sse.data
                );
                continue;
            }
        };

        match process_anthropic_event(event, &mut state, &tx_event).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => {
                let _ = tx_event.send(Err(err)).await;
                return;
            }
        }
    }
}

async fn process_anthropic_event(
    event: AnthropicStreamEvent,
    state: &mut AnthropicStreamState,
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
) -> Result<bool, ApiError> {
    match event.kind.as_str() {
        "message_start" => {
            if let Some(message) = event.message {
                state.message_id = Some(message.id.clone());
                state.server_model = message.model.clone();
                update_usage(state, message.usage.as_ref());
                if let Some(model) = message.model
                    && tx_event
                        .send(Ok(ResponseEvent::ServerModel(model)))
                        .await
                        .is_err()
                {
                    return Ok(true);
                }
                if tx_event.send(Ok(ResponseEvent::Created)).await.is_err() {
                    return Ok(true);
                }
            }
        }
        "content_block_start" => {
            let Some(index) = event.index else {
                return Ok(false);
            };
            let Some(block) = event.content_block else {
                return Ok(false);
            };
            match block.block_type.as_str() {
                "text" => {
                    state.pending_blocks.insert(
                        index,
                        PendingBlock::Text {
                            text: block.text.unwrap_or_default(),
                        },
                    );
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(ResponseItem::Message {
                            id: None,
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
                        return Ok(true);
                    }
                }
                "thinking" => {
                    state.pending_blocks.insert(
                        index,
                        PendingBlock::Thinking {
                            id: format!("anthropic-thinking-{index}"),
                            text: block.thinking.unwrap_or_default(),
                            signature: block.signature,
                        },
                    );
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(
                            ResponseItem::Reasoning {
                                id: format!("anthropic-thinking-{index}"),
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
                        return Ok(true);
                    }
                }
                "tool_use" => {
                    let input_json = match block.input {
                        Some(Value::Object(ref input)) if input.is_empty() => String::new(),
                        Some(input) => serde_json::to_string(&input).map_err(|err| {
                            ApiError::Stream(format!("serialize tool input: {err}"))
                        })?,
                        None => String::new(),
                    };
                    state.pending_blocks.insert(
                        index,
                        PendingBlock::ToolUse {
                            id: block.id.unwrap_or_else(|| format!("toolu_{index}")),
                            name: block.name.unwrap_or_default(),
                            input_json,
                        },
                    );
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let Some(index) = event.index else {
                return Ok(false);
            };
            let Some(delta) = event.delta else {
                return Ok(false);
            };
            match state.pending_blocks.get_mut(&index) {
                Some(PendingBlock::Text { text }) => {
                    if delta.delta_type == "text_delta" {
                        let delta_text = delta.text.unwrap_or_default();
                        text.push_str(&delta_text);
                        if tx_event
                            .send(Ok(ResponseEvent::OutputTextDelta(delta_text)))
                            .await
                            .is_err()
                        {
                            return Ok(true);
                        }
                    }
                }
                Some(PendingBlock::Thinking {
                    text, signature, ..
                }) => match delta.delta_type.as_str() {
                    "thinking_delta" => {
                        let delta_text = delta.thinking.unwrap_or_default();
                        text.push_str(&delta_text);
                        if tx_event
                            .send(Ok(ResponseEvent::ReasoningContentDelta {
                                delta: delta_text,
                                content_index: 0,
                            }))
                            .await
                            .is_err()
                        {
                            return Ok(true);
                        }
                    }
                    "signature_delta" => {
                        *signature = delta.signature;
                    }
                    _ => {}
                },
                Some(PendingBlock::ToolUse { input_json, .. }) => {
                    if delta.delta_type == "input_json_delta" {
                        input_json.push_str(delta.partial_json.as_deref().unwrap_or_default());
                    }
                }
                None => {}
            }
        }
        "content_block_stop" => {
            let Some(index) = event.index else {
                return Ok(false);
            };
            let Some(block) = state.pending_blocks.remove(&index) else {
                return Ok(false);
            };
            let item = match block {
                PendingBlock::Text { text } => ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText { text }],
                    end_turn: None,
                    phase: None,
                },
                PendingBlock::Thinking {
                    id,
                    text,
                    signature,
                } => ResponseItem::Reasoning {
                    id,
                    summary: vec![],
                    content: Some(vec![ReasoningItemContent::ReasoningText { text }]),
                    encrypted_content: signature,
                },
                PendingBlock::ToolUse {
                    id,
                    name,
                    input_json,
                } => ResponseItem::FunctionCall {
                    id: None,
                    name,
                    namespace: None,
                    arguments: normalize_tool_arguments(&input_json)?,
                    call_id: id,
                },
            };
            if tx_event
                .send(Ok(ResponseEvent::OutputItemDone(item)))
                .await
                .is_err()
            {
                return Ok(true);
            }
        }
        "message_delta" => {
            update_usage(state, event.usage.as_ref());
        }
        "message_stop" => {
            let token_usage = build_token_usage(state);
            let response_id = state.message_id.clone().unwrap_or_default();
            let completed = ResponseEvent::Completed {
                response_id,
                token_usage,
            };
            let _ = tx_event.send(Ok(completed)).await;
            return Ok(true);
        }
        "error" => {
            let message = event
                .error
                .and_then(|error| error.message)
                .unwrap_or_else(|| "Anthropic stream error".to_string());
            return Err(ApiError::Stream(message));
        }
        "ping" => {}
        other => {
            trace!("unhandled Anthropic event: {other}");
        }
    }

    Ok(false)
}

fn update_usage(state: &mut AnthropicStreamState, usage: Option<&AnthropicUsage>) {
    let Some(usage) = usage else {
        return;
    };
    if let Some(input_tokens) = usage.input_tokens {
        state.input_tokens = input_tokens;
    }
    if let Some(cache_creation_input_tokens) = usage.cache_creation_input_tokens {
        state.cache_creation_input_tokens = cache_creation_input_tokens;
    }
    if let Some(cache_read_input_tokens) = usage.cache_read_input_tokens {
        state.cache_read_input_tokens = cache_read_input_tokens;
    }
    if let Some(output_tokens) = usage.output_tokens {
        state.output_tokens = output_tokens;
    }
}

fn build_token_usage(state: &AnthropicStreamState) -> Option<TokenUsage> {
    let total_tokens = state
        .input_tokens
        .saturating_add(state.cache_creation_input_tokens)
        .saturating_add(state.cache_read_input_tokens)
        .saturating_add(state.output_tokens);

    if total_tokens == 0 {
        return None;
    }
    Some(TokenUsage {
        input_tokens: state.input_tokens,
        cached_input_tokens: state.cache_read_input_tokens,
        output_tokens: state.output_tokens,
        reasoning_output_tokens: 0,
        total_tokens,
    })
}

fn normalize_tool_arguments(input_json: &str) -> Result<String, ApiError> {
    if input_json.trim().is_empty() {
        return Ok("{}".to_string());
    }
    let parsed: Value = serde_json::from_str(input_json)
        .map_err(|err| ApiError::Stream(format!("parse tool_use input: {err}")))?;
    serde_json::to_string(&parsed)
        .map_err(|err| ApiError::Stream(format!("serialize tool_use input: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::stream;
    use http::HeaderMap;
    use pretty_assertions::assert_eq;

    async fn collect_events(chunks: &[&str]) -> Vec<Result<ResponseEvent, ApiError>> {
        let owned_chunks = chunks
            .iter()
            .map(|chunk| Bytes::from((*chunk).to_string()))
            .collect::<Vec<_>>();
        let stream = stream::iter(
            owned_chunks
                .into_iter()
                .map(Ok::<_, codex_client::TransportError>),
        );
        let (tx, mut rx) = mpsc::channel::<Result<ResponseEvent, ApiError>>(32);
        tokio::spawn(process_sse(
            Box::pin(stream),
            tx,
            Duration::from_secs(5),
            None,
        ));
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            let done = matches!(event, Ok(ResponseEvent::Completed { .. }) | Err(_));
            events.push(event);
            if done {
                break;
            }
        }
        events
    }

    #[tokio::test]
    async fn parses_text_stream_to_added_delta_done() {
        let events = collect_events(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":12}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"READY\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":3}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ])
        .await;

        assert_eq!(events.len(), 6);
        match events[0].as_ref().expect("server model") {
            ResponseEvent::ServerModel(model) => assert_eq!(model, "claude-sonnet"),
            other => panic!("expected server model event, got {other:?}"),
        }
        assert!(matches!(
            events[1].as_ref().expect("created"),
            ResponseEvent::Created
        ));
        match events[2].as_ref().expect("added") {
            ResponseEvent::OutputItemAdded(ResponseItem::Message { content, .. }) => {
                assert_eq!(
                    content,
                    &vec![ContentItem::OutputText {
                        text: String::new(),
                    }]
                );
            }
            other => panic!("expected output item added event, got {other:?}"),
        }
        match events[3].as_ref().expect("delta") {
            ResponseEvent::OutputTextDelta(delta) => assert_eq!(delta, "READY"),
            other => panic!("expected output text delta event, got {other:?}"),
        }
        match events[4].as_ref().expect("done") {
            ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. }) => {
                assert_eq!(
                    content,
                    &vec![ContentItem::OutputText {
                        text: "READY".to_string(),
                    }]
                );
            }
            other => panic!("expected output item done event, got {other:?}"),
        }
        match events[5].as_ref().expect("completed") {
            ResponseEvent::Completed {
                response_id,
                token_usage,
            } => {
                assert_eq!(response_id, "msg_1");
                assert_eq!(
                    token_usage,
                    &Some(TokenUsage {
                        input_tokens: 12,
                        cached_input_tokens: 0,
                        output_tokens: 3,
                        reasoning_output_tokens: 0,
                        total_tokens: 15,
                    })
                );
            }
            other => panic!("expected completed event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parses_tool_use_and_completion_usage() {
        let events = collect_events(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_2\",\"usage\":{\"input_tokens\":20}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Read\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"file_path\\\":\\\"/tmp/a.txt\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ])
        .await;

        assert!(matches!(
            events[0].as_ref().expect("created"),
            ResponseEvent::Created
        ));
        match events[1].as_ref().expect("done") {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                id,
                name,
                namespace,
                arguments,
                call_id,
            }) => {
                assert_eq!(id, &None);
                assert_eq!(name, "Read");
                assert_eq!(namespace, &None);
                assert_eq!(arguments, "{\"file_path\":\"/tmp/a.txt\"}");
                assert_eq!(call_id, "toolu_1");
            }
            other => panic!("expected function call output item done event, got {other:?}"),
        }
        match events[2].as_ref().expect("completed") {
            ResponseEvent::Completed {
                response_id,
                token_usage,
            } => {
                assert_eq!(response_id, "msg_2");
                assert_eq!(
                    token_usage,
                    &Some(TokenUsage {
                        input_tokens: 20,
                        cached_input_tokens: 0,
                        output_tokens: 7,
                        reasoning_output_tokens: 0,
                        total_tokens: 27,
                    })
                );
            }
            other => panic!("expected completed event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parses_tool_use_when_start_block_has_empty_input_object() {
        let events = collect_events(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_3\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\":\\\"printf \\\\\\\"ok\\\\\\\"\\\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\",\\\"description\\\":\\\"write output.txt\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ])
        .await;

        assert!(matches!(
            events[0].as_ref().expect("created"),
            ResponseEvent::Created
        ));
        match events[1].as_ref().expect("done") {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                id,
                name,
                namespace,
                arguments,
                call_id,
            }) => {
                assert_eq!(id, &None);
                assert_eq!(name, "Bash");
                assert_eq!(namespace, &None);
                assert_eq!(
                    arguments,
                    "{\"command\":\"printf \\\"ok\\\"\",\"description\":\"write output.txt\"}"
                );
                assert_eq!(call_id, "toolu_1");
            }
            other => panic!("expected function call output item done event, got {other:?}"),
        }
        assert!(matches!(
            events[2].as_ref().expect("completed"),
            ResponseEvent::Completed { response_id, token_usage }
            if response_id == "msg_3" && token_usage.is_none()
        ));
    }

    #[tokio::test]
    async fn parses_captured_edit_tool_use_stream() {
        let events = collect_events(&[r###"event: message_start
data: {"type":"message_start","message":{"model":"claude-sonnet-4-6","id":"msg_01D2KUE7AyuS2vMBQo8ZDZY5","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"stop_details":null,"usage":{"input_tokens":1,"cache_creation_input_tokens":287,"cache_read_input_tokens":4914,"cache_creation":{"ephemeral_5m_input_tokens":287,"ephemeral_1h_input_tokens":0},"output_tokens":63,"service_tier":"standard","inference_geo":"global"}} }

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01UwwxuyuKZjWYfuNKTXP4wn","name":"Edit","input":{},"caller":{"type":"direct"}}}

event: ping
data: {"type": "ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}       }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\""}        }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":": \"/tmp/cl"}         }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"aude-cor"}         }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"e-ga"}        }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"untlet"}   }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"-run-v"}   }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"6/tui-core-"}      }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"tool"}             }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"-gauntlet/"}         }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"workspace/"}         }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"gaunt"}     }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"let.txt\""}        }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":", \"old_stri"}    }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"ng\": \"T"}       }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"OKEN_OLD\""}         }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":", \"new_strin"}              }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"g\": \"TOKE"}     }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"N_NEW\""}        }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":", \"r"} }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"eplace"} }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"_all"}         }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\": t"}              }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"ru"}       }

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"e}"}  }

event: content_block_stop
data: {"type":"content_block_stop","index":0             }

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null,"stop_details":null},"usage":{"input_tokens":1,"cache_creation_input_tokens":287,"cache_read_input_tokens":4914,"output_tokens":147,"iterations":[{"input_tokens":1,"output_tokens":147,"cache_read_input_tokens":4914,"cache_creation_input_tokens":287,"cache_creation":{"ephemeral_5m_input_tokens":287,"ephemeral_1h_input_tokens":0},"type":"message"}]},"context_management":{"applied_edits":[]}           }

event: message_stop
data: {"type":"message_stop"   }

"###])
        .await;

        assert!(matches!(
            events[0].as_ref().expect("server model"),
            ResponseEvent::ServerModel(model) if model == "claude-sonnet-4-6"
        ));
        assert!(matches!(
            events[1].as_ref().expect("created"),
            ResponseEvent::Created
        ));
        match events[2].as_ref().expect("done") {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            }) => {
                assert_eq!(name, "Edit");
                assert_eq!(call_id, "toolu_01UwwxuyuKZjWYfuNKTXP4wn");
                assert_eq!(
                    arguments,
                    "{\"file_path\":\"/tmp/claude-core-gauntlet-run-v6/tui-core-tool-gauntlet/workspace/gauntlet.txt\",\"new_string\":\"TOKEN_NEW\",\"old_string\":\"TOKEN_OLD\",\"replace_all\":true}"
                );
            }
            other => panic!("expected function call output item done event, got {other:?}"),
        }
        assert!(matches!(
            events[3].as_ref().expect("completed"),
            ResponseEvent::Completed {
                response_id,
                token_usage: Some(TokenUsage {
                    input_tokens: 1,
                    cached_input_tokens: 4914,
                    output_tokens: 63,
                    reasoning_output_tokens: 0,
                    total_tokens: 5265,
                })
            } if response_id == "msg_01D2KUE7AyuS2vMBQo8ZDZY5"
        ));
    }

    #[tokio::test]
    async fn spawn_anthropic_stream_preserves_headers_independently() {
        let stream = StreamResponse {
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(stream::empty()),
        };
        let response_stream = spawn_anthropic_response_stream(stream, Duration::from_secs(5), None);
        assert_eq!(response_stream.rx_event.capacity(), 1600);
    }
}
