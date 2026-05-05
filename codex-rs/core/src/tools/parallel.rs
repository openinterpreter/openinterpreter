use std::sync::Arc;
use std::time::Instant;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::sync::RwLock;
use tokio_util::either::Either;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;
use tracing::Instrument;
use tracing::instrument;
use tracing::trace_span;

use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::AbortedToolOutput;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::context::ToolPayload;
use crate::tools::registry::AnyToolResult;
use crate::tools::registry::ToolArgumentDiffConsumer;
use crate::tools::router::ToolCall;
use crate::tools::router::ToolCallSource;
use crate::tools::router::ToolRouter;
use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem;
use codex_protocol::dynamic_tools::DynamicToolCallRequest;
use codex_protocol::error::CodexErr;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::DynamicToolCallResponseEvent;
use codex_protocol::protocol::EventMsg;
use codex_tools::ToolSpec;

#[derive(Clone)]
pub(crate) struct ToolCallRuntime {
    router: Arc<ToolRouter>,
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    tracker: SharedTurnDiffTracker,
    parallel_execution: Arc<RwLock<()>>,
}

impl ToolCallRuntime {
    pub(crate) fn new(
        router: Arc<ToolRouter>,
        session: Arc<Session>,
        turn_context: Arc<TurnContext>,
        tracker: SharedTurnDiffTracker,
    ) -> Self {
        Self {
            router,
            session,
            turn_context,
            tracker,
            parallel_execution: Arc::new(RwLock::new(())),
        }
    }

    pub(crate) fn find_spec(&self, tool_name: &codex_tools::ToolName) -> Option<ToolSpec> {
        self.router.find_spec(tool_name)
    }

    pub(crate) fn create_diff_consumer(
        &self,
        tool_name: &codex_tools::ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer>> {
        self.router.create_diff_consumer(tool_name)
    }

    #[instrument(level = "trace", skip_all)]
    pub(crate) fn handle_tool_call(
        self,
        call: ToolCall,
        cancellation_token: CancellationToken,
    ) -> impl std::future::Future<Output = Result<ResponseInputItem, CodexErr>> {
        let error_call = call.clone();
        let dynamic_call = visible_harness_function_tool_call(&call);
        let started = Instant::now();
        let event_session = Arc::clone(&self.session);
        let event_turn_context = Arc::clone(&self.turn_context);
        if let Some((tool, arguments)) = dynamic_call.clone() {
            let session = Arc::clone(&event_session);
            let turn_context = Arc::clone(&event_turn_context);
            let call_id = call.call_id.clone();
            tokio::spawn(async move {
                session
                    .send_event(
                        turn_context.as_ref(),
                        EventMsg::DynamicToolCallRequest(DynamicToolCallRequest {
                            call_id,
                            turn_id: turn_context.sub_id.clone(),
                            namespace: None,
                            tool,
                            arguments,
                        }),
                    )
                    .await;
            });
        }
        let future =
            self.handle_tool_call_with_source(call, ToolCallSource::Direct, cancellation_token);
        async move {
            match future.await {
                Ok(response) => {
                    let response_input = response.into_response();
                    emit_visible_harness_function_tool_response(
                        event_session.as_ref(),
                        event_turn_context.as_ref(),
                        &error_call,
                        dynamic_call,
                        &response_input,
                        started.elapsed(),
                        None,
                    )
                    .await;
                    Ok(response_input)
                }
                Err(FunctionCallError::Fatal(message)) => {
                    emit_visible_harness_function_tool_response(
                        event_session.as_ref(),
                        event_turn_context.as_ref(),
                        &error_call,
                        dynamic_call,
                        &Self::failure_response(
                            error_call.clone(),
                            FunctionCallError::Fatal(message.clone()),
                        ),
                        started.elapsed(),
                        Some(message.clone()),
                    )
                    .await;
                    Err(CodexErr::Fatal(message))
                }
                Err(other) => {
                    let error = other.to_string();
                    let response_input = Self::failure_response(error_call.clone(), other);
                    emit_visible_harness_function_tool_response(
                        event_session.as_ref(),
                        event_turn_context.as_ref(),
                        &error_call,
                        dynamic_call,
                        &response_input,
                        started.elapsed(),
                        Some(error),
                    )
                    .await;
                    Ok(response_input)
                }
            }
        }
        .in_current_span()
    }

    #[instrument(level = "trace", skip_all)]
    pub(crate) fn handle_tool_call_with_source(
        self,
        call: ToolCall,
        source: ToolCallSource,
        cancellation_token: CancellationToken,
    ) -> BoxFuture<'static, Result<AnyToolResult, FunctionCallError>> {
        let supports_parallel = self.router.tool_supports_parallel(&call);
        let router = Arc::clone(&self.router);
        let session = Arc::clone(&self.session);
        let turn = Arc::clone(&self.turn_context);
        let tracker = Arc::clone(&self.tracker);
        let lock = Arc::clone(&self.parallel_execution);
        let invocation_cancellation_token = cancellation_token.clone();
        let started = Instant::now();
        let display_name = call.tool_name.display();
        let defer_spawn_until_polled = turn.tools_config.harness.is_kimi_cli();

        let dispatch_span = trace_span!(
            "dispatch_tool_call_with_code_mode_result",
            otel.name = display_name.as_str(),
            tool_name = display_name.as_str(),
            call_id = call.call_id.as_str(),
            aborted = false,
        );

        let dispatch = async move {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    let secs = started.elapsed().as_secs_f32().max(0.1);
                    dispatch_span.record("aborted", true);
                    Ok(Self::aborted_response(&call, secs))
                },
                res = async {
                    let _guard = if supports_parallel {
                        Either::Left(lock.read().await)
                    } else {
                        Either::Right(lock.write().await)
                    };

                    router
                        .dispatch_tool_call_with_code_mode_result(
                            session,
                            turn,
                            invocation_cancellation_token,
                            tracker,
                            call.clone(),
                            source,
                        )
                        .instrument(dispatch_span.clone())
                        .await
                } => res,
            }
        };

        if defer_spawn_until_polled {
            return dispatch.in_current_span().boxed();
        }

        let handle: AbortOnDropHandle<Result<AnyToolResult, FunctionCallError>> =
            AbortOnDropHandle::new(tokio::spawn(dispatch));

        async move {
            handle.await.map_err(|err| {
                FunctionCallError::Fatal(format!("tool task failed to receive: {err:?}"))
            })?
        }
        .in_current_span()
        .boxed()
    }
}

