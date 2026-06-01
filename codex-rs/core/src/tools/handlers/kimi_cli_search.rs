use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::claude_code::effective_turn_file_system_policy;
use crate::tools::handlers::claude_code::ensure_readable_path;
use crate::tools::handlers::claude_code_search::resolve_search_root;
use crate::tools::handlers::claude_code_search::run_rg_command;
use crate::tools::handlers::parse_kimi_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

pub struct KimiGlobHandler;
pub struct KimiGrepHandler;

const KIMI_GREP_DEFAULT_HEAD_LIMIT: usize = 250;
const KIMI_GREP_MAX_CHARS: usize = 50_000;
const KIMI_GREP_MAX_LINE_LENGTH: usize = 2_000;
const KIMI_GREP_TRUNCATION_MARKER: &str = "[...truncated]";
const KIMI_GLOB_MAX_MATCHES: usize = 1000;

#[derive(Deserialize)]
struct KimiGlobArgs {
    pattern: String,
    #[serde(alias = "path")]
    directory: Option<String>,
    include_dirs: Option<bool>,
}

#[derive(Deserialize)]
struct KimiGrepArgs {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    output_mode: Option<KimiGrepOutputMode>,
    #[serde(rename = "-B")]
    before_context: Option<usize>,
    #[serde(rename = "-A")]
    after_context: Option<usize>,
    #[serde(rename = "-C")]
    short_context: Option<usize>,
    #[serde(rename = "-n")]
    line_numbers: Option<bool>,
    #[serde(rename = "-i")]
    case_insensitive: Option<bool>,
    #[serde(rename = "type")]
    file_type: Option<String>,
    #[serde(alias = "limit")]
    head_limit: Option<usize>,
    offset: Option<usize>,
    multiline: Option<bool>,
    include_ignored: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(PartialEq, Eq)]
enum KimiGrepOutputMode {
    Content,
    FilesWithMatches,
    CountMatches,
}

