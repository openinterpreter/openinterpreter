use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::WriteStdinHandler;
use crate::tools::handlers::parse_arguments;

const OUTPUT_PREVIEW_BYTES: usize = 32 * 1_024;
const PAGING_HINT_LINES: usize = 300;
const MIN_TASK_LIST_LIMIT: usize = 1;
const MAX_TASK_LIST_LIMIT: usize = 100;
const MAX_OUTPUT_TIMEOUT_SECONDS: u64 = 3_600;
const MILLISECONDS_PER_SECOND: u64 = 1_000;
const NONBLOCKING_YIELD_MILLISECONDS: u64 = 100;
static TASKS: LazyLock<Mutex<HashMap<String, ProcessTask>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct ProcessTask {
    task_id: String,
    process_id: i32,
    command: String,
    description: String,
    status: TaskStatus,
    started_at: i64,
    ended_at: Option<i64>,
    exit_code: Option<i64>,
    stop_reason: Option<String>,
    output: String,
    output_path: PathBuf,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TaskStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

impl TaskStatus {
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

#[derive(Deserialize)]
struct TaskOutputArgs {
    task_id: String,
    #[serde(default)]
    block: bool,
    #[serde(default = "default_output_timeout")]
    timeout: u64,
}

#[derive(Deserialize)]
struct TaskListArgs {
    #[serde(default = "default_active_only")]
    active_only: bool,
    #[serde(default = "default_list_limit")]
    limit: usize,
}

#[derive(Deserialize)]
struct TaskStopArgs {
    task_id: String,
    #[serde(default)]
    reason: Option<String>,
}

fn default_output_timeout() -> u64 {
    30
}

fn default_active_only() -> bool {
    true
}

fn default_list_limit() -> usize {
    20
}

pub(super) fn register_process(
    invocation: &ToolInvocation,
    process_id: i32,
    command: &str,
    description: &str,
    initial_output: &str,
) -> Result<String, FunctionCallError> {
    let task_id = format!("bash-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let output_path = invocation
        .turn
        .config
        .codex_home
        .as_path()
        .join("kimi-code")
        .join("tasks")
        .join(invocation.session.session_id().to_string())
        .join(&task_id)
        .join("output.log");
    let task = ProcessTask {
        task_id: task_id.clone(),
        process_id,
        command: command.to_string(),
        description: description.to_string(),
        status: TaskStatus::Running,
        started_at: chrono::Utc::now().timestamp_millis(),
        ended_at: None,
        exit_code: None,
        stop_reason: None,
        output: initial_output.to_string(),
        output_path,
    };
    persist_output(&task)?;
    TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(task_id.clone(), task);
    Ok(format!(
        "task_id: {task_id}\npid: {process_id}\ndescription: {description}\nstatus: running\nautomatic_notification: true\nnext_step: The completion arrives automatically in a later turn — do NOT wait, poll, or call TaskOutput on it; continue with your current work.\nnext_step: Use TaskStop only if the task must be cancelled.\nhuman_shell_hint: Tell the human to run /tasks to open the interactive background-task panel."
    ))
}

pub(super) async fn handle_list(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return model_error("TaskList received unsupported payload");
    };
    let args: TaskListArgs = parse_arguments(arguments)?;
    let tasks = TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut tasks = tasks
        .values()
        .filter(|task| !args.active_only || !task.status.is_terminal())
        .cloned()
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    tasks.truncate(args.limit.clamp(MIN_TASK_LIST_LIMIT, MAX_TASK_LIST_LIMIT));
    let label = if args.active_only {
        "active_background_tasks"
    } else {
        "background_tasks"
    };
    let output = if tasks.is_empty() {
        format!("{label}: 0\nNo background tasks found.")
    } else {
        let entries = tasks
            .iter()
            .map(format_task_info)
            .collect::<Vec<_>>()
            .join("\n---\n");
        format!("{label}: {}\n{entries}", tasks.len())
    };
    text_output(output, /*success*/ true)
}

pub(super) async fn handle_output(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return model_error("TaskOutput received unsupported payload");
    };
    let args: TaskOutputArgs = parse_arguments(arguments)?;
    let task = task(&args.task_id)?;
    if task.status == TaskStatus::Running {
        poll_process(&invocation, &task, args.block, args.timeout).await?;
    }
    let task = task(&args.task_id)?;
    let retrieval_status = if task.status.is_terminal() {
        "success"
    } else if args.block {
        "timeout"
    } else {
        "not_ready"
    };
    let output_size_bytes = task.output.len();
    let (preview, output_truncated) = tail_bytes(&task.output, OUTPUT_PREVIEW_BYTES);
    let output_preview_bytes = preview.len();
    let output_path = task.output_path.display();
    let full_output_hint = if output_truncated {
        format!(
            "Only the last {OUTPUT_PREVIEW_BYTES} bytes are shown above. Use the Read tool with the output_path to page through the full log (parameters: path, line_offset, n_lines; read about {PAGING_HINT_LINES} lines per page)."
        )
    } else {
        format!(
            "The preview above is the complete output. Use the Read tool with the output_path if you need to re-read the full log later (parameters: path, line_offset, n_lines; read about {PAGING_HINT_LINES} lines per page)."
        )
    };
    let mut lines = vec![
        format!("retrieval_status: {retrieval_status}"),
        format_task_info(&task),
    ];
    lines.extend([
        format!("output_path: {output_path}"),
        format!("output_size_bytes: {output_size_bytes}"),
        format!("output_preview_bytes: {output_preview_bytes}"),
        format!("output_truncated: {output_truncated}"),
        "full_output_available: true".to_string(),
        "full_output_tool: Read".to_string(),
        format!("full_output_hint: {full_output_hint}"),
    ]);
    if args.block && !task.status.is_terminal() {
        lines.push("next_step: The task is still running after waiting. Do not block on it again — continue with other work or hand back to the user; you will be notified automatically when it completes.".to_string());
    }
    lines.push(String::new());
    if output_truncated {
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

pub(super) async fn handle_stop(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return model_error("TaskStop received unsupported payload");
    };
    let args: TaskStopArgs = parse_arguments(arguments)?;
    let task = task(&args.task_id)?;
    if task.status.is_terminal() {
        let reason = task
            .stop_reason
            .as_deref()
            .unwrap_or("Task already in terminal state");
        return text_output(
            format!(
                "task_id: {}\nstatus: {}\nreason: {reason}",
                task.task_id,
                task.status.as_str()
            ),
            /*success*/ true,
        );
    }

    let reason = args
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("Stopped by TaskStop")
        .to_string();
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task.process_id,
            "chars": "\u{3}",
            "yield_time_ms": 1_000,
        })
        .to_string(),
    };
    let _ = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload,
            ..invocation
        })
        .await?;
    let mut tasks = TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let task = tasks.get_mut(&args.task_id).ok_or_else(|| {
        FunctionCallError::RespondToModel(format!("Task not found: {}", args.task_id))
    })?;
    task.status = TaskStatus::Killed;
    task.ended_at = Some(chrono::Utc::now().timestamp_millis());
    task.stop_reason = Some(reason.clone());
    text_output(
        format!(
            "task_id: {}\nstatus: killed\nreason: {reason}",
            task.task_id
        ),
        /*success*/ true,
    )
}

