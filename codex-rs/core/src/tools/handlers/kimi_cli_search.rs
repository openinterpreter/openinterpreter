use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
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

pub struct KimiGlobHandler;
pub struct KimiGrepHandler;

const KIMI_GREP_DEFAULT_HEAD_LIMIT: usize = 250;
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
    let pattern = absolute_pattern.to_str().ok_or_else(|| {
        FunctionCallError::RespondToModel("Glob pattern is not valid UTF-8".to_string())
    })?;
    let entries = glob::glob(pattern).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "Failed to search for pattern {pattern}. Error: {err}"
        ))
    })?;
    entries
        .map(|entry| {
            entry
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "Failed to search for pattern {pattern}. Error: {err}"
                    ))
                })
                .and_then(|path| {
                    AbsolutePathBuf::try_from(normalize_glob_path(path)).map_err(|err| {
                        FunctionCallError::RespondToModel(format!(
                            "Glob produced an invalid path: {err}"
                        ))
                    })
                })
        })
        .collect()
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
        let search_root =
            resolve_search_root(session.as_ref(), turn.as_ref(), args.path.as_deref()).await?;
        let mut rg_args = vec![
            "--color".to_string(),
            "never".to_string(),
            "--no-heading".to_string(),
        ];
        match args
            .output_mode
            .unwrap_or(KimiGrepOutputMode::FilesWithMatches)
        {
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
                rg_args.push("--count".to_string());
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
        rg_args.push(args.pattern);
        rg_args.push(search_root.as_path().to_string_lossy().into_owned());
        let output = run_rg_command(rg_args.iter().map(String::as_str), turn.cwd.as_path()).await?;
        let lines = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| format_kimi_grep_line(args.path.as_deref(), search_root.as_path(), line))
            .skip(args.offset.unwrap_or(0))
            .take(args.head_limit.unwrap_or(KIMI_GREP_DEFAULT_HEAD_LIMIT))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(FunctionToolOutput::from_text(lines, Some(true)))
    }
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
    let Some(raw_search_root) = raw_search_root else {
        return line.to_string();
    };
    let Some((path, suffix)) = split_grep_path_suffix(line) else {
        return line.to_string();
    };
    let path = Path::new(path);
    if !path.is_absolute() {
        return line.to_string();
    }
    let Ok(relative) = path.strip_prefix(search_root) else {
        return line.to_string();
    };
    let raw_path = Path::new(raw_search_root).join(relative);
    if suffix.is_empty() {
        raw_path.display().to_string()
    } else {
        format!("{}{}", raw_path.display(), suffix)
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
}
