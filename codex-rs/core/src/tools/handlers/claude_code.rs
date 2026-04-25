use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_policy::ExecApprovalRequest;
use crate::function_tool::FunctionCallError;
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
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::plan::handle_update_plan;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::shell::ShellRequest;
use crate::tools::runtimes::shell::ShellRuntime;
use crate::tools::runtimes::shell::ShellRuntimeBackend;
use crate::tools::sandboxing::ToolError;
use chrono::Datelike;
use chrono::Local;
use chrono::Timelike;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::ExecCommandSource;
use codex_protocol::protocol::SessionSource;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use codex_protocol::user_input::UserInput;
use codex_sandboxing::policy_transforms::effective_file_system_sandbox_policy;
use codex_sandboxing::policy_transforms::merge_permission_profiles;
use codex_tools::normalize_request_user_input_args;
use codex_tools::request_user_input_unavailable_message;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub struct ClaudeAskUserQuestionHandler;
pub struct ClaudeBashHandler;
pub struct ClaudeCronCreateHandler;
pub struct ClaudeCronDeleteHandler;
pub struct ClaudeCronListHandler;
pub struct ClaudeEditHandler;
pub struct ClaudeReadHandler;
pub struct ClaudeScheduleWakeupHandler;
pub struct ClaudeTodoWriteHandler;
pub struct ClaudeWriteHandler;

const CLAUDE_BASH_EMPTY_OUTPUT: &str = "(Bash completed with no output)";
const CLAUDE_BASH_DEFAULT_TIMEOUT_MS: u64 = 120_000;
const CLAUDE_BASH_MAX_TIMEOUT_MS: u64 = 600_000;
pub(crate) const CLAUDE_TODO_WRITE_SUCCESS_MESSAGE: &str = "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable";

#[derive(Deserialize)]
struct ClaudeReadArgs {
    file_path: String,
    offset: Option<usize>,
    limit: Option<usize>,
    #[allow(dead_code)]
    pages: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeWriteArgs {
    file_path: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeEditArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    replace_all: Option<bool>,
}

#[derive(Deserialize)]
struct ClaudeBashArgs {
    command: String,
    description: Option<String>,
    timeout: Option<u64>,
    run_in_background: Option<bool>,
}

#[derive(Deserialize)]
struct ClaudeTodoWriteArgs {
    todos: Vec<ClaudeTodoItem>,
}

#[derive(Deserialize)]
struct ClaudeCronCreateArgs {
    cron: String,
    prompt: String,
    recurring: Option<bool>,
    durable: Option<bool>,
}

#[derive(Deserialize)]
struct ClaudeCronDeleteArgs {
    id: String,
}

#[derive(Deserialize)]
struct ClaudeScheduleWakeupArgs {
    #[serde(rename = "delaySeconds")]
    delay_seconds: f64,
    reason: String,
    prompt: String,
}

#[derive(Deserialize)]
struct ClaudeTodoItem {
    content: String,
    status: ClaudeTodoStatus,
    #[serde(rename = "activeForm")]
    active_form: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaudeTodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Deserialize)]
struct ClaudeAskUserQuestionArgs {
    questions: Vec<ClaudeAskUserQuestionItem>,
}

#[derive(Deserialize)]
struct ClaudeAskUserQuestionItem {
    question: String,
    header: String,
    options: Vec<ClaudeAskUserQuestionOption>,
    #[serde(rename = "multiSelect", default)]
    multi_select: bool,
}

#[derive(Deserialize)]
struct ClaudeAskUserQuestionOption {
    label: String,
    description: String,
    #[allow(dead_code)]
    preview: Option<String>,
}

impl ToolHandler for ClaudeReadHandler {
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
                "Read received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeReadArgs = parse_arguments(&arguments)?;
        if args.pages.is_some() {
            return Err(FunctionCallError::RespondToModel(
                "PDF page selection is not implemented for the claude-code harness yet."
                    .to_string(),
            ));
        }
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Read failed: {err}")))?;
        let formatted = format_read_output(
            &content,
            args.offset.unwrap_or(1),
            args.limit.unwrap_or(2000),
        );
        Ok(FunctionToolOutput::from_text(formatted, Some(true)))
    }
}

