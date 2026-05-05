use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_policy::ExecApprovalRequest;
use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::events::ToolEmitter;
use crate::tools::events::ToolEventCtx;
use crate::tools::events::ToolEventFailure;
use crate::tools::events::ToolEventStage;
use crate::tools::handlers::apply_granted_turn_permissions;
use crate::tools::handlers::parse_kimi_arguments;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::shell::ShellRequest;
use crate::tools::runtimes::shell::ShellRuntime;
use crate::tools::runtimes::shell::ShellRuntimeBackend;
use crate::tools::sandboxing::ToolError;
use crate::unified_exec::ExecCommandRequest;
use crate::unified_exec::UnifiedExecContext;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ExecCommandSource;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub struct KimiShellHandler;

const KIMI_SHELL_EMPTY_OUTPUT: &str = "<system>Command executed successfully.</system>";
const KIMI_SHELL_DEFAULT_TIMEOUT_SECONDS: u64 = 60;
const KIMI_SHELL_MAX_FOREGROUND_TIMEOUT_SECONDS: u64 = 300;
const KIMI_SHELL_MAX_BACKGROUND_TIMEOUT_MS: u64 = 86_400_000;
const KIMI_SHELL_BACKGROUND_START_YIELD_MS: u64 = 250;
const KIMI_SHELL_MAX_OUTPUT_CHARS: usize = 50_000;
const KIMI_SHELL_MAX_LINE_CHARS: usize = 2_000;
const KIMI_SHELL_TRUNCATION_MARKER: &str = "[...truncated]";

#[derive(Deserialize)]
struct KimiShellArgs {
    command: String,
    timeout: Option<u64>,
    run_in_background: Option<bool>,
    description: Option<String>,
}

impl ToolHandler for KimiShellHandler {
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
                "Shell received unsupported payload".to_string(),
            ));
        };
        let args: KimiShellArgs = parse_kimi_arguments(&arguments)?;
        if args.run_in_background.unwrap_or(false) {
            return run_background_shell(session, turn, call_id, args).await;
        }
        if args.timeout.unwrap_or(KIMI_SHELL_DEFAULT_TIMEOUT_SECONDS)
            > KIMI_SHELL_MAX_FOREGROUND_TIMEOUT_SECONDS
        {
            return Err(FunctionCallError::RespondToModel(format!(
                "timeout must be <= {KIMI_SHELL_MAX_FOREGROUND_TIMEOUT_SECONDS}s for foreground commands; use run_in_background=true for longer timeouts (up to 86400s)"
            )));
        }
        let Some(_environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "Shell is unavailable in this session".to_string(),
            ));
        };
        let timeout_ms = Some(kimi_shell_timeout_ms(args.timeout));
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
                if output.exit_code == 0 {
                    Ok(kimi_shell_success_output(&output, turn.as_ref()))
                } else {
                    Ok(kimi_shell_failed_output(&output, turn.as_ref()))
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
                Ok(kimi_shell_failed_output(&output, turn.as_ref()))
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

async fn run_background_shell(
    session: std::sync::Arc<crate::session::session::Session>,
    turn: std::sync::Arc<TurnContext>,
    call_id: String,
    args: KimiShellArgs,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let Some(_environment) = turn.environment.as_ref() else {
        return Err(FunctionCallError::RespondToModel(
            "Shell is unavailable in this session".to_string(),
        ));
    };
    let description = args
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "Shell run_in_background requires a non-empty description.".to_string(),
            )
        })?
        .to_string();
    let timeout_ms = kimi_shell_timeout_ms(args.timeout);
    let effective_permissions = apply_granted_turn_permissions(
        session.as_ref(),
        turn.cwd.as_path(),
        SandboxPermissions::UseDefault,
        None,
    )
    .await;
    let command = session
        .user_shell()
        .derive_exec_args(&args.command, turn.tools_config.allow_login_shell);
    let manager = &session.services.unified_exec_manager;
    let process_id = manager.allocate_process_id().await;
    let output = manager
        .exec_command(
            ExecCommandRequest {
                command,
                hook_command: args.command.clone(),
                process_id,
                yield_time_ms: KIMI_SHELL_BACKGROUND_START_YIELD_MS,
                max_output_tokens: None,
                workdir: Some(turn.cwd.clone()),
                network: turn.network.clone(),
                tty: false,
                sandbox_permissions: effective_permissions.sandbox_permissions,
                additional_permissions: None,
                additional_permissions_preapproved: effective_permissions.permissions_preapproved,
                justification: Some(description.clone()),
                prefix_rule: None,
                preserve_on_shutdown: true,
            },
            &UnifiedExecContext::new(session.clone(), turn.clone(), call_id),
        )
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("Shell failed: {err}")))?;

    if let Some(process_id) = output.process_id {
        session
            .set_kimi_shell_task_description(process_id, description.clone())
            .await;
        spawn_background_shell_timeout(session, process_id, timeout_ms);
        Ok(kimi_background_shell_started_output(
            process_id,
            description,
            args.command,
        ))
    } else if output.exit_code == Some(0) {
        Ok(kimi_shell_success_output(
            &exec_command_output_to_shell_output(output),
            turn.as_ref(),
        ))
    } else {
        Ok(kimi_shell_failed_output(
            &exec_command_output_to_shell_output(output),
            turn.as_ref(),
        ))
    }
}

