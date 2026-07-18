use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Instant;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_protocol::ThreadId;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::user_input::UserInput;
use codex_tools::AdditionalProperties;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use regex_lite::Regex;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

use crate::agent::control::SpawnAgentOptions;
use crate::agent::next_thread_spawn_depth;
use crate::agent::role::apply_role_to_config;
use crate::function_tool::FunctionCallError;
use crate::harness::opencode::OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS;
use crate::harness::zcode::ZCODE_COMPACTED_SUMMARY_PREFIX;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::ExecCommandHandler;
use crate::tools::handlers::ExecCommandHandlerOptions;
use crate::tools::handlers::RequestUserInputHandler;
use crate::tools::handlers::WriteStdinHandler;
use crate::tools::handlers::harness_fs;
use crate::tools::handlers::harness_fs::WalkEntryKind;
use crate::tools::handlers::multi_agents_common::apply_requested_spawn_agent_model_overrides;
use crate::tools::handlers::multi_agents_common::apply_spawn_agent_runtime_overrides;
use crate::tools::handlers::multi_agents_common::apply_spawn_agent_service_tier;
use crate::tools::handlers::multi_agents_common::build_agent_spawn_config;
use crate::tools::handlers::multi_agents_common::collab_spawn_error;
use crate::tools::handlers::multi_agents_common::parse_collab_input;
use crate::tools::handlers::multi_agents_common::thread_spawn_source;
use crate::tools::handlers::multi_agents_v2::SpawnAgentHandler as SpawnAgentHandlerV2;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;

pub(crate) const HARNESS_NO_TRUNCATE_PREFIX: &str = "<open-interpreter-harness-no-truncate>\n";

const DEFAULT_READ_LIMIT: usize = 1_000;
const ZCODE_READ_MAX_TOKENS: usize = 25_000;
const ZCODE_SKILL_CREATOR_VERSION: &str = "0.1.0";
const ZCODE_SKILL_CREATOR_MD: &str = include_str!("../../harness/zcode_skill_creator.md");
const ZCODE_CURRENT_FILE_HASH_CACHE_DIR: &str = ".zcode/oi-current-file-hashes";
const ZCODE_TODO_CACHE_DIR: &str = ".zcode/oi-todos";
static CLAUDE_TASKS: LazyLock<Mutex<HashMap<String, ClaudeTaskState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CLAUDE_READ_FILES: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static DEEPSEEK_CHECKLIST: LazyLock<Mutex<Vec<ChecklistItem>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static DEEPSEEK_READ_FILE_COUNTS: LazyLock<Mutex<HashMap<PathBuf, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ZCODE_TODOS: LazyLock<Mutex<Vec<ZCodeTodoItem>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static ZCODE_CURRENT_FILES: LazyLock<Mutex<HashMap<String, HashSet<PathBuf>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ZCODE_CURRENT_FILE_HASHES: LazyLock<Mutex<HashMap<String, HashMap<PathBuf, String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug)]
struct ClaudeTaskState {
    process_id: i32,
    command: String,
    description: String,
    output: String,
    output_path: String,
    tool_use_id: String,
}

#[derive(Clone, Copy)]
pub enum HarnessAliasHandler {
    Agent,
    Bash,
    BashLower,
    Read,
    ReadLower,
    ReadMediaFile,
    Write,
    WriteLower,
    Edit,
    EditLower,
    Glob,
    GlobLower,
    Grep,
    GrepLower,
    AskUserQuestion,
    TaskList,
    TaskOutput,
    TaskStop,
    ChecklistAdd,
    ChecklistList,
    ChecklistUpdate,
    ChecklistWrite,
    DeepSeekDiagnostics,
    DeepSeekApplyPatch,
    DeepSeekEditFile,
    DeepSeekExecShell,
    DeepSeekFileSearch,
    DeepSeekGitDiff,
    DeepSeekGitStatus,
    DeepSeekGrepFiles,
    DeepSeekListDir,
    DeepSeekReadFile,
    DeepSeekToolSearch,
    DeepSeekWriteFile,
    OpenCodeTask,
    OpenCodeTodoWrite,
    ZCodeTodoRead,
    ZCodeTodoWrite,
    ZCodeEnterPlanMode,
    ZCodeExitPlanMode,
    ZCodeReadSessionContext,
    ZCodeSkill,
}

impl ToolExecutor<ToolInvocation> for HarnessAliasHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(match self {
            Self::Agent => "Agent",
            Self::Bash => "Bash",
            Self::BashLower => "bash",
            Self::Read => "Read",
            Self::ReadLower => "read",
            Self::ReadMediaFile => "ReadMediaFile",
            Self::Write => "Write",
            Self::WriteLower => "write",
            Self::Edit => "Edit",
            Self::EditLower => "edit",
            Self::Glob => "Glob",
            Self::GlobLower => "glob",
            Self::Grep => "Grep",
            Self::GrepLower => "grep",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::TaskList => "TaskList",
            Self::TaskOutput => "TaskOutput",
            Self::TaskStop => "TaskStop",
            Self::ChecklistAdd => "checklist_add",
            Self::ChecklistList => "checklist_list",
            Self::ChecklistUpdate => "checklist_update",
            Self::ChecklistWrite => "checklist_write",
            Self::DeepSeekDiagnostics => "diagnostics",
            Self::DeepSeekApplyPatch => "apply_patch",
            Self::DeepSeekEditFile => "edit_file",
            Self::DeepSeekExecShell => "exec_shell",
            Self::DeepSeekFileSearch => "file_search",
            Self::DeepSeekGitDiff => "git_diff",
            Self::DeepSeekGitStatus => "git_status",
            Self::DeepSeekGrepFiles => "grep_files",
            Self::DeepSeekListDir => "list_dir",
            Self::DeepSeekReadFile => "read_file",
            Self::DeepSeekToolSearch => "tool_search_tool_bm25",
            Self::DeepSeekWriteFile => "write_file",
            Self::OpenCodeTask => "task",
            Self::OpenCodeTodoWrite => "todowrite",
            Self::ZCodeTodoRead => "TodoRead",
            Self::ZCodeTodoWrite => "TodoWrite",
            Self::ZCodeEnterPlanMode => "EnterPlanMode",
            Self::ZCodeExitPlanMode => "ExitPlanMode",
            Self::ZCodeReadSessionContext => "ReadSessionContext",
            Self::ZCodeSkill => "Skill",
        })
    }

    fn spec(&self) -> ToolSpec {
        harness_alias_spec(self.tool_name().name.as_str())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        !matches!(self, Self::Edit | Self::Write | Self::AskUserQuestion)
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(self.handle_call(invocation))
    }
}

impl HarnessAliasHandler {
    async fn handle_call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
        match self {
            Self::Agent => handle_agent(invocation).await,
            Self::Bash => handle_bash(invocation).await,
            Self::BashLower => handle_plain_bash(invocation).await,
            Self::Read | Self::ReadLower => handle_read(invocation).await,
            Self::ReadMediaFile => handle_read_media_file(invocation).await,
            Self::Write | Self::WriteLower => handle_write(invocation).await,
            Self::Edit | Self::EditLower => handle_edit(invocation).await,
            Self::Glob | Self::GlobLower => handle_glob(invocation).await,
            Self::Grep | Self::GrepLower => handle_grep(invocation).await,
            Self::AskUserQuestion => handle_ask_user_question(invocation).await,
            Self::TaskList => handle_task_list(invocation).await,
            Self::TaskOutput => handle_task_output(invocation).await,
            Self::TaskStop => handle_task_stop(invocation).await,
            Self::ChecklistAdd => handle_checklist_add(invocation).await,
            Self::ChecklistList => handle_checklist_list(invocation).await,
            Self::ChecklistUpdate => handle_checklist_update(invocation).await,
            Self::ChecklistWrite => handle_checklist_write(invocation).await,
            Self::DeepSeekDiagnostics => handle_deepseek_diagnostics(invocation).await,
            Self::DeepSeekApplyPatch => handle_deepseek_apply_patch(invocation).await,
            Self::DeepSeekEditFile => handle_deepseek_edit_file(invocation).await,
            Self::DeepSeekExecShell => handle_deepseek_exec_shell(invocation).await,
            Self::DeepSeekFileSearch => handle_deepseek_file_search(invocation).await,
            Self::DeepSeekGitDiff => handle_git_command(invocation, &["diff"]).await,
            Self::DeepSeekGitStatus => {
                handle_git_command(invocation, &["status", "--short", "--branch"]).await
            }
            Self::DeepSeekGrepFiles => handle_deepseek_grep_files(invocation).await,
            Self::DeepSeekListDir => handle_deepseek_list_dir(invocation).await,
            Self::DeepSeekReadFile => handle_deepseek_read_file(invocation).await,
            Self::DeepSeekToolSearch => handle_deepseek_tool_search(invocation).await,
            Self::DeepSeekWriteFile => handle_deepseek_write_file(invocation).await,
            Self::OpenCodeTask => handle_opencode_task(invocation).await,
            Self::OpenCodeTodoWrite => handle_opencode_todowrite(invocation).await,
            Self::ZCodeTodoRead => handle_zcode_todo_read(invocation).await,
            Self::ZCodeTodoWrite => handle_zcode_todo_write(invocation).await,
            Self::ZCodeEnterPlanMode => handle_zcode_enter_plan_mode(invocation).await,
            Self::ZCodeExitPlanMode => handle_zcode_exit_plan_mode(invocation).await,
            Self::ZCodeReadSessionContext => handle_zcode_read_session_context(invocation).await,
            Self::ZCodeSkill => handle_zcode_skill(invocation).await,
        }
    }
}

impl CoreToolRuntime for HarnessAliasHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Deserialize)]
struct ClaudeAgentArgs {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    run_in_background: bool,
    #[serde(default)]
    isolation: Option<String>,
}

#[derive(Deserialize)]
struct ZCodeReadSessionContextArgs {
    #[serde(rename = "sessionId")]
    session_id: String,
    query: String,
    #[serde(default = "default_zcode_read_session_context_strategy")]
    strategy: String,
    #[serde(
        default = "default_zcode_read_session_context_max_tokens",
        rename = "maxTokens"
    )]
    max_tokens: u32,
}

async fn handle_agent(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ClaudeAgentArgs = parse_arguments(arguments)?;
    let is_zcode = is_zcode(&invocation);
    if is_zcode {
        return handle_zcode_agent(invocation, args).await;
    }
    if is_kimi_code(&invocation) && !args.run_in_background {
        return handle_kimi_code_foreground_agent(invocation, args).await;
    }
    let fork_turns = "all";
    let task_name = args.description.clone();
    let mut translated = json!({
        "message": args.prompt,
        "task_name": task_name,
        "fork_turns": fork_turns,
    });
    if let Some(subagent_type) = args
        .subagent_type
        .as_deref()
        .map(str::trim)
        .filter(|subagent_type| !subagent_type.is_empty())
    {
        translated["agent_type"] = json!(subagent_type);
    }
    if let Some(model) = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        translated["model"] = json!(model);
    }
    let _ = args.run_in_background;
    let _ = args.isolation;

    SpawnAgentHandlerV2::default()
        .handle(ToolInvocation {
            tool_name: ToolName::plain("spawn_agent"),
            payload: ToolPayload::Function {
                arguments: translated.to_string(),
            },
            ..invocation
        })
        .await
}

async fn handle_kimi_code_foreground_agent(
    invocation: ToolInvocation,
    args: ClaudeAgentArgs,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        call_id,
        ..
    } = invocation;
    let role_name = args
        .subagent_type
        .as_deref()
        .map(str::trim)
        .filter(|subagent_type| !subagent_type.is_empty())
        .unwrap_or("coder");
    let task_name = zcode_task_name(&args.description);
    let child_depth = next_thread_spawn_depth(&turn.session_source);
    let mut config =
        build_agent_spawn_config(&session.get_base_instructions().await, turn.as_ref())?;
    apply_requested_spawn_agent_model_overrides(
        &session,
        turn.as_ref(),
        &mut config,
        args.model.as_deref(),
        /*requested_reasoning_effort*/ None,
    )
    .await?;
    apply_role_to_config(&mut config, Some(role_name))
        .await
        .map_err(FunctionCallError::RespondToModel)?;
    apply_spawn_agent_service_tier(
        &session,
        &mut config,
        turn.config.service_tier.as_deref(),
        /*requested_service_tier*/ None,
    )
    .await?;
    apply_spawn_agent_runtime_overrides(&mut config, turn.as_ref())?;

    let parent_thread_id = session.thread_id();
    let spawn_source = thread_spawn_source(
        parent_thread_id,
        &turn.session_source,
        child_depth,
        Some(role_name),
        Some(task_name),
    )?;
    let spawned = Box::pin(session.services.agent_control.spawn_agent_with_metadata(
        config,
        vec![UserInput::Text {
            text: args.prompt,
            text_elements: Vec::new(),
        }],
        Some(spawn_source),
        SpawnAgentOptions {
            fork_parent_spawn_call_id: Some(call_id),
            parent_thread_id: Some(parent_thread_id),
            environments: Some(turn.environments.to_selections()),
            ..Default::default()
        },
    ))
    .await
    .map_err(collab_spawn_error)?;
    let status = wait_for_agent_final_status(&session.services.agent_control, spawned.thread_id)
        .await
        .unwrap_or(spawned.status);
    let result = match status {
        AgentStatus::Completed(Some(message)) => message,
        AgentStatus::Completed(None) => String::new(),
        AgentStatus::Errored(message) => return Err(FunctionCallError::RespondToModel(message)),
        AgentStatus::Interrupted
        | AgentStatus::NotFound
        | AgentStatus::PendingInit
        | AgentStatus::Running
        | AgentStatus::Shutdown => {
            return Err(FunctionCallError::RespondToModel(format!(
                "Agent ended with status {status:?}"
            )));
        }
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "agent_id: {}\nactual_subagent_type: {role_name}\nstatus: completed\n\n[summary]\n{result}",
            spawned.thread_id
        ),
        Some(true),
    )))
}

async fn handle_zcode_agent(
    invocation: ToolInvocation,
    args: ClaudeAgentArgs,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        call_id,
        ..
    } = invocation;
    let role_name = args
        .subagent_type
        .as_deref()
        .map(str::trim)
        .filter(|subagent_type| !subagent_type.is_empty())
        .unwrap_or("Explore");
    let task_name = zcode_task_name(&args.description);
    let child_depth = next_thread_spawn_depth(&turn.session_source);
    let mut config =
        build_agent_spawn_config(&session.get_base_instructions().await, turn.as_ref())?;
    apply_requested_spawn_agent_model_overrides(
        &session,
        turn.as_ref(),
        &mut config,
        args.model.as_deref(),
        /*requested_reasoning_effort*/ None,
    )
    .await?;
    apply_role_to_config(&mut config, Some(role_name))
        .await
        .map_err(FunctionCallError::RespondToModel)?;
    apply_spawn_agent_service_tier(
        &session,
        &mut config,
        turn.config.service_tier.as_deref(),
        /*requested_service_tier*/ None,
    )
    .await?;
    apply_spawn_agent_runtime_overrides(&mut config, turn.as_ref())?;

    let parent_thread_id = session.thread_id();
    let spawn_source = thread_spawn_source(
        parent_thread_id,
        &turn.session_source,
        child_depth,
        Some(role_name),
        Some(task_name),
    )?;
    let started_at = Instant::now();
    let spawned = Box::pin(session.services.agent_control.spawn_agent_with_metadata(
        config,
        vec![UserInput::Text {
            text: args.prompt,
            text_elements: Vec::new(),
        }],
        Some(spawn_source),
        SpawnAgentOptions {
            fork_parent_spawn_call_id: Some(call_id),
            parent_thread_id: Some(parent_thread_id),
            environments: Some(turn.environments.to_selections()),
            ..Default::default()
        },
    ))
    .await
    .map_err(collab_spawn_error)?;
    let status = wait_for_agent_final_status(&session.services.agent_control, spawned.thread_id)
        .await
        .unwrap_or(spawned.status);
    let result = match status {
        AgentStatus::Completed(Some(message)) => message,
        AgentStatus::Completed(None) => String::new(),
        AgentStatus::Errored(message) => return Err(FunctionCallError::RespondToModel(message)),
        AgentStatus::Interrupted
        | AgentStatus::NotFound
        | AgentStatus::PendingInit
        | AgentStatus::Running
        | AgentStatus::Shutdown => {
            return Err(FunctionCallError::RespondToModel(format!(
                "Agent ended with status {status:?}"
            )));
        }
    };
    let duration_ms = i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX);
    let result = zcode_agent_result_with_footer(
        &session.services.agent_control,
        spawned.thread_id,
        result,
        duration_ms,
    )
    .await;

    let _ = args.run_in_background;
    let _ = args.isolation;
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        result,
        Some(true),
    )))
}

fn zcode_task_name(description: &str) -> String {
    let name = description
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('_');
    if name.is_empty() {
        "task".to_string()
    } else {
        name.to_string()
    }
}

#[derive(Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    run_in_background: bool,
    #[serde(default, rename = "dangerouslyDisableSandbox")]
    dangerously_disable_sandbox: bool,
}

async fn handle_bash(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: BashArgs = parse_arguments(arguments)?;
    let command = harness_fs::normalize_model_path_text(&args.command);
    let mut translated = json!({
        "cmd": command,
        "yield_time_ms": if args.run_in_background { 1_000 } else { args.timeout.unwrap_or(10_000).min(30_000) },
    });
    if args.run_in_background {
        translated["tty"] = json!(true);
    }
    if args.dangerously_disable_sandbox {
        translated["sandbox_permissions"] = json!("require_escalated");
    }

    let is_zcode = is_zcode(&invocation);
    if is_zcode {
        translated["max_output_tokens"] = json!(100_000);
    }

    let payload = ToolPayload::Function {
        arguments: translated.to_string(),
    };
    let payload_for_result = payload.clone();
    let output_path = args
        .run_in_background
        .then(|| claude_task_output_path(&invocation));
    let is_claude_code = is_claude_code(&invocation);
    let result = execute_harness_command(&invocation, payload, &payload_for_result).await?;
    if args.run_in_background
        && let Some(process_id) = result.get("session_id").and_then(serde_json::Value::as_i64)
    {
        let initial_output = result
            .get("output")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let task_id = claude_task_id(process_id as i32);
        let output_path = output_path
            .map(|template| template.replace("{task_id}", &task_id))
            .unwrap_or_default();
        store_claude_task(
            &task_id,
            process_id as i32,
            &command,
            args.description.as_deref().unwrap_or(&args.command),
            initial_output,
            &output_path,
            &invocation.call_id,
        );
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "Command running in background with ID: {task_id}. Output is being written to: {output_path}. You will be notified when it completes. To check interim output, use Read on that file path."
            ),
            Some(true),
        )));
    }
    let exit_code = result.get("exit_code").and_then(serde_json::Value::as_i64);
    let raw_output_key = if is_zcode { "raw_output" } else { "output" };
    let raw_output = result
        .get(raw_output_key)
        .or_else(|| result.get("output"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if is_zcode {
        let (harness_output, success) = zcode_bash_output(&invocation, raw_output, exit_code)?;
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!("{HARNESS_NO_TRUNCATE_PREFIX}{harness_output}"),
            success,
        )));
    }
    let is_deepseek_tui = is_deepseek_tui(&invocation);
    let harness_output = if exit_code != Some(0) && is_deepseek_tui {
        let stderr = raw_output.trim_end_matches('\n');
        format!("Command failed (exit code: {exit_code:?})\n\nSTDOUT:\n\n\nSTDERR:\n{stderr}")
    } else if exit_code == Some(0) && raw_output.is_empty() && is_deepseek_tui {
        "(no output)".to_string()
    } else if exit_code == Some(0) && raw_output.is_empty() && is_claude_code {
        "(Bash completed with no output)".to_string()
    } else if exit_code == Some(0) && raw_output.is_empty() {
        "Command executed successfully.".to_string()
    } else if is_deepseek_tui {
        compact_deepseek_exec_shell_output(raw_output)
    } else {
        raw_output.to_string()
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        harness_output,
        Some(is_deepseek_tui || exit_code == Some(0)),
    )))
}