impl ToolHandler for ClaudeAskUserQuestionHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "AskUserQuestion received unsupported payload".to_string(),
            ));
        };

        if matches!(turn.session_source, SessionSource::SubAgent(_)) {
            return Err(FunctionCallError::RespondToModel(
                "AskUserQuestion can only be used by the root thread".to_string(),
            ));
        }

        let mode = session.collaboration_mode().await.mode;
        if let Some(message) = request_user_input_unavailable_message(
            mode,
            turn.tools_config.default_mode_request_user_input,
        ) {
            return Err(FunctionCallError::RespondToModel(message));
        }

        let args = normalize_claude_ask_user_question_args(parse_arguments(&arguments)?)?;
        let response = session
            .request_user_input(turn.as_ref(), call_id, args)
            .await
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "AskUserQuestion was cancelled before receiving a response".to_string(),
                )
            })?;

        let content = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize AskUserQuestion response: {err}"
            ))
        })?;

        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

impl ToolHandler for ClaudeWriteHandler {
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
                "Write received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeWriteArgs = parse_arguments(&arguments)?;
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        tokio::fs::write(path.as_path(), args.content)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
        Ok(FunctionToolOutput::from_text(
            format!("File created successfully at: {}", path.display()),
            Some(true),
        ))
    }
}

impl ToolHandler for ClaudeTodoWriteHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "TodoWrite received unsupported payload".to_string(),
            ));
        };

        let args: ClaudeTodoWriteArgs = parse_arguments(&arguments)?;
        let update_plan = UpdatePlanArgs {
            explanation: None,
            plan: args
                .todos
                .into_iter()
                .map(|todo| {
                    let _ = todo.active_form;
                    PlanItemArg {
                        step: todo.content,
                        status: match todo.status {
                            ClaudeTodoStatus::Pending => StepStatus::Pending,
                            ClaudeTodoStatus::InProgress => StepStatus::InProgress,
                            ClaudeTodoStatus::Completed => StepStatus::Completed,
                        },
                    }
                })
                .collect(),
        };
        let arguments = serde_json::to_string(&update_plan).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize TodoWrite arguments as update_plan: {err}"
            ))
        })?;
        handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;
        Ok(FunctionToolOutput::from_text(
            CLAUDE_TODO_WRITE_SUCCESS_MESSAGE.to_string(),
            Some(true),
        ))
    }
}

impl ToolHandler for ClaudeCronCreateHandler {
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
                "CronCreate received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeCronCreateArgs = parse_arguments(&arguments)?;
        if args.durable.unwrap_or(false) {
            return Err(FunctionCallError::RespondToModel(
                "CronCreate durable jobs are not implemented yet in this claude-code harness. Retry with durable: false for a session-only job.".to_string(),
            ));
        }
        let recurring = args.recurring.unwrap_or(true);
        let schedule = CronSchedule::parse(&args.cron)?;
        let next_fire = schedule.next_after(Local::now())?;
        let id = format!("cron_{}", Uuid::new_v4());
        let token = CancellationToken::new();
        let job = ScheduledJob {
            id: id.clone(),
            kind: ScheduledJobKind::Cron,
            cron: Some(args.cron.clone()),
            prompt: args.prompt.clone(),
            recurring,
            durable: false,
            next_fire_local: next_fire.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
            reason: None,
            cancel: token.clone(),
        };
        {
            let mut scheduler = CLAUDE_SCHEDULER.lock().await;
            scheduler.jobs.insert(id.clone(), job.clone());
        }
        spawn_cron_job(session, job.clone(), schedule, token);
        Ok(FunctionToolOutput::from_text(
            format!(
                "Scheduled cron job {id}. Next fire: {}",
                job.next_fire_local
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for ClaudeCronDeleteHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "CronDelete received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeCronDeleteArgs = parse_arguments(&arguments)?;
        let removed = {
            let mut scheduler = CLAUDE_SCHEDULER.lock().await;
            scheduler.jobs.remove(&args.id)
        };
        match removed {
            Some(job) => {
                job.cancel.cancel();
                Ok(FunctionToolOutput::from_text(
                    format!("Cancelled cron job {}", args.id),
                    Some(true),
                ))
            }
            None => Err(FunctionCallError::RespondToModel(format!(
                "No cron job found with id {}",
                args.id
            ))),
        }
    }
}

impl ToolHandler for ClaudeCronListHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { .. } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "CronList received unsupported payload".to_string(),
            ));
        };
        let jobs = {
            let scheduler = CLAUDE_SCHEDULER.lock().await;
            scheduler
                .jobs
                .values()
                .filter(|job| matches!(job.kind, ScheduledJobKind::Cron))
                .cloned()
                .collect::<Vec<_>>()
        };
        let body = serde_json::to_string_pretty(&jobs).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize cron jobs: {err}"))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}