fn spawn_background_shell_timeout(
    session: std::sync::Arc<crate::session::session::Session>,
    process_id: i32,
    timeout_ms: u64,
) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
        session
            .services
            .unified_exec_manager
            .terminate_process_if_running(process_id)
            .await;
    });
}

fn kimi_shell_timeout_ms(timeout_seconds: Option<u64>) -> u64 {
    timeout_seconds
        .unwrap_or(KIMI_SHELL_DEFAULT_TIMEOUT_SECONDS)
        .saturating_mul(1000)
        .min(KIMI_SHELL_MAX_BACKGROUND_TIMEOUT_MS)
}

fn kimi_background_shell_started_output(
    process_id: i32,
    description: String,
    command: String,
) -> FunctionToolOutput {
    let body = [
        format!("task_id: {process_id}"),
        "kind: bash".to_string(),
        "status: running".to_string(),
        format!("description: {description}"),
        format!("command: {command}"),
        "automatic_notification: true".to_string(),
        "next_step: You will be automatically notified when it completes.".to_string(),
        "next_step: Use TaskOutput with this task_id for a non-blocking status/output snapshot. Only set block=true when you intentionally want to wait.".to_string(),
        "next_step: Use TaskStop only if the task must be cancelled.".to_string(),
        "human_shell_hint: For users in the interactive shell, the only task-management slash command is /task. Do not suggest /task list, /task output, /task stop, or /tasks.".to_string(),
    ]
    .join("\n");
    FunctionToolOutput::from_text(body, Some(true))
}

fn exec_command_output_to_shell_output(
    output: crate::tools::context::ExecCommandToolOutput,
) -> codex_protocol::exec_output::ExecToolCallOutput {
    let text = String::from_utf8_lossy(&output.raw_output).to_string();
    codex_protocol::exec_output::ExecToolCallOutput {
        exit_code: output.exit_code.unwrap_or(0),
        stdout: codex_protocol::exec_output::StreamOutput::new(text.clone()),
        stderr: codex_protocol::exec_output::StreamOutput::new(String::new()),
        aggregated_output: codex_protocol::exec_output::StreamOutput::new(text),
        duration: output.wall_time,
        timed_out: false,
    }
}

fn kimi_shell_success_output(
    output: &codex_protocol::exec_output::ExecToolCallOutput,
    _turn: &TurnContext,
) -> FunctionToolOutput {
    let output_text = kimi_shell_output_text(output);
    let mut body = vec![FunctionCallOutputContentItem::InputText {
        text: KIMI_SHELL_EMPTY_OUTPUT.to_string(),
    }];
    if !output_text.is_empty() {
        body.push(FunctionCallOutputContentItem::InputText { text: output_text });
    }
    let post_tool_use_response = function_output_items_to_json(&body);
    FunctionToolOutput {
        body,
        success: Some(true),
        post_tool_use_response,
    }
}

