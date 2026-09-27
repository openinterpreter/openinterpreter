use crate::bash::try_parse_shell;
use crate::shell_detect::ShellType;
use std::collections::HashSet;
use tree_sitter::Node;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(super) enum Quoting {
    Posix,
    LegacyBash,
    ZshRcQuotes,
}

impl Quoting {
    fn deferred_modes(shell_type: ShellType) -> &'static [Self] {
        if matches!(shell_type, ShellType::Bash | ShellType::Sh) {
            &[Self::Posix, Self::LegacyBash]
        } else {
            &[Self::Posix, Self::ZshRcQuotes]
        }
    }
}

#[derive(Clone, Copy)]
enum Parsing {
    Snapshot,
    Deferred(Quoting),
}

pub(super) fn literal_words(script: &str, shell_type: ShellType) -> Option<Vec<String>> {
    // Bash invoked as sh still parses its array and alias syntax; other sh dialects do not.
    let shell_type =
        if shell_type == ShellType::Sh && script.starts_with(super::BASH_SH_SNAPSHOT_HEADER) {
            ShellType::Bash
        } else {
            shell_type
        };
    let mut checked_aliases = HashSet::new();
    let mut words =
        literal_words_with_quoting(script, shell_type, Parsing::Snapshot, &mut checked_aliases)?;
    if matches!(shell_type, ShellType::Bash | ShellType::Sh) {
        words.extend(literal_words_with_quoting(
            script,
            shell_type,
            Parsing::Deferred(Quoting::LegacyBash),
            &mut checked_aliases,
        )?);
    }
    Some(words)
}

