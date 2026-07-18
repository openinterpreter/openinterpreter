use std::path::Path;
use std::path::PathBuf;

use regex_lite::Regex;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::harness_aliases::simple_glob_matches;
use crate::tools::handlers::harness_fs;
use crate::tools::handlers::harness_fs::WalkEntryKind;
use crate::tools::handlers::parse_arguments;

const DEFAULT_HEAD_LIMIT: usize = 250;

#[derive(Deserialize)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    output_mode: Option<GrepMode>,
    #[serde(default, rename = "-i")]
    case_insensitive: bool,
    #[serde(default, rename = "-n")]
    line_numbers: Option<bool>,
    #[serde(default, rename = "-A")]
    after_context: Option<usize>,
    #[serde(default, rename = "-B")]
    before_context: Option<usize>,
    #[serde(default, rename = "-C")]
    context: Option<usize>,
    #[serde(default)]
    head_limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GrepMode {
    Content,
    FilesWithMatches,
    CountMatches,
}

pub(crate) async fn handle(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return Err(FunctionCallError::RespondToModel(
            "Grep received unsupported payload".to_string(),
        ));
    };
    let args: GrepArgs = parse_arguments(arguments)?;
    let root = match args.path.as_deref() {
        Some(path) => harness_fs::checked_read_path(&invocation, path, "Grep")?,
        None => {
            let root = harness_fs::primary_cwd(&invocation);
            harness_fs::ensure_read_allowed(&invocation, &root, "Grep")?;
            root
        }
    };
    if !root.exists() {
        return Ok(output(format!("{} does not exist", root.display()), false));
    }

    let pattern = if args.case_insensitive {
        format!("(?i:{})", args.pattern)
    } else {
        args.pattern.clone()
    };
    let regex = Regex::new(&pattern)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
    let cwd = harness_fs::primary_cwd(&invocation);
    let mut files = search_files(&root)?;
    if let Some(glob) = args.glob.as_deref() {
        files.retain(|path| {
            let relative = path.strip_prefix(&cwd).unwrap_or(path);
            simple_glob_matches(glob, &relative.to_string_lossy())
        });
    }

    let mode = args.output_mode.unwrap_or(GrepMode::FilesWithMatches);
    let mut lines = match mode {
        GrepMode::Content => content_matches(&files, &cwd, &regex, &args),
        GrepMode::FilesWithMatches => files_with_matches(&files, &cwd, &regex),
        GrepMode::CountMatches => count_matches(&files, &cwd, &regex),
    };
    let total_count = lines.len();
    let offset = args.offset.unwrap_or(0);
    lines = lines.into_iter().skip(offset).collect();
    let head_limit = args.head_limit.unwrap_or(DEFAULT_HEAD_LIMIT);
    let truncated = head_limit > 0 && lines.len() > head_limit;
    if head_limit > 0 {
        lines.truncate(head_limit);
    }

    let mut sections = Vec::new();
    if matches!(mode, GrepMode::CountMatches) && total_count > 0 {
        let occurrences = lines
            .iter()
            .filter_map(|line| line.rsplit_once(':')?.1.parse::<usize>().ok())
            .sum::<usize>();
        let occurrence_word = if occurrences == 1 {
            "occurrence"
        } else {
            "occurrences"
        };
        let file_word = if total_count == 1 { "file" } else { "files" };
        sections.push(format!(
            "Found {occurrences} total {occurrence_word} across {total_count} {file_word}."
        ));
    }
    if truncated && matches!(mode, GrepMode::CountMatches) {
        sections.push(format!(
            "Results truncated to {head_limit} lines (total: {total_count}). Use offset={} to see more.",
            offset + head_limit
        ));
    }
    if lines.is_empty() && sections.is_empty() {
        sections.push("No non-sensitive matches found".to_string());
    } else {
        sections.push(lines.join("\n"));
    }
    if truncated && !matches!(mode, GrepMode::CountMatches) {
        sections.push(format!(
            "Results truncated to {head_limit} lines (total: {total_count}). Use offset={} to see more.",
            offset + head_limit
        ));
    }

    Ok(output(sections.join("\n"), true))
}

