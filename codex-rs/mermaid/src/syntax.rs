//! Small lexical boundaries shared by the family parsers; no decoding or multiline labels.
//! Source limits are enforced before scanning, and undecoded entities always fail intact.

use super::RenderError;

/// Split statements using family-specific quote and member rules. Sequence quotes are literal;
/// class bodies consume member lines whole. Flowchart label delimiters and other quoted tokens
/// protect semicolons, except in colon-delimited class/state text, where quotes are literal too.
pub(super) fn statements(source: &str) -> Result<Vec<&str>, RenderError> {
    let mut result = Vec::new();
    let mut class_body = false;
    for line in source.lines() {
        let mut rest = line.trim();
        if rest.starts_with("%%{") {
            return Err(RenderError::Unsupported);
        }
        if rest.starts_with("%%") {
            continue;
        }
        // Mermaid #name;/#number; and HTML &name;/&#number; need decoding. Reject before
        // statement splitting can discard the semicolon and turn them into literal text.
        if rest.match_indices(['#', '&']).any(|(index, _)| {
            let after = &rest[index + 1..];
            let len = after
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            len > 0 && after[len..].starts_with(';')
        }) {
            return Err(RenderError::Unsupported);
        }
        while !rest.is_empty() {
            let header = result.first().copied().unwrap_or("");
            if header == "classDiagram"
                && (class_body
                    || rest.starts_with("class ")
                        && rest
                            .split_once('{')
                            .is_some_and(|(declaration, _)| !declaration.contains(';')))
            {
                // Compact bodies are not supported here; let the class parser reject them.
                class_body = rest != "}";
                result.push(rest);
                break;
            }
            let colon_text = matches!(header, "classDiagram" | "stateDiagram" | "stateDiagram-v2");
            let flowchart = matches!(
                header.split_whitespace().next(),
                Some("flowchart" | "graph")
            );
            let mut close = None;
            let mut quoted = false;
            let mut literal = header == "sequenceDiagram";
            let end = rest.char_indices().find_map(|(index, ch)| {
                match ch {
                    '"' if !literal => quoted = !quoted,
                    ':' if colon_text && !quoted => literal = true,
                    ';' if !quoted && close.is_none() => return Some(index),
                    ch if flowchart && !quoted => {
                        close = match (close, ch) {
                            (Some(end), ch) if end == ch => None,
                            (None, '[') => Some(']'),
                            (None, '{') => Some('}'),
                            (None, '|') => Some('|'),
                            (close, _) => close,
                        };
                    }
                    _ => {}
                }
                None
            });
            let (statement, remaining) = match end {
                Some(end) => (&rest[..end], &rest[end + 1..]),
                None => (rest, ""),
            };
            if !statement.trim().is_empty() {
                result.push(statement.trim());
            }
            rest = remaining.trim();
        }
    }
    Ok(result)
}

/// Consume a flowchart label up to its closing delimiter, protecting delimiters inside a leading
/// quoted token. Returns the raw label (including quotes) and untouched remaining syntax.
/// Quotes do not use backslash escaping in Mermaid; entities remain unsupported.
pub(super) fn delimited_label<'a>(
    text: &'a str,
    close: &str,
) -> Result<(&'a str, &'a str), RenderError> {
    if let Some(quoted) = text.strip_prefix('"') {
        let end = quoted.find('"').ok_or(RenderError::Unsupported)? + 2;
        let rest = text[end..]
            .strip_prefix(close)
            .ok_or(RenderError::Unsupported)?;
        Ok((&text[..end], rest))
    } else {
        text.split_once(close).ok_or(RenderError::Unsupported)
    }
}
