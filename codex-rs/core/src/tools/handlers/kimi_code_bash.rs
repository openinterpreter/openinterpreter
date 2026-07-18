const MAX_CHARS: usize = 50_000;
const MAX_LINE_LENGTH: usize = 2_000;
const TRUNCATION_MARKER: &str = "[...truncated]";
const TRUNCATION_MESSAGE: &str = "Output is truncated to fit in the message.";

pub(super) fn format_foreground_output(raw_output: &str, exit_code: Option<i64>) -> (String, bool) {
    let (mut output, truncated) = truncate_output(raw_output);
    let success = exit_code == Some(0);
    let message = if success {
        if truncated || output.is_empty() {
            Some(if truncated {
                TRUNCATION_MESSAGE.to_string()
            } else {
                "Command executed successfully.".to_string()
            })
        } else {
            None
        }
    } else {
        let exit_code = exit_code.unwrap_or(1);
        if output.is_empty() {
            output = format!("Process exited with code {exit_code}");
        }
        let mut message = format!("Command failed with exit code: {exit_code}.");
        if truncated {
            message.push(' ');
            message.push_str(TRUNCATION_MESSAGE);
        }
        Some(message)
    };
    if let Some(message) = message {
        if !output.is_empty() && !output.ends_with('\n') && !output.ends_with('\r') {
            output.push('\n');
        }
        output.push_str(&message);
    }
    (output, success)
}

fn truncate_output(text: &str) -> (String, bool) {
    let mut output = String::new();
    let mut written = 0usize;
    let mut truncated = false;
    for line in lines_with_endings(text) {
        if written >= MAX_CHARS {
            if !line.is_empty() && !truncated {
                output.push_str(TRUNCATION_MARKER);
                truncated = true;
            }
            break;
        }
        let remaining = MAX_CHARS - written;
        let limit = remaining.min(MAX_LINE_LENGTH);
        let line_chars = line.chars().count();
        if line_chars <= limit {
            output.push_str(line);
            written += line_chars;
            continue;
        }

        let line_break = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else if line.ends_with('\r') {
            "\r"
        } else {
            ""
        };
        let suffix = format!("{TRUNCATION_MARKER}{line_break}");
        let effective_limit = limit.max(suffix.chars().count());
        let prefix_chars = effective_limit.saturating_sub(suffix.chars().count());
        output.extend(line.chars().take(prefix_chars));
        output.push_str(&suffix);
        written += effective_limit;
        truncated = true;
    }
    (output, truncated)
}

fn lines_with_endings(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                index += 2;
                lines.push(&text[start..index]);
                start = index;
            }
            b'\r' | b'\n' => {
                index += 1;
                lines.push(&text[start..index]);
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::TRUNCATION_MESSAGE;
    use super::format_foreground_output;

    #[test]
    fn formats_empty_and_failed_commands() {
        assert_eq!(
            format_foreground_output("", Some(0)),
            ("Command executed successfully.".to_string(), true)
        );
        assert_eq!(
            format_foreground_output("EXPECTED_STDERR\n", Some(7)),
            (
                "EXPECTED_STDERR\nCommand failed with exit code: 7.".to_string(),
                false
            )
        );
        assert_eq!(
            format_foreground_output("", Some(7)),
            (
                "Process exited with code 7\nCommand failed with exit code: 7.".to_string(),
                false
            )
        );
    }

    #[test]
    fn truncates_large_command_output() {
        let output = (0..12_000)
            .map(|index| format!("SHELL_LARGE_{index:05}\n"))
            .collect::<String>();

        let (formatted, success) = format_foreground_output(&output, Some(0));

        assert!(success);
        assert!(formatted.contains("[...truncated]"));
        assert!(formatted.ends_with(TRUNCATION_MESSAGE));
        assert!(formatted.chars().count() <= 50_000 + TRUNCATION_MESSAGE.len() + 1);
    }
}