fn kimi_shell_failed_output(
    output: &codex_protocol::exec_output::ExecToolCallOutput,
    _turn: &TurnContext,
) -> FunctionToolOutput {
    let output_text = kimi_shell_output_text(output);
    let message = if output.timed_out {
        format!("Command killed by timeout ({}s)", output.duration.as_secs())
    } else {
        format!("Command failed with exit code: {}.", output.exit_code)
    };
    let mut body = vec![FunctionCallOutputContentItem::InputText {
        text: format!("<system>ERROR: {message}</system>"),
    }];
    if !output_text.is_empty() {
        body.push(FunctionCallOutputContentItem::InputText { text: output_text });
    }
    let post_tool_use_response = function_output_items_to_json(&body);
    FunctionToolOutput {
        body,
        success: Some(false),
        post_tool_use_response,
    }
}

fn kimi_shell_output_text(output: &codex_protocol::exec_output::ExecToolCallOutput) -> String {
    let combined_output = if !output.aggregated_output.text.is_empty() {
        kimi_shell_aggregate_output_text(output)
    } else if output.stderr.text.is_empty() && output.stdout.text.is_empty() {
        String::new()
    } else if output.stderr.text.is_empty() {
        output.stdout.text.clone()
    } else if output.stdout.text.is_empty() {
        output.stderr.text.clone()
    } else {
        format!("{}{}", output.stdout.text, output.stderr.text)
    };
    truncate_kimi_shell_output(&combined_output)
}

fn kimi_shell_aggregate_output_text(
    output: &codex_protocol::exec_output::ExecToolCallOutput,
) -> String {
    let stdout_then_stderr = format!("{}{}", output.stdout.text, output.stderr.text);
    if !output.stderr.text.is_empty()
        && output.aggregated_output.text == stdout_then_stderr
        && kimi_shell_diagnostic_precedes_stdout(&output.stderr.text)
    {
        format!("{}{}", output.stderr.text, output.stdout.text)
    } else {
        output.aggregated_output.text.clone()
    }
}

fn kimi_shell_diagnostic_precedes_stdout(stderr: &str) -> bool {
    stderr.starts_with("/bin/bash:")
        || stderr.starts_with("bash:")
        || stderr.starts_with("/bin/sh:")
        || stderr.starts_with("sh:")
        || stderr.lines().next().is_some_and(|line| {
            (line.starts_with("<stdin>:") || line.starts_with("<string>:"))
                && line.contains("Warning:")
        })
}

fn truncate_kimi_shell_output(text: &str) -> String {
    let mut output = String::new();
    let mut chars_written = 0usize;

    for line in split_lines_keepends(text) {
        if chars_written >= KIMI_SHELL_MAX_OUTPUT_CHARS {
            break;
        }
        let remaining_chars = KIMI_SHELL_MAX_OUTPUT_CHARS - chars_written;
        let line_limit = remaining_chars.min(KIMI_SHELL_MAX_LINE_CHARS);
        let line = truncate_kimi_shell_line(line, line_limit);
        chars_written += line.chars().count();
        output.push_str(&line);
    }
    output
}

fn split_lines_keepends(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').collect()
}

fn truncate_kimi_shell_line(line: &str, max_chars: usize) -> String {
    if line.chars().count() <= max_chars {
        return line.to_string();
    }

    let linebreak_start = line
        .char_indices()
        .rev()
        .find(|(_, ch)| !matches!(ch, '\r' | '\n'))
        .map_or(0, |(idx, ch)| idx + ch.len_utf8());
    let linebreak = &line[linebreak_start..];
    let suffix = format!("{KIMI_SHELL_TRUNCATION_MARKER}{linebreak}");
    let suffix_chars = suffix.chars().count();
    let prefix_chars = max_chars.saturating_sub(suffix_chars);
    let prefix = line.chars().take(prefix_chars).collect::<String>();
    format!("{prefix}{suffix}")
}