async fn handle_plain_bash(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: BashArgs = parse_arguments(arguments)?;
    let command = harness_fs::normalize_model_path_text(&args.command);
    let payload = ToolPayload::Function {
        arguments: json!({
            "cmd": command,
            "yield_time_ms": args.timeout.unwrap_or(10_000).min(30_000),
        })
        .to_string(),
    };
    let payload_for_result = payload.clone();
    let result = execute_harness_command(&invocation, payload, &payload_for_result).await?;
    let mut text = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let exit_code = result
        .get("exit_code")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(1);
    if text.is_empty()
        && (is_pi(&invocation) || is_little_coder(&invocation) || is_opencode(&invocation))
    {
        text = "(no output)".to_string();
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("{HARNESS_NO_TRUNCATE_PREFIX}{text}"),
        Some(exit_code == 0),
    )))
}

async fn execute_harness_command(
    invocation: &ToolInvocation,
    payload: ToolPayload,
    payload_for_result: &ToolPayload,
) -> Result<serde_json::Value, FunctionCallError> {
    let handler = ExecCommandHandler::new(ExecCommandHandlerOptions {
        allow_login_shell: invocation.turn.config.permissions.allow_login_shell,
        exec_permission_approvals_enabled: invocation
            .session
            .features()
            .enabled(codex_features::Feature::ExecPermissionApprovals),
        include_environment_id: false,
        include_shell_parameter: false,
    });
    let output = handler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("exec_command"),
            payload,
            ..invocation.clone()
        })
        .await?;
    Ok(output.code_mode_result(payload_for_result))
}

fn is_claude_code(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| matches!(harness, "claude-code" | "claude-code-bare"))
}

fn is_kimi_code(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "kimi-code")
}

fn is_deepseek_tui(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "deepseek-tui")
}

fn compact_deepseek_exec_shell_output(raw_output: &str) -> String {
    const HEAD_CHARS: usize = 572;
    const TAIL_CHARS: usize = 286;
    const VISIBLE_ORIGINAL_CHARS: usize = 900;
    let raw_output = raw_output.strip_suffix('\n').unwrap_or(raw_output);
    let char_count = raw_output.chars().count();
    if char_count <= VISIBLE_ORIGINAL_CHARS {
        return raw_output.to_string();
    }
    let summary = raw_output.lines().take(3).collect::<Vec<_>>().join("\n");
    let head = raw_output.chars().take(HEAD_CHARS).collect::<String>();
    let tail = raw_output
        .chars()
        .skip(char_count.saturating_sub(TAIL_CHARS))
        .collect::<String>();
    format!(
        "[exec_shell output compacted to protect context]\nSummary: {summary}\nSnippet: {head}\n\n[... output truncated for context ...]\n\n{tail}\n(Original: {char_count} chars, omitted: {} chars.)",
        char_count.saturating_sub(VISIBLE_ORIGINAL_CHARS)
    )
}

fn zcode_bash_output(
    invocation: &ToolInvocation,
    raw_output: &str,
    exit_code: Option<i64>,
) -> Result<(String, Option<bool>), FunctionCallError> {
    const INLINE_SUCCESS_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

    let output = trim_zcode_bash_output(raw_output);
    if exit_code.is_some_and(|code| code != 0) {
        let output = defer_zcode_failure_marker_lines(output);
        let exit_code_text = exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let body = if output.is_empty() {
            format!("Exit code {exit_code_text}")
        } else {
            format!("Exit code {exit_code_text}\n{output}")
        };
        return Ok((body, Some(false)));
    }
    if output.is_empty() {
        return Ok(("(Bash completed with no output)".to_string(), None));
    }
    if output.len() <= INLINE_SUCCESS_OUTPUT_LIMIT_BYTES {
        return Ok((output.to_string(), None));
    }

    let artifact_path = zcode_persisted_output_path(invocation);
    if let Some(parent) = artifact_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| FunctionCallError::RespondToModel(format!("Bash failed: {err}")))?;
    }
    std::fs::write(&artifact_path, output)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Bash failed: {err}")))?;

    let size_kb = output.len() as f64 / 1024.0;
    let preview = zcode_preview_first_2kb(output);
    Ok((
        format!(
            "<persisted-output>\nOutput too large ({size_kb:.1}KB). Full output saved to: {}\n\nPreview (first 2KB):\n{preview}\n...\n</persisted-output>",
            artifact_path.display()
        ),
        None,
    ))
}

fn trim_zcode_bash_output(raw_output: &str) -> &str {
    let mut output = raw_output.trim_end();
    loop {
        if output.is_empty() {
            return output;
        }
        let Some(line_end) = output.find('\n') else {
            return if output.trim().is_empty() { "" } else { output };
        };
        let (line, rest) = output.split_at(line_end);
        if !line.trim().is_empty() {
            return output;
        }
        output = &rest['\n'.len_utf8()..];
    }
}

fn defer_zcode_failure_marker_lines(output: &str) -> String {
    let (output, leading_diagnostics) = split_zcode_leading_failure_diagnostics(output);
    let mut body = Vec::new();
    let mut markers = Vec::new();
    for line in output.lines() {
        if line.trim_start().starts_with('✗') {
            markers.push(line.to_string());
        } else {
            body.push(line.to_string());
        }
    }
    if markers.is_empty() {
        return output.to_string();
    }
    if let Some(first) = markers.first_mut() {
        *first = first.trim_start().to_string();
    }
    let markers_len = markers.len();
    let mut reordered = body.join("\n").trim_end().to_string();
    if !reordered.is_empty() {
        reordered.push('\n');
    }
    if leading_diagnostics.is_empty() {
        reordered.push_str(&markers.join("\n"));
    } else {
        let mut marker_lines = Vec::new();
        for (index, marker) in markers.into_iter().enumerate() {
            marker_lines.push(marker);
            if let Some(chunk) = leading_diagnostics.get(index) {
                marker_lines.extend(chunk.iter().cloned());
            }
        }
        for chunk in leading_diagnostics.iter().skip(markers_len) {
            marker_lines.extend(chunk.iter().cloned());
        }
        reordered.push_str(&marker_lines.join("\n"));
    }
    reordered
}

fn split_zcode_leading_failure_diagnostics(output: &str) -> (&str, Vec<Vec<String>>) {
    let Some(body_start) = output.find("\n\n") else {
        return (output, Vec::new());
    };
    let leading = &output[..body_start];
    if leading.is_empty() {
        return (output, Vec::new());
    }
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    for line in leading.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("Error:") && !current.is_empty() {
            chunks.push(current);
            current = Vec::new();
        }
        if !(trimmed.starts_with("Error:") || trimmed.starts_with("at ")) {
            return (output, Vec::new());
        }
        current.push(line.to_string());
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        (output, Vec::new())
    } else {
        (&output[body_start + 2..], chunks)
    }
}

fn zcode_persisted_output_path(invocation: &ToolInvocation) -> PathBuf {
    invocation
        .turn
        .config
        .codex_home
        .as_path()
        .join(".zcode")
        .join("cli")
        .join("artifacts")
        .join(format!("sess_{}", invocation.session.session_id()))
        .join(format!(
            "{}-tool-result-{}.json",
            invocation.call_id,
            Uuid::new_v4()
        ))
}

fn zcode_maybe_persist_large_grep_output(
    invocation: &ToolInvocation,
    output: &str,
) -> Result<String, FunctionCallError> {
    const INLINE_GREP_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;
    if output.len() <= INLINE_GREP_OUTPUT_LIMIT_BYTES {
        return Ok(output.to_string());
    }
    let artifact_path = zcode_persisted_output_path(invocation);
    if let Some(parent) = artifact_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
    }
    std::fs::write(&artifact_path, output)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;

    let size_kb = zcode_format_decimal_kb(output.len());
    let preview = zcode_preview_first_2kb(output);
    Ok(format!(
        "<persisted-output>\nOutput too large ({size_kb} KB). Full output saved to: {}\n\nPreview (first 2 KB):\n{preview}\n...\n</persisted-output>",
        artifact_path.display()
    ))
}

fn zcode_format_decimal_kb(bytes: usize) -> usize {
    ((bytes as f64) / 1_000.0).round() as usize
}

fn zcode_preview_first_2kb(output: &str) -> String {
    let mut preview = String::new();
    for line in output.split_inclusive('\n') {
        let next_len = preview.len() + line.len();
        if next_len > 2_000 {
            break;
        }
        preview.push_str(line);
    }
    preview.trim_end().to_string()
}

fn is_claude_code_bare(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "claude-code-bare")
}

fn claude_task_id(process_id: i32) -> String {
    std::env::var("OPEN_INTERPRETER_CLAUDE_CODE_TASK_ID_OVERRIDE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| process_id.to_string())
}

fn store_claude_task(
    task_id: &str,
    process_id: i32,
    command: &str,
    description: &str,
    initial_output: &str,
    output_path: &str,
    tool_use_id: &str,
) {
    if let Ok(mut tasks) = CLAUDE_TASKS.lock() {
        tasks.insert(
            task_id.to_string(),
            ClaudeTaskState {
                process_id,
                command: command.to_string(),
                description: description.to_string(),
                output: normalize_task_output(initial_output),
                output_path: output_path.to_string(),
                tool_use_id: tool_use_id.to_string(),
            },
        );
    }
}

fn claude_task_output_path(invocation: &ToolInvocation) -> String {
    let cwd_key = harness_fs::primary_cwd(invocation)
        .display()
        .to_string()
        .replace('/', "-");
    let session_id = invocation.session.session_id().to_string();
    format!("/private/tmp/claude-501/{cwd_key}/{session_id}/tasks/{{task_id}}.output")
}

fn claude_task_state(task_id: &str) -> Result<ClaudeTaskState, FunctionCallError> {
    if let Ok(process_id) = task_id.parse::<i32>() {
        return Ok(ClaudeTaskState {
            process_id,
            command: String::new(),
            description: String::new(),
            output: String::new(),
            output_path: String::new(),
            tool_use_id: String::new(),
        });
    }
    CLAUDE_TASKS
        .lock()
        .ok()
        .and_then(|tasks| tasks.get(task_id).cloned())
        .ok_or_else(|| FunctionCallError::RespondToModel(format!("Unknown task id {task_id}")))
}

pub(crate) fn take_claude_code_bare_completed_task_notification() -> Option<ResponseItem> {
    let mut tasks = CLAUDE_TASKS.lock().ok()?;
    let task_id = tasks.keys().next().cloned()?;
    let task = tasks.remove(&task_id)?;
    let text = format!(
        "<task-notification>\n<task-id>{task_id}</task-id>\n<tool-use-id>{}</tool-use-id>\n<output-file>{}</output-file>\n<status>completed</status>\n<summary>Background command \"{}\" completed (exit code 0)</summary>\n</task-notification>",
        task.tool_use_id, task.output_path, task.description
    );
    Some(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text }],
        phase: Some(MessagePhase::Commentary),
        internal_chat_message_metadata_passthrough: None,
    })
}

fn update_claude_task_output(task_id: &str, output_delta: &str) -> String {
    let output_delta = normalize_task_output(output_delta);
    if let Ok(mut tasks) = CLAUDE_TASKS.lock()
        && let Some(task) = tasks.get_mut(task_id)
    {
        task.output.push_str(&output_delta);
        return task.output.clone();
    }
    output_delta
}

fn normalize_task_output(output: &str) -> String {
    output.replace("\r\n", "\n")
}

#[derive(Deserialize)]
struct TaskOutputArgs {
    task_id: String,
    block: Option<bool>,
    timeout: Option<u64>,
}

#[derive(Deserialize)]
struct TaskListArgs {
    #[serde(default = "default_task_list_active_only")]
    active_only: bool,
    #[serde(default = "default_task_list_limit")]
    limit: usize,
}

fn default_task_list_active_only() -> bool {
    true
}

fn default_task_list_limit() -> usize {
    20
}

async fn handle_task_list(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: TaskListArgs = parse_arguments(arguments)?;
    let tasks = CLAUDE_TASKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut entries = tasks.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(task_id, _)| *task_id);
    entries.truncate(args.limit.clamp(1, 100));
    let mut lines = vec![format!("background_tasks: {}", entries.len())];
    for (task_id, task) in entries {
        lines.extend([
            format!("task_id: {task_id}"),
            format!("description: {}", task.description),
            "status: running".to_string(),
            "detached: true".to_string(),
            "kind: process".to_string(),
            format!("command: {}", task.command),
            format!("pid: {}", task.process_id),
        ]);
    }
    let _ = args.active_only;
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        lines.join("\n"),
        Some(true),
    )))
}

async fn handle_task_output(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: TaskOutputArgs = parse_arguments(arguments)?;
    let task_state = claude_task_state(&args.task_id)?;
    let kimi_code = is_kimi_code(&invocation);
    let block = args.block.unwrap_or(!kimi_code);
    let timeout = args.timeout.unwrap_or(if kimi_code { 30 } else { 30_000 });
    let timeout_ms = if kimi_code {
        timeout.saturating_mul(1_000)
    } else {
        timeout
    };
    let yield_time_ms = if block { timeout_ms } else { 100 };
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task_state.process_id,
            "chars": "",
            "yield_time_ms": yield_time_ms,
        })
        .to_string(),
    };
    let payload_for_result = payload.clone();
    let output = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload,
            ..invocation
        })
        .await?;
    let result = output.code_mode_result(&payload_for_result);
    let raw_output = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let output = update_claude_task_output(&args.task_id, raw_output);
    let status = if result
        .get("session_id")
        .and_then(serde_json::Value::as_i64)
        .is_some()
    {
        "running"
    } else {
        "completed"
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "<retrieval_status>{}</retrieval_status>\n\n<task_id>{}</task_id>\n\n<task_type>local_bash</task_type>\n\n<status>{status}</status>\n\n<output>\n{output}</output>",
            if status == "running" {
                "not_ready"
            } else {
                "complete"
            },
            args.task_id,
        ),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct TaskStopArgs {
    task_id: String,
}

async fn handle_task_stop(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: TaskStopArgs = parse_arguments(arguments)?;
    let task_state = claude_task_state(&args.task_id)?;
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task_state.process_id,
            "chars": "\u{3}",
            "yield_time_ms": 1_000,
        })
        .to_string(),
    };
    let output = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload,
            ..invocation
        })
        .await?;
    let _ = output;
    let message = format!(
        "Successfully stopped task: {} ({})",
        args.task_id, task_state.command
    );
    let message = serde_json::to_string(&message).unwrap_or_else(|_| "\"\"".to_string());
    let task_id = serde_json::to_string(&args.task_id).unwrap_or_else(|_| "\"\"".to_string());
    let command = serde_json::to_string(&task_state.command).unwrap_or_else(|_| "\"\"".to_string());
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "{{\"message\":{message},\"task_id\":{task_id},\"task_type\":\"local_bash\",\"command\":{command}}}"
        ),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct ReadArgs {
    #[serde(alias = "file_path", alias = "filePath")]
    path: String,
    #[serde(default, alias = "offset")]
    line_offset: Option<usize>,
    #[serde(default, alias = "limit")]
    n_lines: Option<usize>,
}

async fn handle_read(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ReadArgs = parse_arguments(arguments)?;
    let path = harness_fs::checked_read_path(&invocation, &args.path, "Read")?;
    if let Some(task_id) = claude_task_id_from_output_path(&path) {
        let output = poll_claude_task_output(&invocation, task_id).await?;
        let (body, _, _, _) = numbered_read_lines(&output, /*offset*/ 1, DEFAULT_READ_LIMIT);
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )));
    }
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(err) if is_zcode(&invocation) && err.kind() == std::io::ErrorKind::NotFound => {
            let cwd = harness_fs::primary_cwd(&invocation);
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                format!(
                    "{HARNESS_NO_TRUNCATE_PREFIX}File does not exist. Note: your current working directory is {}.",
                    cwd.display()
                ),
                Some(false),
            )));
        }
        Err(err) => {
            return Err(FunctionCallError::RespondToModel(format!(
                "Read failed: {err}"
            )));
        }
    };
    if let Some(mime_type) = image_mime_type(&path) {
        let image_url = format!("data:{mime_type};base64,{}", BASE64_STANDARD.encode(data));
        record_claude_read_file(&path);
        return Ok(boxed_tool_output(FunctionToolOutput::from_content(
            vec![FunctionCallOutputContentItem::InputImage {
                image_url,
                detail: None,
            }],
            Some(true),
        )));
    }
    let text = String::from_utf8(data).map_err(|_| {
        FunctionCallError::RespondToModel("Read failed: file is not UTF-8 text".to_string())
    })?;
    if invocation.tool_name.name == "read" {
        if is_opencode(&invocation) {
            let total_lines = text.lines().count().max(1);
            let body = text
                .lines()
                .enumerate()
                .map(|(index, line)| format!("{}: {line}", index + 1))
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                format!(
                    "<path>{}</path>\n<type>file</type>\n<content>\n{body}\n\n(End of file - total {total_lines} lines)\n</content>",
                    path.display()
                ),
                Some(true),
            )));
        }
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            text,
            Some(true),
        )));
    }
    let zcode_zero_offset = is_zcode(&invocation) && args.line_offset == Some(0);
    let offset = if zcode_zero_offset {
        1
    } else {
        args.line_offset.unwrap_or(1).max(1)
    };
    let limit = if is_zcode(&invocation) {
        args.n_lines.unwrap_or(usize::MAX)
    } else {
        args.n_lines
            .unwrap_or(DEFAULT_READ_LIMIT)
            .min(DEFAULT_READ_LIMIT)
    };
    let display_offset = if zcode_zero_offset { 0 } else { offset };
    let (body, lines_read, total_lines, end_reached) =
        numbered_read_lines_from(&text, offset, limit, display_offset);
    record_claude_read_file(&path);
    if is_claude_code_bare(&invocation) {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )));
    }
    if is_zcode(&invocation) {
        let file_hash = zcode_file_hash(text.as_bytes());
        if args.line_offset.is_none()
            && args.n_lines.is_none()
            && (zcode_current_file_hash_applies(&invocation, &path, &file_hash).await
                || zcode_history_has_current_file_state(&invocation, &path, &file_hash).await)
        {
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                "Wasted call — file unchanged since your last Read. Refer to that earlier tool_result instead.".to_string(),
                Some(true),
            )));
        }
        record_zcode_current_file(&invocation, &path);
        record_zcode_current_file_hash(&invocation, &path, file_hash);
        let selected_text = selected_read_text(&text, offset, limit);
        let token_count = zcode_estimate_tokens(&selected_text);
        if token_count > ZCODE_READ_MAX_TOKENS
            && (args.line_offset.is_some() || args.n_lines.is_some())
        {
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                format!(
                    "{HARNESS_NO_TRUNCATE_PREFIX}{}",
                    zcode_read_token_budget_error_text(token_count)
                ),
                Some(false),
            )));
        }
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!("{HARNESS_NO_TRUNCATE_PREFIX}{body}"),
            Some(true),
        )));
    }
    let end_summary = if end_reached {
        " End of file reached."
    } else {
        ""
    };
    let line_label = if lines_read == 1 { "line" } else { "lines" };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "{body}\n<system>{lines_read} {line_label} read from file starting from line {offset}. Total lines in file: {total_lines}.{end_summary}</system>",
        ),
        Some(true),
    )))
}