fn output(text: String, success: bool) -> Box<dyn ToolOutput> {
    boxed_tool_output(FunctionToolOutput::from_text(text, Some(success)))
}

fn search_files(root: &Path) -> Result<Vec<PathBuf>, FunctionCallError> {
    harness_fs::bounded_walk(root)
        .map(|entries| {
            entries
                .into_iter()
                .filter_map(|entry| (entry.kind == WalkEntryKind::File).then_some(entry.path))
                .collect()
        })
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))
}

fn content_matches(files: &[PathBuf], cwd: &Path, regex: &Regex, args: &GrepArgs) -> Vec<String> {
    let context = args.context;
    let before = context.or(args.before_context).unwrap_or(0);
    let after = context.or(args.after_context).unwrap_or(0);
    let line_numbers = args.line_numbers.unwrap_or(true);
    let mut output = Vec::new();
    for path in files {
        let Some(text) = harness_fs::read_search_file(path) else {
            continue;
        };
        let lines = text.lines().collect::<Vec<_>>();
        let matching = lines
            .iter()
            .map(|line| regex.is_match(line))
            .collect::<Vec<_>>();
        let mut ranges = matching
            .iter()
            .enumerate()
            .filter_map(|(index, is_match)| {
                is_match.then_some((
                    index.saturating_sub(before),
                    (index + after + 1).min(lines.len()),
                ))
            })
            .collect::<Vec<_>>();
        ranges.sort_unstable();
        let mut merged = Vec::<(usize, usize)>::new();
        for (start, end) in ranges {
            if let Some((_, previous_end)) = merged.last_mut()
                && start <= *previous_end
            {
                *previous_end = (*previous_end).max(end);
            } else {
                merged.push((start, end));
            }
        }
        let display_path = display_path(path, cwd);
        for (range_index, (start, end)) in merged.into_iter().enumerate() {
            if range_index > 0 {
                output.push("--".to_string());
            }
            for index in start..end {
                let separator = if matching[index] { ':' } else { '-' };
                if line_numbers {
                    output.push(format!(
                        "{display_path}{separator}{}{separator}{}",
                        index + 1,
                        lines[index]
                    ));
                } else {
                    output.push(format!("{display_path}:{}", lines[index]));
                }
            }
        }
    }
    output
}

fn files_with_matches(files: &[PathBuf], cwd: &Path, regex: &Regex) -> Vec<String> {
    let mut matches = files
        .iter()
        .filter_map(|path| {
            let text = harness_fs::read_search_file(path)?;
            regex.is_match(&text).then(|| {
                let modified = std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok();
                (modified, display_path(path, cwd))
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|(left_modified, _), (right_modified, _)| right_modified.cmp(left_modified));
    matches.into_iter().map(|(_, path)| path).collect()
}

fn count_matches(files: &[PathBuf], cwd: &Path, regex: &Regex) -> Vec<String> {
    files
        .iter()
        .filter_map(|path| {
            let text = harness_fs::read_search_file(path)?;
            let count = regex.find_iter(&text).count();
            (count > 0).then(|| format!("{}:{count}", display_path(path, cwd)))
        })
        .collect()
}

fn display_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_mode_merges_overlapping_context_ranges() {
        let workspace = tempfile::tempdir().expect("workspace temp dir");
        let path = workspace.path().join("grep.txt");
        std::fs::write(&path, "before\nNEEDLE one\nNEEDLE two\nafter\n").expect("write fixture");
        let regex = Regex::new("NEEDLE").expect("valid regex");
        let args = GrepArgs {
            pattern: "NEEDLE".to_string(),
            path: None,
            glob: None,
            output_mode: Some(GrepMode::Content),
            case_insensitive: false,
            line_numbers: Some(true),
            after_context: Some(1),
            before_context: Some(1),
            context: None,
            head_limit: Some(0),
            offset: Some(0),
        };

        assert_eq!(
            content_matches(&[path], workspace.path(), &regex, &args),
            vec![
                "grep.txt-1-before",
                "grep.txt:2:NEEDLE one",
                "grep.txt:3:NEEDLE two",
                "grep.txt-4-after",
            ]
        );
    }
}
