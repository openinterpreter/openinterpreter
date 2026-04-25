use crate::agent::AgentStatus;
use crate::agent::status::is_final;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::multi_agents_common::collab_agent_error;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::ThreadId;
use serde::Deserialize;
use serde::Serialize;
use tokio::time::Duration;
use tokio::time::timeout;

pub struct KimiTaskListHandler;
pub struct KimiTaskOutputHandler;
pub struct KimiTaskStopHandler;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KimiTaskListArgs {
    active_only: Option<bool>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KimiTaskOutputArgs {
    task_id: String,
    block: Option<bool>,
    timeout: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KimiTaskStopArgs {
    task_id: String,
    reason: Option<String>,
}

#[derive(Serialize)]
struct KimiTaskListEntry {
    task_id: String,
    status: String,
    description: Option<String>,
}

#[derive(Serialize)]
struct KimiTaskOutputResult {
    task_id: String,
    status: String,
    description: Option<String>,
    output: Option<String>,
    total_tokens: Option<i64>,
    tool_uses: Option<i64>,
    duration_ms: Option<i64>,
    wait_timed_out: bool,
}

#[derive(Serialize)]
struct KimiTaskStopResult {
    task_id: String,
    status: String,
    reason: String,
}

impl ToolHandler for KimiTaskListHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "TaskList received unsupported payload".to_string(),
            ));
        };
        let args: KimiTaskListArgs = parse_arguments(&arguments)?;
        let mut task_ids = session
            .services
            .agent_control
            .list_live_agent_subtree_thread_ids(session.conversation_id)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("TaskList failed: {err}")))?;
        task_ids.retain(|task_id| *task_id != session.conversation_id);
        task_ids.sort_by_key(ToString::to_string);

        let mut tasks = Vec::new();
        for task_id in task_ids {
            let status = session.services.agent_control.get_status(task_id).await;
            if args.active_only.unwrap_or(true) && is_final(&status) {
                continue;
            }
            let description = session
                .services
                .agent_control
                .get_agent_metadata(task_id)
                .and_then(|metadata| metadata.last_task_message);
            tasks.push(KimiTaskListEntry {
                task_id: task_id.to_string(),
                status: kimi_task_status(&status).to_string(),
                description,
            });
        }
        tasks.truncate(args.limit.unwrap_or(20));
        let body = serde_json::to_string(&tasks).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize TaskList output: {err}"))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}

impl ToolHandler for KimiTaskOutputHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "TaskOutput received unsupported payload".to_string(),
            ));
        };
        let args: KimiTaskOutputArgs = parse_arguments(&arguments)?;
        let task_id = parse_task_id(&args.task_id)?;
        let timeout_duration = Duration::from_secs(args.timeout.unwrap_or(30));
        let mut wait_timed_out = false;

        if args.block.unwrap_or(false) {
            let mut status_rx = session
                .services
                .agent_control
                .subscribe_status(task_id)
                .await
                .map_err(|err| collab_agent_error(task_id, err))?;
            let wait_result = timeout(timeout_duration, async {
                while !is_final(&status_rx.borrow().clone()) {
                    if status_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
            wait_timed_out = wait_result.is_err();
        }

        let status = session.services.agent_control.get_status(task_id).await;
        let description = session
            .services
            .agent_control
            .get_agent_metadata(task_id)
            .and_then(|metadata| metadata.last_task_message);
        let observable_usage = session
            .services
            .agent_control
            .get_observable_usage(task_id)
            .await;
        let total_tokens = session
            .services
            .agent_control
            .get_last_token_usage(task_id)
            .await
            .map(|usage| usage.total_tokens);
        let output = match &status {
            AgentStatus::Completed(message) => message.clone(),
            AgentStatus::Errored(message) => Some(message.clone()),
            AgentStatus::Interrupted => Some("Task interrupted.".to_string()),
            AgentStatus::Shutdown => Some("Task stopped.".to_string()),
            AgentStatus::NotFound => Some("Task not found.".to_string()),
            AgentStatus::PendingInit | AgentStatus::Running => None,
        };
        let result = KimiTaskOutputResult {
            task_id: task_id.to_string(),
            status: kimi_task_status(&status).to_string(),
            description,
            output,
            total_tokens,
            tool_uses: observable_usage.map(|usage| usage.tool_uses),
            duration_ms: observable_usage.and_then(|usage| usage.duration_ms),
            wait_timed_out,
        };
        let body = serde_json::to_string(&result).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize TaskOutput result: {err}"))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}

impl ToolHandler for KimiTaskStopHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "TaskStop received unsupported payload".to_string(),
            ));
        };
        let args: KimiTaskStopArgs = parse_arguments(&arguments)?;
        let task_id = parse_task_id(&args.task_id)?;
        session
            .services
            .agent_control
            .close_agent(task_id)
            .await
            .map_err(|err| collab_agent_error(task_id, err))?;
        let body = serde_json::to_string(&KimiTaskStopResult {
            task_id: task_id.to_string(),
            status: "stopped".to_string(),
            reason: args
                .reason
                .unwrap_or_else(|| "Stopped by TaskStop".to_string()),
        })
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize TaskStop result: {err}"))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}

fn parse_task_id(raw: &str) -> Result<ThreadId, FunctionCallError> {
    ThreadId::from_string(raw.trim()).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid task id `{raw}`: {err:?}"))
    })
}

fn kimi_task_status(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::PendingInit => "pending",
        AgentStatus::Running => "running",
        AgentStatus::Completed(_) => "completed",
        AgentStatus::Errored(_) => "errored",
        AgentStatus::Interrupted => "interrupted",
        AgentStatus::Shutdown => "stopped",
        AgentStatus::NotFound => "not_found",
    }
}
