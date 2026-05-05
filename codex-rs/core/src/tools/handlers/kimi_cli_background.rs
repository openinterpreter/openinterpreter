use crate::agent::AgentStatus;
use crate::agent::status::is_final;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::multi_agents_common::collab_agent_error;
use crate::tools::handlers::parse_kimi_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::unified_exec::UnifiedExecTaskSnapshot;
use crate::unified_exec::UnifiedExecTaskStatus;
use codex_protocol::ThreadId;
use serde::Deserialize;
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
        let args: KimiTaskListArgs = parse_kimi_arguments(&arguments)?;
        let active_only = args.active_only.unwrap_or(true);
        let mut entries = Vec::new();
        for task in session
            .services
            .unified_exec_manager
            .list_processes()
            .await
            .into_iter()
            .filter(|task| !active_only || matches!(task.status, UnifiedExecTaskStatus::Running))
        {
            let description = session
                .kimi_shell_task_description(task.process_id)
                .await
                .unwrap_or_else(|| task.description.clone());
            entries.push(format_shell_task(&task, &description, true));
        }

        let mut task_ids = session
            .services
            .agent_control
            .list_live_agent_subtree_thread_ids(session.conversation_id)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("TaskList failed: {err}")))?;
        task_ids.retain(|task_id| *task_id != session.conversation_id);
        task_ids.sort_by_key(ToString::to_string);

        for task_id in task_ids {
            let status = session.services.agent_control.get_status(task_id).await;
            if active_only && is_final(&status) {
                continue;
            }
            let description = session
                .services
                .agent_control
                .get_agent_metadata(task_id)
                .and_then(|metadata| metadata.last_task_message);
            entries.push(format_agent_task(task_id, &status, description.as_deref()));
        }
        entries.truncate(args.limit.unwrap_or(20));
        let header = if active_only {
            "active_background_tasks"
        } else {
            "background_tasks"
        };
        let body = if entries.is_empty() {
            format!("{header}: 0\n[no tasks]")
        } else {
            let mut lines = vec![format!("{header}: {}", entries.len()), String::new()];
            for (index, entry) in entries.into_iter().enumerate() {
                lines.push(format!("[{}]", index + 1));
                lines.push(entry);
                lines.push(String::new());
            }
            lines.join("\n").trim_end().to_string()
        };
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
        let args: KimiTaskOutputArgs = parse_kimi_arguments(&arguments)?;
        if let Some(process_id) = parse_shell_task_id(&args.task_id) {
            return handle_shell_task_output(session, args, process_id).await;
        }

        let task_id = parse_agent_task_id(&args.task_id)?;
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
        let body = format_agent_task_output(KimiAgentTaskOutputView {
            task_id,
            status: &status,
            description: description.as_deref(),
            output: output.as_deref(),
            total_tokens,
            tool_uses: observable_usage.map(|usage| usage.tool_uses),
            duration_ms: observable_usage.and_then(|usage| usage.duration_ms),
            wait_timed_out,
        });
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
        let args: KimiTaskStopArgs = parse_kimi_arguments(&arguments)?;
        if let Some(process_id) = parse_shell_task_id(&args.task_id) {
            let reason = args
                .reason
                .unwrap_or_else(|| "Stopped by TaskStop".to_string());
            session
                .services
                .unified_exec_manager
                .terminate_process(process_id)
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!("TaskStop failed: {err}"))
                })?;
            let description = session
                .kimi_shell_task_description(process_id)
                .await
                .unwrap_or_else(|| process_id.to_string());
            let body = [
                format!("task_id: {process_id}"),
                "kind: bash".to_string(),
                "status: killed".to_string(),
                format!("description: {description}"),
                format!("reason: {reason}"),
            ]
            .join("\n");
            return Ok(FunctionToolOutput::from_text(body, Some(true)));
        }

        let task_id = parse_agent_task_id(&args.task_id)?;
        let reason = args
            .reason
            .unwrap_or_else(|| "Stopped by TaskStop".to_string());
        session
            .services
            .agent_control
            .close_agent(task_id)
            .await
            .map_err(|err| collab_agent_error(task_id, err))?;
        let body = [
            format!("task_id: {task_id}"),
            "kind: agent".to_string(),
            "status: killed".to_string(),
            format!("reason: {reason}"),
        ]
        .join("\n");
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}