impl ToolHandler for ClaudeScheduleWakeupHandler {
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
                "ScheduleWakeup received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeScheduleWakeupArgs = parse_arguments(&arguments)?;
        let delay_seconds = args.delay_seconds.clamp(60.0, 3600.0).round() as u64;
        let id = format!("wakeup_{}", Uuid::new_v4());
        let cancel = CancellationToken::new();
        let next_fire = Local::now() + chrono::Duration::seconds(delay_seconds as i64);
        let job = ScheduledJob {
            id: id.clone(),
            kind: ScheduledJobKind::Wakeup,
            cron: None,
            prompt: args.prompt,
            recurring: false,
            durable: false,
            next_fire_local: next_fire.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
            reason: Some(args.reason),
            cancel: cancel.clone(),
        };
        {
            let mut scheduler = CLAUDE_SCHEDULER.lock().await;
            scheduler.jobs.insert(id.clone(), job.clone());
        }
        spawn_one_shot_job(
            session,
            job.clone(),
            Duration::from_secs(delay_seconds),
            cancel,
        );
        Ok(FunctionToolOutput::from_text(
            format!(
                "Scheduled wakeup {id} in {delay_seconds}s. Reason: {}",
                job.reason.as_deref().unwrap_or("")
            ),
            Some(true),
        ))
    }
}

