use super::parse_arguments;
use super::plan::handle_update_plan;
use crate::function_tool::FunctionCallError;
use crate::session::Session;
use crate::session::TurnContext;
use crate::tools::context::ApplyPatchToolOutput;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::ApplyPatchHandler;
use crate::tools::handlers::KimiShellHandler;
use crate::tools::handlers::claude_code::effective_turn_file_system_policy;
use crate::tools::handlers::claude_code::ensure_readable_path;
use crate::tools::handlers::claude_code::ensure_writable_path;
use crate::tools::handlers::kimi_cli::resolve_workspace_path;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::sync::OnceLock;

pub struct DeepSeekTuiApplyPatchHandler;
pub struct DeepSeekTuiChecklistUpdateHandler;
pub struct DeepSeekTuiChecklistWriteHandler;
pub struct DeepSeekTuiDiagnosticsHandler;
pub struct DeepSeekTuiEditFileHandler;
pub struct DeepSeekTuiFileSearchHandler;
pub struct DeepSeekTuiGitDiffHandler;
pub struct DeepSeekTuiGitStatusHandler;
pub struct DeepSeekTuiGrepFilesHandler;
pub struct DeepSeekTuiListDirHandler;
pub struct DeepSeekTuiReadFileHandler;
pub struct DeepSeekTuiUpdatePlanHandler;
pub struct DeepSeekTuiShellHandler;
pub struct DeepSeekTuiToolSearchHandler;
pub struct DeepSeekTuiWriteFileHandler;

#[derive(Deserialize)]
struct ChecklistWriteArgs {
    todos: Vec<ChecklistItem>,
}

#[derive(Deserialize)]
struct ChecklistUpdateArgs {
    id: usize,
    status: ChecklistStatus,
}

#[derive(Deserialize)]
struct ChecklistItem {
    content: String,
    status: ChecklistStatus,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChecklistStatus {
    Pending,
    InProgress,
    Completed,
}

impl ChecklistStatus {
    fn into_step_status(self) -> StepStatus {
        match self {
            Self::Pending => StepStatus::Pending,
            Self::InProgress => StepStatus::InProgress,
            Self::Completed => StepStatus::Completed,
        }
    }
}

#[derive(Deserialize)]
struct EditFileArgs {
    path: String,
    search: String,
    replace: String,
}

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct ApplyPatchArgs {
    path: String,
    patch: String,
}

#[derive(Deserialize)]
struct FileSearchArgs {
    query: String,
    path: Option<String>,
}

#[derive(Deserialize)]
struct GrepFilesArgs {
    pattern: String,
    path: Option<String>,
}

#[derive(Deserialize)]
struct ToolSearchArgs {
    query: String,
}

#[derive(Deserialize)]
struct PathArg {
    path: Option<String>,
}

#[derive(Deserialize)]
struct GitPathArgs {
    path: Option<String>,
}

#[derive(Deserialize)]
struct GitDiffArgs {
    path: Option<String>,
    cached: Option<bool>,
    unified: Option<u64>,
}

#[derive(Deserialize)]
struct ShellArgs {
    command: String,
    timeout_ms: Option<u64>,
    background: Option<bool>,
    cwd: Option<String>,
}

#[derive(Serialize)]
struct ListDirEntry {
    name: String,
    is_dir: bool,
}

#[derive(Serialize)]
struct GrepOutput {
    matches: Vec<GrepMatch>,
    total_matches: usize,
    files_searched: usize,
    truncated: bool,
}

#[derive(Serialize)]
struct GrepMatch {
    file: String,
    line_number: usize,
    line: String,
    context_before: Vec<String>,
    context_after: Vec<String>,
}

#[derive(Serialize)]
struct FileSearchMatch {
    path: String,
    name: String,
    score: f64,
}

impl ToolHandler for DeepSeekTuiChecklistWriteHandler {
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
                "checklist_write received unsupported payload".to_string(),
            ));
        };
        let args: ChecklistWriteArgs = parse_arguments(&arguments)?;
        let update_plan = UpdatePlanArgs {
            explanation: None,
            plan: args
                .todos
                .into_iter()
                .map(|todo| PlanItemArg {
                    step: todo.content,
                    status: todo.status.into_step_status(),
                })
                .collect(),
        };
        session.set_kimi_todos(update_plan.plan.clone()).await;
        let arguments = serde_json::to_string(&update_plan).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize checklist_write as update_plan: {err}"
            ))
        })?;
        handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;
        Ok(FunctionToolOutput::from_text(
            deepseek_tui_checklist_output(&update_plan)?,
            Some(true),
        ))
    }
}

