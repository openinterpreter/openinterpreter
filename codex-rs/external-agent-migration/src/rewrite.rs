use linkify::LinkFinder;

/// Describes source-specific terms that should be rewritten in migrated artifacts.
#[derive(Clone, Copy)]
pub struct RewriteProfile {
    doc_file_name: &'static str,
    term_variants: &'static [&'static str],
    case_sensitive_term_variants: &'static [&'static str],
}

impl RewriteProfile {
    pub const fn new(doc_file_name: &'static str, term_variants: &'static [&'static str]) -> Self {
        Self {
            doc_file_name,
            term_variants,
            case_sensitive_term_variants: &[],
        }
    }

    pub const fn with_case_sensitive_term_variants(
        mut self,
        term_variants: &'static [&'static str],
    ) -> Self {
        self.case_sensitive_term_variants = term_variants;
        self
    }

    pub const fn doc_file_name(self) -> &'static str {
        self.doc_file_name
    }

    pub const fn term_variants(self) -> &'static [&'static str] {
        self.term_variants
    }

    pub const fn case_sensitive_term_variants(self) -> &'static [&'static str] {
        self.case_sensitive_term_variants
    }

    /// Rewrites source-specific terms outside recognized URLs and email addresses.
    pub fn rewrite(self, content: &str) -> String {
        let mut finder = LinkFinder::new();
        finder.url_must_have_scheme(/* url_must_have_scheme */ false);
        let mut rewritten = String::with_capacity(content.len());
        let mut start = 0;
        for link in finder.links(content) {
            // The .md suffix is also a TLD, but this filename must still migrate,
            // including references with a :line or :line:column suffix.
            if link.as_str().split(['?', '#']).next().is_some_and(|path| {
                let mut parts = path.splitn(3, ':');
                parts
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case(self.doc_file_name))
                    && parts.all(|part| {
                        !part.is_empty() && part.as_bytes().iter().all(u8::is_ascii_digit)
                    })
            }) {
                continue;
            }
            rewritten.push_str(&self.rewrite_terms(&content[start..link.start()]));
            rewritten.push_str(link.as_str());
            start = link.end();
        }
        rewritten.push_str(&self.rewrite_terms(&content[start..]));
        rewritten
    }

    fn rewrite_terms(self, content: &str) -> String {
        let mut rewritten =
            replace_case_insensitive_with_boundaries(content, self.doc_file_name, "AGENTS.md");
        for from in self.term_variants {
            rewritten = replace_case_insensitive_with_boundaries(&rewritten, from, "Codex");
        }
        for from in self.case_sensitive_term_variants {
            rewritten = replace_with_boundaries(&rewritten, from, "Codex");
        }
        rewritten
    }
}

fn replace_with_boundaries(input: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return input.to_string();
    }

    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut last_emitted = 0usize;
    let mut search_start = 0usize;

    while let Some(relative_pos) = input[search_start..].find(needle) {
        let start = search_start + relative_pos;
        let end = start + needle.len();
        let boundary_before = start == 0 || !is_word_byte(bytes[start - 1]);
        let boundary_after = end == bytes.len() || !is_word_byte(bytes[end]);

        if boundary_before && boundary_after {
            output.push_str(&input[last_emitted..start]);
            output.push_str(replacement);
            last_emitted = end;
        }

        search_start = end;
    }

    if last_emitted == 0 {
        return input.to_string();
    }

    output.push_str(&input[last_emitted..]);
    output
}

fn replace_case_insensitive_with_boundaries(
    input: &str,
    needle: &str,
    replacement: &str,
) -> String {
    let needle_lower = needle.to_ascii_lowercase();
    if needle_lower.is_empty() {
        return input.to_string();
    }

    let haystack_lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut last_emitted = 0usize;
    let mut search_start = 0usize;

    while let Some(relative_pos) = haystack_lower[search_start..].find(&needle_lower) {
        let start = search_start + relative_pos;
        let end = start + needle_lower.len();
        let boundary_before = start == 0 || !is_word_byte(bytes[start - 1]);
        let boundary_after = end == bytes.len() || !is_word_byte(bytes[end]);

        if boundary_before && boundary_after {
            output.push_str(&input[last_emitted..start]);
            output.push_str(replacement);
            last_emitted = end;
        }

        search_start = start + 1;
    }

    if last_emitted == 0 {
        return input.to_string();
    }

    output.push_str(&input[last_emitted..]);
    output
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
#[path = "rewrite_tests.rs"]
mod tests;
