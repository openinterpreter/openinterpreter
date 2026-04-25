use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::claude_code::effective_turn_file_system_policy;
use crate::tools::handlers::claude_code::ensure_readable_path;
use crate::tools::handlers::claude_code::parse_absolute_path;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use std::path::Path;
use std::time::SystemTime;
use tokio::process::Command;

pub struct ClaudeGlobHandler;
pub struct ClaudeGrepHandler;

const CLAUDE_GREP_DEFAULT_HEAD_LIMIT: usize = 250;

#[derive(Deserialize)]
struct ClaudeGlobArgs {
    pattern: String,
    path: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeGrepArgs {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    output_mode: Option<ClaudeGrepOutputMode>,
    #[serde(rename = "-B")]
    before_context: Option<usize>,
    #[serde(rename = "-A")]
    after_context: Option<usize>,
    #[serde(rename = "-C")]
    short_context: Option<usize>,
    context: Option<usize>,
    #[serde(rename = "-n")]
    line_numbers: Option<bool>,
    #[serde(rename = "-i")]
    case_insensitive: Option<bool>,
    #[serde(rename = "type")]
    file_type: Option<String>,
    head_limit: Option<usize>,
    offset: Option<usize>,
    multiline: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaudeGrepOutputMode {
    Content,
    FilesWithMatches,
    Count,
}

impl ToolHandler for ClaudeGlobHandler {
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
                "Glob received unsupported payload".to_string(),
            ));
        };

        let args: ClaudeGlobArgs = parse_arguments(&arguments)?;
        let search_root =
            resolve_search_root(session.as_ref(), turn.as_ref(), args.path.as_deref()).await?;
        if !search_root.as_path().is_dir() {
            return Err(FunctionCallError::RespondToModel(format!(
                "Glob path {} is not a directory",
                search_root.display()
            )));
        }

        let output = run_rg_command(
            [
                "--files",
                "--hidden",
                "--glob",
                &args.pattern,
                search_root.as_path().to_string_lossy().as_ref(),
            ],
            turn.cwd.as_path(),
        )
        .await?;
        let mut paths = parse_results(&output.stdout, usize::MAX)
            .into_iter()
            .map(|path| canonicalize_rg_path(search_root.as_path(), Path::new(&path)))
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort_by(compare_paths_by_modified_desc);
        let output = paths
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

impl ToolHandler for ClaudeGrepHandler {
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
                "Grep received unsupported payload".to_string(),
            ));
        };

        let args: ClaudeGrepArgs = parse_arguments(&arguments)?;
        let search_root =
            resolve_search_root(session.as_ref(), turn.as_ref(), args.path.as_deref()).await?;
        let mut rg_args = vec![
            "--color".to_string(),
            "never".to_string(),
            "--no-heading".to_string(),
        ];

        let output_mode = args
            .output_mode
            .unwrap_or(ClaudeGrepOutputMode::FilesWithMatches);
        match output_mode {
            ClaudeGrepOutputMode::Content => {
                if args.line_numbers.unwrap_or(true) {
                    rg_args.push("--line-number".to_string());
                }
                if let Some(context) = args.context.or(args.short_context) {
                    rg_args.push("--context".to_string());
                    rg_args.push(context.to_string());
                } else {
                    if let Some(before_context) = args.before_context {
                        rg_args.push("--before-context".to_string());
                        rg_args.push(before_context.to_string());
                    }
                    if let Some(after_context) = args.after_context {
                        rg_args.push("--after-context".to_string());
                        rg_args.push(after_context.to_string());
                    }
                }
            }
            ClaudeGrepOutputMode::FilesWithMatches => {
                rg_args.push("--files-with-matches".to_string());
            }
            ClaudeGrepOutputMode::Count => {
                rg_args.push("--count".to_string());
            }
        }

        if args.case_insensitive.unwrap_or(false) {
            rg_args.push("--ignore-case".to_string());
        }
        if let Some(glob) = args.glob {
            rg_args.push("--glob".to_string());
            rg_args.push(glob);
        }
        if let Some(file_type) = args.file_type {
            rg_args.push("--type".to_string());
            rg_args.push(file_type);
        }
        if args.multiline.unwrap_or(false) {
            rg_args.push("--multiline".to_string());
            rg_args.push("--multiline-dotall".to_string());
        }

        rg_args.push(args.pattern);
        rg_args.push(search_root.as_path().to_string_lossy().into_owned());

        let output = run_rg_command(rg_args.iter().map(String::as_str), turn.cwd.as_path()).await?;
        let lines = String::from_utf8_lossy(&output.stdout)
            .lines()
            .skip(args.offset.unwrap_or(0))
            .take(args.head_limit.unwrap_or(CLAUDE_GREP_DEFAULT_HEAD_LIMIT))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(FunctionToolOutput::from_text(lines, Some(true)))
    }
}

pub(super) async fn resolve_search_root(
    session: &crate::session::Session,
    turn: &TurnContext,
    path: Option<&str>,
) -> Result<AbsolutePathBuf, FunctionCallError> {
    let root = match path {
        Some(path) => parse_absolute_path(path)?,
        None => turn.cwd.clone(),
    };
    let file_system_policy = effective_turn_file_system_policy(session, turn).await;
    ensure_readable_path(&file_system_policy, turn, &root)?;
    Ok(root)
}

pub(super) fn parse_results(stdout: &[u8], limit: usize) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .take(limit)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
async fn run_rg_search(
    pattern: &str,
    glob: Option<&str>,
    path: &Path,
    limit: usize,
    cwd: &Path,
) -> Result<Vec<String>, FunctionCallError> {
    let mut args = vec![
        "--color".to_string(),
        "never".to_string(),
        "--no-heading".to_string(),
        "--files-with-matches".to_string(),
    ];
    if let Some(glob) = glob {
        args.push("--glob".to_string());
        args.push(glob.to_string());
    }
    args.push(pattern.to_string());
    args.push(path.to_string_lossy().into_owned());
    let output = run_rg_command(args.iter().map(String::as_str), cwd).await?;
    Ok(parse_results(&output.stdout, limit))
}

pub(super) async fn run_rg_command<'a>(
    args: impl IntoIterator<Item = &'a str>,
    cwd: &Path,
) -> Result<std::process::Output, FunctionCallError> {
    let output = Command::new("rg")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
    if output.status.success() || output.status.code() == Some(1) {
        Ok(output)
    } else {
        Err(FunctionCallError::RespondToModel(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

pub(super) fn canonicalize_rg_path(
    search_root: &Path,
    path: &Path,
) -> Result<AbsolutePathBuf, FunctionCallError> {
    if path.is_absolute() {
        AbsolutePathBuf::try_from(path.to_path_buf()).map_err(|err| {
            FunctionCallError::RespondToModel(format!("Glob produced an invalid path: {err}"))
        })
    } else {
        AbsolutePathBuf::try_from(search_root.join(path)).map_err(|err| {
            FunctionCallError::RespondToModel(format!("Glob produced an invalid path: {err}"))
        })
    }
}

pub(super) fn compare_paths_by_modified_desc(
    left: &AbsolutePathBuf,
    right: &AbsolutePathBuf,
) -> std::cmp::Ordering {
    let left_modified = std::fs::metadata(left.as_path())
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let right_modified = std::fs::metadata(right.as_path())
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    right_modified
        .cmp(&left_modified)
        .then_with(|| left.as_path().cmp(right.as_path()))
}

#[cfg(test)]
#[path = "grep_files_tests.rs"]
mod tests;