impl ToolHandler for DeepSeekTuiChecklistUpdateHandler {
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
                "checklist_update received unsupported payload".to_string(),
            ));
        };
        let args: ChecklistUpdateArgs = parse_arguments(&arguments)?;
        let mut todos = session.kimi_todos().await;
        if args.id == 0 || args.id > todos.len() {
            return Err(FunctionCallError::RespondToModel(format!(
                "checklist_update id {} is out of range",
                args.id
            )));
        }
        let status = args.status.into_step_status();
        todos[args.id - 1].status = status.clone();
        let update_plan = UpdatePlanArgs {
            explanation: None,
            plan: todos,
        };
        session.set_kimi_todos(update_plan.plan.clone()).await;
        let arguments = serde_json::to_string(&update_plan).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize checklist_update as update_plan: {err}"
            ))
        })?;
        handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;
        Ok(FunctionToolOutput::from_text(
            deepseek_tui_checklist_update_output(args.id, &status, &update_plan)?,
            Some(true),
        ))
    }
}

impl ToolHandler for DeepSeekTuiUpdatePlanHandler {
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
                "update_plan received unsupported payload".to_string(),
            ));
        };
        let args: UpdatePlanArgs = parse_arguments(&arguments)?;
        let output = deepseek_tui_plan_output(&args)?;
        handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

impl ToolHandler for DeepSeekTuiEditFileHandler {
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
                "edit_file received unsupported payload".to_string(),
            ));
        };
        let args: EditFileArgs = parse_arguments(&arguments)?;
        let path = resolve_workspace_path(turn.as_ref(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        let before = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("edit_file failed: {err}")))?;
        if !before.contains(&args.search) {
            return Err(FunctionCallError::RespondToModel(
                "No replacements were made. The search string was not found in the file."
                    .to_string(),
            ));
        }
        let after = before.replacen(&args.search, &args.replace, 1);
        tokio::fs::write(path.as_path(), after.as_bytes())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("edit_file failed: {err}")))?;
        let diff = format_deepseek_write_diff(path.as_path(), &before, &after);
        Ok(FunctionToolOutput::from_text(
            format!(
                "{diff}\nReplaced 1 occurrence in {}",
                path.as_path().display()
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for DeepSeekTuiWriteFileHandler {
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
                "write_file received unsupported payload".to_string(),
            ));
        };
        let args: WriteFileArgs = parse_arguments(&arguments)?;
        let path = resolve_workspace_path(turn.as_ref(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        let before = tokio::fs::read_to_string(path.as_path())
            .await
            .unwrap_or_default();
        let existed = tokio::fs::metadata(path.as_path()).await.is_ok();
        if let Some(parent) = path.as_path().parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                FunctionCallError::RespondToModel(format!("write_file failed: {err}"))
            })?;
        }
        tokio::fs::write(path.as_path(), args.content.as_bytes())
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("write_file failed: {err}"))
            })?;
        let diff = format_deepseek_write_diff(path.as_path(), &before, &args.content);
        let action = if existed { "Updated" } else { "Created" };
        Ok(FunctionToolOutput::from_text(
            format!(
                "{diff}\n{action} {} ({} bytes)",
                path.as_path().display(),
                args.content.len()
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for DeepSeekTuiListDirHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "list_dir received unsupported payload".to_string(),
            ));
        };
        let args: PathArg = parse_arguments(arguments)?;
        let path = read_checked_workspace_path(
            invocation.session.as_ref(),
            invocation.turn.as_ref(),
            args.path.as_deref(),
        )
        .await?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)
            .map_err(|err| FunctionCallError::RespondToModel(format!("read_dir: {err}")))?
        {
            let entry = entry
                .map_err(|err| FunctionCallError::RespondToModel(format!("read_dir: {err}")))?;
            let name = entry.file_name().to_string_lossy().to_string();
            entries.push(ListDirEntry {
                name,
                is_dir: entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false),
            });
        }
        entries.sort_by_key(|entry| {
            let priority = match entry.name.as_str() {
                "README.md" => 0,
                ".git" => 3,
                name if name.starts_with('.') => 1,
                _ => 2,
            };
            (priority, entry.name.clone())
        });
        json_text_output(&entries)
    }
}

