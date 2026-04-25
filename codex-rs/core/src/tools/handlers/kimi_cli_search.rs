use crate::function_tool::FunctionCallError;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::claude_code_search::canonicalize_rg_path;
use crate::tools::handlers::claude_code_search::compare_paths_by_modified_desc;
use crate::tools::handlers::claude_code_search::parse_results;
use crate::tools::handlers::claude_code_search::resolve_search_root;
use crate::tools::handlers::claude_code_search::run_rg_command;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use serde::Deserialize;
use std::path::Path;
use std::path::PathBuf;

pub struct KimiGlobHandler;
pub struct KimiGrepHandler;

const KIMI_GREP_DEFAULT_HEAD_LIMIT: usize = 250;

#[derive(Deserialize)]
struct KimiGlobArgs {
    pattern: String,
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
        let args: KimiGlobArgs = parse_arguments(&arguments)?;
        if let Some(error) = kimi_glob_directory_error(turn.as_ref(), args.directory.as_deref()) {
            return Err(FunctionCallError::RespondToModel(error));
        }
        let search_root =
            resolve_search_root(session.as_ref(), turn.as_ref(), args.directory.as_deref()).await?;
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
        if !args.include_dirs.unwrap_or(true) {
            paths.retain(|path| path.as_path().is_file());
        }
        paths.sort_by(compare_paths_by_modified_desc);
        let output = paths
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
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
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
        let args: KimiGrepArgs = parse_arguments(&arguments)?;
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
            .map(|line| format_kimi_grep_line(search_root.as_path(), line))
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

fn format_kimi_grep_line(search_root: &Path, line: &str) -> String {
    let Some((path, suffix)) = split_grep_path_suffix(line) else {
        return line.to_string();
    };
    let path = Path::new(path);
    if !path.is_absolute() {
        return line.to_string();
    }
    let relative = path
        .strip_prefix(search_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| {
            lexical_strip_prefix(path, search_root).unwrap_or_else(|| path.to_path_buf())
        });
    if suffix.is_empty() {
        relative.display().to_string()
    } else {
        format!("{}{}", relative.display(), suffix)
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

fn lexical_strip_prefix(path: &Path, prefix: &Path) -> Option<PathBuf> {
    let mut path_components = path.components();
    for prefix_component in prefix.components() {
        if path_components.next()? != prefix_component {
            return None;
        }
    }
    Some(path_components.as_path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::format_kimi_grep_line;
    use super::lexical_strip_prefix;
    use std::path::Path;

    #[test]
    fn kimi_grep_formats_absolute_files_with_matches_as_relative() {
        assert_eq!(
            format_kimi_grep_line(
                Path::new("/tmp/workspace/docs"),
                "/tmp/workspace/docs/source.txt"
            ),
            "source.txt"
        );
    }

    #[test]
    fn kimi_grep_preserves_content_suffix_after_relative_path() {
        assert_eq!(
            format_kimi_grep_line(
                Path::new("/tmp/workspace/docs"),
                "/tmp/workspace/docs/source.txt:2:NEEDLE"
            ),
            "source.txt:2:NEEDLE"
        );
    }

    #[test]
    fn lexical_prefix_strips_without_canonicalizing_symlinks() {
        assert_eq!(
            lexical_strip_prefix(
                Path::new("/tmp/workspace/docs/source.txt"),
                Path::new("/tmp/workspace/docs")
            )
            .as_deref(),
            Some(Path::new("source.txt"))
        );
    }
}