fn visible_harness_function_tool_call(call: &ToolCall) -> Option<(String, serde_json::Value)> {
    let tool = call.tool_name.display();
    if !matches!(
        tool.as_str(),
        "Read"
            | "Write"
            | "Edit"
            | "Glob"
            | "Grep"
            | "TodoWrite"
            | "Agent"
            | "LSP"
            | "WebFetch"
            | "WebSearch"
            | "AskUserQuestion"
            | "CronCreate"
            | "CronDelete"
            | "CronList"
            | "ScheduleWakeup"
            | "ReadFile"
            | "WriteFile"
            | "ReadManyFiles"
            | "Shell"
    ) {
        return None;
    }
    let ToolPayload::Function { arguments } = &call.payload else {
        return None;
    };
    let arguments = serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::String(arguments.clone()));
    Some((tool, arguments))
}

async fn emit_visible_harness_function_tool_response(
    session: &Session,
    turn_context: &TurnContext,
    call: &ToolCall,
    dynamic_call: Option<(String, serde_json::Value)>,
    response_input: &ResponseInputItem,
    duration: std::time::Duration,
    error: Option<String>,
) {
    let Some((tool, arguments)) = dynamic_call else {
        return;
    };
    let content_items = dynamic_tool_content_items_from_response(response_input);
    let success = dynamic_tool_success_from_response(response_input).unwrap_or(error.is_none());
    session
        .send_event(
            turn_context,
            EventMsg::DynamicToolCallResponse(DynamicToolCallResponseEvent {
                call_id: call.call_id.clone(),
                turn_id: turn_context.sub_id.clone(),
                namespace: None,
                tool,
                arguments,
                content_items,
                success,
                error,
                duration,
            }),
        )
        .await;
}

fn dynamic_tool_content_items_from_response(
    response_input: &ResponseInputItem,
) -> Vec<DynamicToolCallOutputContentItem> {
    let ResponseInputItem::FunctionCallOutput { output, .. } = response_input else {
        return Vec::new();
    };
    match &output.body {
        FunctionCallOutputBody::Text(text) => {
            vec![DynamicToolCallOutputContentItem::InputText { text: text.clone() }]
        }
        FunctionCallOutputBody::ContentItems(items) => items
            .iter()
            .map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => {
                    DynamicToolCallOutputContentItem::InputText { text: text.clone() }
                }
                FunctionCallOutputContentItem::InputImage { image_url, .. } => {
                    DynamicToolCallOutputContentItem::InputImage {
                        image_url: image_url.clone(),
                    }
                }
            })
            .collect(),
    }
}

fn dynamic_tool_success_from_response(response_input: &ResponseInputItem) -> Option<bool> {
    match response_input {
        ResponseInputItem::FunctionCallOutput { output, .. } => output.success,
        _ => None,
    }
}

impl ToolCallRuntime {
    fn failure_response(call: ToolCall, err: FunctionCallError) -> ResponseInputItem {
        let message = err.to_string();
        match call.payload {
            ToolPayload::ToolSearch { .. } => ResponseInputItem::ToolSearchOutput {
                call_id: call.call_id,
                status: "completed".to_string(),
                execution: "client".to_string(),
                tools: Vec::new(),
            },
            ToolPayload::Custom { .. } => ResponseInputItem::CustomToolCallOutput {
                call_id: call.call_id,
                name: None,
                output: codex_protocol::models::FunctionCallOutputPayload {
                    body: codex_protocol::models::FunctionCallOutputBody::Text(message),
                    success: Some(false),
                },
            },
            _ => ResponseInputItem::FunctionCallOutput {
                call_id: call.call_id,
                output: codex_protocol::models::FunctionCallOutputPayload {
                    body: codex_protocol::models::FunctionCallOutputBody::Text(message),
                    success: Some(false),
                },
            },
        }
    }

    fn aborted_response(call: &ToolCall, secs: f32) -> AnyToolResult {
        AnyToolResult {
            call_id: call.call_id.clone(),
            payload: call.payload.clone(),
            result: Box::new(AbortedToolOutput {
                message: Self::abort_message(call, secs),
            }),
            post_tool_use_payload: None,
        }
    }

    fn abort_message(call: &ToolCall, secs: f32) -> String {
        if call.tool_name.namespace.is_none()
            && matches!(
                call.tool_name.name.as_str(),
                "shell" | "container.exec" | "local_shell" | "shell_command" | "unified_exec"
            )
        {
            format!("Wall time: {secs:.1} seconds\naborted by user")
        } else {
            format!("aborted by user after {secs:.1}s")
        }
    }
}
