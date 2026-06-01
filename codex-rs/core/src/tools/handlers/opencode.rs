use crate::agent::AgentStatus;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::next_thread_spawn_depth;
use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_policy::ExecApprovalRequest;
use crate::function_tool::FunctionCallError;
use crate::harness::opencode::OPENCODE_TASK_AGENT_BASE_INSTRUCTIONS;
use crate::session::Session;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::events::ToolEmitter;
use crate::tools::events::ToolEventCtx;
use crate::tools::events::ToolEventFailure;
use crate::tools::events::ToolEventStage;
use crate::tools::handlers::apply_granted_turn_permissions;
use crate::tools::handlers::claude_code::effective_turn_file_system_policy;
use crate::tools::handlers::claude_code::ensure_readable_path;
use crate::tools::handlers::claude_code::ensure_writable_path;
use crate::tools::handlers::claude_code::parse_absolute_path;
use crate::tools::handlers::multi_agents_common::apply_spawn_agent_overrides;
use crate::tools::handlers::multi_agents_common::apply_spawn_agent_runtime_overrides;
use crate::tools::handlers::multi_agents_common::build_agent_spawn_config;
use crate::tools::handlers::multi_agents_common::collab_spawn_error;
use crate::tools::handlers::multi_agents_common::thread_spawn_source;
use crate::tools::handlers::parse_arguments;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::shell::ShellRequest;
use crate::tools::runtimes::shell::ShellRuntime;
use crate::tools::runtimes::shell::ShellRuntimeBackend;
use crate::tools::sandboxing::ToolError;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::AgentStatus as ProtocolAgentStatus;
use codex_protocol::protocol::ExecCommandSource;
use codex_protocol::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use regex_lite::Regex;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

pub struct OpenCodeBashHandler;
pub struct OpenCodeEditHandler;
pub struct OpenCodeGlobHandler;
pub struct OpenCodeGrepHandler;
pub struct OpenCodeReadHandler;
pub struct OpenCodeSkillHandler;
pub struct OpenCodeTaskHandler;
pub struct OpenCodeTodoWriteHandler;
pub struct OpenCodeWebFetchHandler;
pub struct OpenCodeWriteHandler;

const OPENCODE_BASH_DEFAULT_TIMEOUT_MS: u64 = 120_000;
const OPENCODE_BASH_MAX_TIMEOUT_MS: u64 = 600_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCodeReadArgs {
    file_path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCodeWriteArgs {
    file_path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCodeEditArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    replace_all: Option<bool>,
}

#[derive(Deserialize)]
struct OpenCodeBashArgs {
    command: String,
    timeout: Option<u64>,
    workdir: Option<String>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct OpenCodeGlobArgs {
    pattern: String,
    path: Option<String>,
}

#[derive(Deserialize)]
struct OpenCodeGrepArgs {
    pattern: String,
    path: Option<String>,
    include: Option<String>,
}

#[derive(Deserialize)]
struct OpenCodeTodoWriteArgs {
    todos: JsonValue,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCodeTaskArgs {
    description: Option<String>,
    prompt: String,
    subagent_type: Option<String>,
    task_id: Option<String>,
    command: Option<String>,
}

impl ToolHandler for OpenCodeReadHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "read received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeReadArgs = parse_arguments(&arguments)?;
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        if path.as_path().is_dir() {
            return Ok(FunctionToolOutput::from_text(
                format_directory_output(path.as_path())?,
                Some(true),
            ));
        }
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    FunctionCallError::RespondToModel(format!("File not found: {}", path.display()))
                } else {
                    FunctionCallError::RespondToModel(format!("Read failed: {err}"))
                }
            })?;
        Ok(FunctionToolOutput::from_text(
            format_read_output(
                path.as_path(),
                &content,
                args.offset.unwrap_or(1),
                args.limit.unwrap_or(2000),
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for OpenCodeWriteHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "write received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeWriteArgs = parse_arguments(&arguments)?;
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        tokio::fs::write(path.as_path(), args.content)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
        Ok(FunctionToolOutput::from_text(
            "Wrote file successfully.".to_string(),
            Some(true),
        ))
    }
}

fn format_directory_output(path: &Path) -> Result<String, FunctionCallError> {
    let mut entries = fs::read_dir(path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Read failed: {err}")))?
        .filter_map(Result::ok)
        .map(|entry| {
            let mut name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() {
                name.push('/');
            }
            name
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_hidden = left.starts_with('.');
        let right_hidden = right.starts_with('.');
        let left_dir = left.ends_with('/');
        let right_dir = right.ends_with('/');
        right_hidden
            .cmp(&left_hidden)
            .then_with(|| right_dir.cmp(&left_dir))
            .then_with(|| right.cmp(left))
    });
    let mut output = format!(
        "<path>{}</path>\n<type>directory</type>\n<entries>\n",
        path.display()
    );
    if entries.is_empty() {
        output.push('\n');
    } else {
        output.push_str(&entries.join("\n"));
        output.push('\n');
    }
    output.push_str(&format!("\n({} entries)\n</entries>", entries.len()));
    Ok(output)
}

impl ToolHandler for OpenCodeEditHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "edit received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeEditArgs = parse_arguments(&arguments)?;
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
        let updated = replace_exact_text(&content, &args)?;
        tokio::fs::write(path.as_path(), updated)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
        Ok(FunctionToolOutput::from_text(
            "Edit applied successfully.".to_string(),
            Some(true),
        ))
    }
}

