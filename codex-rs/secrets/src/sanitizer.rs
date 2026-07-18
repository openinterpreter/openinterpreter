use regex::Regex;
use std::sync::LazyLock;

// Matches the legacy `sk-...` key form (unchanged) plus the current prefixed
// variants `sk-proj-...`, `sk-svcacct-...`, and `sk-admin-...`, whose random
// tail can contain `-` and `_` and so was previously cut short.
static OPENAI_KEY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(r"sk-(?:proj|svcacct|admin)-[A-Za-z0-9_-]{16,}|sk-[A-Za-z0-9]{20,}")
});
static AWS_ACCESS_KEY_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r"\bAKIA[0-9A-Z]{16}\b"));
static BEARER_TOKEN_REGEX: LazyLock<Regex> =
    LazyLock::new(|| compile_regex(r"(?i)\bBearer\s+[A-Za-z0-9._\-]{16,}\b"));
static SECRET_ASSIGNMENT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    compile_regex(r#"(?i)\b(api[_-]?key|token|secret|password)\b(\s*[:=]\s*)(["']?)[^\s"']{8,}"#)
});

/// Remove secret and keys from a String. This is done on best effort basis following some
/// well-known REGEX.
pub fn redact_secrets(input: String) -> String {
    let redacted = OPENAI_KEY_REGEX.replace_all(&input, "[REDACTED_SECRET]");
    let redacted = AWS_ACCESS_KEY_ID_REGEX.replace_all(&redacted, "[REDACTED_SECRET]");
    let redacted = BEARER_TOKEN_REGEX.replace_all(&redacted, "Bearer [REDACTED_SECRET]");
    let redacted = SECRET_ASSIGNMENT_REGEX.replace_all(&redacted, "$1$2$3[REDACTED_SECRET]");

    redacted.to_string()
}

fn compile_regex(pattern: &str) -> Regex {
    match Regex::new(pattern) {
        Ok(regex) => regex,
        // Panic is ok thanks to `load_regex` test.
        Err(err) => panic!("invalid regex pattern `{pattern}`: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_regex() {
        // The goal of this test is just to compile all the regex to prevent the panic
        let _ = redact_secrets("secret".to_string());
    }

    fn assert_redacted(secret: &str) {
        let input = format!("prefix {secret} suffix");
        let redacted = redact_secrets(input);
        assert!(
            !redacted.contains(secret),
            "expected secret to be redacted, got: {redacted}"
        );
        assert!(redacted.contains("[REDACTED_SECRET]"), "got: {redacted}");
    }

    #[test]
    fn redacts_legacy_openai_key() {
        assert_redacted("sk-abcdEFGH1234ijklMNOP5678");
    }

    #[test]
    fn redacts_prefixed_openai_keys() {
        assert_redacted("sk-proj-abcdEFGH1234ijklMNOP5678_qrst-uvwx");
        assert_redacted("sk-svcacct-ABCdef123456GHIjkl789012");
        assert_redacted("sk-admin-ABCdef123456GHIjkl789012");
    }

    #[test]
    fn still_redacts_aws_access_key_id() {
        assert_redacted("AKIAIOSFODNN7EXAMPLE");
    }

    #[test]
    fn still_redacts_bearer_and_assignment() {
        let redacted = redact_secrets("Authorization: Bearer sometokenvalue1234567890".to_string());
        assert!(
            redacted.contains("Bearer [REDACTED_SECRET]"),
            "got: {redacted}"
        );

        let redacted = redact_secrets("password=supersecretvalue".to_string());
        assert!(!redacted.contains("supersecretvalue"), "got: {redacted}");
    }

    #[test]
    fn leaves_ordinary_text_untouched() {
        let input = "The quick brown fox jumps over the lazy dog.".to_string();
        assert_eq!(redact_secrets(input.clone()), input);
    }
}
