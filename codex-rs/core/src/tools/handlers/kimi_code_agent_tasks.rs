use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Duration;

use codex_protocol::ThreadId;
use codex_protocol::protocol::AgentStatus;
use uuid::Uuid;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::boxed_tool_output;

const OUTPUT_PREVIEW_BYTES: usize = 32 * 1_024;
const MAX_WAIT_SECONDS: u64 = 3_600;
static TASKS: LazyLock<Mutex<HashMap<String, AgentTask>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct AgentTask {
    task_id: String,
    thread_id: ThreadId,
    description: String,
    subagent_type: String,
    status: AgentTaskStatus,
    started_at: i64,
    ended_at: Option<i64>,
    output: String,
    stop_reason: Option<String>,
    output_path: PathBuf,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AgentTaskStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

impl AgentTaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }

    fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

pub(super) fn register_agent(
    codex_home: &Path,
    session_id: &str,
    thread_id: ThreadId,
    description: &str,
    subagent_type: &str,
) -> Result<String, FunctionCallError> {
    let task_id = format!("agent-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let output_path = codex_home
        .join("kimi-code")
        .join("tasks")
        .join(session_id)
        .join(&task_id)
        .join("output.log");
    let task = AgentTask {
        task_id: task_id.clone(),
        thread_id,
        description: description.to_string(),
        subagent_type: subagent_type.to_string(),
        status: AgentTaskStatus::Running,
        started_at: chrono::Utc::now().timestamp_millis(),
        ended_at: None,
        output: String::new(),
        stop_reason: None,
        output_path,
    };
    persist_output(&task)?;
    TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(task_id.clone(), task);
    let agent_id = thread_id;
    Ok(format!(
        "task_id: {task_id}\nstatus: running\nagent_id: {agent_id}\nactual_subagent_type: {subagent_type}\nautomatic_notification: true\n\ndescription: {description}\n\nnext_step: The completion arrives automatically in a later turn — do NOT wait, poll, or call TaskOutput on it; continue with other work or hand back to the user. (If you have nothing to do until it finishes, run such tasks in the foreground next time.)\nresume_hint: To continue or recover this same subagent later, call Agent(resume=\"{agent_id}\", prompt=\"...\"). The parameter is agent_id (\"{agent_id}\"), NOT task_id (\"{task_id}\") or source_id from a later <notification>. Recovery cases: a later <notification type=\"task.lost\" | \"task.failed\" | \"task.killed\"> for this subagent — its conversation history is preserved across session restarts and resume will pick it up."
    ))
}

pub(super) fn list(active_only: bool) -> Vec<(String, String)> {
    TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .filter(|task| !active_only || !task.status.is_terminal())
        .map(|task| (task.task_id.clone(), format_task_info(task)))
        .collect()
}

pub(super) async fn handle_output(
    invocation: ToolInvocation,
    task_id: &str,
    block: bool,
    timeout_seconds: u64,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let task = get_agent_task(task_id)?;
    if task.status == AgentTaskStatus::Running {
        let status = if block {
            tokio::time::timeout(
                Duration::from_secs(timeout_seconds.min(MAX_WAIT_SECONDS)),
                wait_for_final_status(&invocation, task.thread_id),
            )
            .await
            .ok()
            .flatten()
        } else {
            Some(
                invocation
                    .session
                    .services
                    .agent_control
                    .get_status(task.thread_id)
                    .await,
            )
        };
        if let Some(status) = status {
            update_status(task_id, status)?;
        }
    }
    let task = get_agent_task(task_id)?;
    format_output(&task, block)
}

pub(super) async fn handle_stop(
    invocation: ToolInvocation,
    task_id: &str,
    reason: Option<&str>,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let task = get_agent_task(task_id)?;
    if task.status.is_terminal() {
        return text_output(
            format!(
                "task_id: {}\nstatus: {}\nreason: {}",
                task.task_id,
                task.status.as_str(),
                task.stop_reason
                    .as_deref()
                    .unwrap_or("Task already in terminal state")
            ),
            /*success*/ true,
        );
    }
    let reason = reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("Stopped by TaskStop")
        .to_string();
    invocation
        .session
        .services
        .agent_control
        .interrupt_agent(task.thread_id)
        .await
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
    let mut tasks = TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let task = tasks
        .get_mut(task_id)
        .ok_or_else(|| FunctionCallError::RespondToModel(format!("Task not found: {task_id}")))?;
    task.status = AgentTaskStatus::Killed;
    task.ended_at = Some(chrono::Utc::now().timestamp_millis());
    task.stop_reason = Some(reason.clone());
    text_output(
        format!("task_id: {task_id}\nstatus: killed\nreason: {reason}"),
        /*success*/ true,
    )
}

async fn wait_for_final_status(
    invocation: &ToolInvocation,
    thread_id: ThreadId,
) -> Option<AgentStatus> {
    let control = &invocation.session.services.agent_control;
    let mut status_rx = control.subscribe_status(thread_id).await.ok()?;
    let mut status = status_rx.borrow().clone();
    while matches!(
        status,
        AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
    ) {
        if status_rx.changed().await.is_err() {
            return Some(control.get_status(thread_id).await);
        }
        status = status_rx.borrow().clone();
    }
    Some(status)
}

fn update_status(task_id: &str, status: AgentStatus) -> Result<(), FunctionCallError> {
    let mut tasks = TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let task = tasks
        .get_mut(task_id)
        .ok_or_else(|| FunctionCallError::RespondToModel(format!("Task not found: {task_id}")))?;
    match status {
        AgentStatus::Completed(message) => {
            task.status = AgentTaskStatus::Completed;
            task.output = message.unwrap_or_default();
            task.ended_at = Some(chrono::Utc::now().timestamp_millis());
        }
        AgentStatus::Errored(message) => {
            task.status = AgentTaskStatus::Failed;
            task.stop_reason = Some(message);
            task.ended_at = Some(chrono::Utc::now().timestamp_millis());
        }
        AgentStatus::Shutdown | AgentStatus::NotFound => {
            task.status = AgentTaskStatus::Failed;
            task.stop_reason = Some("Subagent is unavailable".to_string());
            task.ended_at = Some(chrono::Utc::now().timestamp_millis());
        }
        AgentStatus::Interrupted | AgentStatus::PendingInit | AgentStatus::Running => return Ok(()),
    }
    persist_output(task)
}

fn format_output(task: &AgentTask, block: bool) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let retrieval_status = if task.status.is_terminal() {
        "success"
    } else if block {
        "timeout"
    } else {
        "not_ready"
    };
    let (preview, truncated) = tail_bytes(&task.output, OUTPUT_PREVIEW_BYTES);
    let output_path = task.output_path.display();
    let full_output_hint = if truncated {
        format!(
            "Only the last {OUTPUT_PREVIEW_BYTES} bytes are shown above. Use the Read tool with the output_path to page through the full log (parameters: path, line_offset, n_lines; read about 300 lines per page)."
        )
    } else {
        "The preview above is the complete output. Use the Read tool with the output_path if you need to re-read the full log later (parameters: path, line_offset, n_lines; read about 300 lines per page).".to_string()
    };
    let mut lines = vec![
        format!("retrieval_status: {retrieval_status}"),
        format_task_info(task),
        format!("output_path: {output_path}"),
        format!("output_size_bytes: {}", task.output.len()),
        format!("output_preview_bytes: {}", preview.len()),
        format!("output_truncated: {truncated}"),
        "full_output_available: true".to_string(),
        "full_output_tool: Read".to_string(),
        format!("full_output_hint: {full_output_hint}"),
    ];
    if block && !task.status.is_terminal() {
        lines.push("next_step: The task is still running after waiting. Do not block on it again — continue with other work or hand back to the user; you will be notified automatically when it completes.".to_string());
    }
    lines.push(String::new());
    if truncated {
        lines.push(format!("[Truncated. Full output: {output_path}]"));
    }
    lines.extend([
        "[output]".to_string(),
        if preview.is_empty() {
            "[no output available]".to_string()
        } else {
            preview.to_string()
        },
    ]);
    text_output(lines.join("\n"), /*success*/ true)
}