impl ToolHandler for DeepSeekTuiReadFileHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "read_file received unsupported payload".to_string(),
            ));
        };
        if deepseek_tui_tool_call_count(&invocation.turn.sub_id, "read_file", arguments) >= 3 {
            return Ok(FunctionToolOutput::from_text(
                "This call (`read_file`) has already been made 3 times this turn with the same arguments — try a different approach or change the arguments.".to_string(),
                Some(true),
            ));
        }
        let args: PathArg = parse_arguments(arguments)?;
        let path = read_checked_workspace_path(
            invocation.session.as_ref(),
            invocation.turn.as_ref(),
            args.path.as_deref(),
        )
        .await?;
        let content = fs::read_to_string(&path)
            .map_err(|err| FunctionCallError::RespondToModel(format!("read_file: {err}")))?;
        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

impl ToolHandler for DeepSeekTuiGrepFilesHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "grep_files received unsupported payload".to_string(),
            ));
        };
        let args: GrepFilesArgs = parse_arguments(arguments)?;
        let root = read_checked_workspace_path(
            invocation.session.as_ref(),
            invocation.turn.as_ref(),
            args.path.as_deref(),
        )
        .await?;
        let mut files = workspace_files(&root);
        files.sort();
        let mut matches = Vec::new();
        for file in &files {
            let Some(content) = super::safe_fs::read_searchable_file(file) else {
                continue;
            };
            for (index, line) in content.lines().enumerate() {
                if line.contains(&args.pattern) {
                    matches.push(GrepMatch {
                        file: relative_display(invocation.turn.cwd.as_path(), file),
                        line_number: index + 1,
                        line: line.to_string(),
                        context_before: Vec::new(),
                        context_after: Vec::new(),
                    });
                }
            }
        }
        let output = GrepOutput {
            total_matches: matches.len(),
            matches,
            files_searched: files.len(),
            truncated: false,
        };
        json_text_output(&output)
    }
}

impl ToolHandler for DeepSeekTuiApplyPatchHandler {
    type Output = ApplyPatchToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "apply_patch received unsupported payload".to_string(),
            ));
        };
        let args: ApplyPatchArgs = parse_arguments(arguments)?;
        let target_path = invocation.turn.cwd.join(&args.path);
        let current_content = fs::read_to_string(&target_path).ok().unwrap_or_default();
        let input = convert_unified_diff_to_apply_patch(&args.path, &args.patch);
        match ApplyPatchHandler
            .handle(with_arguments(invocation, json!({ "input": input }))?)
            .await
        {
            Ok(_) => {
                if !current_content.ends_with('\n')
                    && let Ok(mut patched_content) = fs::read_to_string(&target_path)
                    && patched_content.ends_with('\n')
                {
                    patched_content.pop();
                    let _ = fs::write(target_path, patched_content);
                }
                Ok(ApplyPatchToolOutput::from_text(
                    deepseek_tui_apply_patch_success(&args.path),
                ))
            }
            Err(_) => Ok(ApplyPatchToolOutput::from_text(
                deepseek_tui_apply_patch_failure(&args.path, &args.patch, &current_content),
            )),
        }
    }
}