impl ToolHandler for OpenCodeTodoWriteHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "todowrite received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeTodoWriteArgs = parse_arguments(&arguments)?;
        let output = format_todos(&args.todos)?;
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

/// Resolve a `glob`/`grep` base path and confirm the session policy allows
/// reading it, so these search tools cannot escape the workspace (e.g. a base
/// outside the project). Relative paths resolve against the turn's working
/// directory.
async fn read_checked_search_base(
    session: &Session,
    turn: &TurnContext,
    path: Option<&str>,
) -> Result<PathBuf, FunctionCallError> {
    let base = match path {
        Some(raw) => {
            let raw = Path::new(raw);
            if raw.is_absolute() {
                raw.to_path_buf()
            } else {
                turn.cwd.as_path().join(raw)
            }
        }
        None => turn.cwd.as_path().to_path_buf(),
    };
    let absolute = AbsolutePathBuf::try_from(base.clone()).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid path `{}`: {err}", base.display()))
    })?;
    let file_system_policy = effective_turn_file_system_policy(session, turn).await;
    ensure_readable_path(&file_system_policy, turn, &absolute)?;
    Ok(base)
}

impl ToolHandler for OpenCodeGlobHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "glob received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeGlobArgs = parse_arguments(&arguments)?;
        let base =
            read_checked_search_base(session.as_ref(), turn.as_ref(), args.path.as_deref()).await?;
        let pattern = base.join(args.pattern);
        let mut matches =
            super::safe_fs::bounded_glob_paths(&base, &pattern.to_string_lossy(), false)
                .into_iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
        matches.sort();
        Ok(FunctionToolOutput::from_text(
            matches.join("\n"),
            Some(true),
        ))
    }
}

impl ToolHandler for OpenCodeGrepHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "grep received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeGrepArgs = parse_arguments(&arguments)?;
        let regex = Regex::new(&args.pattern)
            .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
        let include = args.include.as_deref();
        let base =
            read_checked_search_base(session.as_ref(), turn.as_ref(), args.path.as_deref()).await?;
        let paths = super::safe_fs::bounded_collect_files(&base);
        let mut results: Vec<(String, Vec<String>)> = Vec::new();
        for path in paths {
            if !path.is_file() || !include_matches(&path, include) {
                continue;
            }
            let Some(content) = super::safe_fs::read_searchable_file(&path) else {
                continue;
            };
            let mut lines = Vec::new();
            for (index, line) in content.split('\n').enumerate() {
                if regex.is_match(line) {
                    lines.push(format!("  Line {}: {line}", index + 1));
                }
            }
            if !lines.is_empty() {
                results.push((path.display().to_string(), lines));
            }
        }
        results.sort_by(|left, right| left.0.cmp(&right.0));
        let match_count = results.iter().map(|(_, lines)| lines.len()).sum::<usize>();
        let mut output = format!("Found {match_count} matches");
        for (path, lines) in results {
            output.push('\n');
            output.push_str(&path);
            output.push_str(":\n");
            output.push_str(&lines.join("\n"));
        }
        if !output.ends_with("\\n") {
            output.push('\n');
        }
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