async fn handle_shell_task_output(
    session: std::sync::Arc<crate::session::session::Session>,
    args: KimiTaskOutputArgs,
    process_id: i32,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let timeout_ms = if args.block.unwrap_or(false) {
        args.timeout.unwrap_or(30).saturating_mul(1000)
    } else {
        0
    };
    let output = match session
        .services
        .unified_exec_manager
        .read_process_output(process_id, timeout_ms, None)
        .await
    {
        Ok(output) => output,
        Err(crate::unified_exec::UnifiedExecError::UnknownProcessId { .. }) => {
            return Ok(FunctionToolOutput::from_text(
                format!("Task not found: {process_id}"),
                Some(false),
            ));
        }
        Err(err) => {
            return Err(FunctionCallError::RespondToModel(format!(
                "TaskOutput failed: {err}"
            )));
        }
    };
    let status = if output.process_id.is_some() {
        "running"
    } else if output.exit_code == Some(0) {
        "completed"
    } else {
        "failed"
    };
    let text = String::from_utf8_lossy(&output.raw_output).to_string();
    let description = session
        .kimi_shell_task_description(process_id)
        .await
        .or_else(|| output.hook_command.clone())
        .unwrap_or_else(|| process_id.to_string());
    let body = format_shell_task_output(KimiShellTaskOutputView {
        task_id: process_id,
        status,
        description: &description,
        command: output.hook_command.as_deref(),
        output: &text,
        output_token_count: output
            .original_token_count
            .and_then(|tokens| i64::try_from(tokens).ok()),
        duration_ms: i64::try_from(output.wall_time.as_millis()).ok(),
        wait_timed_out: args.block.unwrap_or(false) && status == "running",
        exit_code: output.exit_code,
    });
    Ok(FunctionToolOutput::from_text(body, Some(true)))
}

fn parse_shell_task_id(raw: &str) -> Option<i32> {
    raw.trim().parse::<i32>().ok()
}

fn parse_agent_task_id(raw: &str) -> Result<ThreadId, FunctionCallError> {
    ThreadId::from_string(raw.trim()).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid task id `{raw}`: {err:?}"))
    })
}

fn kimi_shell_task_status(status: &UnifiedExecTaskStatus) -> &'static str {
    match status {
        UnifiedExecTaskStatus::Running => "running",
        UnifiedExecTaskStatus::Completed => "completed",
        UnifiedExecTaskStatus::Failed => "failed",
    }
}

fn kimi_task_status(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::PendingInit => "pending",
        AgentStatus::Running => "running",
        AgentStatus::Completed(_) => "completed",
        AgentStatus::Errored(_) => "failed",
        AgentStatus::Interrupted => "interrupted",
        AgentStatus::Shutdown => "killed",
        AgentStatus::NotFound => "not_found",
    }
}

fn format_shell_task(
    task: &UnifiedExecTaskSnapshot,
    description: &str,
    include_command: bool,
) -> String {
    let mut lines = vec![
        format!("task_id: {}", task.process_id),
        "kind: bash".to_string(),
        format!("status: {}", kimi_shell_task_status(&task.status)),
        format!("description: {description}"),
    ];
    if include_command {
        lines.push(format!("command: {}", task.description));
    }
    if let Some(exit_code) = task.exit_code {
        lines.push(format!("exit_code: {exit_code}"));
    }
    lines.join("\n")
}

fn format_agent_task(task_id: ThreadId, status: &AgentStatus, description: Option<&str>) -> String {
    let mut lines = vec![
        format!("task_id: {task_id}"),
        "kind: agent".to_string(),
        format!("status: {}", kimi_task_status(status)),
    ];
    if let Some(description) = description {
        lines.push(format!("description: {description}"));
    }
    lines.join("\n")
}