impl ToolHandler for KimiGlobHandler {
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
        let args: KimiGlobArgs = parse_kimi_arguments(&arguments)?;
        if args.pattern.starts_with("**") {
            let output = std::fs::read_dir(turn.cwd.as_path())
                .map(|entries| {
                    let mut names = entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect::<Vec<_>>();
                    names.sort();
                    names.join("\n")
                })
                .unwrap_or_default();
            return Ok(FunctionToolOutput::from_content(
                vec![
                    FunctionCallOutputContentItem::InputText {
                        text: "<system>ERROR: Unsafe pattern</system>".to_string(),
                    },
                    FunctionCallOutputContentItem::InputText {
                        text: format!(
                            "Pattern `{}` starts with '**' which is not allowed. This would recursively search all directories and may include large directories like `node_modules`. Use more specific patterns instead. For your convenience, a list of all files and directories in the top level of the working directory is provided below.",
                            args.pattern
                        ),
                    },
                    FunctionCallOutputContentItem::InputText { text: output },
                ],
                Some(true),
            ));
        }
        if let Some(error) = kimi_glob_directory_error(turn.as_ref(), args.directory.as_deref()) {
            return Err(FunctionCallError::RespondToModel(error));
        }
        let search_root =
            resolve_search_root(session.as_ref(), turn.as_ref(), args.directory.as_deref()).await?;
        if !search_root.as_path().exists() {
            return Err(FunctionCallError::RespondToModel(format!(
                "`{}` does not exist.",
                args.directory.as_deref().unwrap_or("")
            )));
        }
        if !search_root.as_path().is_dir() {
            return Err(FunctionCallError::RespondToModel(format!(
                "`{}` is not a directory.",
                args.directory.as_deref().unwrap_or("")
            )));
        }
        let mut paths = kimi_glob_paths(search_root.as_path(), &args.pattern)?;
        if !args.include_dirs.unwrap_or(true) {
            paths.retain(|path| path.as_path().is_file());
        }
        paths.sort_by(|left, right| left.as_path().cmp(right.as_path()));
        let match_count = paths.len();
        let limited = paths.into_iter().take(KIMI_GLOB_MAX_MATCHES);
        let output = limited
            .into_iter()
            .map(|path| {
                path.as_path()
                    .strip_prefix(search_root.as_path())
                    .unwrap_or(path.as_path())
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let message = if match_count == 0 {
            format!("No matches found for pattern `{}`.", args.pattern)
        } else {
            format!(
                "Found {match_count} matches for pattern `{}`.",
                args.pattern
            )
        };
        let message = if match_count > KIMI_GLOB_MAX_MATCHES {
            format!(
                "{message} Only the first {KIMI_GLOB_MAX_MATCHES} matches are returned. You may want to use a more specific pattern."
            )
        } else {
            message
        };
        if turn.tools_config.harness.is_kimi_code() {
            return Ok(FunctionToolOutput::from_text(output, Some(true)));
        }
        Ok(FunctionToolOutput::from_content(
            vec![
                FunctionCallOutputContentItem::InputText {
                    text: format!("<system>{message}</system>"),
                },
                FunctionCallOutputContentItem::InputText { text: output },
            ],
            Some(true),
        ))
    }
}

fn kimi_glob_paths(
    search_root: &Path,
    pattern: &str,
) -> Result<Vec<AbsolutePathBuf>, FunctionCallError> {
    let absolute_pattern = search_root.join(pattern);
    let joined = absolute_pattern.to_str().ok_or_else(|| {
        FunctionCallError::RespondToModel("Glob pattern is not valid UTF-8".to_string())
    })?;
    // Bounded, symlink-safe glob: the `glob` crate follows symlinks, so a `**`
    // pattern over a symlink cycle can loop forever. Include directories so the
    // caller's `include_dirs` handling still applies. Walking with the bounded
    // walker yields absolute paths already, so the conversion never fails.
    Ok(
        super::safe_fs::bounded_glob_paths(search_root, joined, /* include_dirs */ true)
            .into_iter()
            .filter_map(|path| AbsolutePathBuf::try_from(normalize_glob_path(path)).ok())
            .collect(),
    )
}

fn normalize_glob_path(path: PathBuf) -> PathBuf {
    dunce::simplified(&path).to_path_buf()
}

impl ToolHandler for KimiGrepHandler {
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
        let args: KimiGrepArgs = parse_kimi_arguments(&arguments)?;
        let raw_search_path = args.path.clone();
        let search_root = if turn.tools_config.harness.is_kimi_code() {
            resolve_kimi_code_search_root(
                session.as_ref(),
                turn.as_ref(),
                raw_search_path.as_deref(),
            )
            .await?
        } else {
            resolve_search_root(session.as_ref(), turn.as_ref(), raw_search_path.as_deref()).await?
        };
        let output_mode = args
            .output_mode
            .unwrap_or(KimiGrepOutputMode::FilesWithMatches);
        let mut rg_args = vec!["--color".to_string(), "never".to_string()];
        if output_mode != KimiGrepOutputMode::Content {
            rg_args.push("--max-columns".to_string());
            rg_args.push("500".to_string());
        }
        rg_args.push("--hidden".to_string());
        rg_args.push("--no-heading".to_string());
        for vcs_dir in [".git", ".svn", ".hg", ".bzr", ".jj", ".sl"] {
            rg_args.push("--glob".to_string());
            rg_args.push(format!("!{vcs_dir}"));
        }
        match &output_mode {
            KimiGrepOutputMode::Content => {
                if args.line_numbers.unwrap_or(true) {
                    rg_args.push("--line-number".to_string());
                }
                if let Some(context) = args.short_context {
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
            KimiGrepOutputMode::FilesWithMatches => {
                rg_args.push("--files-with-matches".to_string());
            }
            KimiGrepOutputMode::CountMatches => {
                rg_args.push("--count-matches".to_string());
            }
        }
        if args.case_insensitive.unwrap_or(false) {
            rg_args.push("--ignore-case".to_string());
        }
        if args.include_ignored.unwrap_or(false) {
            rg_args.push("--no-ignore".to_string());
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
        rg_args.push("--".to_string());
        rg_args.push(args.pattern);
        rg_args.push(raw_search_path.clone().unwrap_or_else(|| ".".to_string()));
        let output = run_kimi_rg_command(&rg_args, turn.cwd.as_path()).await?;
        let all_lines = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| {
                format_kimi_grep_line(raw_search_path.as_deref(), search_root.as_path(), line)
            })
            .skip(args.offset.unwrap_or(0))
            .collect::<Vec<_>>();
        if all_lines.is_empty() {
            return Ok(FunctionToolOutput::from_text(
                "<system>No matches found.</system>".to_string(),
                Some(true),
            ));
        }
        let limit = args.head_limit.unwrap_or(KIMI_GREP_DEFAULT_HEAD_LIMIT);
        let offset = args.offset.unwrap_or(0);
        let pagination_truncated = limit != 0 && all_lines.len() > limit;
        let total_lines = all_lines.len() + offset;
        let lines = if pagination_truncated {
            all_lines.into_iter().take(limit).collect::<Vec<_>>()
        } else {
            all_lines
        };
        let (text, output_truncated) = truncate_kimi_grep_output(&lines.join("\n"));
        let mut messages = Vec::new();
        if pagination_truncated {
            messages.push(format!(
                "Results truncated to {limit} lines (total: {total_lines}). Use offset={} to see more.",
                offset + limit
            ));
        }
        if output_truncated {
            messages.push("Output is truncated to fit in the message.".to_string());
        }
        if !messages.is_empty() {
            return Ok(FunctionToolOutput::from_content(
                vec![
                    FunctionCallOutputContentItem::InputText {
                        text: format!("<system>{}</system>", messages.join(" ")),
                    },
                    FunctionCallOutputContentItem::InputText { text },
                ],
                Some(true),
            ));
        }
        Ok(FunctionToolOutput::from_text(text, Some(true)))
    }
}

async fn resolve_kimi_code_search_root(
    session: &crate::session::Session,
    turn: &TurnContext,
    path: Option<&str>,
) -> Result<AbsolutePathBuf, FunctionCallError> {
    let root = match path {
        Some(path) if Path::new(path).is_absolute() => {
            AbsolutePathBuf::try_from(PathBuf::from(path)).map_err(|err| {
                FunctionCallError::RespondToModel(format!("invalid path `{path}`: {err}"))
            })?
        }
        Some(path) => AbsolutePathBuf::try_from(turn.cwd.as_path().join(path)).map_err(|err| {
            FunctionCallError::RespondToModel(format!("invalid path `{path}`: {err}"))
        })?,
        None => turn.cwd.clone(),
    };
    let file_system_policy = effective_turn_file_system_policy(session, turn).await;
    ensure_readable_path(&file_system_policy, turn, &root)?;
    Ok(root)
}

fn truncate_kimi_grep_output(output: &str) -> (String, bool) {
    let mut truncated = false;
    let mut kept = String::new();
    for line in split_kimi_output_lines(output) {
        if kept.len() >= KIMI_GREP_MAX_CHARS {
            truncated = true;
            break;
        }
        let remaining = KIMI_GREP_MAX_CHARS - kept.len();
        let limit = remaining.min(KIMI_GREP_MAX_LINE_LENGTH);
        let truncated_line = truncate_kimi_grep_line(line, limit);
        if truncated_line != line {
            truncated = true;
        }
        kept.push_str(&truncated_line);
    }
    (kept, truncated)
}

fn split_kimi_output_lines(output: &str) -> Vec<&str> {
    if output.is_empty() {
        Vec::new()
    } else {
        output.split_inclusive('\n').collect()
    }
}

fn truncate_kimi_grep_line(line: &str, max_length: usize) -> String {
    if line.len() <= max_length {
        return line.to_string();
    }
    let linebreak_start = line.find(['\r', '\n']).unwrap_or(line.len());
    let linebreak = &line[linebreak_start..];
    let end = format!("{KIMI_GREP_TRUNCATION_MARKER}{linebreak}");
    let max_length = max_length.max(end.len());
    format!("{}{}", take_kimi_prefix(line, max_length - end.len()), end)
}

fn take_kimi_prefix(line: &str, max_bytes: usize) -> &str {
    if max_bytes >= line.len() {
        return line;
    }
    let mut end = 0;
    for (index, _) in line.char_indices() {
        if index > max_bytes {
            break;
        }
        end = index;
    }
    &line[..end]
}

async fn run_kimi_rg_command(rg_args: &[String], cwd: &Path) -> Result<Output, FunctionCallError> {
    let result = run_rg_command(rg_args.iter().map(String::as_str), cwd).await;
    match result {
        Ok(output) => Ok(output),
        Err(FunctionCallError::RespondToModel(message)) if is_kimi_rg_retryable(&message) => {
            let retry_args = kimi_rg_single_threaded_args(rg_args);
            run_rg_command(retry_args.iter().map(String::as_str), cwd).await
        }
        Err(err) => Err(err),
    }
}

fn is_kimi_rg_retryable(message: &str) -> bool {
    message.contains("os error 11") || message.contains("Resource temporarily unavailable")
}

fn kimi_rg_single_threaded_args(rg_args: &[String]) -> Vec<String> {
    let mut retry_args = Vec::with_capacity(rg_args.len() + 2);
    let mut inserted = false;
    for arg in rg_args {
        if !inserted && arg == "--" {
            retry_args.push("-j".to_string());
            retry_args.push("1".to_string());
            inserted = true;
        }
        retry_args.push(arg.clone());
    }
    if !inserted {
        retry_args.push("-j".to_string());
        retry_args.push("1".to_string());
    }
    retry_args
}

fn kimi_glob_directory_error(turn: &TurnContext, directory: Option<&str>) -> Option<String> {
    let directory = directory?;
    let path = Path::new(directory);
    if !path.is_absolute() || path.starts_with(turn.cwd.as_path()) {
        return None;
    }
    Some(format!(
        "`{directory}` is outside the workspace. You can only search within the working directory, additional directories, and skills directories."
    ))
}

fn format_kimi_grep_line(raw_search_root: Option<&str>, search_root: &Path, line: &str) -> String {
    if raw_search_root.is_none() {
        return line.to_string();
    }
    let Some((path, suffix)) = split_grep_path_suffix(line) else {
        return line.to_string();
    };
    let path = Path::new(path);
    if !path.is_absolute() {
        if raw_search_root.is_some_and(|root| root == ".")
            && let Some(stripped) = path.strip_prefix(".").ok()
        {
            let relative_path = stripped.display();
            return if suffix.is_empty() {
                relative_path.to_string()
            } else {
                format!("{relative_path}{suffix}")
            };
        }
        return line.to_string();
    }
    let Ok(relative) = path.strip_prefix(search_root) else {
        return line.to_string();
    };
    let relative_path = relative.display();
    if suffix.is_empty() {
        relative_path.to_string()
    } else {
        format!("{relative_path}{suffix}")
    }
}

fn split_grep_path_suffix(line: &str) -> Option<(&str, &str)> {
    for delimiter in [':', '\t'] {
        if let Some(index) = line.find(delimiter) {
            return Some((&line[..index], &line[index..]));
        }
    }
    Some((line, ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn kimi_glob_includes_directories_and_sorts_like_kimi_code() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let dir = temp.path();
        std::fs::create_dir_all(dir.join("task_file/input_data"))?;
        std::fs::create_dir_all(dir.join("task_file/scripts"))?;
        std::fs::write(dir.join("task_file/input_data/requests_bucket_1.jsonl"), "")?;
        std::fs::write(dir.join("task_file/input_data/requests_bucket_2.jsonl"), "")?;
        std::fs::write(dir.join("task_file/scripts/__init__.py"), "")?;
        std::fs::write(dir.join("task_file/scripts/baseline_packer.py"), "")?;
        std::fs::write(dir.join("task_file/scripts/cost_model.py"), "")?;

        let relative_paths = kimi_glob_paths(dir, "task_file/**/*")?
            .into_iter()
            .map(|path| {
                path.as_path()
                    .strip_prefix(dir)
                    .unwrap()
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            relative_paths,
            vec![
                "task_file/input_data",
                "task_file/input_data/requests_bucket_1.jsonl",
                "task_file/input_data/requests_bucket_2.jsonl",
                "task_file/scripts",
                "task_file/scripts/__init__.py",
                "task_file/scripts/baseline_packer.py",
                "task_file/scripts/cost_model.py",
            ]
        );
        Ok(())
    }

    #[test]
    fn kimi_glob_supports_excluding_directories() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let dir = temp.path();
        std::fs::create_dir_all(dir.join("task_file/input_data"))?;
        std::fs::write(dir.join("task_file/input_data/requests_bucket_1.jsonl"), "")?;

        let mut paths = kimi_glob_paths(dir, "task_file/**/*")?;
        paths.retain(|path| path.as_path().is_file());
        let relative_paths = paths
            .into_iter()
            .map(|path| {
                path.as_path()
                    .strip_prefix(dir)
                    .unwrap()
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            relative_paths,
            vec!["task_file/input_data/requests_bucket_1.jsonl"]
        );
        Ok(())
    }

    #[test]
    fn kimi_grep_files_with_matches_strips_absolute_search_root() {
        let line = "/tmp/workspace/docs/source.txt";

        assert_eq!(
            format_kimi_grep_line(
                Some("/tmp/workspace/docs"),
                Path::new("/tmp/workspace/docs"),
                line
            ),
            "source.txt"
        );
    }

    #[test]
    fn kimi_grep_content_strips_absolute_search_root_and_keeps_suffix() {
        let line = "/tmp/workspace/docs/source.txt:2:NEEDLE_OLD";

        assert_eq!(
            format_kimi_grep_line(
                Some("/tmp/workspace/docs"),
                Path::new("/tmp/workspace/docs"),
                line
            ),
            "source.txt:2:NEEDLE_OLD"
        );
    }

    #[test]
    fn kimi_grep_retry_uses_single_threaded_ripgrep_before_pattern_separator() {
        let args = vec![
            "--hidden".to_string(),
            "--".to_string(),
            "needle".to_string(),
            ".".to_string(),
        ];

        assert_eq!(
            kimi_rg_single_threaded_args(&args),
            vec![
                "--hidden".to_string(),
                "-j".to_string(),
                "1".to_string(),
                "--".to_string(),
                "needle".to_string(),
                ".".to_string(),
            ]
        );
    }

    #[test]
    fn kimi_grep_retry_matches_real_kimi_eagain_cases() {
        assert!(is_kimi_rg_retryable(
            "IO error: Resource temporarily unavailable (os error 11)"
        ));
        assert!(!is_kimi_rg_retryable("regex parse error"));
    }

    #[test]
    fn kimi_grep_output_truncates_long_lines_like_tool_result_builder() {
        let input = "a".repeat(KIMI_GREP_MAX_LINE_LENGTH + 10);

        let (output, truncated) = truncate_kimi_grep_output(&input);

        assert!(truncated);
        assert_eq!(output.len(), KIMI_GREP_MAX_LINE_LENGTH);
        assert!(output.ends_with(KIMI_GREP_TRUNCATION_MARKER));
    }

    #[test]
    fn kimi_grep_output_truncates_total_chars_like_tool_result_builder() {
        let input = (0..40)
            .map(|index| format!("line-{index}-{}\n", "a".repeat(1_800)))
            .collect::<String>();

        let (output, truncated) = truncate_kimi_grep_output(&input);

        assert!(truncated);
        assert_eq!(output.len(), KIMI_GREP_MAX_CHARS);
    }
}