impl ToolHandler for OpenCodeBashHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            tool_name,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "bash received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeBashArgs = parse_arguments(&arguments)?;
        let Some(_environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "bash is unavailable in this session".to_string(),
            ));
        };
        let cwd = args
            .workdir
            .as_deref()
            .filter(|workdir| !workdir.is_empty())
            .map(PathBuf::from)
            .and_then(|path| codex_utils_absolute_path::AbsolutePathBuf::try_from(path).ok())
            .unwrap_or_else(|| turn.cwd.clone());
        let timeout_ms = Some(
            args.timeout
                .unwrap_or(OPENCODE_BASH_DEFAULT_TIMEOUT_MS)
                .min(OPENCODE_BASH_MAX_TIMEOUT_MS),
        );
        let mut command = session
            .user_shell()
            .derive_exec_args(&args.command, turn.tools_config.allow_login_shell);
        if std::env::consts::OS == "linux" && command.first().is_some_and(|arg| arg == "/bin/bash")
        {
            command[0] = "/usr/bin/bash".to_string();
        }
        let exec_params = ExecParams {
            command: command.clone(),
            cwd: cwd.clone(),
            expiration: timeout_ms.into(),
            capture_policy: ExecCapturePolicy::ShellToolFullOutput,
            env: create_env(
                &turn.shell_environment_policy,
                Some(session.conversation_id),
            ),
            network: turn.network.clone(),
            sandbox_permissions: SandboxPermissions::UseDefault,
            windows_sandbox_level: turn.windows_sandbox_level,
            windows_sandbox_private_desktop: turn
                .config
                .permissions
                .windows_sandbox_private_desktop,
            justification: args.description.clone(),
            arg0: None,
        };
        let emitter = ToolEmitter::shell(
            exec_params.command.clone(),
            exec_params.cwd.clone(),
            ExecCommandSource::Agent,
            /*freeform*/ false,
        );
        let event_ctx = ToolEventCtx::new(
            session.as_ref(),
            turn.as_ref(),
            &call_id,
            /*turn_diff_tracker*/ None,
        );
        emitter.begin(event_ctx).await;
        let effective_permissions = apply_granted_turn_permissions(
            session.as_ref(),
            turn.cwd.as_path(),
            SandboxPermissions::UseDefault,
            None,
        )
        .await;
        let exec_approval_requirement = session
            .services
            .exec_policy
            .create_exec_approval_requirement_for_command(ExecApprovalRequest {
                command: &exec_params.command,
                approval_policy: turn.approval_policy.value(),
                sandbox_policy: turn.sandbox_policy.get(),
                file_system_sandbox_policy: &turn.file_system_sandbox_policy,
                sandbox_permissions: if effective_permissions.permissions_preapproved {
                    SandboxPermissions::UseDefault
                } else {
                    effective_permissions.sandbox_permissions
                },
                prefix_rule: None,
            })
            .await;
        let request = ShellRequest {
            command: exec_params.command.clone(),
            hook_command: codex_shell_command::parse_command::shlex_join(&exec_params.command),
            cwd: exec_params.cwd.clone(),
            timeout_ms,
            capture_policy: exec_params.capture_policy,
            env: exec_params.env.clone(),
            explicit_env_overrides: turn.shell_environment_policy.r#set.clone(),
            network: exec_params.network.clone(),
            sandbox_permissions: effective_permissions.sandbox_permissions,
            additional_permissions: None,
            #[cfg(unix)]
            additional_permissions_preapproved: effective_permissions.permissions_preapproved,
            justification: exec_params.justification.clone(),
            exec_approval_requirement,
        };
        let mut orchestrator = ToolOrchestrator::new();
        let mut runtime = ShellRuntime::for_shell_command(ShellRuntimeBackend::ShellCommandClassic);
        let tool_ctx = crate::tools::sandboxing::ToolCtx {
            session: session.clone(),
            turn: turn.clone(),
            call_id: call_id.clone(),
            tool_name: tool_name.display(),
        };
        let result = orchestrator
            .run(
                &mut runtime,
                &request,
                &tool_ctx,
                &turn,
                turn.approval_policy.value(),
            )
            .await
            .map(|output| output.output);
        match result {
            Ok(output) => {
                emitter
                    .emit(event_ctx, ToolEventStage::Success(output.clone()))
                    .await;
                let text = if output.exit_code == 0 {
                    opencode_bash_output_text(&output.aggregated_output.text)
                } else {
                    crate::tools::format_exec_output_str(&output, turn.truncation_policy)
                };
                if output.exit_code == 0 {
                    Ok(FunctionToolOutput::from_text(text, Some(true)))
                } else {
                    Err(FunctionCallError::RespondToModel(text))
                }
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Timeout { output })))
            | Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Output((*output).clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(
                    crate::tools::format_exec_output_str(output.as_ref(), turn.truncation_policy),
                ))
            }
            Err(ToolError::Rejected(message)) => {
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Rejected(message.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(message))
            }
            Err(ToolError::Codex(err)) => {
                let message = format!("execution error: {err:?}");
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Message(message.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(message))
            }
        }
    }
}