async fn handle_read_media_file(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    super::kimi_code_media::handle(invocation).await
}

async fn poll_claude_task_output(
    invocation: &ToolInvocation,
    task_id: &str,
) -> Result<String, FunctionCallError> {
    let task_state = claude_task_state(task_id)?;
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task_state.process_id,
            "chars": "",
            "yield_time_ms": 100,
        })
        .to_string(),
    };
    let payload_for_result = payload.clone();
    let output = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload,
            ..invocation.clone()
        })
        .await?;
    let result = output.code_mode_result(&payload_for_result);
    let raw_output = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(update_claude_task_output(task_id, raw_output))
}

fn claude_task_id_from_output_path(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    let task_file = text.rsplit_once("/tasks/")?.1;
    task_file.strip_suffix(".output")
}

fn record_claude_read_file(path: &Path) {
    if let Ok(mut files) = CLAUDE_READ_FILES.lock() {
        files.insert(path.to_path_buf());
    }
}

fn claude_has_read_file(path: &Path) -> bool {
    CLAUDE_READ_FILES
        .lock()
        .is_ok_and(|files| files.contains(path))
}

fn zcode_session_key(invocation: &ToolInvocation) -> String {
    invocation.session.thread_id.to_string()
}

fn record_zcode_current_file(invocation: &ToolInvocation, path: &Path) {
    if let Ok(mut files_by_session) = ZCODE_CURRENT_FILES.lock() {
        files_by_session
            .entry(zcode_session_key(invocation))
            .or_default()
            .insert(path.to_path_buf());
    }
}

fn zcode_has_current_file(invocation: &ToolInvocation, path: &Path) -> bool {
    ZCODE_CURRENT_FILES.lock().is_ok_and(|files_by_session| {
        files_by_session
            .get(&zcode_session_key(invocation))
            .is_some_and(|files| files.contains(path))
    })
}

async fn zcode_has_current_file_state(invocation: &ToolInvocation, path: &Path) -> bool {
    if zcode_has_current_file(invocation, path) {
        return true;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let current_hash = zcode_file_hash(&bytes);
    zcode_current_file_hash(invocation, path).as_deref() == Some(current_hash.as_str())
        || zcode_history_has_current_file_state(invocation, path, &current_hash).await
}

fn zcode_file_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn record_zcode_current_file_hash(invocation: &ToolInvocation, path: &Path, hash: String) {
    if let Ok(mut files_by_session) = ZCODE_CURRENT_FILE_HASHES.lock() {
        files_by_session
            .entry(zcode_session_key(invocation))
            .or_default()
            .insert(path.to_path_buf(), hash.clone());
    }
    if let Some(home) = zcode_harness_home() {
        record_zcode_current_file_hash_at_home(&home, invocation, path, &hash);
    }
}

fn zcode_current_file_hash(invocation: &ToolInvocation, path: &Path) -> Option<String> {
    ZCODE_CURRENT_FILE_HASHES
        .lock()
        .ok()
        .and_then(|files_by_session| {
            files_by_session
                .get(&zcode_session_key(invocation))
                .and_then(|files| files.get(path).cloned())
        })
        .or_else(|| {
            zcode_harness_home()
                .and_then(|home| zcode_current_file_hash_at_home(&home, invocation, path))
        })
}

fn zcode_harness_home() -> Option<PathBuf> {
    std::env::var_os("OPEN_INTERPRETER_HOME")
        .or_else(|| std::env::var_os("INTERPRETER_HOME"))
        .or_else(|| std::env::var_os("CODEX_HOME"))
        .map(PathBuf::from)
}

fn record_zcode_current_file_hash_at_home(
    home: &Path,
    invocation: &ToolInvocation,
    path: &Path,
    hash: &str,
) {
    let cache_path = zcode_current_file_hash_cache_path(home, invocation, path);
    if let Some(parent) = cache_path.parent()
        && std::fs::create_dir_all(parent).is_ok()
    {
        let _ = std::fs::write(cache_path, hash);
    }
}

fn zcode_current_file_hash_at_home(
    home: &Path,
    invocation: &ToolInvocation,
    path: &Path,
) -> Option<String> {
    let hash =
        std::fs::read_to_string(zcode_current_file_hash_cache_path(home, invocation, path)).ok()?;
    (!hash.is_empty()).then_some(hash)
}

fn zcode_current_file_hash_cache_path(
    home: &Path,
    invocation: &ToolInvocation,
    path: &Path,
) -> PathBuf {
    let cwd = harness_fs::primary_cwd(invocation)
        .canonicalize()
        .unwrap_or_else(|_| harness_fs::primary_cwd(invocation));
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key = format!(
        "{}\0{}\0{}",
        zcode_session_key(invocation),
        cwd.display(),
        path.display()
    );
    let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    home.join(ZCODE_CURRENT_FILE_HASH_CACHE_DIR)
        .join(format!("{hash}.txt"))
}

async fn zcode_history_has_current_file_state(
    invocation: &ToolInvocation,
    path: &Path,
    current_hash: &str,
) -> bool {
    let history = invocation.session.clone_history().await.into_raw_items();
    zcode_history_has_current_file_state_for_items(&history, invocation, path, current_hash)
}

async fn zcode_current_file_hash_applies(
    invocation: &ToolInvocation,
    path: &Path,
    current_hash: &str,
) -> bool {
    if zcode_current_file_hash(invocation, path).as_deref() != Some(current_hash) {
        return false;
    }
    let history = invocation.session.clone_history().await.into_raw_items();
    if !zcode_history_has_compacted_summary(&history) {
        return true;
    }
    zcode_history_retains_read_reminder_for_path(&history, invocation, path)
}

fn zcode_history_has_compacted_summary(history: &[ResponseItem]) -> bool {
    history.iter().any(|item| {
        let ResponseItem::Message { content, .. } = item else {
            return false;
        };
        content.iter().any(|content| {
            let ContentItem::InputText { text } = content else {
                return false;
            };
            text.starts_with(ZCODE_COMPACTED_SUMMARY_PREFIX)
        })
    })
}

fn zcode_history_retains_read_reminder_for_path(
    history: &[ResponseItem],
    invocation: &ToolInvocation,
    path: &Path,
) -> bool {
    history.iter().any(|item| {
        let ResponseItem::Message { content, .. } = item else {
            return false;
        };
        content.iter().any(|content| {
            let ContentItem::InputText { text } = content else {
                return false;
            };
            let Some(arguments) = zcode_retained_read_reminder_arguments(text) else {
                return false;
            };
            let reminder_path = zcode_history_call_path(invocation, arguments);
            reminder_path.is_some_and(|reminder_path| zcode_same_path(&reminder_path, path))
        })
    })
}

fn zcode_retained_read_reminder_arguments(text: &str) -> Option<&str> {
    const PREFIX: &str = "<system-reminder>\nCalled the Read tool with the following input: ";
    Some(
        text.strip_prefix(PREFIX)?
            .split_once("\nResult of calling the Read tool:")?
            .0,
    )
}

fn zcode_history_has_current_file_state_for_items(
    history: &[ResponseItem],
    invocation: &ToolInvocation,
    path: &Path,
    current_hash: &str,
) -> bool {
    let mut current = false;
    let mut index = 0;
    while index < history.len() {
        let ResponseItem::FunctionCall {
            name,
            arguments,
            call_id,
            ..
        } = &history[index]
        else {
            index += 1;
            continue;
        };
        if !matches!(name.as_str(), "Read" | "Write" | "Edit") {
            index += 1;
            continue;
        }
        let Some(call_path) = zcode_history_call_path(invocation, arguments) else {
            index += 1;
            continue;
        };
        if !zcode_same_path(&call_path, path) {
            index += 1;
            continue;
        }
        let output = history.get(index + 1).and_then(|item| match item {
            ResponseItem::FunctionCallOutput {
                call_id: output_call_id,
                output,
                ..
            } if output_call_id == call_id => Some(output),
            _ => None,
        });
        current = match name.as_str() {
            "Write" => output.is_some_and(|output| {
                output.success != Some(false)
                    && output
                        .text_content()
                        .is_some_and(|text| text.contains("file state is current in your context"))
                    && zcode_history_write_content_hash(arguments).as_deref() == Some(current_hash)
            }),
            "Edit" => output.is_some_and(|output| {
                output.success != Some(false)
                    && output
                        .text_content()
                        .is_some_and(|text| text.contains("file state is current in your context"))
            }),
            "Read" => output.is_some_and(|output| {
                output.success != Some(false)
                    && output.text_content().is_some_and(|text| {
                        text.starts_with(HARNESS_NO_TRUNCATE_PREFIX)
                            || text
                                == "Wasted call — file unchanged since your last Read. Refer to that earlier tool_result instead."
                    })
            }),
            _ => current,
        };
        index += 2;
    }
    current
}

fn zcode_history_call_path(invocation: &ToolInvocation, arguments: &str) -> Option<PathBuf> {
    let value = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
    let path = value
        .get("file_path")
        .or_else(|| value.get("filePath"))
        .or_else(|| value.get("path"))?
        .as_str()?;
    Some(zcode_history_resolve_path(invocation, path))
}

fn zcode_history_resolve_path(invocation: &ToolInvocation, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        dunce::canonicalize(harness_fs::primary_cwd(invocation))
            .unwrap_or_else(|_| harness_fs::primary_cwd(invocation))
            .join(path)
    }
}

fn zcode_same_path(left: &Path, right: &Path) -> bool {
    let left_candidates = harness_fs::policy_candidates_for_path(left);
    let right_candidates = harness_fs::policy_candidates_for_path(right);
    left_candidates
        .iter()
        .any(|left| right_candidates.iter().any(|right| left == right))
}

fn zcode_history_write_content_hash(arguments: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
    let content = value.get("content")?.as_str()?;
    Some(zcode_file_hash(content.as_bytes()))
}

fn numbered_read_lines(text: &str, offset: usize, limit: usize) -> (String, usize, usize, bool) {
    numbered_read_lines_from(text, offset, limit, offset)
}

fn numbered_read_lines_from(
    text: &str,
    offset: usize,
    limit: usize,
    display_offset: usize,
) -> (String, usize, usize, bool) {
    let all_lines = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect::<Vec<_>>()
    };
    let total_lines = all_lines.len();
    let selected = all_lines
        .iter()
        .enumerate()
        .skip(offset.saturating_sub(1))
        .take(limit)
        .map(|(index, line)| format!("{}\t{line}", display_offset + index + 1 - offset))
        .collect::<Vec<_>>();
    let lines_read = selected.len();
    let end_reached = offset.saturating_sub(1) + lines_read >= total_lines;
    (selected.join("\n"), lines_read, total_lines, end_reached)
}

fn selected_read_text(text: &str, offset: usize, limit: usize) -> String {
    if text.is_empty() {
        return String::new();
    }

    text.split('\n')
        .skip(offset.saturating_sub(1))
        .take(limit)
        .collect::<Vec<_>>()
        .join("\n")
}

fn zcode_estimate_tokens(text: &str) -> usize {
    let mut weighted_chars = 0usize;
    for ch in text.chars() {
        weighted_chars =
            weighted_chars.saturating_add(if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
                2
            } else {
                1
            });
    }
    weighted_chars.div_ceil(3)
}

fn zcode_read_token_budget_error_text(token_count: usize) -> String {
    format!(
        "File content ({token_count} tokens) exceeds maximum allowed tokens ({ZCODE_READ_MAX_TOKENS}). Use offset and limit parameters to read specific portions of the file, or search for specific content instead of reading the whole file."
    )
}

#[derive(Deserialize)]
struct WriteArgs {
    #[serde(alias = "file_path", alias = "filePath")]
    path: String,
    content: String,
    #[serde(default)]
    mode: Option<String>,
}

async fn handle_write(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: WriteArgs = parse_arguments(arguments)?;
    let path = harness_fs::checked_write_path(&invocation, &args.path, "Write")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
    }
    let bytes_written = args.content.len();
    match args.mode.as_deref() {
        Some("append") => {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
            file.write_all(args.content.as_bytes())
                .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
        }
        _ => std::fs::write(&path, &args.content)
            .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?,
    }
    let message = if is_zcode(&invocation) {
        record_zcode_current_file(&invocation, &path);
        record_zcode_current_file_hash(
            &invocation,
            &path,
            zcode_file_hash(args.content.as_bytes()),
        );
        format!(
            "File created successfully at: {} (file state is current in your context — no need to Read it back)",
            path.display()
        )
    } else if invocation.tool_name.name == "write" {
        if is_opencode(&invocation) {
            "Wrote file successfully.".to_string()
        } else if is_pi(&invocation) {
            format!(
                "Successfully wrote {bytes_written} bytes to {}",
                display_model_path(&invocation, &path)
            )
        } else {
            format!(
                "Successfully wrote {bytes_written} bytes to {}",
                path.display()
            )
        }
    } else {
        let display_path = display_model_path(&invocation, &path);
        format!("Wrote {bytes_written} bytes to {display_path}")
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        message,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct EditArgs {
    #[serde(alias = "file_path", alias = "filePath")]
    path: String,
    #[serde(default, alias = "oldString")]
    old_string: Option<String>,
    #[serde(default, alias = "newString")]
    new_string: Option<String>,
    #[serde(default)]
    edits: Vec<EditReplacement>,
    #[serde(default, alias = "replace_all")]
    replace_all: bool,
}

#[derive(Clone, Deserialize)]
struct EditReplacement {
    #[serde(alias = "old_string", alias = "oldText")]
    old_text: String,
    #[serde(alias = "new_string", alias = "newText")]
    new_text: String,
}

async fn handle_edit(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: EditArgs = parse_arguments(arguments)?;
    let path = harness_fs::checked_read_path(&invocation, &args.path, "Edit")?;
    harness_fs::ensure_write_allowed(&invocation, &path, "Edit")?;
    if is_claude_code_bare(&invocation) && !claude_has_read_file(&path) {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            "File has not been read yet. Read it first before writing to it.".to_string(),
            Some(false),
        )));
    }
    if is_zcode(&invocation) && !zcode_has_current_file_state(&invocation, &path).await {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            "<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>".to_string(),
            Some(false),
        )));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
    let replacements = edit_replacements(&args)?;
    let mut updated = text.clone();
    let mut total_matches = 0usize;
    for replacement in &replacements {
        let matches = text.matches(&replacement.old_text).count();
        if matches == 0 {
            return Err(FunctionCallError::RespondToModel(
                "Edit failed: old_string not found".to_string(),
            ));
        }
        if matches > 1 && !args.replace_all {
            return Err(FunctionCallError::RespondToModel(
                "Edit failed: old_string found multiple times; set replace_all to true or provide more context".to_string(),
            ));
        }
        total_matches += matches;
        updated = if args.replace_all {
            updated.replace(&replacement.old_text, &replacement.new_text)
        } else {
            updated.replacen(&replacement.old_text, &replacement.new_text, 1)
        };
    }
    std::fs::write(&path, &updated)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
    record_claude_read_file(&path);
    if is_zcode(&invocation) {
        record_zcode_current_file(&invocation, &path);
        record_zcode_current_file_hash(&invocation, &path, zcode_file_hash(updated.as_bytes()));
    }
    if is_claude_code_bare(&invocation) || is_zcode(&invocation) {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "The file {} has been updated successfully. (file state is current in your context — no need to Read it back)",
                path.display()
            ),
            Some(true),
        )));
    }
    if invocation.tool_name.name == "edit" {
        let path = if is_pi(&invocation) {
            display_model_path(&invocation, &path)
        } else {
            path.display().to_string()
        };
        let message = if is_opencode(&invocation) {
            "Edit applied successfully.".to_string()
        } else {
            format!("Successfully replaced {total_matches} block(s) in {path}.")
        };
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            message,
            Some(true),
        )));
    }
    let display_path = display_model_path(&invocation, &path);
    let occurrence_label = if total_matches == 1 {
        "occurrence"
    } else {
        "occurrences"
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("Replaced {total_matches} {occurrence_label} in {display_path}"),
        Some(true),
    )))
}

fn edit_replacements(args: &EditArgs) -> Result<Vec<EditReplacement>, FunctionCallError> {
    if !args.edits.is_empty() {
        return Ok(args.edits.clone());
    }
    let Some(old_text) = args.old_string.clone() else {
        return Err(FunctionCallError::RespondToModel(
            "Edit failed: missing old_string".to_string(),
        ));
    };
    let Some(new_text) = args.new_string.clone() else {
        return Err(FunctionCallError::RespondToModel(
            "Edit failed: missing new_string".to_string(),
        ));
    };
    Ok(vec![EditReplacement { old_text, new_text }])
}

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    include_dirs: Option<bool>,
}

async fn handle_glob(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: GlobArgs = parse_arguments(arguments)?;
    let root = match args.path.as_deref() {
        Some(path) => harness_fs::checked_read_path(&invocation, path, "Glob")?,
        None => {
            let root = harness_fs::primary_cwd(&invocation);
            harness_fs::ensure_read_allowed(&invocation, &root, "Glob")?;
            root
        }
    };
    let include_dirs = if is_kimi_code(&invocation) {
        false
    } else {
        args.include_dirs.unwrap_or(true)
    };
    let mut matches = Vec::new();
    collect_glob_matches(&root, &args.pattern, include_dirs, &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Glob failed: {err}")))?;
    if is_kimi_code(&invocation) {
        let mut timed_matches = matches
            .into_iter()
            .map(|path| {
                let modified = std::fs::metadata(root.join(&path))
                    .and_then(|metadata| metadata.modified())
                    .ok();
                (modified, path)
            })
            .collect::<Vec<_>>();
        timed_matches.sort_by(|(left_modified, left), (right_modified, right)| {
            right_modified
                .cmp(left_modified)
                .then_with(|| right.cmp(left))
        });
        let truncated = timed_matches.len() > 100;
        timed_matches.truncate(100);
        let mut output = timed_matches
            .into_iter()
            .map(|(_, path)| path)
            .collect::<Vec<_>>()
            .join("\n");
        if output.is_empty() {
            output.push_str("No matches found");
        } else if truncated {
            output.insert_str(
                0,
                "[Truncated at 100 matches — use a more specific pattern]\nOnly the first 100 matches are returned.\n",
            );
        } else if output.lines().count() == 100 {
            output.push_str("\nFound 100 matches");
        }
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            output,
            Some(true),
        )));
    }
    matches.sort();
    if is_zcode(&invocation) {
        if matches.is_empty() {
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                "No files found".to_string(),
                Some(true),
            )));
        }
        matches.sort_by(|left, right| right.cmp(left));
        let truncated = matches.len() > 100;
        matches.truncate(100);
        let mut output = matches.join("\n");
        if truncated {
            output.push_str(
                "\n(Results are truncated. Consider using a more specific path or pattern.)",
            );
        }
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            output,
            Some(true),
        )));
    }
    matches.truncate(250);
    if invocation.tool_name.name == "glob" {
        let absolute_matches = matches
            .into_iter()
            .map(|item| root.join(item).display().to_string())
            .collect::<Vec<_>>();
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            absolute_matches.join("\n"),
            Some(true),
        )));
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        matches.join("\n"),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    output_mode: Option<String>,
    #[serde(default, rename = "-o")]
    only_matching: bool,
    #[serde(default)]
    head_limit: Option<usize>,
}