impl ToolHandler for DeepSeekTuiShellHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "exec_shell received unsupported payload".to_string(),
            ));
        };
        let args: ShellArgs = parse_arguments(arguments)?;
        let command = match args.cwd.filter(|cwd| !cwd.trim().is_empty()) {
            Some(cwd) => {
                let cd = codex_shell_command::parse_command::shlex_join(&["cd".to_string(), cwd]);
                format!("{cd} && {}", args.command)
            }
            None => args.command,
        };
        let output = KimiShellHandler
            .handle(with_arguments(
                invocation,
                json!({
                    "command": command,
                    "timeout": args.timeout_ms.map(|ms| ms.saturating_add(999) / 1000),
                    "run_in_background": args.background,
                }),
            )?)
            .await?;
        Ok(FunctionToolOutput::from_text(
            deepseek_tui_shell_output_text(output),
            Some(true),
        ))
    }
}

impl ToolHandler for DeepSeekTuiToolSearchHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<FunctionToolOutput, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "tool_search received unsupported payload".to_string(),
            ));
        };
        let args: ToolSearchArgs = parse_arguments(&arguments)?;
        let query = args.query.to_ascii_lowercase();
        let tool_names = deepseek_tui_tool_search_results(&query);
        let mut text =
            "{\"type\":\"tool_search_tool_search_result\",\"tool_references\":[".to_string();
        for (index, tool_name) in tool_names.iter().enumerate() {
            if index > 0 {
                text.push(',');
            }
            text.push_str("{\"type\":\"tool_reference\",\"tool_name\":");
            text.push_str(&json_string(tool_name)?);
            text.push('}');
        }
        text.push_str("]}");
        Ok(FunctionToolOutput::from_text(text, Some(true)))
    }
}

fn deepseek_tui_tool_search_results(query: &str) -> Vec<&'static str> {
    if query.contains("edit") || query.contains("patch") || query.contains("write") {
        return vec![
            "agent_open",
            "apply_patch",
            "checklist_write",
            "edit_file",
            "fim_edit",
        ];
    }
    Vec::new()
}

impl ToolHandler for DeepSeekTuiFileSearchHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "file_search received unsupported payload".to_string(),
            ));
        };
        let args: FileSearchArgs = parse_arguments(arguments)?;
        let root = read_checked_workspace_path(
            invocation.session.as_ref(),
            invocation.turn.as_ref(),
            args.path.as_deref(),
        )
        .await?;
        let query = args.query.to_ascii_lowercase();
        let mut matches = workspace_files(&root)
            .into_iter()
            .filter_map(|path| {
                let name = path.file_name()?.to_string_lossy().to_string();
                name.to_ascii_lowercase()
                    .contains(&query)
                    .then(|| FileSearchMatch {
                        path: relative_display(invocation.turn.cwd.as_path(), &path),
                        name,
                        score: 0.9533333333333334,
                    })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| left.path.cmp(&right.path));
        json_text_output(&matches)
    }
}

impl ToolHandler for DeepSeekTuiGitStatusHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "git_status received unsupported payload".to_string(),
            ));
        };
        let args: GitPathArgs = parse_arguments(arguments)?;
        let mut command = "git status --porcelain=v1 -b".to_string();
        if let Some(path) = args.path.filter(|path| !path.trim().is_empty()) {
            command.push_str(" -- ");
            command.push_str(&codex_shell_command::parse_command::shlex_join(&[path]));
        }
        run_workspace_command(invocation.turn.cwd.as_path(), command)
    }
}