fn normalize_claude_ask_user_question_args(
    args: ClaudeAskUserQuestionArgs,
) -> Result<RequestUserInputArgs, FunctionCallError> {
    let mut question_ids = HashMap::<String, usize>::new();
    let request = RequestUserInputArgs {
        questions: args
            .questions
            .into_iter()
            .enumerate()
            .map(|(index, question)| {
                if question.multi_select {
                    return Err(FunctionCallError::RespondToModel(
                        "AskUserQuestion multiSelect is not implemented for the claude-code harness yet."
                            .to_string(),
                    ));
                }

                let base_id = slugify_identifier(&question.header);
                let next_index = question_ids.entry(base_id.clone()).or_insert(0);
                let id = if *next_index == 0 {
                    base_id
                } else {
                    format!("{base_id}_{}", *next_index + 1)
                };
                *next_index += 1;

                Ok(RequestUserInputQuestion {
                    id: if id.is_empty() {
                        format!("question_{}", index + 1)
                    } else {
                        id
                    },
                    header: question.header,
                    question: question.question,
                    is_other: true,
                    is_secret: false,
                    options: Some(
                        question
                            .options
                            .into_iter()
                            .map(|option| RequestUserInputQuestionOption {
                                label: option.label,
                                description: option.description,
                            })
                            .collect(),
                    ),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    normalize_request_user_input_args(request).map_err(FunctionCallError::RespondToModel)
}

fn slugify_identifier(input: &str) -> String {
    let mut slug = String::new();
    let mut last_was_underscore = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_underscore = false;
        } else if !last_was_underscore && !slug.is_empty() {
            slug.push('_');
            last_was_underscore = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

impl ToolHandler for ClaudeEditHandler {
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
                "Edit received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeEditArgs = parse_arguments(&arguments)?;
        if args.old_string == args.new_string {
            return Err(FunctionCallError::RespondToModel(
                "old_string and new_string must differ".to_string(),
            ));
        }
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
        let message = if args.replace_all.unwrap_or(false) {
            format!(
                "The file {} has been updated. All occurrences were successfully replaced.",
                path.display()
            )
        } else {
            format!("The file {} has been updated.", path.display())
        };
        Ok(FunctionToolOutput::from_text(message, Some(true)))
    }
}

impl ToolHandler for ClaudeBashHandler {
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
            tracker: _tracker,
            call_id,
            tool_name,
            payload,
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "Bash received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeBashArgs = parse_arguments(&arguments)?;
        if args.run_in_background.unwrap_or(false) {
            return Err(FunctionCallError::RespondToModel(
                "run_in_background is not implemented for the claude-code harness yet.".to_string(),
            ));
        }
        let Some(_environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "Bash is unavailable in this session".to_string(),
            ));
        };

        let timeout_ms = Some(
            args.timeout
                .unwrap_or(CLAUDE_BASH_DEFAULT_TIMEOUT_MS)
                .min(CLAUDE_BASH_MAX_TIMEOUT_MS),
        );
        let command = session
            .user_shell()
            .derive_exec_args(&args.command, turn.tools_config.allow_login_shell);
        let exec_params = ExecParams {
            command: command.clone(),
            cwd: turn.cwd.clone(),
            expiration: timeout_ms.into(),
            capture_policy: ExecCapturePolicy::ShellTool,
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

        let effective_permissions =
            apply_granted_turn_permissions(session.as_ref(), SandboxPermissions::UseDefault, None)
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
            env: exec_params.env.clone(),
            explicit_env_overrides: HashMap::new(),
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
                let text = bash_output_text(&output, turn.as_ref());
                if output.exit_code == 0 {
                    Ok(FunctionToolOutput {
                        body: vec![
                            codex_protocol::models::FunctionCallOutputContentItem::InputText {
                                text: text.clone(),
                            },
                        ],
                        success: Some(true),
                        post_tool_use_response: Some(JsonValue::String(text)),
                    })
                } else {
                    Err(FunctionCallError::RespondToModel(text))
                }
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Timeout { output })))
            | Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
                let output = *output;
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Output(output.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(bash_output_text(
                    &output,
                    turn.as_ref(),
                )))
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

pub(super) fn parse_absolute_path(path: &str) -> Result<AbsolutePathBuf, FunctionCallError> {
    if !Path::new(path).is_absolute() {
        return Err(FunctionCallError::RespondToModel(
            "file_path must be an absolute path".to_string(),
        ));
    }
    AbsolutePathBuf::try_from(path.to_string()).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid file_path `{path}`: {err}"))
    })
}

pub(super) async fn effective_turn_file_system_policy(
    session: &Session,
    turn: &TurnContext,
) -> codex_protocol::permissions::FileSystemSandboxPolicy {
    let granted_permissions = merge_permission_profiles(
        session.granted_session_permissions().await.as_ref(),
        session.granted_turn_permissions().await.as_ref(),
    );
    effective_file_system_sandbox_policy(
        &turn.file_system_sandbox_policy,
        granted_permissions.as_ref(),
    )
}

pub(super) fn ensure_readable_path(
    file_system_policy: &codex_protocol::permissions::FileSystemSandboxPolicy,
    turn: &TurnContext,
    path: &AbsolutePathBuf,
) -> Result<(), FunctionCallError> {
    if file_system_policy.can_read_path_with_cwd(path.as_path(), turn.cwd.as_path()) {
        Ok(())
    } else {
        Err(FunctionCallError::RespondToModel(format!(
            "Read is not allowed for {} in this session.",
            path.display()
        )))
    }
}

pub(super) fn ensure_writable_path(
    file_system_policy: &codex_protocol::permissions::FileSystemSandboxPolicy,
    turn: &TurnContext,
    path: &AbsolutePathBuf,
) -> Result<(), FunctionCallError> {
    let writable_target = path.parent().unwrap_or_else(|| path.clone());
    if file_system_policy.can_write_path_with_cwd(writable_target.as_path(), turn.cwd.as_path()) {
        Ok(())
    } else {
        Err(FunctionCallError::RespondToModel(format!(
            "Write is not allowed for {} in this session.",
            path.display()
        )))
    }
}