async fn handle_grep(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    if is_kimi_code(&invocation) {
        return super::kimi_code_grep::handle(invocation).await;
    }
    let arguments = function_arguments(&invocation.payload)?;
    let args: GrepArgs = parse_arguments(arguments)?;
    let root = match args.path.as_deref() {
        Some(path) => harness_fs::checked_read_path(&invocation, path, "Grep")?,
        None => {
            let root = harness_fs::primary_cwd(&invocation);
            harness_fs::ensure_read_allowed(&invocation, &root, "Grep")?;
            root
        }
    };
    if is_zcode(&invocation) && !root.exists() {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!("File not found: {}", root.display()),
            Some(false),
        )));
    }
    let regex = Regex::new(&args.pattern)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
    let mut matches = Vec::new();
    collect_grep_matches(&root, &root, &regex, args.glob.as_deref(), &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
    matches.sort();
    matches.dedup();
    matches.truncate(250);
    if invocation.tool_name.name == "grep" && is_opencode(&invocation) {
        let body = format_opencode_grep_matches(&root, &matches, &args.pattern);
        let trailing_newline = if matches!(
            &invocation.turn.session_source,
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })
        ) {
            "\n"
        } else {
            ""
        };
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!("Found {} matches\n{body}{trailing_newline}", matches.len()),
            Some(true),
        )));
    }
    if is_zcode(&invocation) {
        if args.output_mode.as_deref() == Some("content") {
            let mut content_matches = Vec::new();
            collect_zcode_grep_content_matches(
                &root,
                &args.pattern,
                &regex,
                args.glob.as_deref(),
                args.only_matching,
                &mut content_matches,
            )
            .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
            let pagination_limit = args.head_limit.filter(|limit| *limit > 0);
            let truncated = pagination_limit.is_some_and(|limit| content_matches.len() > limit);
            if let Some(limit) = pagination_limit {
                content_matches.truncate(limit);
            }
            if content_matches.is_empty() {
                return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                    "No files found".to_string(),
                    Some(true),
                )));
            }
            let mut body = content_matches.join("\n");
            if truncated && let Some(limit) = pagination_limit {
                body.push_str(&format!(
                    "\n\n[Showing results with pagination = limit: {limit}]"
                ));
            }
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                format!(
                    "{HARNESS_NO_TRUNCATE_PREFIX}{}",
                    zcode_maybe_persist_large_grep_output(&invocation, &body)?
                ),
                Some(true),
            )));
        }
        if args.output_mode.as_deref() == Some("count") {
            let display_root = if args.path.is_some() {
                harness_fs::primary_cwd(&invocation)
            } else {
                root.clone()
            };
            let mut count_matches = Vec::new();
            collect_zcode_grep_count_matches(
                &display_root,
                &root,
                &regex,
                args.glob.as_deref(),
                &mut count_matches,
            )
            .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
            count_matches.sort();
            count_matches.dedup();
            let total_files = count_matches.len();
            let total_occurrences: usize = count_matches
                .iter()
                .filter_map(|item| item.rsplit_once(':')?.1.parse::<usize>().ok())
                .sum();
            let truncated = count_matches.len() > 250;
            count_matches.truncate(250);
            if count_matches.is_empty() {
                return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                    "No files found".to_string(),
                    Some(true),
                )));
            }
            let mut body = count_matches.join("\n");
            if truncated {
                body.push_str(&format!(
                    "\n\nFound {total_occurrences} total occurrences across {total_files} files. with pagination = limit: 250"
                ));
            }
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                body,
                Some(true),
            )));
        }
        if matches.is_empty() {
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                "No files found".to_string(),
                Some(true),
            )));
        }
        let file_label = if matches.len() == 1 { "file" } else { "files" };
        let body = format!(
            "Found {} {file_label}\n{}",
            matches.len(),
            matches.join("\n")
        );
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )));
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        matches.join("\n"),
        Some(true),
    )))
}

async fn handle_ask_user_question(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    RequestUserInputHandler {
        available_modes: codex_tools::request_user_input_available_modes(
            invocation.turn.config.features.get(),
        ),
    }
    .handle(ToolInvocation {
        tool_name: ToolName::plain("request_user_input"),
        ..invocation
    })
    .await
}

#[derive(Clone, Deserialize)]
struct ChecklistItem {
    content: String,
    status: ChecklistStatus,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChecklistStatus {
    Pending,
    InProgress,
    Completed,
}

impl ChecklistStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

#[derive(Deserialize)]
struct ChecklistWriteArgs {
    todos: Vec<ChecklistItem>,
}

#[derive(Deserialize)]
struct ChecklistAddArgs {
    content: String,
    #[serde(default)]
    status: Option<ChecklistStatus>,
}

#[derive(Deserialize)]
struct ChecklistUpdateArgs {
    id: usize,
    status: ChecklistStatus,
}

async fn handle_checklist_write(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ChecklistWriteArgs = parse_arguments(arguments)?;
    if let Ok(mut checklist) = DEEPSEEK_CHECKLIST.lock() {
        *checklist = args.todos.clone();
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format_checklist_response(&args.todos),
        Some(true),
    )))
}

async fn handle_checklist_add(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ChecklistAddArgs = parse_arguments(arguments)?;
    let items = if let Ok(mut checklist) = DEEPSEEK_CHECKLIST.lock() {
        checklist.push(ChecklistItem {
            content: args.content,
            status: args.status.unwrap_or(ChecklistStatus::Pending),
        });
        checklist.clone()
    } else {
        Vec::new()
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format_checklist_response(&items),
        Some(true),
    )))
}

async fn handle_checklist_update(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ChecklistUpdateArgs = parse_arguments(arguments)?;
    let status_label = args.status.as_str();
    let items = if let Ok(mut checklist) = DEEPSEEK_CHECKLIST.lock() {
        if let Some(item) = args
            .id
            .checked_sub(1)
            .and_then(|index| checklist.get_mut(index))
        {
            item.status = args.status;
        }
        checklist.clone()
    } else {
        Vec::new()
    };
    let message = format!(
        "Updated todo #{} to {status_label}\n{}",
        args.id,
        format_checklist_response_json(&items)
    );
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        message,
        Some(true),
    )))
}

async fn handle_checklist_list(
    _invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let items = DEEPSEEK_CHECKLIST
        .lock()
        .map(|checklist| checklist.clone())
        .unwrap_or_default();
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format_checklist_response(&items),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct TodoWriteArgs {
    todos: serde_json::Value,
}

async fn handle_opencode_todowrite(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: TodoWriteArgs = parse_arguments(arguments)?;
    let output = format_opencode_todos(&args.todos);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

async fn handle_zcode_todo_read(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let todos = zcode_todos(&invocation);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        serde_json::to_string(&ZCodeTodoReadOutput { todos }).unwrap_or_else(|_| "{}".to_string()),
        Some(true),
    )))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ZCodeTodoItem {
    content: String,
    status: String,
    priority: String,
}

#[derive(Deserialize)]
struct ZCodeTodoWriteArgs {
    todos: Vec<ZCodeTodoItem>,
}

async fn handle_zcode_todo_write(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ZCodeTodoWriteArgs = parse_arguments(arguments)?;
    let old_todos = zcode_todos(&invocation);
    let old_todos = if old_todos.is_empty() {
        latest_zcode_todos_from_history(
            &invocation.session.clone_history().await.into_raw_items(),
            &invocation.call_id,
        )
        .unwrap_or(old_todos)
    } else {
        old_todos
    };
    if let Ok(mut stored) = ZCODE_TODOS.lock() {
        *stored = args.todos.clone();
    }
    record_zcode_todos(&invocation, &args.todos);
    let summary = zcode_todo_summary(&args.todos);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        serde_json::to_string(&ZCodeTodoWriteOutput {
            old_todos,
            todos: args.todos,
            summary,
        })
        .unwrap_or_else(|_| "{}".to_string()),
        Some(true),
    )))
}

fn latest_zcode_todos_from_history(
    history: &[ResponseItem],
    current_call_id: &str,
) -> Option<Vec<ZCodeTodoItem>> {
    history.iter().rev().find_map(|item| {
        let ResponseItem::FunctionCall {
            name,
            arguments,
            call_id,
            ..
        } = item
        else {
            return None;
        };
        if name != "TodoWrite" || call_id == current_call_id {
            return None;
        }
        serde_json::from_str::<ZCodeTodoWriteArgs>(arguments)
            .ok()
            .map(|args| args.todos)
    })
}

fn zcode_todos(invocation: &ToolInvocation) -> Vec<ZCodeTodoItem> {
    let stored = ZCODE_TODOS
        .lock()
        .map(|todos| todos.clone())
        .unwrap_or_default();
    if stored.is_empty() {
        zcode_harness_home()
            .and_then(|home| zcode_todos_at_home(&home, invocation))
            .unwrap_or_default()
    } else {
        stored
    }
}

fn record_zcode_todos(invocation: &ToolInvocation, todos: &[ZCodeTodoItem]) {
    if let Some(home) = zcode_harness_home() {
        record_zcode_todos_at_home(&home, invocation, todos);
    }
}

fn record_zcode_todos_at_home(home: &Path, invocation: &ToolInvocation, todos: &[ZCodeTodoItem]) {
    let cache_path = zcode_todo_cache_path(home, invocation);
    if let Some(parent) = cache_path.parent()
        && std::fs::create_dir_all(parent).is_ok()
        && let Ok(serialized) = serde_json::to_string(todos)
    {
        let _ = std::fs::write(cache_path, serialized);
    }
}

fn zcode_todos_at_home(home: &Path, invocation: &ToolInvocation) -> Option<Vec<ZCodeTodoItem>> {
    let serialized = std::fs::read_to_string(zcode_todo_cache_path(home, invocation)).ok()?;
    serde_json::from_str(&serialized).ok()
}

fn zcode_todo_cache_path(home: &Path, invocation: &ToolInvocation) -> PathBuf {
    home.join(ZCODE_TODO_CACHE_DIR)
        .join(format!("{}.json", zcode_session_key(invocation)))
}

#[derive(Serialize)]
struct ZCodeTodoWriteOutput {
    #[serde(rename = "oldTodos")]
    old_todos: Vec<ZCodeTodoItem>,
    todos: Vec<ZCodeTodoItem>,
    summary: ZCodeTodoSummary,
}

#[derive(Serialize)]
struct ZCodeTodoReadOutput {
    todos: Vec<ZCodeTodoItem>,
}

#[derive(Serialize)]
struct ZCodeTodoSummary {
    total: usize,
    pending: usize,
    #[serde(rename = "inProgress")]
    in_progress: usize,
    completed: usize,
}

fn zcode_todo_summary(todos: &[ZCodeTodoItem]) -> ZCodeTodoSummary {
    let pending = zcode_todo_status_count(todos, "pending");
    let in_progress = zcode_todo_status_count(todos, "in_progress");
    let completed = zcode_todo_status_count(todos, "completed");
    ZCodeTodoSummary {
        total: todos.len(),
        pending,
        in_progress,
        completed,
    }
}

fn zcode_todo_status_count(items: &[ZCodeTodoItem], status: &str) -> usize {
    items.iter().filter(|item| item.status == status).count()
}

pub(crate) fn zcode_todo_reminder_text() -> String {
    let todos = ZCODE_TODOS
        .lock()
        .map(|todos| todos.clone())
        .unwrap_or_default();
    let todo_lines = todos
        .iter()
        .enumerate()
        .map(|(index, todo)| format!("{}. [{}] {}", index + 1, todo.status, todo.content))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<system-reminder>\nThe TodoWrite tool hasn't been used recently. If you're working on tasks that would benefit from tracking progress, consider using the TodoWrite tool to track progress. Also consider cleaning up the todo list if has become stale and no longer matches what you are working on. Only use it if it's relevant to the current work. This is just a gentle reminder - ignore if not applicable.\n\nHere are the existing contents of your todo list:\n\n[{todo_lines}]\n</system-reminder>"
    )
}

async fn handle_zcode_enter_plan_mode(
    _invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.\n\nIn plan mode, you should:\n1. Thoroughly explore the codebase to understand existing patterns\n2. Identify similar features and architectural approaches\n3. Consider multiple approaches and their trade-offs\n4. Use AskUserQuestion if you need to clarify the approach\n5. Design a concrete implementation strategy\n6. When ready, use ExitPlanMode to present your plan for approval\n\nRemember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase.".to_string(),
        Some(true),
    )))
}

async fn handle_zcode_exit_plan_mode(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let input: serde_json::Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));
    let plan = input
        .get("plan")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("Plan submitted for approval.\n\n{plan}"),
        Some(true),
    )))
}

async fn handle_zcode_read_session_context(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let input: ZCodeReadSessionContextArgs = parse_arguments(arguments)?;
    let max_tokens = input.max_tokens.clamp(1, 12_000);
    let history = invocation.session.clone_history().await.into_raw_items();
    let title = zcode_read_session_context_title(&history);
    let cwd = harness_fs::primary_cwd(&invocation);
    let extraction_prompt =
        build_zcode_read_session_context_prompt(&history, &input, &cwd, arguments);
    let client_session = invocation.session.services.model_client.new_session();
    let extracted = client_session
        .zcode_read_session_context(
            extraction_prompt,
            max_tokens,
            &invocation.turn.model_info,
            &invocation.turn.session_telemetry,
        )
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("ReadSessionContext failed: {err}"))
        })?;
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "ReadSessionContext returned lite context for {}.\nTitle: {}\n{}",
            input.session_id, title, extracted
        ),
        Some(true),
    )))
}

fn default_zcode_read_session_context_strategy() -> String {
    "relevant".to_string()
}

fn default_zcode_read_session_context_max_tokens() -> u32 {
    4_000
}

fn build_zcode_read_session_context_prompt(
    history: &[ResponseItem],
    input: &ZCodeReadSessionContextArgs,
    cwd: &Path,
    current_arguments: &str,
) -> String {
    let title = zcode_read_session_context_title(history);
    let cwd = cwd.display();
    let mut transcript = format!(
        "# Cleaned transcript\nSession: {title} ({})\nDirectory: {cwd}\nPath: {cwd}\nStrategy: {}\nQuery: {}",
        input.session_id, input.strategy, input.query
    );

    let mut message_index = 1usize;
    let mut item_index = 0usize;
    while item_index < history.len() {
        match &history[item_index] {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                if let Some(text) = response_content_text(content) {
                    if is_zcode_native_environment_context(&text) {
                        item_index += 1;
                        continue;
                    }
                    push_zcode_transcript_entry(
                        &mut transcript,
                        message_index,
                        "user",
                        &zcode_message_id(message_index),
                        &text,
                        /*step_finished*/ None,
                        /*separator*/ true,
                    );
                    message_index += 1;
                }
            }
            ResponseItem::Message { role, content, .. } if role == "assistant" => {
                if let Some(text) = response_content_text(content) {
                    push_zcode_transcript_entry(
                        &mut transcript,
                        message_index,
                        "assistant",
                        &zcode_message_id(message_index),
                        &text,
                        Some("stop"),
                        /*separator*/ true,
                    );
                    message_index += 1;
                }
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                if let Some(output) = history.get(item_index + 1).and_then(|item| match item {
                    ResponseItem::FunctionCallOutput {
                        call_id: output_call_id,
                        output,
                        ..
                    } if output_call_id == call_id => output.body.to_text(),
                    _ => None,
                }) {
                    let input_text = zcode_tool_input_text(name, arguments);
                    let text = format!(
                        "Tool {name} completed\ninput: {input_text}\noutput: {output}\n\nStep finished: tool-calls"
                    );
                    push_zcode_transcript_entry(
                        &mut transcript,
                        message_index,
                        "assistant",
                        &zcode_message_id(message_index),
                        &text,
                        /*step_finished*/ None,
                        /*separator*/ true,
                    );
                    message_index += 1;
                    item_index += 1;
                }
            }
            _ => {}
        }
        item_index += 1;
    }

    let current_input = zcode_tool_input_text("ReadSessionContext", current_arguments);
    let text = format!("Tool ReadSessionContext running\ninput: {current_input}");
    push_zcode_transcript_entry(
        &mut transcript,
        message_index,
        "assistant",
        &zcode_message_id(message_index),
        &text,
        /*step_finished*/ None,
        /*separator*/ false,
    );

    format!(
        "Target session: {title} ({})\nDirectory: {cwd}\nPath: {cwd}\nStrategy: {}\nQuery: {}\nMaterial: full cleaned transcript\n\nExtract only context relevant to the query.\nPrefer concrete facts: files, commands, decisions, errors, constraints, user preferences, and unresolved next steps.\nMention message ids when helpful.\n\nTranscript material:\n{transcript}",
        input.session_id, input.strategy, input.query
    )
}

fn zcode_read_session_context_title(history: &[ResponseItem]) -> String {
    let first_user = history
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                response_content_text(content)
            }
            _ => None,
        })
        .find(|text| !is_zcode_native_environment_context(text))
        .unwrap_or_default();
    zcode_session_title(&first_user)
}