fn literal_words_with_quoting(
    script: &str,
    shell_type: ShellType,
    parsing: Parsing,
    checked_aliases: &mut HashSet<(String, Quoting)>,
) -> Option<Vec<String>> {
    let tree = try_parse_shell(script)?;
    let mut cursor = tree.root_node().walk();
    let mut quoting = match parsing {
        Parsing::Snapshot => Quoting::Posix,
        Parsing::Deferred(quoting) => quoting,
    };
    let mut pending = Vec::new();
    // Zsh snapshots emit function definitions before restoring options and aliases.
    for node in tree.root_node().named_children(&mut cursor) {
        if matches!(parsing, Parsing::Snapshot) && node.kind() == "command" {
            match node.utf8_text(script.as_bytes()).ok()? {
                "setopt rcquotes" => quoting = Quoting::ZshRcQuotes,
                "unsetopt rcquotes" => quoting = Quoting::Posix,
                _ => {}
            }
        }
        pending.push((node, quoting));
    }
    let mut words = Vec::new();
    let mut checked_nodes = HashSet::new();
    while let Some((node, quoting)) = pending.pop() {
        if !checked_nodes.insert((node.id(), quoting)) {
            continue;
        }
        if matches!(node.kind(), "command_substitution" | "process_substitution") {
            if let Some(body) = node
                .utf8_text(script.as_bytes())
                .ok()?
                .strip_prefix('`')
                .and_then(|body| body.strip_suffix('`'))
            {
                // Backticks remove one escape layer before their command body is parsed.
                let mut context = node.parent();
                if shell_type == ShellType::Zsh {
                    while context.is_some_and(|parent| {
                        matches!(parent.kind(), "expansion" | "concatenation")
                    }) {
                        context = context.and_then(|parent| parent.parent());
                    }
                }
                let in_double_quotes = context.is_some_and(|parent| parent.kind() == "string");
                let mut chars = body.chars().peekable();
                let mut body = String::new();
                while let Some(character) = chars.next() {
                    if character == '\\' {
                        match chars.peek() {
                            Some('\n') => {
                                chars.next();
                                continue;
                            }
                            Some('$' | '`' | '\\') => {
                                body.push(chars.next()?);
                                continue;
                            }
                            Some('"') if in_double_quotes => {
                                body.push(chars.next()?);
                                continue;
                            }
                            _ => {}
                        }
                    }
                    body.push(character);
                }
                for &quoting in Quoting::deferred_modes(shell_type) {
                    if checked_aliases.insert((body.clone(), quoting)) {
                        words.extend(literal_words_with_quoting(
                            &body,
                            shell_type,
                            Parsing::Deferred(quoting),
                            checked_aliases,
                        )?);
                    }
                }
                continue;
            }
            // Deferred bodies may be parsed after the snapshot restores RC_QUOTES.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                pending.extend(
                    Quoting::deferred_modes(shell_type)
                        .iter()
                        .map(|&quoting| (child, quoting)),
                );
            }
            continue;
        }
        if node.kind() == "heredoc_body" {
            let parent = node.parent()?;
            let mut cursor = parent.walk();
            let delimiter = parent
                .named_children(&mut cursor)
                .find(|child| child.kind() == "heredoc_start")?
                .utf8_text(script.as_bytes())
                .ok()?;
            let quoted = delimiter.contains(['\'', '"', '\\']);
            let strip_tabs = parent
                .children(&mut cursor)
                .any(|child| child.kind() == "<<-");
            let mut start = node.start_byte();
            let mut cursor = node.walk();
            // Decode static heredoc text without joining across an expansion.
            for end in node
                .named_children(&mut cursor)
                .filter(|child| !quoted && child.kind() != "heredoc_content")
                .map(Some)
                .chain(std::iter::once(None))
            {
                let text = &script[start..end.map_or(node.end_byte(), |child| child.start_byte())];
                let text = if strip_tabs {
                    text.split_inclusive('\n')
                        .map(|line| line.trim_start_matches('\t'))
                        .collect::<String>()
                } else {
                    text.to_string()
                };
                let mut chars = text.chars().peekable();
                let mut decoded = String::new();
                while let Some(character) = chars.next() {
                    if !quoted && character == '\\' {
                        match chars.peek() {
                            Some('\n') => {
                                chars.next();
                                continue;
                            }
                            Some('$' | '`' | '\\') => {
                                decoded.push(chars.next()?);
                                continue;
                            }
                            _ => {}
                        }
                    }
                    decoded.push(character);
                }
                words.push(decoded);
                if let Some(child) = end {
                    start = child.end_byte();
                    pending.push((child, quoting));
                }
            }
            continue;
        }
        let mut command_words: Vec<(std::ops::Range<usize>, Option<Vec<u8>>)> = Vec::new();
        let declaration = if node.kind() == "command" {
            let mut cursor = node.walk();
            for argument in node
                .child_by_field_name("name")
                .into_iter()
                .chain(node.children_by_field_name("argument", &mut cursor))
            {
                let word = literal_word_bytes(argument, script, shell_type, quoting);
                if shell_type == ShellType::Zsh
                    && let Some((previous_range, previous_word)) = command_words.last_mut()
                    && has_continuation_gap(script, previous_range.end, argument.start_byte())
                {
                    *previous_word = previous_word.take().zip(word).map(|(mut previous, word)| {
                        previous.extend(word);
                        previous
                    });
                    previous_range.end = argument.end_byte();
                } else {
                    command_words.push((argument.byte_range(), word));
                }
            }
            let mut arguments = command_words.iter().map(|(_, word)| {
                word.as_ref()
                    .map(|word| String::from_utf8_lossy(word).into_owned())
            });
            let mut name = arguments.next().flatten();
            loop {
                match name.take().as_deref() {
                    Some(
                        name @ ("alias" | "declare" | "typeset" | "local" | "readonly" | "export"),
                    ) => {
                        break Some(name.to_string());
                    }
                    Some("noglob" | "-") if shell_type == ShellType::Zsh => {
                        name = arguments.next().flatten();
                    }
                    Some("time") => {
                        name = arguments.next().flatten();
                        if shell_type == ShellType::Bash && name.as_deref() == Some("-p") {
                            name = arguments.next().flatten();
                        }
                    }
                    Some(prefix @ ("builtin" | "command")) => {
                        name = arguments.next().flatten();
                        if prefix == "command" {
                            while name.as_deref().is_some_and(|argument| {
                                argument.strip_prefix('-').is_some_and(|options| {
                                    !options.is_empty() && options.bytes().all(|byte| byte == b'p')
                                })
                            }) {
                                name = arguments.next().flatten();
                            }
                        }
                        if (prefix == "command"
                            || prefix == "builtin" && shell_type != ShellType::Zsh)
                            && name.as_deref() == Some("--")
                        {
                            name = arguments.next().flatten();
                        }
                    }
                    _ => break None,
                }
            }
        } else {
            None
        };
        let is_alias = declaration.as_deref() == Some("alias");
        let may_initialize_array = shell_type == ShellType::Bash
            && (node.kind() == "declaration_command" || declaration.is_some() && !is_alias);
        if is_alias || may_initialize_array {
            // Bash can inherit an array's type from an earlier declaration or calling scope.
            // Aliases and quoted compound-array initializers are parsed again by the shell.
            let mut cursor = node.walk();
            for argument in node.named_children(&mut cursor) {
                if command_words.iter().any(|(range, _)| {
                    range.start < argument.start_byte() && argument.end_byte() <= range.end
                }) {
                    continue;
                }
                let continued = command_words.iter().find(|(range, _)| {
                    range.start == argument.start_byte() && range.end > argument.end_byte()
                });
                if let Some(assignment) = continued
                    .map_or_else(
                        || literal_word_bytes(argument, script, shell_type, quoting),
                        |(_, word)| word.clone(),
                    )
                    .or_else(|| {
                        // A dynamic edge prevents full decoding, but not inspection of the
                        // static fragments in the rest of the continued argument.
                        let end = continued.map_or(argument.end_byte(), |(range, _)| range.end);
                        let mut pieces = std::iter::successors(Some(argument), |piece| {
                            piece.next_named_sibling()
                        })
                        .take_while(|piece| piece.end_byte() <= end)
                        .collect::<Vec<_>>();
                        pieces.reverse();
                        let mut word = Vec::new();
                        while let Some(piece) = pieces.pop() {
                            if let Some(literal) =
                                literal_word_bytes(piece, script, shell_type, quoting)
                            {
                                word.extend(literal);
                            } else {
                                match piece.kind() {
                                    "variable_assignment" => {
                                        word.extend_from_slice(b"_=");
                                        pieces.push(piece.child_by_field_name("value")?);
                                    }
                                    "concatenation" | "string" => pieces.extend(
                                        (0..piece.named_child_count())
                                            .rev()
                                            .filter_map(|index| piece.named_child(index)),
                                    ),
                                    "string_content" => {
                                        let text = piece.utf8_text(script.as_bytes()).ok()?;
                                        let decoded = shlex::split(&format!("\"{text}\""))?;
                                        let [literal] = decoded.as_slice() else {
                                            return None;
                                        };
                                        word.extend_from_slice(literal.as_bytes());
                                    }
                                    "simple_expansion"
                                    | "expansion"
                                    | "command_substitution"
                                    | "process_substitution"
                                    | "arithmetic_expansion" => {
                                        word.extend_from_slice(b"${__codex_snapshot_dynamic}");
                                    }
                                    _ => return None,
                                }
                            }
                        }
                        Some(word)
                    })
                    .map(|word| String::from_utf8_lossy(&word).into_owned())
                    && let Some((_, body)) = assignment.split_once('=')
                {
                    let body = if may_initialize_array {
                        let body = body
                            .trim_start_matches("${__codex_snapshot_dynamic}")
                            .trim_end_matches("${__codex_snapshot_dynamic}");
                        if !body.starts_with('(') || !body.ends_with(')') {
                            continue;
                        }
                        format!("_={body}")
                    } else {
                        body.to_string()
                    };
                    for &quoting in Quoting::deferred_modes(shell_type) {
                        if checked_aliases.insert((body.clone(), quoting)) {
                            words.extend(literal_words_with_quoting(
                                &body,
                                shell_type,
                                Parsing::Deferred(quoting),
                                checked_aliases,
                            )?);
                        }
                    }
                }
            }
        }
        let mut pieces = vec![node];
        if shell_type == ShellType::Zsh
            && node.prev_named_sibling().is_none_or(|previous| {
                !has_continuation_gap(script, previous.end_byte(), node.start_byte())
            })
        {
            // Tree-sitter splits unquoted continuations into sibling words. Decode the
            // pieces separately: removing the gap would change adjacent RC_QUOTES strings.
            let mut previous = node;
            while let Some(next) = previous.next_named_sibling() {
                if !has_continuation_gap(script, previous.end_byte(), next.start_byte()) {
                    break;
                }
                pieces.push(next);
                previous = next;
            }
        }
        let continued = pieces.len() > 1;
        if node.kind() == "ansi_c_string" {
            words.push(decode_ansi_c(
                node.utf8_text(script.as_bytes()).ok()?,
                shell_type,
                quoting,
            )?);
            if !continued {
                continue;
            }
        } else if let Some(word) = literal_word(node, script, shell_type, quoting) {
            words.push(word);
            if !continued {
                continue;
            }
        }
        if continued || matches!(node.kind(), "string" | "concatenation") {
            // Join adjacent static pieces, but never join across an expansion.
            pieces.reverse();
            let mut literal = Vec::new();
            let mut raw_string_end = None;
            while let Some(piece) = pieces.pop() {
                if let Some(word) = literal_word_bytes(piece, script, shell_type, quoting) {
                    if matches!(quoting, Quoting::ZshRcQuotes)
                        && piece.kind() == "raw_string"
                        && raw_string_end == Some(piece.start_byte())
                    {
                        literal.push(b'\'');
                    }
                    literal.extend(word);
                    raw_string_end = (piece.kind() == "raw_string").then_some(piece.end_byte());
                } else if piece.kind() == "variable_assignment" {
                    if !literal.is_empty() {
                        words.push(String::from_utf8_lossy(&literal).into_owned());
                        literal.clear();
                    }
                    // Only the value can continue into the next word, never the assignment name.
                    pieces.extend(piece.child_by_field_name("value"));
                } else if piece.kind() == "string_content" {
                    let content = piece.utf8_text(script.as_bytes()).ok()?;
                    let decoded = shlex::split(&format!("\"{content}\""))?;
                    let [word] = decoded.as_slice() else {
                        return None;
                    };
                    literal.extend_from_slice(word.as_bytes());
                } else if matches!(piece.kind(), "string" | "concatenation" | "command_name") {
                    if shell_type == ShellType::Zsh
                        && piece.prev_sibling().is_some_and(|previous| {
                            previous.kind() == "$" && previous.end_byte() == piece.start_byte()
                        })
                    {
                        literal.push(b'$');
                    }
                    pieces.extend(
                        (0..piece.named_child_count())
                            .rev()
                            .filter_map(|index| piece.named_child(index)),
                    );
                } else if !literal.is_empty() {
                    words.push(String::from_utf8_lossy(&literal).into_owned());
                    literal.clear();
                }
            }
            if !literal.is_empty() {
                words.push(String::from_utf8_lossy(&literal).into_owned());
            }
        }
        let mut cursor = node.walk();
        pending.extend(
            node.named_children(&mut cursor)
                .map(|child| (child, quoting)),
        );
    }
    Some(words)
}