fn format_read_output(content: &str, offset: usize, limit: usize) -> String {
    let start_index = offset.saturating_sub(1);
    content
        .split('\n')
        .enumerate()
        .skip(start_index)
        .take(limit)
        .map(|(index, line)| format!("{}\t{line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_exact_text(content: &str, args: &ClaudeEditArgs) -> Result<String, FunctionCallError> {
    let replace_all = args.replace_all.unwrap_or(false);
    if replace_all {
        if !content.contains(&args.old_string) {
            return Err(FunctionCallError::RespondToModel(
                "old_string was not found in the file".to_string(),
            ));
        }
        return Ok(content.replace(&args.old_string, &args.new_string));
    }

    let match_count = content.match_indices(&args.old_string).count();
    match match_count {
        0 => Err(FunctionCallError::RespondToModel(
            "old_string was not found in the file".to_string(),
        )),
        1 => Ok(content.replacen(&args.old_string, &args.new_string, 1)),
        _ => Err(FunctionCallError::RespondToModel(
            "old_string is not unique in the file; provide more context or set replace_all to true"
                .to_string(),
        )),
    }
}

fn bash_output_text(
    output: &codex_protocol::exec_output::ExecToolCallOutput,
    turn: &TurnContext,
) -> String {
    let text = crate::tools::format_exec_output_str(output, turn.truncation_policy);
    if text.is_empty() {
        CLAUDE_BASH_EMPTY_OUTPUT.to_string()
    } else {
        text
    }
}

static CLAUDE_SCHEDULER: LazyLock<Mutex<ClaudeSchedulerState>> =
    LazyLock::new(|| Mutex::new(ClaudeSchedulerState::default()));

#[derive(Default)]
struct ClaudeSchedulerState {
    jobs: HashMap<String, ScheduledJob>,
}

#[derive(Clone, Serialize)]
struct ScheduledJob {
    id: String,
    kind: ScheduledJobKind,
    cron: Option<String>,
    prompt: String,
    recurring: bool,
    durable: bool,
    next_fire_local: String,
    reason: Option<String>,
    #[serde(skip)]
    cancel: CancellationToken,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum ScheduledJobKind {
    Cron,
    Wakeup,
}

fn spawn_one_shot_job(
    session: Arc<Session>,
    job: ScheduledJob,
    delay: Duration,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => {}
            _ = tokio::time::sleep(delay) => {
                fire_scheduled_prompt(Arc::clone(&session), &job.prompt).await;
                let mut scheduler = CLAUDE_SCHEDULER.lock().await;
                scheduler.jobs.remove(&job.id);
            }
        }
    });
}

fn spawn_cron_job(
    session: Arc<Session>,
    job: ScheduledJob,
    schedule: CronSchedule,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let mut job = job;
        loop {
            let now = Local::now();
            let next_fire = match schedule.next_after(now) {
                Ok(next_fire) => next_fire,
                Err(_) => {
                    let mut scheduler = CLAUDE_SCHEDULER.lock().await;
                    scheduler.jobs.remove(&job.id);
                    return;
                }
            };
            let delay = next_fire
                .signed_duration_since(now)
                .to_std()
                .unwrap_or_else(|_| Duration::from_secs(0));
            {
                let mut scheduler = CLAUDE_SCHEDULER.lock().await;
                if let Some(stored) = scheduler.jobs.get_mut(&job.id) {
                    stored.next_fire_local = next_fire.format("%Y-%m-%d %H:%M:%S %Z").to_string();
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(delay) => {
                    fire_scheduled_prompt(Arc::clone(&session), &job.prompt).await;
                    if !job.recurring {
                        let mut scheduler = CLAUDE_SCHEDULER.lock().await;
                        scheduler.jobs.remove(&job.id);
                        return;
                    }
                    job.next_fire_local = next_fire.format("%Y-%m-%d %H:%M:%S %Z").to_string();
                }
            }
        }
    });
}