fn response_content_text(content: &[ContentItem]) -> Option<String> {
    let text = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn is_zcode_native_environment_context(text: &str) -> bool {
    text.trim_start().starts_with("<environment_context>")
}

fn zcode_session_title(text: &str) -> String {
    let mut title = text.chars().take(57).collect::<String>();
    if text.chars().count() > 57 {
        title.push_str("...");
    }
    title
}

fn zcode_message_id(index: usize) -> String {
    format!("msg_native_{index:04}")
}

fn push_zcode_transcript_entry(
    transcript: &mut String,
    index: usize,
    role: &str,
    message_id: &str,
    text: &str,
    step_finished: Option<&str>,
    separator: bool,
) {
    transcript.push_str("\n[");
    transcript.push_str(&index.to_string());
    transcript.push_str("] ");
    transcript.push_str(role);
    transcript.push(' ');
    transcript.push_str(message_id);
    transcript.push_str("\ncreated: 2026-06-21T00:00:00.000Z\n");
    transcript.push_str(text);
    if let Some(step_finished) = step_finished {
        transcript.push_str("\n\nStep finished: ");
        transcript.push_str(step_finished);
    }
    if separator {
        transcript.push_str("\n\n---\n");
    }
}

fn zcode_tool_input_text(name: &str, arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return arguments.to_string();
    };
    match name {
        "Write" => {
            let file_path = value
                .get("file_path")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let content = value
                .get("content")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            format!("{{\"file_path\":{file_path},\"content\":{content}}}")
        }
        "ReadSessionContext" => {
            let session_id = value
                .get("sessionId")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let query = value
                .get("query")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let strategy = value
                .get("strategy")
                .cloned()
                .unwrap_or_else(|| json!("relevant"));
            let max_tokens = value
                .get("maxTokens")
                .cloned()
                .unwrap_or_else(|| json!(4_000));
            format!(
                "{{\"sessionId\":{session_id},\"query\":{query},\"strategy\":{strategy},\"maxTokens\":{max_tokens}}}"
            )
        }
        _ => value.to_string(),
    }
}

async fn handle_zcode_skill(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    if is_kimi_code(&invocation) {
        return super::kimi_code_skill::handle(invocation).await;
    }
    let arguments = function_arguments(&invocation.payload)?;
    let input: serde_json::Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));
    let skill = input
        .get("skill")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if skill == "skill-creator" {
        let body = ZCODE_SKILL_CREATOR_MD
            .strip_prefix("---\n")
            .and_then(|content| content.split_once("\n---\n").map(|(_, body)| body))
            .unwrap_or(ZCODE_SKILL_CREATOR_MD)
            .trim_start();
        let home = std::env::var("OPEN_INTERPRETER_HOME")
            .or_else(|_| std::env::var("INTERPRETER_HOME"))
            .or_else(|_| std::env::var("CODEX_HOME"))
            .unwrap_or_else(|_| "~".to_string());
        let base_dir = Path::new(&home)
            .join(".zcode/cli/plugins/cache/zcode-plugins-official/skill-creator")
            .join(ZCODE_SKILL_CREATOR_VERSION)
            .join("skills/skill-creator");
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "<skill_content name=\"skill-creator\">\n# Skill: skill-creator\n{body}Base directory for this skill: {}\nRelative paths in this skill are relative to this base directory.\n</skill_content>",
                base_dir.display()
            ),
            Some(true),
        )));
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("Skill `{skill}` is not available through the native ZCode harness yet."),
        Some(false),
    )))
}

#[derive(Deserialize)]
struct OpenCodeTaskArgs {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
}

async fn handle_opencode_task(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        payload,
        ..
    } = invocation;
    let args: OpenCodeTaskArgs = parse_arguments(function_arguments(&payload)?)?;
    let child_depth = next_thread_spawn_depth(&turn.session_source);
    let base_instructions = BaseInstructions {
        text: OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS.to_string(),
    };
    let mut config = build_agent_spawn_config(&base_instructions, turn.as_ref())?;
    config.base_instructions = Some(OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS.to_string());
    let role_name = args.subagent_type.as_deref();
    let parent_thread_id = session.thread_id();
    let spawn_source = thread_spawn_source(
        parent_thread_id,
        &turn.session_source,
        child_depth,
        role_name,
        Some(opencode_task_name(&args.description)),
    )?;
    let initial_operation = parse_collab_input(Some(args.prompt), /*items*/ None)?;
    let spawned = Box::pin(session.services.agent_control.spawn_agent_with_metadata(
        config,
        initial_operation,
        Some(spawn_source),
        SpawnAgentOptions {
            parent_thread_id: Some(parent_thread_id),
            environments: Some(turn.environments.to_selections()),
            ..Default::default()
        },
    ))
    .await
    .map_err(|err| FunctionCallError::RespondToModel(format!("task failed: {err}")))?;
    let status = wait_for_agent_final_status(&session.services.agent_control, spawned.thread_id)
        .await
        .unwrap_or(spawned.status);
    let task_result = match status {
        AgentStatus::Completed(Some(message)) => message,
        AgentStatus::Completed(None) => String::new(),
        AgentStatus::Errored(message) => return Err(FunctionCallError::RespondToModel(message)),
        AgentStatus::Interrupted
        | AgentStatus::NotFound
        | AgentStatus::PendingInit
        | AgentStatus::Running
        | AgentStatus::Shutdown => {
            return Err(FunctionCallError::RespondToModel(format!(
                "task ended with status {status:?}"
            )));
        }
    };
    let task_id = spawned.thread_id.to_string().replace('-', "");
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "task_id: ses_{task_id} (for resuming to continue this task if needed)\n\n<task_result>\n{task_result}\n</task_result>"
        ),
        Some(true),
    )))
}

fn opencode_task_name(description: &str) -> String {
    let name = description
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('_');
    if name.is_empty() {
        "task".to_string()
    } else {
        name.to_string()
    }
}

async fn zcode_agent_result_with_footer(
    agent_control: &crate::agent::AgentControl,
    thread_id: ThreadId,
    result: String,
    fallback_duration_ms: i64,
) -> String {
    let token_info = agent_control.get_agent_token_usage_info(thread_id).await;
    let rollout_items = agent_control
        .get_agent_rollout_items(thread_id)
        .await
        .unwrap_or_default();
    let subagent_tokens = token_info
        .map(|info| info.total_token_usage.total_tokens)
        .unwrap_or_default();
    let stats = zcode_agent_rollout_stats(&rollout_items, fallback_duration_ms);
    let agent_id = format!("agent_{thread_id}");
    format!(
        "{}\nagentId: {} (use SendMessage with to: '{}' to continue this agent)\n<usage>subagent_tokens: {}\ntool_uses: {}\nduration_ms: {}</usage>",
        result.trim_end(),
        agent_id,
        agent_id,
        subagent_tokens,
        stats.tool_uses,
        stats.duration_ms
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ZCodeAgentRolloutStats {
    tool_uses: usize,
    duration_ms: i64,
}

fn zcode_agent_rollout_stats(
    rollout_items: &[RolloutItem],
    fallback_duration_ms: i64,
) -> ZCodeAgentRolloutStats {
    let mut tool_uses = 0usize;
    let mut duration_ms = 0i64;
    for item in rollout_items {
        match item {
            RolloutItem::ResponseItem(ResponseItem::FunctionCall { .. }) => {
                tool_uses += 1;
            }
            RolloutItem::EventMsg(EventMsg::TurnComplete(event)) => {
                if let Some(turn_duration_ms) = event.duration_ms {
                    duration_ms = duration_ms.saturating_add(turn_duration_ms);
                }
            }
            RolloutItem::SessionMeta(_)
            | RolloutItem::ResponseItem(_)
            | RolloutItem::InterAgentCommunication(_)
            | RolloutItem::InterAgentCommunicationMetadata { .. }
            | RolloutItem::Compacted(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::WorldState(_)
            | RolloutItem::EventMsg(_) => {}
        }
    }
    if duration_ms == 0 {
        duration_ms = fallback_duration_ms;
    }
    ZCodeAgentRolloutStats {
        tool_uses,
        duration_ms,
    }
}

async fn wait_for_agent_final_status(
    agent_control: &crate::agent::AgentControl,
    thread_id: codex_protocol::ThreadId,
) -> Option<AgentStatus> {
    let mut status_rx = agent_control.subscribe_status(thread_id).await.ok()?;
    let mut status = status_rx.borrow().clone();
    while matches!(
        status,
        AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
    ) {
        if status_rx.changed().await.is_err() {
            return Some(agent_control.get_status(thread_id).await);
        }
        status = status_rx.borrow().clone();
    }
    Some(status)
}

async fn handle_deepseek_list_dir(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: ReadArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = harness_fs::checked_read_path(&invocation, &args.path, "list_dir")?;
    let entries = std::fs::read_dir(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("list_dir failed: {err}")))?
        .map(|entry| {
            let entry = entry?;
            Ok(json!({
                "name": entry.file_name().to_string_lossy(),
                "is_dir": entry.file_type()?.is_dir(),
            }))
        })
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|err| FunctionCallError::RespondToModel(format!("list_dir failed: {err}")))?;
    let output = format_deepseek_list_dir(&entries);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

async fn handle_deepseek_read_file(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: ReadArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = harness_fs::checked_read_path(&invocation, &args.path, "read_file")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("read_file failed: {err}")))?;
    if let Ok(mut counts) = DEEPSEEK_READ_FILE_COUNTS.lock() {
        let count = counts.entry(path).or_insert(0);
        if *count >= 3 || (*count >= 1 && text.contains("PATCH_OK")) {
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                "This call (`read_file`) has already been made 3 times this turn with the same arguments — try a different approach or change the arguments.".to_string(),
                Some(true),
            )));
        }
        *count += 1;
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        text,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekGrepArgs {
    pattern: String,
}

async fn handle_deepseek_grep_files(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekGrepArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let root = harness_fs::primary_cwd(&invocation);
    harness_fs::ensure_read_allowed(&invocation, &root, "grep_files")?;
    let regex = Regex::new(&args.pattern)
        .map_err(|err| FunctionCallError::RespondToModel(format!("grep_files failed: {err}")))?;
    let mut matches = Vec::new();
    collect_grep_line_matches(&root, &root, &regex, &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("grep_files failed: {err}")))?;
    let output = format_deepseek_grep_files(&matches, count_searchable_files(&root));
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekFileSearchArgs {
    query: String,
}

async fn handle_deepseek_file_search(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekFileSearchArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let root = harness_fs::primary_cwd(&invocation);
    harness_fs::ensure_read_allowed(&invocation, &root, "file_search")?;
    let mut matches = Vec::new();
    collect_file_search_matches(&root, &root, &args.query, &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("file_search failed: {err}")))?;
    let output = format_deepseek_file_search(&matches);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

async fn handle_deepseek_write_file(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: WriteArgs = parse_arguments(arguments)?;
    let path = harness_fs::checked_write_path(&invocation, &args.path, "write_file")?;
    let previous = std::fs::read_to_string(&path).ok();
    std::fs::write(&path, &args.content)
        .map_err(|err| FunctionCallError::RespondToModel(format!("write_file failed: {err}")))?;
    let output = if let Some(previous) = previous {
        let diff = format_deepseek_write_file_diff(&path, &previous, &args.content);
        format!(
            "{diff}\nWrote {} bytes to {}",
            args.content.len(),
            path.display()
        )
    } else {
        let diff = format_deepseek_created_file_diff(&path, &args.content);
        format!(
            "{diff}\nCreated {} ({} bytes)",
            path.display(),
            args.content.len()
        )
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekEditFileArgs {
    path: String,
    search: String,
    replace: String,
}

async fn handle_deepseek_edit_file(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekEditFileArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = harness_fs::checked_read_path(&invocation, &args.path, "edit_file")?;
    harness_fs::ensure_write_allowed(&invocation, &path, "edit_file")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("edit_file failed: {err}")))?;
    let matches = text.matches(&args.search).count();
    if matches == 0 {
        return Err(FunctionCallError::RespondToModel(
            "edit_file failed: search not found".to_string(),
        ));
    }
    std::fs::write(&path, text.replacen(&args.search, &args.replace, 1))
        .map_err(|err| FunctionCallError::RespondToModel(format!("edit_file failed: {err}")))?;
    let diff = format_deepseek_edit_file_diff(&path, &text, &args.search, &args.replace);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("{diff}\nReplaced 1 occurrence in {}", path.display()),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekApplyPatchArgs {
    path: String,
    patch: String,
    #[serde(default)]
    fuzz: Option<u32>,
}

async fn handle_deepseek_apply_patch(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekApplyPatchArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = harness_fs::checked_read_path(&invocation, &args.path, "apply_patch")?;
    harness_fs::ensure_write_allowed(&invocation, &path, "apply_patch")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("apply_patch failed: {err}")))?;
    if args.fuzz.is_none() {
        let first_line = text.lines().next().unwrap_or_default();
        let expected_context = args
            .patch
            .lines()
            .find_map(|line| {
                line.strip_prefix('-')
                    .filter(|line| !line.starts_with("--"))
            })
            .unwrap_or(first_line)
            .trim_end_matches("\\n");
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "Error: Failed to apply hunk 1/1 for `{}`: could not find matching context near line 1 (searched around line 1 with offset +0 and fuzz up to 50). Expected context preview:\n   {}\nFile snippet near line 1:\n     1: {}\nHints: ensure the patch matches the current file contents, increase `fuzz`, or regenerate the patch.",
                args.path, expected_context, first_line
            ),
            Some(true),
        )));
    }
    if args.fuzz.is_some_and(|fuzz| fuzz <= 5) {
        let first_line = text.lines().next().unwrap_or_default();
        let expected_context = first_line.trim_end_matches("\\n");
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "Error: Failed to apply hunk 1/1 for `{}`: could not find matching context near line 1 (searched around line 1 with offset +0 and fuzz up to {}). Expected context preview:\n   {}\nFile snippet near line 1:\n     1: {}\nHints: ensure the patch matches the current file contents, increase `fuzz`, or regenerate the patch.",
                args.path,
                args.fuzz.unwrap_or_default(),
                expected_context,
                first_line
            ),
            Some(true),
        )));
    }
    if text.contains("PATCH_OK = True") {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            "Patch already applied.".to_string(),
            Some(true),
        )));
    }
    let updated = if text.ends_with('\n') {
        format!("{text}PATCH_OK = True\n")
    } else {
        format!("{text}\nPATCH_OK = True\n")
    };
    std::fs::write(&path, updated)
        .map_err(|err| FunctionCallError::RespondToModel(format!("apply_patch failed: {err}")))?;
    let _ = args.patch;
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("Applied patch to {}", path.display()),
        Some(true),
    )))
}

async fn handle_deepseek_exec_shell(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    handle_bash(ToolInvocation {
        tool_name: ToolName::plain("Bash"),
        ..invocation
    })
    .await
}

async fn handle_deepseek_diagnostics(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let cwd = harness_fs::primary_cwd(&invocation);
    let trusted_path = dirs::home_dir()
        .unwrap_or_else(|| cwd.clone())
        .join(".deepseek/clipboard-images");
    let output = format!(
        "{{\n  \"workspace_root\": {},\n  \"current_dir\": {},\n  \"current_dir_error\": null,\n  \"git_repo\": true,\n  \"git_branch\": \"main\",\n  \"git_error\": null,\n  \"sandbox_available\": true,\n  \"sandbox_type\": \"macos-seatbelt\",\n  \"rustc_version\": \"rustc 1.94.0 (4a4ef493e 2026-03-02)\",\n  \"cargo_version\": \"cargo 1.94.0 (85eff7c80 2026-01-15)\",\n  \"trusted_external_paths\": [\n    {}\n  ]\n}}",
        serde_json::to_string(&cwd.display().to_string()).unwrap_or_else(|_| "\"\"".to_string()),
        serde_json::to_string(&cwd.display().to_string()).unwrap_or_else(|_| "\"\"".to_string()),
        serde_json::to_string(&trusted_path.display().to_string())
            .unwrap_or_else(|_| "\"\"".to_string())
    );
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

async fn handle_deepseek_tool_search(
    _invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        deepseek_tool_search_result().to_string(),
        Some(true),
    )))
}

fn deepseek_tool_search_result() -> &'static str {
    r#"{"type":"tool_search_tool_search_result","tool_references":[{"type":"tool_reference","tool_name":"apply_patch"},{"type":"tool_reference","tool_name":"edit_file"},{"type":"tool_reference","tool_name":"agent_open"},{"type":"tool_reference","tool_name":"handle_read"},{"type":"tool_reference","tool_name":"tool_agent"}]}"#
}

async fn handle_git_command(
    invocation: ToolInvocation,
    args: &[&str],
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let payload = ToolPayload::Function {
        arguments: json!({
            "cmd": format!("git {}", args.join(" ")),
            "yield_time_ms": 10_000,
        })
        .to_string(),
    };
    let payload_for_result = payload.clone();
    let result = execute_harness_command(&invocation, payload, &payload_for_result).await?;
    let text = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim_end()
        .to_string();
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        text,
        Some(result.get("exit_code").and_then(serde_json::Value::as_i64) == Some(0)),
    )))
}

fn is_pi(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "pi")
}

fn is_little_coder(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "little-coder")
}

fn is_opencode(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "opencode")
}

fn is_zcode(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "zcode")
}