impl ToolHandler for OpenCodeTaskHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "task received unsupported payload".to_string(),
            ));
        };
        let args: OpenCodeTaskArgs = parse_arguments(&arguments)?;
        let _ = (&args.description, &args.command);
        if args.prompt.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "task requires a non-empty prompt".to_string(),
            ));
        }
        if args
            .task_id
            .as_deref()
            .is_some_and(|task_id| !task_id.trim().is_empty())
        {
            return Err(FunctionCallError::RespondToModel(
                "task resume is not supported yet in the opencode harness".to_string(),
            ));
        }
        let child_depth = next_thread_spawn_depth(&turn.session_source);
        if exceeds_thread_spawn_depth_limit(child_depth, turn.config.agent_max_depth) {
            return Err(FunctionCallError::RespondToModel(
                "Task depth limit reached. Solve the task yourself.".to_string(),
            ));
        }

        let mut config =
            build_agent_spawn_config(&session.get_base_instructions().await, turn.as_ref())?;
        config.base_instructions = Some(OPENCODE_TASK_AGENT_BASE_INSTRUCTIONS.to_string());
        apply_spawn_agent_runtime_overrides(&mut config, turn.as_ref())?;
        apply_spawn_agent_overrides(&mut config, child_depth);
        let agent_role = args
            .subagent_type
            .as_deref()
            .map(str::trim)
            .filter(|agent_type| !agent_type.is_empty())
            .unwrap_or("explore");
        let spawn_source = thread_spawn_source(
            session.conversation_id,
            &turn.session_source,
            child_depth,
            Some(agent_role),
            None,
        )?;
        let agent_id = session
            .services
            .agent_control
            .spawn_agent_with_metadata(
                config,
                vec![UserInput::Text {
                    text: args.prompt.trim().to_string(),
                    text_elements: Vec::new(),
                }]
                .into(),
                Some(spawn_source),
                Default::default(),
            )
            .await
            .map_err(collab_spawn_error)?
            .thread_id;

        let mut status_rx = session
            .services
            .agent_control
            .subscribe_status(agent_id)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Task failed: {err}")))?;
        let mut status = status_rx.borrow().clone();
        while matches!(
            status,
            AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
        ) {
            if status_rx.changed().await.is_err() {
                status = session.services.agent_control.get_status(agent_id).await;
                break;
            }
            status = status_rx.borrow().clone();
        }
        let message = match status {
            ProtocolAgentStatus::Completed(Some(message)) => message,
            ProtocolAgentStatus::Completed(None) => String::new(),
            ProtocolAgentStatus::Errored(message) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "Task failed: {message}"
                )));
            }
            ProtocolAgentStatus::Interrupted => {
                return Err(FunctionCallError::RespondToModel(
                    "Task was interrupted before it completed.".to_string(),
                ));
            }
            ProtocolAgentStatus::Shutdown => {
                return Err(FunctionCallError::RespondToModel(
                    "Task shut down before it completed.".to_string(),
                ));
            }
            ProtocolAgentStatus::NotFound => {
                return Err(FunctionCallError::RespondToModel(
                    "Task disappeared before it completed.".to_string(),
                ));
            }
            ProtocolAgentStatus::PendingInit | ProtocolAgentStatus::Running => {
                return Err(FunctionCallError::RespondToModel(
                    "Task did not reach a final state.".to_string(),
                ));
            }
        };
        Ok(FunctionToolOutput::from_text(
            format!(
                "task_id: {} (for resuming to continue this task if needed)\n\n<task_result>\n{}\n</task_result>",
                opencode_task_id(agent_id),
                message.trim()
            ),
            Some(true),
        ))
    }
}

fn opencode_task_id(agent_id: codex_protocol::ThreadId) -> String {
    format!("ses_{}", agent_id.to_string().replace('-', ""))
}

impl ToolHandler for OpenCodeSkillHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, _invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        Ok(FunctionToolOutput::from_text(String::new(), Some(true)))
    }
}