async fn fire_scheduled_prompt(session: Arc<Session>, prompt: &str) {
    let item = ResponseInputItem::from(vec![UserInput::Text {
        text: prompt.to_string(),
        text_elements: Vec::new(),
    }]);
    session.queue_response_items_for_next_turn(vec![item]).await;
    session.maybe_start_turn_for_pending_work().await;
}

#[derive(Clone)]
struct CronSchedule {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

impl CronSchedule {
    fn parse(expr: &str) -> Result<Self, FunctionCallError> {
        let parts = expr.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 5 {
            return Err(FunctionCallError::RespondToModel(
                "CronCreate cron must be a standard 5-field expression: minute hour day-of-month month day-of-week".to_string(),
            ));
        }
        Ok(Self {
            minute: CronField::parse(parts[0], 0, 59)?,
            hour: CronField::parse(parts[1], 0, 23)?,
            day_of_month: CronField::parse(parts[2], 1, 31)?,
            month: CronField::parse(parts[3], 1, 12)?,
            day_of_week: CronField::parse(parts[4], 0, 7)?,
        })
    }

    fn next_after(
        &self,
        now: chrono::DateTime<Local>,
    ) -> Result<chrono::DateTime<Local>, FunctionCallError> {
        let mut candidate = now + chrono::Duration::minutes(1)
            - chrono::Duration::seconds(now.second() as i64)
            - chrono::Duration::nanoseconds(now.nanosecond() as i64);
        for _ in 0..=(60 * 24 * 8) {
            if self.matches(candidate) {
                return Ok(candidate);
            }
            candidate += chrono::Duration::minutes(1);
        }
        Err(FunctionCallError::RespondToModel(
            "CronCreate could not find a matching fire time in the next 8 days".to_string(),
        ))
    }

    fn matches(&self, time: chrono::DateTime<Local>) -> bool {
        let dow = time.weekday().num_days_from_sunday();
        self.minute.matches(time.minute())
            && self.hour.matches(time.hour())
            && self.day_of_month.matches(time.day())
            && self.month.matches(time.month())
            && (self.day_of_week.matches(dow) || (dow == 0 && self.day_of_week.matches(7)))
    }
}

#[derive(Clone)]
enum CronField {
    Any,
    Step(u32),
    Values(Vec<u32>),
}

impl CronField {
    fn parse(raw: &str, min: u32, max: u32) -> Result<Self, FunctionCallError> {
        if raw == "*" {
            return Ok(Self::Any);
        }
        if let Some(step) = raw.strip_prefix("*/") {
            let step = step.parse::<u32>().map_err(|_| {
                FunctionCallError::RespondToModel(format!("Invalid cron step field: {raw}"))
            })?;
            if step == 0 {
                return Err(FunctionCallError::RespondToModel(
                    "Cron step must be greater than zero".to_string(),
                ));
            }
            return Ok(Self::Step(step));
        }
        let mut values = Vec::new();
        for part in raw.split(',') {
            if let Some((start, end)) = part.split_once('-') {
                let start = parse_cron_value(start, raw, min, max)?;
                let end = parse_cron_value(end, raw, min, max)?;
                if start > end {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "Invalid cron range field: {raw}"
                    )));
                }
                values.extend(start..=end);
            } else {
                values.push(parse_cron_value(part, raw, min, max)?);
            }
        }
        values.sort_unstable();
        values.dedup();
        Ok(Self::Values(values))
    }

    fn matches(&self, value: u32) -> bool {
        match self {
            Self::Any => true,
            Self::Step(step) => value % step == 0,
            Self::Values(values) => values.contains(&value),
        }
    }
}

fn parse_cron_value(part: &str, raw: &str, min: u32, max: u32) -> Result<u32, FunctionCallError> {
    let value = part
        .parse::<u32>()
        .map_err(|_| FunctionCallError::RespondToModel(format!("Invalid cron field: {raw}")))?;
    if value < min || value > max {
        return Err(FunctionCallError::RespondToModel(format!(
            "Cron field {raw} contains value {value}, outside allowed range {min}-{max}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
#[path = "claude_code_tests.rs"]
mod tests;