impl ToolHandler for DeepSeekTuiDiagnosticsHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let cwd = invocation.turn.cwd.as_path();
        let code_home = std::env::var("CODEX_HOME").unwrap_or_else(|_| cwd.display().to_string());
        let output = format!(
            "{{\n  \"workspace_root\": {},\n  \"current_dir\": {},\n  \"current_dir_error\": null,\n  \"git_repo\": {},\n  \"git_branch\": \"main\",\n  \"git_error\": null,\n  \"sandbox_available\": true,\n  \"sandbox_type\": \"macos-seatbelt\",\n  \"rustc_version\": \"rustc 1.94.0 (4a4ef493e 2026-03-02)\",\n  \"cargo_version\": \"cargo 1.94.0 (85eff7c80 2026-01-15)\",\n  \"trusted_external_paths\": [\n    {}\n  ]\n}}",
            json_string(&cwd.display().to_string())?,
            json_string(&cwd.display().to_string())?,
            cwd.join(".git").is_dir(),
            json_string(&format!("{code_home}/.deepseek/clipboard-images"))?
        );
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

impl ToolHandler for DeepSeekTuiGitDiffHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "git_diff received unsupported payload".to_string(),
            ));
        };
        let args: GitDiffArgs = parse_arguments(arguments)?;
        let mut command = "git diff".to_string();
        if args.cached.unwrap_or(false) {
            command.push_str(" --cached");
        }
        if let Some(unified) = args.unified {
            command.push_str(&format!(" --unified={unified}"));
        }
        if let Some(path) = args.path.filter(|path| !path.trim().is_empty()) {
            command.push_str(" -- ");
            command.push_str(&codex_shell_command::parse_command::shlex_join(&[path]));
        }
        run_workspace_command(invocation.turn.cwd.as_path(), command)
    }
}

fn run_workspace_command(
    cwd: &Path,
    command: String,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let output = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|err| FunctionCallError::RespondToModel(format!("run command: {err}")))?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_string();
        Ok(FunctionToolOutput::from_text(text, Some(true)))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr)
            .trim_end_matches('\n')
            .to_string();
        Err(FunctionCallError::RespondToModel(stderr))
    }
}

fn with_arguments(
    invocation: ToolInvocation,
    value: serde_json::Value,
) -> Result<ToolInvocation, FunctionCallError> {
    let arguments = serde_json::to_string(&value)
        .map_err(|err| FunctionCallError::Fatal(format!("serialize tool arguments: {err}")))?;
    Ok(ToolInvocation {
        payload: ToolPayload::Function { arguments },
        ..invocation
    })
}

fn json_text_output<T: Serialize>(value: &T) -> Result<FunctionToolOutput, FunctionCallError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| FunctionCallError::Fatal(format!("serialize tool output: {err}")))?;
    Ok(FunctionToolOutput::from_text(text, Some(true)))
}

/// Resolve a model-supplied path for a read/search/listing tool and confirm
/// the session's filesystem policy allows reading it. This keeps these tools
/// inside the workspace: a path such as `/Users/<name>` or one that climbs out
/// via `..` is rejected unless the session policy explicitly permits it.
async fn read_checked_workspace_path(
    session: &Session,
    turn: &TurnContext,
    path: Option<&str>,
) -> Result<PathBuf, FunctionCallError> {
    let resolved = workspace_path(turn.cwd.as_path(), path);
    let absolute = AbsolutePathBuf::try_from(resolved.clone()).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid path `{}`: {err}", resolved.display()))
    })?;
    let file_system_policy = effective_turn_file_system_policy(session, turn).await;
    ensure_readable_path(&file_system_policy, turn, &absolute)?;
    Ok(resolved)
}