async fn poll_process(
    invocation: &ToolInvocation,
    task: &ProcessTask,
    block: bool,
    timeout: u64,
) -> Result<(), FunctionCallError> {
    let yield_time_ms = if block {
        timeout
            .min(MAX_OUTPUT_TIMEOUT_SECONDS)
            .saturating_mul(MILLISECONDS_PER_SECOND)
    } else {
        NONBLOCKING_YIELD_MILLISECONDS
    };
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task.process_id,
            "chars": "",
            "yield_time_ms": yield_time_ms,
        })
        .to_string(),
    };
    let output = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload: payload.clone(),
            ..invocation.clone()
        })
        .await?;
    let result = output.code_mode_result(&payload);
    let delta = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let still_running = result
        .get("session_id")
        .and_then(serde_json::Value::as_i64)
        .is_some();
    let exit_code = result.get("exit_code").and_then(serde_json::Value::as_i64);
    let mut tasks = TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let task = tasks.get_mut(&task.task_id).ok_or_else(|| {
        FunctionCallError::RespondToModel(format!("Task not found: {}", task.task_id))
    })?;
    task.output.push_str(delta);
    if !still_running {
        task.exit_code = exit_code;
        task.status = if exit_code == Some(0) {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
        task.ended_at = Some(chrono::Utc::now().timestamp_millis());
    }
    persist_output(task)
}

fn format_task_info(task: &ProcessTask) -> String {
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
    if let Some(stop_reason) = task.stop_reason.as_deref() {
        lines.push(format!("stop_reason: {stop_reason}"));
    }
    lines.extend([
        "kind: process".to_string(),
        format!("command: {}", task.command),
        format!("pid: {}", task.process_id),
    ]);
    if let Some(exit_code) = task.exit_code {
        lines.push(format!("exit_code: {exit_code}"));
    }
    lines.join("\n")
}

fn task(task_id: &str) -> Result<ProcessTask, FunctionCallError> {
    TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(task_id)
        .cloned()
        .ok_or_else(|| FunctionCallError::RespondToModel(format!("Task not found: {task_id}")))
}

fn persist_output(task: &ProcessTask) -> Result<(), FunctionCallError> {
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

fn model_error(message: &str) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    text_output(message.to_string(), /*success*/ false)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::ProcessTask;
    use super::TaskStatus;
    use super::format_task_info;
    use super::tail_bytes;

    #[test]
    fn formats_process_task_fields_in_provider_order() {
        let task = ProcessTask {
            task_id: "bash-abcd1234".to_string(),
            process_id: 42,
            command: "echo hi".to_string(),
            description: "Say hi".to_string(),
            status: TaskStatus::Completed,
            started_at: 100,
            ended_at: Some(200),
            exit_code: Some(0),
            stop_reason: None,
            output: "hi\n".to_string(),
            output_path: PathBuf::from("/tmp/output.log"),
        };

        assert_eq!(
            format_task_info(&task),
            "task_id: bash-abcd1234\ndescription: Say hi\nstatus: completed\ndetached: true\nstarted_at: 100\nended_at: 200\nkind: process\ncommand: echo hi\npid: 42\nexit_code: 0"
        );
    }

    #[test]
    fn takes_a_utf8_safe_tail() {
        let input = format!("é{}", "x".repeat(32_767));

        let (tail, truncated) = tail_bytes(&input, 32_768);

        assert!(truncated);
        assert_eq!(tail.len(), 32_767);
        assert!(tail.chars().all(|ch| ch == 'x'));
    }
}