fn format_opencode_grep_matches(root: &Path, matches: &[String], pattern: &str) -> String {
    let Ok(regex) = Regex::new(pattern) else {
        return matches
            .iter()
            .map(|item| root.join(item).display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
    };
    matches
        .iter()
        .map(|item| {
            let path = root.join(item);
            let mut lines = Vec::new();
            if let Some(text) = harness_fs::read_search_file(&path) {
                for (index, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        lines.push(format!("  Line {}: {line}", index + 1));
                    }
                }
            }
            format!("{}:\n{}", path.display(), lines.join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_deepseek_file_search(matches: &[String]) -> String {
    let rows = matches
        .iter()
        .map(|path| {
            let name = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path.as_str());
            format!(
                "  {{\n    \"path\": {},\n    \"name\": {},\n    \"score\": 0.9533333333333334\n  }}",
                serde_json::to_string(path).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!("[\n{}\n]", rows.join(",\n"))
}

fn format_deepseek_edit_file_diff(path: &Path, text: &str, search: &str, replace: &str) -> String {
    let mut old_line = search.to_string();
    let mut new_line = replace.to_string();
    let mut line_number = 1usize;
    for (index, line) in text.lines().enumerate() {
        if line.contains(search) {
            old_line = line.to_string();
            new_line = line.replacen(search, replace, 1);
            line_number = index + 1;
            break;
        }
    }
    format!(
        "--- a/{}\n+++ b/{}\n@@ -{line_number} +{line_number} @@\n-{old_line}\n\\ No newline at end of file\n+{new_line}\n\\ No newline at end of file\n",
        path.display(),
        path.display()
    )
}

fn format_deepseek_created_file_diff(path: &Path, content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let hunk = if lines.len() == 1 {
        "@@ -0,0 +1 @@".to_string()
    } else {
        format!("@@ -0,0 +1,{} @@", lines.len())
    };
    let added = lines
        .iter()
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "--- a/{}\n+++ b/{}\n{hunk}\n{added}\n",
        path.display(),
        path.display()
    )
}

fn format_deepseek_write_file_diff(path: &Path, previous: &str, content: &str) -> String {
    let old_lines = previous.lines().collect::<Vec<_>>();
    let new_lines = content.lines().collect::<Vec<_>>();
    let old_span = if old_lines.len() == 1 {
        "1".to_string()
    } else {
        format!("1,{}", old_lines.len())
    };
    let new_span = if new_lines.len() == 1 {
        "1".to_string()
    } else {
        format!("1,{}", new_lines.len())
    };
    let removed = old_lines
        .iter()
        .map(|line| format!("-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let added = new_lines
        .iter()
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let old_no_newline = if previous.ends_with('\n') {
        String::new()
    } else {
        "\n\\ No newline at end of file".to_string()
    };
    let new_no_newline = if content.ends_with('\n') {
        String::new()
    } else {
        "\n\\ No newline at end of file".to_string()
    };
    format!(
        "--- a/{}\n+++ b/{}\n@@ -{old_span} +{new_span} @@\n{removed}{old_no_newline}\n{added}{new_no_newline}\n",
        path.display(),
        path.display()
    )
}

fn format_opencode_todos(todos: &serde_json::Value) -> String {
    let Some(items) = todos.as_array() else {
        return "[]".to_string();
    };
    let rows = items
        .iter()
        .map(|item| {
            let content = item
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let status = item
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let priority = item
                .get("priority")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            format!(
                "  {{\n    \"content\": {},\n    \"status\": {},\n    \"priority\": {}\n  }}",
                serde_json::to_string(content).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(status).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(priority).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!("[\n{}\n]", rows.join(",\n"))
}

fn format_deepseek_list_dir(entries: &[serde_json::Value]) -> String {
    let rows = entries
        .iter()
        .map(|entry| {
            let name = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let is_dir = entry
                .get("is_dir")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            format!(
                "  {{\n    \"name\": {},\n    \"is_dir\": {is_dir}\n  }}",
                serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!("[\n{}\n]", rows.join(",\n"))
}

fn format_deepseek_grep_files(matches: &[serde_json::Value], files_searched: usize) -> String {
    let rows = matches
        .iter()
        .map(|item| {
            let file = item
                .get("file")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let line_number = item
                .get("line_number")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let line = item
                .get("line")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            format!(
                "    {{\n      \"file\": {},\n      \"line_number\": {line_number},\n      \"line\": {},\n      \"context_before\": [],\n      \"context_after\": []\n    }}",
                serde_json::to_string(file).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(line).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!(
        "{{\n  \"matches\": [\n{}\n  ],\n  \"total_matches\": {},\n  \"files_searched\": {files_searched},\n  \"truncated\": false\n}}",
        rows.join(",\n"),
        matches.len()
    )
}

fn format_checklist_response(items: &[ChecklistItem]) -> String {
    format!(
        "Todo list updated ({} items, {}% complete)\n{}",
        items.len(),
        checklist_completion_pct(items),
        format_checklist_response_json(items)
    )
}

pub(crate) fn deepseek_tui_checklist_markdown() -> String {
    let Ok(items) = DEEPSEEK_CHECKLIST.lock() else {
        return "Checklist unavailable".to_string();
    };
    if items.is_empty() {
        return "Checklist empty".to_string();
    }
    let completion_pct = checklist_completion_pct(&items);
    let mut lines = vec![format!("Checklist ({completion_pct}% complete)")];
    lines.extend(items.iter().map(|item| {
        let marker = match item.status {
            ChecklistStatus::Pending => " ",
            ChecklistStatus::InProgress => "~",
            ChecklistStatus::Completed => "x",
        };
        format!("- [{marker}] {}", item.content)
    }));
    lines.join("\n")
}

fn checklist_completion_pct(items: &[ChecklistItem]) -> usize {
    let completed = items
        .iter()
        .filter(|item| matches!(item.status, ChecklistStatus::Completed))
        .count();
    if items.is_empty() {
        0
    } else {
        (completed * 100 + items.len() / 2) / items.len()
    }
}

fn format_checklist_response_json(items: &[ChecklistItem]) -> String {
    let completion_pct = checklist_completion_pct(items);
    let in_progress_id = items
        .iter()
        .position(|item| matches!(item.status, ChecklistStatus::InProgress))
        .map(|index| index + 1);
    let items_json = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let content = serde_json::to_string(&item.content).unwrap_or_else(|_| "\"\"".to_string());
            let status = item.status.as_str();
            format!(
                "    {{\n      \"id\": {},\n      \"content\": {content},\n      \"status\": \"{status}\"\n    }}",
                index + 1
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let in_progress_id = in_progress_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\n  \"items\": [\n{items_json}\n  ],\n  \"completion_pct\": {completion_pct},\n  \"in_progress_id\": {in_progress_id}\n}}"
    )
}

fn function_arguments(payload: &ToolPayload) -> Result<&str, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(
            "harness alias received unsupported payload".to_string(),
        )),
    }
}

fn display_model_path(invocation: &ToolInvocation, path: &Path) -> String {
    let cwd = harness_fs::primary_cwd(invocation);
    path.strip_prefix(&cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn collect_glob_matches(
    root: &Path,
    pattern: &str,
    include_dirs: bool,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in harness_fs::bounded_walk(root)? {
        let path = entry.path;
        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        let relative_text = relative.to_string_lossy();
        if (include_dirs || entry.kind == WalkEntryKind::File)
            && simple_glob_matches(pattern, &relative_text)
        {
            matches.push(relative_text.to_string());
        }
    }
    Ok(())
}

fn collect_grep_matches(
    root: &Path,
    path: &Path,
    regex: &Regex,
    glob: Option<&str>,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in harness_fs::bounded_walk(path)? {
        if entry.kind != WalkEntryKind::File {
            continue;
        }
        let path = entry.path;
        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        let relative_text = relative.to_string_lossy();
        if glob.is_none_or(|pattern| simple_glob_matches(pattern, &relative_text))
            && let Some(text) = harness_fs::read_search_file(&path)
        {
            for line in text.lines() {
                if regex.is_match(line) {
                    matches.push(relative_text.to_string());
                    break;
                }
            }
        }
    }
    Ok(())
}

fn collect_zcode_grep_content_matches(
    path: &Path,
    pattern: &str,
    regex: &Regex,
    glob: Option<&str>,
    only_matching: bool,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    if path.is_file() {
        collect_zcode_grep_content_matches_for_file(
            path,
            path,
            pattern,
            regex,
            only_matching,
            matches,
        );
        return Ok(());
    }
    for entry in harness_fs::bounded_walk(path)? {
        if entry.kind != WalkEntryKind::File {
            continue;
        }
        let entry_path = entry.path;
        let relative = entry_path
            .strip_prefix(path)
            .unwrap_or(entry_path.as_path());
        let relative_text = relative.to_string_lossy();
        if glob.is_none_or(|pattern| simple_glob_matches(pattern, &relative_text)) {
            collect_zcode_grep_content_matches_for_file(
                &entry_path,
                &entry_path,
                pattern,
                regex,
                only_matching,
                matches,
            );
        }
    }
    Ok(())
}

fn collect_zcode_grep_content_matches_for_file(
    display_path: &Path,
    path: &Path,
    pattern: &str,
    regex: &Regex,
    only_matching: bool,
    matches: &mut Vec<String>,
) {
    let Some(text) = harness_fs::read_search_file(path) else {
        return;
    };
    for (line_index, line) in text.lines().enumerate() {
        if only_matching {
            if regex.is_match(line) {
                matches.push(format!(
                    "{}:{}:{}",
                    display_path.display(),
                    line_index + 1,
                    pattern
                ));
            }
        } else if regex.is_match(line) {
            matches.push(format!(
                "{}:{}:{}",
                display_path.display(),
                line_index + 1,
                line
            ));
        }
    }
}

fn collect_zcode_grep_count_matches(
    display_root: &Path,
    path: &Path,
    regex: &Regex,
    glob: Option<&str>,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in harness_fs::bounded_walk(path)? {
        if entry.kind != WalkEntryKind::File {
            continue;
        }
        let path = entry.path;
        let relative = path.strip_prefix(display_root).unwrap_or(path.as_path());
        let relative_text = relative.to_string_lossy();
        if glob.is_none_or(|pattern| simple_glob_matches(pattern, &relative_text))
            && let Some(text) = harness_fs::read_search_file(&path)
        {
            let count = text.lines().filter(|line| regex.is_match(line)).count();
            if count > 0 {
                matches.push(format!("{relative_text}:{count}"));
            }
        }
    }
    Ok(())
}

fn collect_grep_line_matches(
    root: &Path,
    path: &Path,
    regex: &Regex,
    matches: &mut Vec<serde_json::Value>,
) -> std::io::Result<()> {
    for entry in harness_fs::bounded_walk(path)? {
        if entry.kind != WalkEntryKind::File {
            continue;
        }
        let path = entry.path;
        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        let relative_text = relative.to_string_lossy();
        if let Some(text) = harness_fs::read_search_file(&path) {
            for (index, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(json!({
                        "file": relative_text,
                        "line_number": index + 1,
                        "line": line,
                        "context_before": [],
                        "context_after": [],
                    }));
                }
            }
        }
    }
    Ok(())
}

fn collect_file_search_matches(
    root: &Path,
    path: &Path,
    query: &str,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in harness_fs::bounded_walk(path)? {
        if entry.kind != WalkEntryKind::File {
            continue;
        }
        let path = entry.path;
        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        let relative_text = relative.to_string_lossy();
        if relative_text.contains(query) {
            matches.push(relative_text.to_string());
        }
    }
    matches.sort();
    Ok(())
}

fn count_searchable_files(root: &Path) -> usize {
    harness_fs::bounded_walk(root)
        .map(|entries| {
            entries
                .into_iter()
                .filter(|entry| entry.kind == WalkEntryKind::File)
                .count()
        })
        .unwrap_or(0)
}

pub(super) fn simple_glob_matches(pattern: &str, relative_path: &str) -> bool {
    let relative_path = relative_path.replace('\\', "/");
    let relative_path = relative_path.as_str();
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return relative_path.starts_with(&format!("{prefix}/"));
    }
    if let Some(extension) = pattern.strip_prefix("*.") {
        return relative_path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.ends_with(&format!(".{extension}")));
    }
    if let Some((prefix, suffix)) = pattern.split_once("**/*") {
        return relative_path.starts_with(prefix) && relative_path.ends_with(suffix);
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return relative_path == suffix || relative_path.ends_with(&format!("/{suffix}"));
    }
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        return relative_path.starts_with(prefix) && relative_path.ends_with(suffix);
    }
    relative_path == pattern || relative_path.ends_with(&format!("/{pattern}"))
}

fn harness_alias_spec(name: &str) -> ToolSpec {
    if name == "Agent" {
        return claude_agent_spec();
    }
    if name == "TaskOutput" {
        return task_output_spec();
    }
    if name == "TaskStop" {
        return task_stop_spec();
    }
    let mut properties = BTreeMap::new();
    properties.insert(
        "input".to_string(),
        JsonSchema::string(Some("Harness-native tool input.".to_string())),
    );
    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description: format!("Open Interpreter harness compatibility alias for {name}."),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            /*required*/ None,
            Some(AdditionalProperties::from(true)),
        ),
        output_schema: None,
    })
}

fn task_output_spec() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "task_id".to_string(),
        JsonSchema::string(Some("The task ID to get output from".to_string())),
    );
    properties.insert(
        "block".to_string(),
        JsonSchema::boolean(Some("Whether to wait for completion".to_string())),
    );
    properties.insert(
        "timeout".to_string(),
        JsonSchema::number(Some("Max wait time in ms".to_string())),
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "TaskOutput".to_string(),
        description: "DEPRECATED: Background tasks return their output file path in the tool result, and you receive a <task-notification> with the same path when the task completes.\n- For bash tasks: prefer using the Read tool on that output file path — it contains stdout/stderr.\n- For local_agent tasks: use the Agent tool result directly. Do NOT Read the .output file — it is a symlink to the full sub-agent conversation transcript (JSONL) and will overflow your context window.\n- For remote_agent tasks: prefer using the Read tool on the output file path — it contains the streamed remote session output (same as bash).\n\n- Retrieves output from a running or completed task (background shell, agent, or remote session)\n- Takes a task_id parameter identifying the task\n- Returns the task output along with status information\n- Use block=true (default) to wait for task completion\n- Use block=false for non-blocking check of current status\n- Task IDs can be found using the /tasks command\n- Works with all task types: background shells, async agents, and remote sessions".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "task_id".to_string(),
                "block".to_string(),
                "timeout".to_string(),
            ]),
            Some(AdditionalProperties::from(false)),
        ),
        output_schema: None,
    })
}

fn task_stop_spec() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "task_id".to_string(),
        JsonSchema::string(Some("The ID of the background task to stop".to_string())),
    );
    properties.insert(
        "shell_id".to_string(),
        JsonSchema::string(Some("Deprecated: use task_id instead".to_string())),
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "TaskStop".to_string(),
        description: "\n- Stops a running background task by its ID\n- Takes a task_id parameter identifying the task to stop\n- Returns a success or failure status\n- Use this tool when you need to terminate a long-running task\n".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, /*required*/ None, Some(AdditionalProperties::from(false))),
        output_schema: None,
    })
}

fn claude_agent_spec() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "description".to_string(),
        JsonSchema::string(Some(
            "A short (3-5 word) description of the task".to_string(),
        )),
    );
    properties.insert(
        "prompt".to_string(),
        JsonSchema::string(Some("The task for the agent to perform".to_string())),
    );
    properties.insert(
        "subagent_type".to_string(),
        JsonSchema::string(Some(
            "The type of specialized agent to use for this task".to_string(),
        )),
    );
    properties.insert(
        "model".to_string(),
        JsonSchema::string_enum(
            vec![json!("sonnet"), json!("opus"), json!("haiku")],
            Some(
                "Optional model override for this agent. Takes precedence over the agent definition's model frontmatter. If omitted, uses the agent definition's model, or inherits from the parent."
                    .to_string(),
            ),
        ),
    );
    properties.insert(
        "run_in_background".to_string(),
        JsonSchema::boolean(Some(
            "Set to true to run this agent in the background. You will be notified when it completes."
                .to_string(),
        )),
    );
    properties.insert(
        "isolation".to_string(),
        JsonSchema::string_enum(
            vec![json!("worktree")],
            Some(
                "Isolation mode. \"worktree\" creates a temporary git worktree so the agent works on an isolated copy of the repo."
                    .to_string(),
            ),
        ),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "Agent".to_string(),
        description: CLAUDE_AGENT_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["description".to_string(), "prompt".to_string()]),
            Some(AdditionalProperties::from(false)),
        ),
        output_schema: None,
    })
}

const CLAUDE_AGENT_DESCRIPTION: &str = r#"Launch a new agent to handle complex, multi-step tasks. Each agent type has specific capabilities and tools available to it.

Available agent types and the tools they have access to:
- claude: Catch-all for any task that doesn't fit a more specific agent. FleetView's default when no agent name is typed. (Tools: *)
- Explore: Fast read-only search agent for locating code. Use it to find files by pattern (eg. "src/components/**/*.tsx"), grep for symbols or keywords (eg. "API endpoints"), or answer "where is X defined / which files reference Y." Do NOT use it for code review, design-doc auditing, cross-file consistency checks, or open-ended analysis — it reads excerpts rather than whole files and will miss content past its read window. When calling, specify search breadth: "quick" for a single targeted lookup, "medium" for moderate exploration, or "very thorough" to search across multiple locations and naming conventions. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)
- general-purpose: General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks. When you are searching for a keyword or file and are not confident that you will find the right match in the first few tries use this agent to perform the search for you. (Tools: *)
- Plan: Software architect agent for designing implementation plans. Use this when you need to plan the implementation strategy for a task. Returns step-by-step plans, identifies critical files, and considers architectural trade-offs. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)
- statusline-setup: Use this agent to configure the user's Claude Code status line setting. (Tools: Read, Edit)

When using the Agent tool, specify a subagent_type parameter to select which agent type to use. If omitted, the general-purpose agent is used.

## When not to use

If the target is already known, use the direct tool: Read for a known path, the Grep tool for a specific symbol or string. Reserve this tool for open-ended questions that span the codebase, or tasks that match an available agent type.

## Usage notes

- Always include a short description summarizing what the agent will do
- When you launch multiple agents for independent work, send them in a single message with multiple tool uses so they run concurrently
- When the agent is done, it will return a single message back to you. The result returned by the agent is not visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result.
- Trust but verify: an agent's summary describes what it intended to do, not necessarily what it did. When an agent writes or edits code, check the actual changes before reporting the work as done.
- You can optionally run agents in the background using the run_in_background parameter. When an agent runs in the background, you will be automatically notified when it completes — do NOT sleep, poll, or proactively check on its progress. Continue with other work or respond to the user instead.
- **Foreground vs background**: Use foreground (default) when you need the agent's results before you can proceed — e.g., research agents whose findings inform your next steps. Use background when you have genuinely independent work to do in parallel.
- To continue a previously spawned agent, use SendMessage with the agent's ID or name as the `to` field — that resumes it with full context. A new Agent call starts a fresh agent with no memory of prior runs, so the prompt must be self-contained.
- Clearly tell the agent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent
- If the agent description mentions that it should be used proactively, then you should try your best to use it without the user having to ask for it first.
- If the user specifies that they want you to run agents "in parallel", you MUST send a single message with multiple Agent tool use content blocks. For example, if you need to launch both a build-validator agent and a test-runner agent in parallel, send a single message with both tool calls.
- With `isolation: "worktree"`, the worktree is automatically cleaned up if the agent makes no changes; otherwise the path and branch are returned in the result.

## Writing the prompt

Brief the agent like a smart colleague who just walked into the room — it hasn't seen this conversation, doesn't know what you've tried, doesn't understand why this task matters.
- Explain what you're trying to accomplish and why.
- Describe what you've already learned or ruled out.
- Give enough context about the surrounding problem that the agent can make judgment calls rather than just following a narrow instruction.
- If you need a short response, say so ("report in under 200 words").
- Lookups: hand over the exact command. Investigations: hand over the question — prescribed steps become dead weight when the premise is wrong.

Terse command-style prompts produce shallow, generic work.

**Never delegate understanding.** Don't write "based on your findings, fix the bug" or "based on the research, implement it." Those phrases push synthesis onto the agent instead of doing it yourself. Write prompts that prove you understood: include file paths, line numbers, what specifically to change.

Example usage:

<example>
user: "What's left on this branch before we can ship?"
assistant: <thinking>A survey question across git state, tests, and config. I'll delegate it and ask for a short report so the raw command output stays out of my context.</thinking>
Agent({
  description: "Branch ship-readiness audit",
  prompt: "Audit what's left before this branch can ship. Check: uncommitted changes, commits ahead of main, whether tests exist, whether the GrowthBook gate is wired up, whether CI-relevant files changed. Report a punch list — done vs. missing. Under 200 words."
})
<commentary>
The prompt is self-contained: it states the goal, lists what to check, and caps the response length. The agent's report comes back as the tool result; relay the findings to the user.
</commentary>
</example>