fn format_task_info(task: &AgentTask) -> String {
    let mut lines = vec![
        format!("task_id: {}", task.task_id),
        format!("description: {}", task.description),
        format!("status: {}", task.status.as_str()),
        "detached: true".to_string(),
        format!("started_at: {}", task.started_at),
    ];
    if let Some(ended_at) = task.ended_at {
        lines.push(format!("ended_at: {ended_at}"));
    }
    if let Some(reason) = task.stop_reason.as_deref() {
        lines.push(format!("stop_reason: {reason}"));
    }
    lines.extend([
        "kind: agent".to_string(),
        format!("agent_id: {}", task.thread_id),
        format!("subagent_type: {}", task.subagent_type),
    ]);
    lines.join("\n")
}

fn get_agent_task(task_id: &str) -> Result<AgentTask, FunctionCallError> {
    TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(task_id)
        .cloned()
        .ok_or_else(|| FunctionCallError::RespondToModel(format!("Task not found: {task_id}")))
}

fn persist_output(task: &AgentTask) -> Result<(), FunctionCallError> {
    if let Some(parent) = task.output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to persist task output: {err}"))
        })?;
    }
    std::fs::write(&task.output_path, &task.output).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to persist task output: {err}"))
    })
}

fn tail_bytes(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    (&text[start..], true)
}

fn text_output(text: String, success: bool) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        text,
        Some(success),
    )))
}