impl ToolHandler for OpenCodeWebFetchHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, _invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            "webfetch is unavailable in this session".to_string(),
        ))
    }
}

fn format_read_output(path: &Path, content: &str, offset: usize, limit: usize) -> String {
    let lines = content.split('\n').collect::<Vec<_>>();
    let start = offset.saturating_sub(1);
    let end = lines.len().min(start.saturating_add(limit));
    let mut output = format!(
        "<path>{}</path>\n<type>file</type>\n<content>\n",
        path.display()
    );
    for (index, line) in lines.iter().enumerate().take(end).skip(start) {
        if index == lines.len().saturating_sub(1) && line.is_empty() {
            continue;
        }
        output.push_str(&format!("{}: {line}\n", index + 1));
    }
    let total_lines = content.lines().count();
    if end < total_lines {
        output.push_str(&format!(
            "\n(Showing lines {}-{} of {}. Use offset={} to continue.)\n</content>",
            start + 1,
            end,
            total_lines,
            end + 1
        ));
    } else {
        output.push_str(&format!(
            "\n(End of file - total {total_lines} lines)\n</content>"
        ));
    }
    output
}

fn replace_exact_text(content: &str, args: &OpenCodeEditArgs) -> Result<String, FunctionCallError> {
    let replace_all = args.replace_all.unwrap_or(false);
    if replace_all {
        if !content.contains(&args.old_string) {
            return Err(FunctionCallError::RespondToModel(
                "oldString was not found in the file".to_string(),
            ));
        }
        return Ok(content.replace(&args.old_string, &args.new_string));
    }

    let match_count = content.match_indices(&args.old_string).count();
    match match_count {
        0 => Err(FunctionCallError::RespondToModel(
            "oldString was not found in the file".to_string(),
        )),
        1 => Ok(content.replacen(&args.old_string, &args.new_string, 1)),
        _ => Err(FunctionCallError::RespondToModel(
            "oldString is not unique in the file".to_string(),
        )),
    }
}

fn format_todos(todos: &JsonValue) -> Result<String, FunctionCallError> {
    let Some(items) = todos.as_array() else {
        return serde_json::to_string_pretty(todos)
            .map_err(|err| FunctionCallError::Fatal(format!("failed to serialize todos: {err}")));
    };
    let mut output = String::from("[");
    for (index, item) in items.iter().enumerate() {
        let content = item
            .get("content")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let status = item
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let priority = item
            .get("priority")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        if index > 0 {
            output.push(',');
        }
        output.push_str("\n  {\n");
        output.push_str(&format!(
            "    \"content\": {},\n",
            serde_json::to_string(content).map_err(|err| FunctionCallError::Fatal(format!(
                "failed to serialize todo content: {err}"
            )))?
        ));
        output.push_str(&format!(
            "    \"status\": {},\n",
            serde_json::to_string(status).map_err(|err| FunctionCallError::Fatal(format!(
                "failed to serialize todo status: {err}"
            )))?
        ));
        output.push_str(&format!(
            "    \"priority\": {}\n",
            serde_json::to_string(priority).map_err(|err| FunctionCallError::Fatal(format!(
                "failed to serialize todo priority: {err}"
            )))?
        ));
        output.push_str("  }");
    }
    if !items.is_empty() {
        output.push('\n');
    }
    output.push(']');
    Ok(output)
}

fn include_matches(path: &Path, include: Option<&str>) -> bool {
    let Some(include) = include else {
        return true;
    };
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| glob::Pattern::new(include).is_ok_and(|pattern| pattern.matches(name)))
}

fn opencode_bash_output_text(text: &str) -> String {
    if text.is_empty() {
        "(no output)".to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_read_output_reports_continuation_for_partial_reads() {
        let content = (1..=30)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        let output = format_read_output(Path::new("/app/file.txt"), &content, 11, 5);

        assert!(output.contains("11: line 11\n"));
        assert!(output.contains("15: line 15\n"));
        assert!(output.contains("(Showing lines 11-15 of 30. Use offset=16 to continue.)"));
    }

    #[test]
    fn format_read_output_reports_end_for_complete_reads() {
        let output = format_read_output(Path::new("/app/file.txt"), "first\nsecond\n", 1, 10);

        assert!(output.contains("1: first\n"));
        assert!(output.contains("2: second\n"));
        assert!(output.contains("(End of file - total 2 lines)"));
    }
}