struct KimiShellTaskOutputView<'a> {
    task_id: i32,
    status: &'a str,
    description: &'a str,
    command: Option<&'a str>,
    output: &'a str,
    output_token_count: Option<i64>,
    duration_ms: Option<i64>,
    wait_timed_out: bool,
    exit_code: Option<i32>,
}

fn format_shell_task_output(view: KimiShellTaskOutputView<'_>) -> String {
    let retrieval_status = if view.wait_timed_out {
        "timeout"
    } else if view.status == "running" {
        "not_ready"
    } else {
        "success"
    };
    let output_size_bytes = view.output.len();
    let terminal_reason = view.status;
    let mut lines = vec![
        format!("retrieval_status: {retrieval_status}"),
        format!("task_id: {}", view.task_id),
        "kind: bash".to_string(),
        format!("status: {}", view.status),
        format!("description: {}", view.description),
    ];
    if let Some(command) = view.command {
        lines.push(format!("command: {command}"));
    }
    lines.push("interrupted: false".to_string());
    lines.push("timed_out: false".to_string());
    lines.push(format!("terminal_reason: {terminal_reason}"));
    if let Some(exit_code) = view.exit_code {
        lines.push(format!("exit_code: {exit_code}"));
    }
    if let Some(tokens) = view.output_token_count {
        lines.push(format!("total_tokens: {tokens}"));
    }
    if let Some(duration_ms) = view.duration_ms {
        lines.push(format!("duration_ms: {duration_ms}"));
    }
    lines.extend([
        String::new(),
        "output_path: ".to_string(),
        format!("output_size_bytes: {output_size_bytes}"),
        format!("output_preview_bytes: {output_size_bytes}"),
        "output_truncated: false".to_string(),
        String::new(),
        "full_output_available: false".to_string(),
        "full_output_tool: ReadFile".to_string(),
        "full_output_hint: No output file is currently available for this task.".to_string(),
        String::new(),
        "[output]".to_string(),
        if view.output.is_empty() {
            "[no output available]".to_string()
        } else {
            view.output.trim_end_matches('\n').to_string()
        },
    ]);
    lines.join("\n")
}

struct KimiAgentTaskOutputView<'a> {
    task_id: ThreadId,
    status: &'a AgentStatus,
    description: Option<&'a str>,
    output: Option<&'a str>,
    total_tokens: Option<i64>,
    tool_uses: Option<i64>,
    duration_ms: Option<i64>,
    wait_timed_out: bool,
}

fn format_agent_task_output(view: KimiAgentTaskOutputView<'_>) -> String {
    let status = kimi_task_status(view.status);
    let retrieval_status = if view.wait_timed_out {
        "timeout"
    } else if matches!(view.status, AgentStatus::PendingInit | AgentStatus::Running) {
        "not_ready"
    } else {
        "success"
    };
    let output = view.output.unwrap_or("[no output available]");
    let mut lines = vec![
        format!("retrieval_status: {retrieval_status}"),
        format!("task_id: {}", view.task_id),
        "kind: agent".to_string(),
        format!("status: {status}"),
    ];
    if let Some(description) = view.description {
        lines.push(format!("description: {description}"));
    }
    lines.push("interrupted: false".to_string());
    lines.push("timed_out: false".to_string());
    lines.push(format!("terminal_reason: {status}"));
    if let Some(total_tokens) = view.total_tokens {
        lines.push(format!("total_tokens: {total_tokens}"));
    }
    if let Some(tool_uses) = view.tool_uses {
        lines.push(format!("tool_uses: {tool_uses}"));
    }
    if let Some(duration_ms) = view.duration_ms {
        lines.push(format!("duration_ms: {duration_ms}"));
    }
    lines.extend([
        String::new(),
        "output_path: ".to_string(),
        format!("output_size_bytes: {}", output.len()),
        format!("output_preview_bytes: {}", output.len()),
        "output_truncated: false".to_string(),
        String::new(),
        "full_output_available: false".to_string(),
        "full_output_tool: ReadFile".to_string(),
        "full_output_hint: No output file is currently available for this task.".to_string(),
        String::new(),
        "[output]".to_string(),
        output.to_string(),
    ]);
    lines.join("\n")
}