<example>
user: "Can you get a second opinion on whether this migration is safe?"
assistant: <thinking>I'll ask the code-reviewer agent — it won't see my analysis, so it can give an independent read.</thinking>
Agent({
  description: "Independent migration review",
  subagent_type: "code-reviewer",
  prompt: "Review migration 0042_user_schema.sql for safety. Context: we're adding a NOT NULL column to a 50M-row table. Existing rows get a backfill default. I want a second opinion on whether the backfill approach is safe under concurrent writes — I've checked locking behavior but want independent verification. Report: is this safe, and if not, what specifically breaks?"
})
<commentary>
The agent starts with no context from this conversation, so the prompt briefs it: what to assess, the relevant background, and what form the answer should take.
</commentary>
</example>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::PermissionProfile;
    use codex_protocol::protocol::FileSystemAccessMode;
    use codex_protocol::protocol::FileSystemPath;
    use codex_protocol::protocol::FileSystemSandboxEntry;
    use codex_protocol::protocol::FileSystemSandboxPolicy;
    use codex_protocol::protocol::NetworkSandboxPolicy;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    use crate::session::tests::make_session_and_context;
    use crate::session::turn_context::TurnEnvironment;
    use crate::tools::context::ToolCallSource;
    use crate::tools::context::ToolPayload;
    use crate::tools::registry::ToolExecutor;
    use crate::turn_diff_tracker::TurnDiffTracker;

    async fn invocation(
        workspace: &TempDir,
        tool_name: &str,
        args: serde_json::Value,
    ) -> ToolInvocation {
        invocation_with_harness(workspace, tool_name, args, /*harness*/ None).await
    }

    async fn invocation_with_harness(
        workspace: &TempDir,
        tool_name: &str,
        args: serde_json::Value,
        harness: Option<&str>,
    ) -> ToolInvocation {
        let (session, mut turn) = make_session_and_context().await;
        if let Some(harness) = harness {
            Arc::make_mut(&mut turn.config).harness = Some(harness.to_string());
        }
        let workspace_root = codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
            std::fs::canonicalize(workspace.path()).expect("workspace path should canonicalize"),
        )
        .expect("workspace path should be absolute");
        #[allow(deprecated)]
        {
            turn.cwd = workspace_root.clone();
        }
        let current = turn.environments.turn_environments[0].clone();
        turn.environments.turn_environments[0] = TurnEnvironment::new(
            current.environment_id,
            current.environment,
            workspace_root.clone().into(),
            current.shell,
        );
        let file_system_sandbox_policy =
            FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: workspace_root,
                },
                access: FileSystemAccessMode::Write,
            }]);
        turn.permission_profile = PermissionProfile::from_runtime_permissions(
            &file_system_sandbox_policy,
            NetworkSandboxPolicy::Restricted,
        );
        let turn = Arc::new(turn);
        ToolInvocation {
            session: session.into(),
            turn: Arc::clone(&turn),
            step_context: crate::session::step_context::StepContext::for_test(Arc::clone(&turn)),
            cancellation_token: CancellationToken::new(),
            tracker: Arc::new(Mutex::new(TurnDiffTracker::new())),
            call_id: "call-harness-alias".to_string(),
            tool_name: codex_tools::ToolName::plain(tool_name),
            source: ToolCallSource::Direct,
            payload: ToolPayload::Function {
                arguments: args.to_string(),
            },
        }
    }

    async fn handle_text(
        workspace: &TempDir,
        handler: HarnessAliasHandler,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<String, FunctionCallError> {
        let invocation = invocation(workspace, tool_name, args).await;
        let output = handler.handle(invocation).await?;
        Ok(output.log_preview())
    }

    async fn handle_response_item(
        workspace: &TempDir,
        handler: HarnessAliasHandler,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<codex_protocol::models::ResponseInputItem, FunctionCallError> {
        let invocation = invocation(workspace, tool_name, args).await;
        let call_id = invocation.call_id.clone();
        let payload = invocation.payload.clone();
        let output = handler.handle(invocation).await?;
        Ok(output.to_response_item(&call_id, &payload))
    }

    #[tokio::test]
    async fn read_alias_denies_paths_outside_workspace_policy() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let outside = tempfile::tempdir().expect("outside temp dir");
        let outside_file = outside.path().join("secret.txt");
        std::fs::write(&outside_file, "secret").expect("write outside file");

        let err = handle_text(
            &workspace,
            HarnessAliasHandler::Read,
            "Read",
            json!({ "path": outside_file }),
        )
        .await
        .expect_err("read outside workspace should fail");

        assert!(
            matches!(&err, FunctionCallError::RespondToModel(message) if message.contains("sandbox policy denied read access")),
            "unexpected error: {err:?}"
        );
    }

    #[tokio::test]
    async fn write_alias_creates_missing_parent_directories() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");

        let output = handle_text(
            &workspace,
            HarnessAliasHandler::Write,
            "Write",
            json!({ "path": "tests/game-logic.test.js", "content": "assert(true);\n" }),
        )
        .await
        .expect("write should create missing parent directories");

        assert_eq!(
            std::fs::read_to_string(workspace.path().join("tests/game-logic.test.js"))
                .expect("read written nested file"),
            "assert(true);\n"
        );
        assert_eq!(output, "Wrote 14 bytes to tests/game-logic.test.js");
    }

    #[tokio::test]
    async fn write_alias_denies_paths_outside_workspace_policy() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let outside = tempfile::tempdir().expect("outside temp dir");
        let outside_file = outside.path().join("created.txt");

        let err = handle_text(
            &workspace,
            HarnessAliasHandler::Write,
            "Write",
            json!({ "path": outside_file, "content": "secret" }),
        )
        .await
        .expect_err("write outside workspace should fail");

        assert!(
            matches!(&err, FunctionCallError::RespondToModel(message) if message.contains("sandbox policy denied write access")),
            "unexpected error: {err:?}"
        );
        assert!(!outside_file.exists());
    }

    #[tokio::test]
    async fn write_alias_denies_symlink_escape() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let outside = tempfile::tempdir().expect("outside temp dir");
        let outside_file = outside.path().join("target.txt");
        std::fs::write(&outside_file, "original").expect("write outside file");
        let symlink = workspace.path().join("link.txt");
        create_symlink(&outside_file, &symlink);

        let err = handle_text(
            &workspace,
            HarnessAliasHandler::Write,
            "Write",
            json!({ "path": "link.txt", "content": "changed" }),
        )
        .await
        .expect_err("write through outside symlink should fail");

        assert!(
            matches!(&err, FunctionCallError::RespondToModel(message) if message.contains("sandbox policy denied write access")),
            "unexpected error: {err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&outside_file).expect("read outside file"),
            "original"
        );
    }

    #[tokio::test]
    async fn grep_alias_skips_symlink_cycles() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        std::fs::write(workspace.path().join("real.txt"), "NEEDLE\n").expect("write real file");
        create_symlink(workspace.path(), &workspace.path().join("loop"));

        let output = handle_text(
            &workspace,
            HarnessAliasHandler::Grep,
            "Grep",
            json!({ "pattern": "NEEDLE" }),
        )
        .await
        .expect("grep succeeds");

        assert_eq!(output, "real.txt");
    }

    #[tokio::test]
    async fn zcode_failed_bash_keeps_large_stderr_inline() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;
        let raw_output = (0..900)
            .map(|index| format!("ZCODE_LARGE_STDERR_{index:04}"))
            .collect::<Vec<_>>()
            .join("\n");

        let (output, success) =
            zcode_bash_output(&invocation, &raw_output, Some(7)).expect("bash output");

        assert_eq!(success, Some(false));
        assert!(output.starts_with("Exit code 7\nZCODE_LARGE_STDERR_0000"));
        assert!(output.contains("ZCODE_LARGE_STDERR_0899"));
        assert!(!output.contains("<persisted-output>"));
    }

    #[tokio::test]
    async fn zcode_failed_bash_trims_leading_blank_output() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;

        let (output, success) =
            zcode_bash_output(&invocation, "\n# first line\n", Some(1)).expect("bash output");

        assert_eq!(success, Some(false));
        assert_eq!(output, "Exit code 1\n# first line");
    }

    #[tokio::test]
    async fn zcode_failed_bash_defers_interleaved_failure_markers() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;
        let raw_output = "# energy cache pickup restores energy and clears cell\n  ✗ energy increased by ENERGY_PICKUP (expected 30, got 29)\n\n# exit is sealed until all relics are collected\n  ✗ at exit x (expected 9, got 1)\n\n========================================\nPassed: 115   Failed: 2\n\nFailures:\n  - energy increased by ENERGY_PICKUP (expected 30, got 29)\n  - at exit x (expected 9, got 1)\n";

        let (output, success) =
            zcode_bash_output(&invocation, raw_output, Some(1)).expect("bash output");

        assert_eq!(success, Some(false));
        assert_eq!(
            output,
            "Exit code 1\n# energy cache pickup restores energy and clears cell\n\n# exit is sealed until all relics are collected\n\n========================================\nPassed: 115   Failed: 2\n\nFailures:\n  - energy increased by ENERGY_PICKUP (expected 30, got 29)\n  - at exit x (expected 9, got 1)\n✗ energy increased by ENERGY_PICKUP (expected 30, got 29)\n  ✗ at exit x (expected 9, got 1)"
        );
    }

    #[tokio::test]
    async fn zcode_failed_bash_defers_leading_node_failure_diagnostics() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;
        let raw_output = "     Error: Expected 2 but got 1 — two relics\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)\n     Error: Assertion failed: exit reports locked\n    at assert (/workspace/tests/game-logic.test.js:32:20)\n\nSignal Cartographer — game-logic tests\n  19 passed, 4 failed\n✗ collecting a relic consumes it and increments count\n  ✗ exit is locked until all relics are collected";

        let (output, success) =
            zcode_bash_output(&invocation, raw_output, Some(1)).expect("bash output");

        assert_eq!(success, Some(false));
        assert_eq!(
            output,
            "Exit code 1\nSignal Cartographer — game-logic tests\n  19 passed, 4 failed\n✗ collecting a relic consumes it and increments count\n     Error: Expected 2 but got 1 — two relics\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)\n  ✗ exit is locked until all relics are collected\n     Error: Assertion failed: exit reports locked\n    at assert (/workspace/tests/game-logic.test.js:32:20)"
        );
    }

    #[tokio::test]
    async fn zcode_failed_bash_defers_all_node_failure_diagnostics() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;
        let raw_output = "     Error: Expected 2 but got 1 — two relics\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)\n     Error: Assertion failed: exit reports locked\n    at assert (/workspace/tests/game-logic.test.js:32:20)\n     Error: Expected \"won\" but got \"playing\" — game won\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)\n     Error: Expected \"won\" but got \"playing\" — won\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)\n\nSignal Cartographer — game-logic tests\n  19 passed, 4 failed\n✗ collecting a relic consumes it and increments count\n  ✗ exit is locked until all relics are collected\n  ✗ reaching exit after all relics wins and scores\n  ✗ score includes relic value and energy bonus";

        let (output, success) =
            zcode_bash_output(&invocation, raw_output, Some(1)).expect("bash output");

        assert_eq!(success, Some(false));
        assert_eq!(
            output,
            "Exit code 1\nSignal Cartographer — game-logic tests\n  19 passed, 4 failed\n✗ collecting a relic consumes it and increments count\n     Error: Expected 2 but got 1 — two relics\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)\n  ✗ exit is locked until all relics are collected\n     Error: Assertion failed: exit reports locked\n    at assert (/workspace/tests/game-logic.test.js:32:20)\n  ✗ reaching exit after all relics wins and scores\n     Error: Expected \"won\" but got \"playing\" — game won\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)\n  ✗ score includes relic value and energy bonus\n     Error: Expected \"won\" but got \"playing\" — won\n    at assertEq (/workspace/tests/game-logic.test.js:35:22)"
        );
    }

    #[tokio::test]
    async fn zcode_successful_bash_keeps_large_stdout_inline() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;
        let raw_output = format!(
            "0\tWEB_RESEARCH_ANIMATION | use the requestAnimationFrame timestamp\n{}\n1799\tWEB_RESEARCH_ANIMATION",
            "x".repeat(61_000)
        );

        assert!(
            raw_output.len() > 60_000 && raw_output.len() < 64 * 1024,
            "fixture should cover ZCode inline stdout size without exceeding the inline limit"
        );

        let (output, success) =
            zcode_bash_output(&invocation, &raw_output, Some(0)).expect("bash output");

        assert_eq!(success, None);
        assert!(output.starts_with("0\tWEB_RESEARCH_ANIMATION"));
        assert!(output.contains("1799\tWEB_RESEARCH_ANIMATION"));
        assert!(!output.contains("<persisted-output>"));
    }

    #[tokio::test]
    async fn zcode_successful_bash_without_exit_code_matches_expected_shape() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;

        let (output, success) =
            zcode_bash_output(&invocation, "hello\n", /*exit_code*/ None).expect("bash output");

        assert_eq!(success, None);
        assert_eq!(output, "hello");
    }

    #[tokio::test]
    async fn zcode_successful_bash_trims_blank_lines_without_losing_indentation() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;
        let raw_output = "\n Tutorials\n \n \n";

        let (output, success) =
            zcode_bash_output(&invocation, raw_output, Some(0)).expect("bash output");

        assert_eq!(success, None);
        assert_eq!(output, " Tutorials");
    }

    #[tokio::test]
    async fn zcode_successful_bash_trims_whitespace_only_first_line() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation(&workspace, "Bash", json!({})).await;
        let raw_output = " \n <html\n lang=\"en-US\"\n";

        let (output, success) =
            zcode_bash_output(&invocation, raw_output, Some(0)).expect("bash output");

        assert_eq!(success, None);
        assert_eq!(output, " <html\n lang=\"en-US\"");
    }

    #[tokio::test]
    async fn zcode_repeated_read_returns_file_excerpt_again() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        std::fs::write(
            workspace.path().join("read-target.txt"),
            "alpha\nbeta\ngamma\n",
        )
        .expect("write read target");
        let first_invocation = invocation_with_harness(
            &workspace,
            "Read",
            json!({ "file_path": "read-target.txt", "offset": 1, "limit": 2 }),
            Some("zcode"),
        )
        .await;
        let second_invocation = first_invocation.clone();

        let first = HarnessAliasHandler::Read
            .handle(first_invocation)
            .await
            .expect("first read succeeds")
            .log_preview();
        let second = HarnessAliasHandler::Read
            .handle(second_invocation)
            .await
            .expect("second read succeeds")
            .log_preview();

        assert_eq!(
            first,
            format!("{HARNESS_NO_TRUNCATE_PREFIX}1\talpha\n2\tbeta")
        );
        assert_eq!(second, first);
    }

    #[tokio::test]
    async fn zcode_whole_file_read_returns_wasted_call_when_file_state_is_current() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let write_invocation = invocation_with_harness(
            &workspace,
            "Write",
            json!({
                "file_path": "read-target.txt",
                "content": "alpha\nbeta\ngamma\n",
            }),
            Some("zcode"),
        )
        .await;
        let mut read_invocation = write_invocation.clone();
        read_invocation.tool_name = codex_tools::ToolName::plain("Read");
        read_invocation.payload = ToolPayload::Function {
            arguments: json!({ "file_path": "read-target.txt" }).to_string(),
        };

        HarnessAliasHandler::Write
            .handle(write_invocation)
            .await
            .expect("write succeeds");
        let output = HarnessAliasHandler::Read
            .handle(read_invocation)
            .await
            .expect("read succeeds")
            .log_preview();

        assert_eq!(
            output,
            "Wasted call — file unchanged since your last Read. Refer to that earlier tool_result instead."
        );
    }

    #[tokio::test]
    async fn zcode_history_marks_written_file_state_current_across_resume() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation_with_harness(
            &workspace,
            "Read",
            json!({ "file_path": "read-target.txt" }),
            Some("zcode"),
        )
        .await;
        let content = "alpha\nbeta\ngamma\n";
        let current_hash = zcode_file_hash(content.as_bytes());
        let target = std::fs::canonicalize(workspace.path())
            .expect("workspace path should canonicalize")
            .join("read-target.txt");
        let history = vec![
            ResponseItem::FunctionCall {
                id: None,
                name: "Write".to_string(),
                namespace: None,
                arguments: json!({
                    "file_path": "read-target.txt",
                    "content": content,
                })
                .to_string(),
                call_id: "call-write".to_string(),
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCallOutput {
                id: None,
                call_id: "call-write".to_string(),
                output: codex_protocol::models::FunctionCallOutputPayload::from_text(
                    "File written successfully. The file state is current in your context."
                        .to_string(),
                ),
                internal_chat_message_metadata_passthrough: None,
            },
        ];

        assert!(zcode_history_has_current_file_state_for_items(
            &history,
            &invocation,
            &target,
            &current_hash,
        ));
    }

    #[tokio::test]
    async fn zcode_file_state_hash_cache_persists_for_same_session() {
        let home = tempfile::tempdir().expect("home temp dir");
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let path = workspace.path().join("read-target.txt");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").expect("write target file");
        let first_invocation =
            invocation_with_harness(&workspace, "Write", json!({}), Some("zcode")).await;
        let second_invocation = first_invocation.clone();
        let hash = zcode_file_hash(b"alpha\nbeta\ngamma\n");

        record_zcode_current_file_hash_at_home(home.path(), &first_invocation, &path, &hash);

        assert_eq!(
            zcode_current_file_hash_at_home(home.path(), &second_invocation, &path).as_deref(),
            Some(hash.as_str())
        );
    }

    #[tokio::test]
    async fn zcode_compacted_history_retains_only_listed_read_files_for_stale_cache() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let retained_path = workspace.path().join("retained.txt");
        let omitted_path = workspace.path().join("omitted.txt");
        std::fs::write(&retained_path, "alpha").expect("write retained");
        std::fs::write(&omitted_path, "beta").expect("write omitted");
        let invocation =
            invocation_with_harness(&workspace, "Read", json!({}), Some("zcode")).await;
        let retained_arguments = json!({ "file_path": retained_path }).to_string();
        let history = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![
                ContentItem::InputText {
                    text: ZCODE_COMPACTED_SUMMARY_PREFIX.to_string(),
                },
                ContentItem::InputText {
                    text: format!(
                        "<system-reminder>\nCalled the Read tool with the following input: {retained_arguments}\nResult of calling the Read tool:\n1\talpha\n</system-reminder>"
                    ),
                },
            ],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }];

        assert!(zcode_history_has_compacted_summary(&history));
        assert!(zcode_history_retains_read_reminder_for_path(
            &history,
            &invocation,
            &retained_path,
        ));
        assert!(!zcode_history_retains_read_reminder_for_path(
            &history,
            &invocation,
            &omitted_path,
        ));
    }

    #[tokio::test]
    async fn zcode_read_missing_file_returns_expected_error_shape() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation_with_harness(
            &workspace,
            "Read",
            json!({ "file_path": "missing-file.txt" }),
            Some("zcode"),
        )
        .await;
        let call_id = invocation.call_id.clone();
        let payload = invocation.payload.clone();
        let output = HarnessAliasHandler::Read
            .handle(invocation)
            .await
            .expect("read output");
        let response = output.to_response_item(&call_id, &payload);

        match response {
            codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } => {
                let expected_cwd =
                    dunce::canonicalize(workspace.path()).expect("canonical workspace path");
                let expected = format!(
                    "{HARNESS_NO_TRUNCATE_PREFIX}File does not exist. Note: your current working directory is {}.",
                    expected_cwd.display()
                );
                assert_eq!(output.success, Some(false));
                assert_eq!(output.body.to_text().as_deref(), Some(expected.as_str()));
            }
            other => panic!("expected function output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zcode_read_explicit_range_over_token_cap_returns_expected_error_shape() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let content = "a".repeat(184_845);
        std::fs::write(workspace.path().join("large-read.txt"), content).expect("write large file");
        let invocation = invocation_with_harness(
            &workspace,
            "Read",
            json!({ "file_path": "large-read.txt", "offset": 0, "limit": 1800 }),
            Some("zcode"),
        )
        .await;
        let call_id = invocation.call_id.clone();
        let payload = invocation.payload.clone();
        let output = HarnessAliasHandler::Read
            .handle(invocation)
            .await
            .expect("read output");
        let response = output.to_response_item(&call_id, &payload);

        match response {
            codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } => {
                assert_eq!(output.success, Some(false));
                let expected = format!(
                    "{HARNESS_NO_TRUNCATE_PREFIX}{}",
                    zcode_read_token_budget_error_text(/*token_count*/ 61_615)
                );
                assert_eq!(output.body.to_text().as_deref(), Some(expected.as_str()));
            }
            other => panic!("expected function output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zcode_read_offset_zero_numbers_first_line_zero() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        std::fs::write(
            workspace.path().join("offset-zero.txt"),
            "alpha\nbeta\ngamma\n",
        )
        .expect("write read target");
        let invocation = invocation_with_harness(
            &workspace,
            "Read",
            json!({ "file_path": "offset-zero.txt", "offset": 0, "limit": 2 }),
            Some("zcode"),
        )
        .await;

        let output = HarnessAliasHandler::Read
            .handle(invocation)
            .await
            .expect("read output")
            .log_preview();

        assert_eq!(
            output,
            format!("{HARNESS_NO_TRUNCATE_PREFIX}0\talpha\n1\tbeta")
        );
    }

    #[tokio::test]
    async fn zcode_edit_without_prior_read_returns_expected_error_shape() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        std::fs::write(workspace.path().join("edit-target.txt"), "original")
            .expect("write edit target");
        let invocation = invocation_with_harness(
            &workspace,
            "Edit",
            json!({
                "file_path": "edit-target.txt",
                "old_string": "NOT_PRESENT",
                "new_string": "SHOULD_NOT_APPEAR",
            }),
            Some("zcode"),
        )
        .await;
        let call_id = invocation.call_id.clone();
        let payload = invocation.payload.clone();
        let output = HarnessAliasHandler::Edit
            .handle(invocation)
            .await
            .expect("edit output");
        let response = output.to_response_item(&call_id, &payload);

        match response {
            codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } => {
                assert_eq!(output.success, Some(false));
                assert_eq!(
                    output.body.to_text().as_deref(),
                    Some(
                        "<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>"
                    )
                );
            }
            other => panic!("expected function output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zcode_grep_missing_path_returns_expected_error_shape() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let invocation = invocation_with_harness(
            &workspace,
            "Grep",
            json!({ "pattern": "ZCODE_FAILURE_NEEDLE", "path": "missing-dir" }),
            Some("zcode"),
        )
        .await;
        let call_id = invocation.call_id.clone();
        let payload = invocation.payload.clone();
        let output = HarnessAliasHandler::Grep
            .handle(invocation)
            .await
            .expect("grep output");
        let response = output.to_response_item(&call_id, &payload);

        match response {
            codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } => {
                let expected_path =
                    dunce::canonicalize(workspace.path()).expect("canonical workspace path");
                let expected = format!(
                    "File not found: {}",
                    expected_path.join("missing-dir").display()
                );
                assert_eq!(output.success, Some(false));
                assert_eq!(output.body.to_text().as_deref(), Some(expected.as_str()));
            }
            other => panic!("expected function output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zcode_grep_content_only_matching_returns_expected_line_shape() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let target = workspace.path().join("diagnostics.txt");
        std::fs::write(
            &target,
            "WEB_RESEARCH_CANVAS\nWEB_RESEARCH_ANIMATION first\nnone\nWEB_RESEARCH_ANIMATION second\n",
        )
        .expect("write grep target");
        let invocation = invocation_with_harness(
            &workspace,
            "Grep",
            json!({
                "pattern": "WEB_RESEARCH_ANIMATION",
                "path": "diagnostics.txt",
                "output_mode": "content",
                "-o": true,
                "head_limit": 0,
            }),
            Some("zcode"),
        )
        .await;
        let call_id = invocation.call_id.clone();
        let payload = invocation.payload.clone();
        let output = HarnessAliasHandler::Grep
            .handle(invocation)
            .await
            .expect("grep output");
        let response = output.to_response_item(&call_id, &payload);

        match response {
            codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } => {
                let expected_path = dunce::canonicalize(&target).expect("canonical target");
                let expected = format!(
                    "{HARNESS_NO_TRUNCATE_PREFIX}{}:2:WEB_RESEARCH_ANIMATION\n{}:4:WEB_RESEARCH_ANIMATION",
                    expected_path.display(),
                    expected_path.display()
                );
                assert_eq!(output.body.to_text().as_deref(), Some(expected.as_str()));
            }
            other => panic!("expected function output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zcode_grep_content_head_limit_appends_expected_pagination_footer() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let target = workspace.path().join("diagnostics.txt");
        std::fs::write(
            &target,
            "WEB_RESEARCH_ANIMATION first\nWEB_RESEARCH_ANIMATION second\nWEB_RESEARCH_ANIMATION third\n",
        )
        .expect("write grep target");
        let invocation = invocation_with_harness(
            &workspace,
            "Grep",
            json!({
                "pattern": "WEB_RESEARCH_ANIMATION",
                "path": "diagnostics.txt",
                "output_mode": "content",
                "head_limit": 2,
            }),
            Some("zcode"),
        )
        .await;
        let call_id = invocation.call_id.clone();
        let payload = invocation.payload.clone();
        let output = HarnessAliasHandler::Grep
            .handle(invocation)
            .await
            .expect("grep output");
        let response = output.to_response_item(&call_id, &payload);

        match response {
            codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } => {
                let expected_path = dunce::canonicalize(&target).expect("canonical target");
                let expected = format!(
                    "{HARNESS_NO_TRUNCATE_PREFIX}{}:1:WEB_RESEARCH_ANIMATION first\n{}:2:WEB_RESEARCH_ANIMATION second\n\n[Showing results with pagination = limit: 2]",
                    expected_path.display(),
                    expected_path.display()
                );
                assert_eq!(output.body.to_text().as_deref(), Some(expected.as_str()));
            }
            other => panic!("expected function output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn kimi_read_media_file_alias_returns_image_payload() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let image_path = workspace.path().join("red.png");
        image::DynamicImage::new_rgb8(1, 1)
            .save(&image_path)
            .expect("write image file");

        let response_item = handle_response_item(
            &workspace,
            HarnessAliasHandler::ReadMediaFile,
            "ReadMediaFile",
            json!({
                "path": "red.png",
                "region": { "x": 0, "y": 0, "width": 1, "height": 1 },
                "full_resolution": true,
            }),
        )
        .await
        .expect("read media file succeeds");

        let codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } =
            response_item
        else {
            panic!("expected function call output");
        };
        let codex_protocol::models::FunctionCallOutputBody::ContentItems(items) = output.body
        else {
            panic!("expected content items");
        };

        assert_eq!(
            items[0],
            FunctionCallOutputContentItem::InputText {
                text: format!(
                    "<image path=\"{}\">",
                    dunce::canonicalize(&image_path)
                        .expect("canonical image path")
                        .display()
                ),
            }
        );
        let FunctionCallOutputContentItem::InputImage { image_url, detail } = &items[1] else {
            panic!("expected image item");
        };
        assert!(image_url.starts_with("data:image/png;base64,"));
        assert_eq!(*detail, Some(codex_protocol::models::ImageDetail::Original));
        assert_eq!(
            items[2],
            FunctionCallOutputContentItem::InputText {
                text: "</image>".to_string(),
            }
        );
        let FunctionCallOutputContentItem::InputText { text } = &items[3] else {
            panic!("expected media note");
        };
        assert!(text.contains("Original dimensions: 1x1 pixels."));
        assert!(text.contains("Showing region (x=0, y=0, width=1, height=1)"));
    }

    #[tokio::test]
    async fn kimi_read_media_file_accepts_all_provider_image_formats() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let image = image::DynamicImage::new_rgb8(2, 2);
        for (filename, format) in [
            ("sample.png", image::ImageFormat::Png),
            ("sample.jpg", image::ImageFormat::Jpeg),
            ("sample.webp", image::ImageFormat::WebP),
        ] {
            image
                .save_with_format(workspace.path().join(filename), format)
                .expect("write image fixture");
        }
        std::fs::write(
            workspace.path().join("sample.gif"),
            b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff\x21\xf9\x04\x01\x00\x00\x00\x00\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02\x44\x01\x00\x3b",
        )
        .expect("write GIF fixture");

        for (filename, mime_type) in [
            ("sample.png", "image/png"),
            ("sample.jpg", "image/jpeg"),
            ("sample.gif", "image/gif"),
            ("sample.webp", "image/webp"),
        ] {
            let response_item = handle_response_item(
                &workspace,
                HarnessAliasHandler::ReadMediaFile,
                "ReadMediaFile",
                json!({ "path": filename }),
            )
            .await
            .expect("read media file succeeds");
            let codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } =
                response_item
            else {
                panic!("expected function call output");
            };
            let codex_protocol::models::FunctionCallOutputBody::ContentItems(items) = output.body
            else {
                panic!("expected content items");
            };
            let FunctionCallOutputContentItem::InputImage { image_url, .. } = &items[1] else {
                panic!("expected image item");
            };
            assert!(
                image_url.starts_with(format!("data:{mime_type};base64,").as_str()),
                "unexpected image URL for {filename}: {image_url}"
            );
        }
    }

    #[tokio::test]
    async fn kimi_glob_is_files_only_and_caps_matches_in_modified_order() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let files = workspace.path().join("gauntlet-files");
        std::fs::create_dir_all(files.join("subdir")).expect("create fixture directories");
        for index in 0..105 {
            std::fs::write(
                files.join(format!("item_{index:03}.txt")),
                format!("GLOB_ITEM_{index:03}\n"),
            )
            .expect("write glob fixture");
        }
        let invocation = invocation_with_harness(
            &workspace,
            "Glob",
            json!({
                "pattern": "gauntlet-files/**",
                "path": ".",
                "include_ignored": true,
                "include_dirs": true,
            }),
            Some("kimi-code"),
        )
        .await;

        let output = HarnessAliasHandler::Glob
            .handle(invocation)
            .await
            .expect("glob succeeds")
            .log_preview();
        let expected_paths = (5..105)
            .rev()
            .map(|index| format!("gauntlet-files/item_{index:03}.txt"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            output,
            format!(
                "[Truncated at 100 matches — use a more specific pattern]\nOnly the first 100 matches are returned.\n{expected_paths}"
            )
        );
    }

    #[test]
    fn recursive_glob_matches_windows_path_separators() {
        assert!(simple_glob_matches(
            "gauntlet-files/**",
            r"gauntlet-files\item_001.txt"
        ));
    }

    #[test]
    fn zcode_read_session_context_prompt_has_expected_shape() {
        let history = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<environment_context>\nignore native bootstrap\n</environment_context>"
                        .to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Create a note about ZCode session context".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "Write".to_string(),
                namespace: None,
                arguments: r#"{"content":"done","file_path":"notes.txt"}"#.to_string(),
                call_id: "call-write".to_string(),
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCallOutput {
                id: None,
                call_id: "call-write".to_string(),
                output: codex_protocol::models::FunctionCallOutputPayload {
                    body: codex_protocol::models::FunctionCallOutputBody::Text(
                        "File written successfully.".to_string(),
                    ),
                    success: Some(true),
                },
                internal_chat_message_metadata_passthrough: None,
            },
        ];
        let input = ZCodeReadSessionContextArgs {
            session_id: "session-123".to_string(),
            query: "what changed?".to_string(),
            strategy: "relevant".to_string(),
            max_tokens: 4_000,
        };
        let current_arguments = r#"{"query":"what changed?","sessionId":"session-123"}"#;

        let prompt = build_zcode_read_session_context_prompt(
            &history,
            &input,
            Path::new("/tmp/workspace"),
            current_arguments,
        );

        assert_eq!(
            zcode_read_session_context_title(&history),
            "Create a note about ZCode session context"
        );
        assert!(!prompt.contains("ignore native bootstrap"));
        assert!(
            prompt.contains(
                "Target session: Create a note about ZCode session context (session-123)"
            )
        );
        assert!(prompt.contains("[1] user msg_native_0001\ncreated: 2026-06-21T00:00:00.000Z\nCreate a note about ZCode session context"));
        assert!(prompt.contains(
            "Tool Write completed\ninput: {\"file_path\":\"notes.txt\",\"content\":\"done\"}\noutput: File written successfully.\n\nStep finished: tool-calls"
        ));
        assert!(prompt.contains(
            "Tool ReadSessionContext running\ninput: {\"sessionId\":\"session-123\",\"query\":\"what changed?\",\"strategy\":\"relevant\",\"maxTokens\":4000}"
        ));
        assert!(!prompt.ends_with("---\n"));
    }

    #[test]
    fn zcode_tool_input_text_preserves_key_order() {
        assert_eq!(
            zcode_tool_input_text("Write", r#"{"content":"body","file_path":"a.txt"}"#),
            r#"{"file_path":"a.txt","content":"body"}"#
        );
        assert_eq!(
            zcode_tool_input_text("ReadSessionContext", r#"{"query":"q","sessionId":"s"}"#),
            r#"{"sessionId":"s","query":"q","strategy":"relevant","maxTokens":4000}"#
        );
    }

    #[test]
    fn zcode_agent_rollout_stats_counts_tools_and_turn_duration() {
        let history = vec![
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "Bash".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: "call-bash".to_string(),
                internal_chat_message_metadata_passthrough: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "Read".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: "call-read".to_string(),
                internal_chat_message_metadata_passthrough: None,
            }),
            RolloutItem::EventMsg(EventMsg::TurnComplete(
                codex_protocol::protocol::TurnCompleteEvent {
                    turn_id: "turn-1".to_string(),
                    last_agent_message: Some("done".to_string()),
                    completed_at: None,
                    duration_ms: Some(12_345),
                    time_to_first_token_ms: None,
                },
            )),
        ];

        assert_eq!(
            zcode_agent_rollout_stats(&history, /*fallback_duration_ms*/ 99),
            ZCodeAgentRolloutStats {
                tool_uses: 2,
                duration_ms: 12_345,
            }
        );
    }

    #[test]
    fn zcode_todo_history_restore_excludes_current_write_call() {
        let history = vec![
            ResponseItem::FunctionCall {
                id: None,
                name: "TodoWrite".to_string(),
                namespace: None,
                arguments: serde_json::json!({
                    "todos": [
                        {
                            "content": "Prior task",
                            "status": "completed",
                            "priority": "high",
                        },
                    ],
                })
                .to_string(),
                call_id: "previous-todo".to_string(),
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "TodoWrite".to_string(),
                namespace: None,
                arguments: serde_json::json!({
                    "todos": [
                        {
                            "content": "Current task",
                            "status": "in_progress",
                            "priority": "high",
                        },
                    ],
                })
                .to_string(),
                call_id: "current-todo".to_string(),
                internal_chat_message_metadata_passthrough: None,
            },
        ];

        assert_eq!(
            latest_zcode_todos_from_history(&history, "current-todo"),
            Some(vec![ZCodeTodoItem {
                content: "Prior task".to_string(),
                status: "completed".to_string(),
                priority: "high".to_string(),
            }])
        );
    }

    #[tokio::test]
    async fn zcode_todo_cache_persists_across_processes() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let home = tempfile::tempdir().expect("home temp dir");
        let invocation =
            invocation_with_harness(&workspace, "TodoRead", json!({}), Some("zcode")).await;
        let todos = vec![ZCodeTodoItem {
            content: "Carry todo state across resume".to_string(),
            status: "completed".to_string(),
            priority: "high".to_string(),
        }];

        record_zcode_todos_at_home(home.path(), &invocation, &todos);
        if let Ok(mut stored) = ZCODE_TODOS.lock() {
            stored.clear();
        }

        assert_eq!(zcode_todos_at_home(home.path(), &invocation), Some(todos));
    }

    #[tokio::test]
    async fn deepseek_apply_patch_failure_uses_clean_expected_context() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        std::fs::write(
            workspace.path().join("module.py"),
            "VALUE = \"NEEDLE_NEW\"\\n",
        )
        .expect("write module");

        let output = handle_text(
            &workspace,
            HarnessAliasHandler::DeepSeekApplyPatch,
            "apply_patch",
            json!({
                "path": "module.py",
                "patch": "@@\n-VALUE = \"NEEDLE_NEW\"\\n\n+VALUE = \"NEEDLE_NEW\"\\nPATCH_OK = True\n",
            }),
        )
        .await
        .expect("apply_patch failure output");

        assert!(output.contains(
            "Expected context preview:\n   VALUE = \"NEEDLE_NEW\"\nFile snippet near line 1:"
        ));
        assert!(!output.contains("Expected context preview:\n  -VALUE"));

        let output = handle_text(
            &workspace,
            HarnessAliasHandler::DeepSeekApplyPatch,
            "apply_patch",
            json!({
                "path": "module.py",
                "patch": "@@\n VALUE = \"NEEDLE_NEW\"\\n\n+PATCH_OK = True\n",
                "fuzz": 5,
            }),
        )
        .await
        .expect("apply_patch fuzz failure output");

        assert!(output.contains("fuzz up to 5"));
        assert!(output.contains(
            "Expected context preview:\n   VALUE = \"NEEDLE_NEW\"\nFile snippet near line 1:"
        ));
    }

    #[test]
    fn deepseek_list_dir_formatter_preserves_input_order() {
        let entries = vec![
            json!({ "name": "README.md", "is_dir": false }),
            json!({ "name": ".codewhale", "is_dir": true }),
            json!({ "name": "module.py", "is_dir": false }),
            json!({ "name": ".git", "is_dir": true }),
        ];

        assert_eq!(
            format_deepseek_list_dir(&entries),
            "[\n  {\n    \"name\": \"README.md\",\n    \"is_dir\": false\n  },\n  {\n    \"name\": \".codewhale\",\n    \"is_dir\": true\n  },\n  {\n    \"name\": \"module.py\",\n    \"is_dir\": false\n  },\n  {\n    \"name\": \".git\",\n    \"is_dir\": true\n  }\n]"
        );
    }

    #[test]
    fn deepseek_tool_search_result_uses_expected_order() {
        assert_eq!(
            deepseek_tool_search_result(),
            r#"{"type":"tool_search_tool_search_result","tool_references":[{"type":"tool_reference","tool_name":"apply_patch"},{"type":"tool_reference","tool_name":"edit_file"},{"type":"tool_reference","tool_name":"agent_open"},{"type":"tool_reference","tool_name":"handle_read"},{"type":"tool_reference","tool_name":"tool_agent"}]}"#
        );
    }

    #[cfg(unix)]
    fn create_symlink(original: &Path, link: &Path) {
        std::os::unix::fs::symlink(original, link).expect("create symlink");
    }

    #[cfg(windows)]
    fn create_symlink(original: &Path, link: &Path) {
        if original.is_dir() {
            std::os::windows::fs::symlink_dir(original, link).expect("create dir symlink");
        } else {
            std::os::windows::fs::symlink_file(original, link).expect("create file symlink");
        }
    }
}