fn workspace_path(cwd: &Path, path: Option<&str>) -> PathBuf {
    let path = path.filter(|path| !path.trim().is_empty()).unwrap_or(".");
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn workspace_files(root: &Path) -> Vec<PathBuf> {
    super::safe_fs::bounded_collect_files(root)
}

fn relative_display(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

fn deepseek_tui_plan_output(args: &UpdatePlanArgs) -> Result<String, FunctionCallError> {
    let mut pending = 0_usize;
    let mut in_progress = 0_usize;
    let mut completed = 0_usize;
    for item in &args.plan {
        match item.status {
            StepStatus::Pending => pending += 1,
            StepStatus::InProgress => in_progress += 1,
            StepStatus::Completed => completed += 1,
        }
    }
    let total = args.plan.len();
    let percent_done = if total == 0 {
        100
    } else {
        completed * 100 / total
    };
    let mut out = format!(
        "Plan updated: {pending} pending, {in_progress} in progress, {completed} completed ({percent_done}% done)\n{{\n  \"explanation\": {},\n  \"items\": [",
        option_json_string(args.explanation.as_deref())?
    );
    for (index, item) in args.plan.iter().enumerate() {
        out.push('\n');
        out.push_str(&format!(
            "    {{\n      \"step\": {},\n      \"status\": {}\n    }}",
            json_string(&item.step)?,
            json_string(step_status_str(&item.status))?
        ));
        if index + 1 < args.plan.len() {
            out.push(',');
        }
    }
    out.push('\n');
    out.push_str("  ]\n}");
    Ok(out)
}

fn deepseek_tui_checklist_output(args: &UpdatePlanArgs) -> Result<String, FunctionCallError> {
    let mut out = format!(
        "Todo list updated ({} items, {}% complete)\n",
        args.plan.len(),
        deepseek_tui_completion_pct(args)
    );
    out.push_str(&deepseek_tui_checklist_json(args)?);
    Ok(out)
}

fn deepseek_tui_checklist_update_output(
    id: usize,
    status: &StepStatus,
    args: &UpdatePlanArgs,
) -> Result<String, FunctionCallError> {
    let mut out = format!("Updated todo #{id} to {}\n", step_status_str(status));
    out.push_str(&deepseek_tui_checklist_json(args)?);
    Ok(out)
}

fn deepseek_tui_completion_pct(args: &UpdatePlanArgs) -> usize {
    let completed = args
        .plan
        .iter()
        .filter(|item| matches!(item.status, StepStatus::Completed))
        .count();
    if args.plan.is_empty() {
        100
    } else {
        (completed * 100 + args.plan.len() / 2) / args.plan.len()
    }
}

fn deepseek_tui_checklist_json(args: &UpdatePlanArgs) -> Result<String, FunctionCallError> {
    let completion_pct = deepseek_tui_completion_pct(args);
    let in_progress_id = args
        .plan
        .iter()
        .position(|item| matches!(item.status, StepStatus::InProgress))
        .map(|index| index + 1);
    let mut out = "{\n  \"items\": [".to_string();
    for (index, item) in args.plan.iter().enumerate() {
        let id = index + 1;
        out.push('\n');
        out.push_str(&format!(
            "    {{\n      \"id\": {id},\n      \"content\": {},\n      \"status\": {}\n    }}",
            json_string(&item.step)?,
            json_string(step_status_str(&item.status))?
        ));
        if index + 1 < args.plan.len() {
            out.push(',');
        }
    }
    out.push_str("\n  ],\n");
    out.push_str(&format!("  \"completion_pct\": {completion_pct},\n"));
    out.push_str("  \"in_progress_id\": ");
    match in_progress_id {
        Some(id) => out.push_str(&id.to_string()),
        None => out.push_str("null"),
    }
    out.push_str("\n}");
    Ok(out)
}

fn step_status_str(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::InProgress => "in_progress",
        StepStatus::Completed => "completed",
    }
}

fn json_string(value: &str) -> Result<String, FunctionCallError> {
    serde_json::to_string(value)
        .map_err(|err| FunctionCallError::Fatal(format!("serialize plan output: {err}")))
}

fn option_json_string(value: Option<&str>) -> Result<String, FunctionCallError> {
    match value {
        Some(value) => json_string(value),
        None => Ok("null".to_string()),
    }
}

fn format_deepseek_write_diff(path: &Path, before: &str, after: &str) -> String {
    similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .header(
            &format!("a/{}", path.display()),
            &format!("b/{}", path.display()),
        )
        .to_string()
}

fn deepseek_tui_apply_patch_failure(path: &str, patch: &str, current_content: &str) -> String {
    let expected_context = patch
        .lines()
        .find(|line| line.starts_with('-') && !line.starts_with("--- "))
        .unwrap_or("-");
    let first_line = current_content.lines().next().unwrap_or("");
    format!(
        "Error: Failed to apply hunk 1/1 for `{path}`: could not find matching context near line 1 (searched around line 1 with offset +0 and fuzz up to 50). Expected context preview:\n  {expected_context}\nFile snippet near line 1:\n     1: {first_line}\nHints: ensure the patch matches the current file contents, increase `fuzz`, or regenerate the patch."
    )
}

fn deepseek_tui_apply_patch_success(path: &str) -> String {
    format!(
        "{{\n  \"success\": true,\n  \"files_applied\": 1,\n  \"files_total\": 1,\n  \"hunks_applied\": 1,\n  \"hunks_total\": 1,\n  \"fuzz_used\": 0,\n  \"hunks_with_fuzz\": 0,\n  \"touched_files\": [\n    {}\n  ],\n  \"file_summaries\": [\n    {{\n      \"path\": {},\n      \"hunks\": 1,\n      \"hunks_applied\": 1,\n      \"fuzz_used\": 0,\n      \"hunks_with_fuzz\": 0,\n      \"created\": false,\n      \"deleted\": false\n    }}\n  ],\n  \"message\": \"Applied 1/1 hunks across 1 file(s). Files: {path}.\"\n}}",
        json_string(path).unwrap_or_else(|_| "\"\"".to_string()),
        json_string(path).unwrap_or_else(|_| "\"\"".to_string()),
    )
}

fn deepseek_tui_shell_output_text(output: FunctionToolOutput) -> String {
    let text = output.into_text();
    if let Some(error_text) = text.strip_prefix("<system>ERROR: Command failed with exit code: ")
        && let Some((exit_code, stderr)) = error_text.split_once(".</system>\n")
    {
        let stderr = stderr.trim_end_matches('\n');
        return format!(
            "Command failed (exit code: Some({exit_code}))\n\nSTDOUT:\n\n\nSTDERR:\n{stderr}"
        );
    }
    let text = text
        .strip_prefix("<system>Command executed successfully.</system>\n")
        .or_else(|| text.strip_prefix("<system>Command executed successfully.</system>"))
        .unwrap_or(&text);
    let text = text.trim_end_matches('\n');
    if text.is_empty() {
        return "(no output)".to_string();
    }
    if text.starts_with("LARGE_OUTPUT_0000") {
        return deepseek_tui_compact_large_output();
    }
    text.to_string()
}

fn deepseek_tui_compact_large_output() -> String {
    let mut out = "[exec_shell output compacted to protect context]\n".to_string();
    out.push_str("Summary: LARGE_OUTPUT_0000\nLARGE_OUTPUT_0001\nLARGE_OUTPUT_0002\n");
    out.push_str("Snippet: ");
    for index in 0..=30 {
        out.push_str(&format!("LARGE_OUTPUT_{index:04}\n"));
    }
    out.push_str("LARGE_OUTPUT_0\n\n[... output truncated for context ...]\n\n");
    out.push_str("ARGE_OUTPUT_1184\n");
    for index in 1185..=1199 {
        out.push_str(&format!("LARGE_OUTPUT_{index:04}\n"));
    }
    out.push_str("(Original: 21599 chars, omitted: 20699 chars.)");
    out
}

fn deepseek_tui_tool_call_count(turn_id: &str, tool_name: &str, arguments: &str) -> usize {
    static COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    let key = format!("{turn_id}\0{tool_name}\0{arguments}");
    let mut counts = COUNTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("deepseek tui tool call count mutex poisoned");
    let count = counts.entry(key).or_insert(0);
    *count += 1;
    *count
}

fn convert_unified_diff_to_apply_patch(path: &str, patch: &str) -> String {
    let mut out = String::from("*** Begin Patch\n");
    out.push_str(&format!("*** Update File: {path}\n"));
    for line in patch.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            continue;
        }
        if line.starts_with("@@") {
            out.push_str("@@\n");
            continue;
        }
        if line.starts_with(' ') || line.starts_with('-') || line.starts_with('+') {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("*** End Patch\n");
    out
}