fn has_continuation_gap(script: &str, left_end: usize, right_start: usize) -> bool {
    let gap = &script[left_end..right_start];
    !gap.is_empty() && gap.as_bytes().chunks(2).all(|pair| pair == b"\\\n")
}

fn literal_word(
    node: Node<'_>,
    script: &str,
    shell_type: ShellType,
    quoting: Quoting,
) -> Option<String> {
    let bytes = literal_word_bytes(node, script, shell_type, quoting)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn literal_word_bytes(
    node: Node<'_>,
    script: &str,
    shell_type: ShellType,
    quoting: Quoting,
) -> Option<Vec<u8>> {
    let mut word = match node.kind() {
        "command_name" => node
            .named_child(0)
            .and_then(|name| literal_word_bytes(name, script, shell_type, quoting)),
        "concatenation" => {
            let mut cursor = node.walk();
            let mut word = Vec::new();
            let mut raw_string_end = None;
            for child in node.named_children(&mut cursor) {
                // RC_QUOTES treats adjacent single-quoted strings as one quoted apostrophe.
                if matches!(quoting, Quoting::ZshRcQuotes)
                    && child.kind() == "raw_string"
                    && raw_string_end == Some(child.start_byte())
                {
                    word.push(b'\'');
                }
                word.extend(literal_word_bytes(child, script, shell_type, quoting)?);
                raw_string_end = (child.kind() == "raw_string").then_some(child.end_byte());
            }
            Some(word)
        }
        "ansi_c_string" => {
            decode_ansi_c_bytes(node.utf8_text(script.as_bytes()).ok()?, shell_type, quoting)
        }
        "word" | "number" | "string" | "raw_string" => {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|child| child.kind() != "string_content")
            {
                return None;
            }
            let text = node.utf8_text(script.as_bytes()).ok()?;
            let mut context = node.parent();
            while context
                .is_some_and(|parent| matches!(parent.kind(), "expansion" | "concatenation"))
            {
                context = context.and_then(|parent| parent.parent());
            }
            let words = if shell_type == ShellType::Zsh
                && node.kind() == "word"
                && context.is_some_and(|parent| parent.kind() == "string")
            {
                shlex::split(&format!("\"{text}\""))?
            } else {
                shlex::split(text)?
            };
            let [word] = words.as_slice() else {
                return None;
            };
            Some(word.as_bytes().to_vec())
        }
        _ => None,
    }?;
    if shell_type == ShellType::Zsh
        && matches!(node.kind(), "string" | "concatenation")
        && node.prev_sibling().is_some_and(|previous| {
            previous.kind() == "$" && previous.end_byte() == node.start_byte()
        })
    {
        word.insert(0, b'$');
    }
    Some(word)
}

