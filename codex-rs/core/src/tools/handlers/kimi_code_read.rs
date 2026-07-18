use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::harness_fs;
use crate::tools::handlers::parse_arguments;

const MAX_LINES: usize = 1_000;
const MAX_LINE_LENGTH: usize = 2_000;
const MAX_BYTES: usize = 100 * 1_024;

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    line_offset: Option<i64>,
    #[serde(default)]
    n_lines: Option<usize>,
}

pub(crate) async fn handle(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return Err(FunctionCallError::RespondToModel(
            "Read received unsupported payload".to_string(),
        ));
    };
    let args: ReadArgs = parse_arguments(arguments)?;
    let path = harness_fs::checked_read_path(&invocation, &args.path, "Read")?;
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(output(format!("\"{}\" does not exist.", args.path), false));
        }
        Err(err) => return Ok(output(err.to_string(), false)),
    };
    if !metadata.is_file() {
        return Ok(output(format!("\"{}\" is not a file.", args.path), false));
    }
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(err) => return Ok(output(err.to_string(), false)),
    };
    let text = match String::from_utf8(data) {
        Ok(text) if !text.contains('\0') => text,
        _ => return Ok(output(not_readable_message(&args.path), false)),
    };
    let line_offset = args.line_offset.unwrap_or(1);
    if line_offset == 0 || line_offset < -(MAX_LINES as i64) {
        return Ok(output(
            format!("line_offset must be at least -{MAX_LINES} and cannot be zero"),
            false,
        ));
    }
    if args.n_lines == Some(0) {
        return Ok(output("n_lines must be positive".to_string(), false));
    }
    Ok(output(
        render_read(&text, line_offset, args.n_lines.unwrap_or(MAX_LINES)),
        true,
    ))
}

fn not_readable_message(path: &str) -> String {
    format!(
        "\"{path}\" is not readable as UTF-8 text. If it is an image or video, use ReadMediaFile. For other binary formats, use Bash or an MCP tool if available."
    )
}

fn render_read(text: &str, line_offset: i64, requested_lines: usize) -> String {
    let lines = text_lines(text);
    let effective_limit = requested_lines.min(MAX_LINES);
    let start = if line_offset < 0 {
        lines
            .len()
            .saturating_sub(line_offset.unsigned_abs() as usize)
    } else {
        (line_offset as usize).saturating_sub(1).min(lines.len())
    };
    let selected_end = start.saturating_add(effective_limit).min(lines.len());
    let style = line_ending_style(text);
    let mut rendered = Vec::new();
    let mut truncated_lines = Vec::new();
    let mut bytes = 0usize;
    let mut max_bytes_reached = false;

    for (index, raw) in lines[start..selected_end].iter().enumerate() {
        let line_number = start + index + 1;
        let visible = if style == LineEndingStyle::CrLf {
            raw.strip_suffix('\r').unwrap_or(raw).to_string()
        } else if style == LineEndingStyle::Mixed {
            raw.replace('\r', "\\r")
        } else {
            (*raw).to_string()
        };
        let (visible, was_truncated) = truncate_chars(&visible, MAX_LINE_LENGTH);
        let rendered_line = format!("{line_number}\t{visible}");
        let line_bytes = rendered_line.len() + usize::from(!rendered.is_empty());
        if !rendered.is_empty() && bytes.saturating_add(line_bytes) > MAX_BYTES {
            max_bytes_reached = true;
            break;
        }
        bytes = bytes.saturating_add(line_bytes);
        rendered.push(rendered_line);
        if was_truncated {
            truncated_lines.push(line_number);
        }
        if bytes >= MAX_BYTES {
            max_bytes_reached = true;
            break;
        }
    }

    let line_count = rendered.len();
    let start_line = if line_count == 0 { 0 } else { start + 1 };
    let max_lines_reached = line_offset > 0
        && effective_limit >= MAX_LINES
        && start.saturating_add(effective_limit) < lines.len();
    let mut status = if line_count == 0 {
        "No lines read from file.".to_string()
    } else {
        let word = if line_count == 1 { "line" } else { "lines" };
        format!("{line_count} {word} read from file starting from line {start_line}.")
    };
    status.push_str(&format!(" Total lines in file: {}.", lines.len()));
    if max_lines_reached {
        status.push_str(&format!(" Max {MAX_LINES} lines reached."));
    } else if max_bytes_reached {
        status.push_str(&format!(" Max {MAX_BYTES} bytes reached."));
    } else if line_count < requested_lines {
        status.push_str(" End of file reached.");
    }
    if !truncated_lines.is_empty() {
        let numbers = truncated_lines
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        status.push_str(&format!(" Lines [{numbers}] were truncated."));
    }
    if style == LineEndingStyle::Mixed {
        status.push_str(
            " Mixed or lone carriage-return line endings are shown as \\r. Use exact \\r\\n or \\r escapes in Edit.old_string for those lines.",
        );
    }
    if rendered.is_empty() {
        format!("<system>{status}</system>")
    } else {
        format!("{}\n<system>{status}</system>", rendered.join("\n"))
    }
}

fn text_lines(text: &str) -> Vec<&str> {
    text.split_terminator('\n').collect()
}

fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_string(), false);
    }
    let prefix = text
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    (format!("{prefix}..."), true)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineEndingStyle {
    Lf,
    CrLf,
    Mixed,
}

fn line_ending_style(text: &str) -> LineEndingStyle {
    let bytes = text.as_bytes();
    let mut has_crlf = false;
    let mut has_lf = false;
    let mut has_lone_cr = false;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                has_crlf = true;
                index += 2;
            }
            b'\r' => {
                has_lone_cr = true;
                index += 1;
            }
            b'\n' => {
                has_lf = true;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if has_lone_cr || (has_crlf && has_lf) {
        LineEndingStyle::Mixed
    } else if has_crlf {
        LineEndingStyle::CrLf
    } else {
        LineEndingStyle::Lf
    }
}

fn output(text: String, success: bool) -> Box<dyn ToolOutput> {
    boxed_tool_output(FunctionToolOutput::from_text(text, Some(success)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_offset_reads_from_end() {
        assert_eq!(
            render_read("one\ntwo\nthree\nfour\n", -3, 2),
            "2\ttwo\n3\tthree\n<system>2 lines read from file starting from line 2. Total lines in file: 4.</system>"
        );
    }

    #[test]
    fn forward_read_reports_byte_limit() {
        let text = (0..1_000)
            .map(|index| format!("{index:04} {}\n", "R".repeat(220)))
            .collect::<String>();
        let rendered = render_read(&text, 1, 1_000);
        assert!(rendered.contains("Max 102400 bytes reached."));
        assert!(!rendered.contains("Max 1000 lines reached."));
    }
}