fn function_output_items_to_json(items: &[FunctionCallOutputContentItem]) -> Option<JsonValue> {
    let values = items
        .iter()
        .map(|item| match item {
            FunctionCallOutputContentItem::InputText { text } => JsonValue::String(text.clone()),
            FunctionCallOutputContentItem::InputImage { image_url, .. } => {
                JsonValue::String(image_url.clone())
            }
        })
        .collect::<Vec<_>>();
    match values.as_slice() {
        [single] => Some(single.clone()),
        _ => Some(JsonValue::Array(values)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::exec_output::ExecToolCallOutput;
    use codex_protocol::exec_output::StreamOutput;

    #[test]
    fn truncates_long_kimi_shell_lines_like_source_builder() {
        let input = format!("{}{}\nnext", "a".repeat(KIMI_SHELL_MAX_LINE_CHARS), "b");

        let output = truncate_kimi_shell_output(&input);

        assert!(output.contains(KIMI_SHELL_TRUNCATION_MARKER));
        assert!(output.contains("\nnext"));
        assert!(output.lines().next().unwrap().chars().count() <= KIMI_SHELL_MAX_LINE_CHARS);
    }

    #[test]
    fn truncates_total_kimi_shell_output() {
        let input = "x\n".repeat(KIMI_SHELL_MAX_OUTPUT_CHARS);

        let output = truncate_kimi_shell_output(&input);

        assert_eq!(output.chars().count(), KIMI_SHELL_MAX_OUTPUT_CHARS);
        assert!(!output.ends_with(KIMI_SHELL_TRUNCATION_MARKER));
    }

    #[test]
    fn formats_successful_kimi_shell_output_from_mixed_aggregate() {
        let output = ExecToolCallOutput {
            stdout: StreamOutput::new("stdout\n".to_string()),
            stderr: StreamOutput::new("stderr\n".to_string()),
            aggregated_output: StreamOutput::new("stdout\nstderr\n".to_string()),
            ..Default::default()
        };

        assert_eq!(kimi_shell_output_text(&output), "stdout\nstderr\n");
    }

    #[test]
    fn formats_kimi_shell_output_preserving_stderr_first_aggregate() {
        let output = ExecToolCallOutput {
            stdout: StreamOutput::new("stdout\n".to_string()),
            stderr: StreamOutput::new("warning\n".to_string()),
            aggregated_output: StreamOutput::new("warning\nstdout\n".to_string()),
            ..Default::default()
        };

        assert_eq!(kimi_shell_output_text(&output), "warning\nstdout\n");
    }

    #[test]
    fn formats_kimi_shell_output_with_shell_diagnostic_before_stdout() {
        let output = ExecToolCallOutput {
            stdout: StreamOutput::new("stdout\n".to_string()),
            stderr: StreamOutput::new("/bin/bash: command substitution failed\n".to_string()),
            aggregated_output: StreamOutput::new(
                "stdout\n/bin/bash: command substitution failed\n".to_string(),
            ),
            ..Default::default()
        };

        assert_eq!(
            kimi_shell_output_text(&output),
            "/bin/bash: command substitution failed\nstdout\n"
        );
    }

    #[test]
    fn formats_kimi_shell_output_with_python_warning_before_stdout() {
        let output = ExecToolCallOutput {
            stdout: StreamOutput::new("stdout\n".to_string()),
            stderr: StreamOutput::new("<stdin>:4: UserWarning: warning\n".to_string()),
            aggregated_output: StreamOutput::new(
                "stdout\n<stdin>:4: UserWarning: warning\n".to_string(),
            ),
            ..Default::default()
        };

        assert_eq!(
            kimi_shell_output_text(&output),
            "<stdin>:4: UserWarning: warning\nstdout\n"
        );
    }

    #[test]
    fn formats_kimi_shell_output_with_stdout_then_stderr_fallback() {
        let output = ExecToolCallOutput {
            stdout: StreamOutput::new("stdout\n".to_string()),
            stderr: StreamOutput::new("stderr\n".to_string()),
            aggregated_output: StreamOutput::new(String::new()),
            ..Default::default()
        };

        assert_eq!(kimi_shell_output_text(&output), "stdout\nstderr\n");
    }

    #[test]
    fn formats_kimi_shell_output_from_stdout_when_stderr_is_empty() {
        let output = ExecToolCallOutput {
            stdout: StreamOutput::new("combined\n".to_string()),
            stderr: StreamOutput::new(String::new()),
            aggregated_output: StreamOutput::new("combined\n".to_string()),
            ..Default::default()
        };

        assert_eq!(kimi_shell_output_text(&output), "combined\n");
    }

    #[test]
    fn kimi_shell_timeout_matches_source_default_and_maximum() {
        assert_eq!(
            kimi_shell_timeout_ms(None),
            KIMI_SHELL_DEFAULT_TIMEOUT_SECONDS * 1000
        );
        assert_eq!(kimi_shell_timeout_ms(Some(300)), 300_000);
        assert_eq!(
            kimi_shell_timeout_ms(Some(86_401)),
            KIMI_SHELL_MAX_BACKGROUND_TIMEOUT_MS
        );
    }
}