// Decode the literal quoting emitted by Bash and Zsh, never shell expansions.
fn decode_ansi_c(raw: &str, shell_type: ShellType, quoting: Quoting) -> Option<String> {
    let bytes = decode_ansi_c_bytes(raw, shell_type, quoting)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn decode_ansi_c_bytes(raw: &str, shell_type: ShellType, quoting: Quoting) -> Option<Vec<u8>> {
    let body = raw.strip_prefix("$'")?;
    let body = body.strip_suffix('\'').or_else(|| {
        // Array-shaped scalar templates may contain incomplete shell syntax.
        (shell_type == ShellType::Bash).then_some(body)
    })?;
    let mut chars = body.chars().peekable();
    let mut decoded = Vec::new();
    let mut modifiers = ZshByteModifiers::default();
    while let Some(character) = chars.next() {
        if character != '\\' {
            let mut bytes = [0; 4];
            let bytes = character.encode_utf8(&mut bytes).as_bytes();
            decoded.push(modifiers.apply(bytes[0]));
            decoded.extend_from_slice(&bytes[1..]);
            continue;
        }
        let Some(escape) = chars.next() else {
            if shell_type == ShellType::Zsh {
                return None;
            }
            decoded.push(b'\\');
            break;
        };
        let byte = match escape {
            'a' => 7,
            'b' => 8,
            'e' | 'E' => 27,
            'f' => 12,
            'n' => b'\n',
            'r' => b'\r',
            't' => b'\t',
            'v' => 11,
            '\\' | '\'' | '"' | '?' => escape as u8,
            'c' if shell_type != ShellType::Zsh => {
                let Some(character) = chars.next() else {
                    decoded.push(b'\\');
                    if quoting != Quoting::LegacyBash {
                        decoded.push(b'c');
                    }
                    break;
                };
                // Bash 3.2 leaves the second backslash; newer Bash consumes the pair.
                if character == '\\'
                    && quoting != Quoting::LegacyBash
                    && chars.peek() == Some(&'\\')
                {
                    chars.next();
                }
                let mut bytes = [0; 4];
                let bytes = character.encode_utf8(&mut bytes).as_bytes();
                decoded.push(if bytes[0] == b'?' { 127 } else { bytes[0] & 31 });
                decoded.extend_from_slice(&bytes[1..]);
                continue;
            }
            'C' | 'M' if shell_type == ShellType::Zsh => {
                if chars.peek() == Some(&'-') {
                    chars.next();
                }
                if escape == 'C' {
                    modifiers.control = true;
                } else {
                    modifiers.meta = Some(if modifiers.control {
                        MetaOrder::BeforeControl
                    } else {
                        MetaOrder::AfterControl
                    });
                }
                continue;
            }
            '0'..='7' | 'x' | 'u' | 'U' => {
                if matches!(escape, 'u' | 'U') && quoting == Quoting::LegacyBash {
                    decoded.extend_from_slice(&[b'\\', escape as u8]);
                    continue;
                }
                let (radix, digits) = match escape {
                    'x' => (16, 2),
                    'u' => (16, 4),
                    'U' => (16, 8),
                    _ => (8, 2),
                };
                let mut value = escape.to_digit(8).unwrap_or(0);
                let mut found = escape.is_ascii_digit();
                for _ in 0..digits {
                    let Some(digit) = chars.peek().and_then(|next| next.to_digit(radix)) else {
                        break;
                    };
                    chars.next();
                    value = value.checked_mul(radix)?.checked_add(digit)?;
                    found = true;
                }
                if !found {
                    if shell_type == ShellType::Zsh {
                        return None;
                    }
                    decoded.extend_from_slice(&[b'\\', escape as u8]);
                    continue;
                }
                if matches!(escape, 'u' | 'U') {
                    let Some(character) = char::from_u32(value) else {
                        if shell_type == ShellType::Zsh {
                            return None;
                        }
                        // Bash emits extended UTF-8, including surrogate and 5/6-byte values.
                        let length = match value {
                            0..=0xffff => 3,
                            0x10000..=0x1fffff => 4,
                            0x200000..=0x3ffffff => 5,
                            0x4000000..=0x7fffffff => 6,
                            _ => continue,
                        };
                        let mut bytes = [0; 6];
                        for index in (1..length).rev() {
                            bytes[index] = 0x80 | (value & 0x3f) as u8;
                            value >>= 6;
                        }
                        bytes[0] = (0xff << (8 - length)) | value as u8;
                        decoded.extend_from_slice(&bytes[..length]);
                        continue;
                    };
                    // Zsh leaves byte modifiers pending across Unicode escapes.
                    let mut bytes = [0; 4];
                    decoded.extend_from_slice(character.encode_utf8(&mut bytes).as_bytes());
                    continue;
                }
                value as u8
            }
            _ => {
                if shell_type != ShellType::Zsh {
                    decoded.push(b'\\');
                }
                let mut bytes = [0; 4];
                let bytes = escape.encode_utf8(&mut bytes).as_bytes();
                decoded.push(modifiers.apply(bytes[0]));
                decoded.extend_from_slice(&bytes[1..]);
                continue;
            }
        };
        decoded.push(modifiers.apply(byte));
    }
    Some(decoded)
}

enum MetaOrder {
    BeforeControl,
    AfterControl,
}

#[derive(Default)]
struct ZshByteModifiers {
    control: bool,
    meta: Option<MetaOrder>,
}

impl ZshByteModifiers {
    fn apply(&mut self, mut byte: u8) -> u8 {
        // Escape order matters for control-? and bytes whose high bit is already set.
        let meta = self.meta.take();
        if matches!(meta, Some(MetaOrder::BeforeControl)) {
            byte |= 0x80;
        }
        if std::mem::take(&mut self.control) {
            byte = if byte == b'?' { 0x7f } else { byte & 0x9f };
        }
        if matches!(meta, Some(MetaOrder::AfterControl)) {
            byte |= 0x80;
        }
        byte
    }
}
