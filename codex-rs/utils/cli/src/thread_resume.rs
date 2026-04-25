use codex_protocol::ThreadId;

/// Trim a thread name and return `None` if it is empty after trimming.
pub fn normalize_thread_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn resume_command(thread_name: Option<&str>, thread_id: Option<ThreadId>) -> Option<String> {
    let resume_target = thread_name
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| thread_id.map(|thread_id| thread_id.to_string()));
    resume_target.map(|target| {
        let needs_double_dash = target.starts_with('-');
        let escaped = shlex::try_join([target.as_str()]).unwrap_or_else(|_| target.clone());
        if needs_double_dash {
            format!("codex resume -- {escaped}")
        } else {
            format!("codex resume {escaped}")
        }
    })
}

#[cfg(test)]
mod tests {
    use codex_protocol::ThreadId;
    use pretty_assertions::assert_eq;

    use super::normalize_thread_name;
    use super::resume_command;

    #[test]
    fn normalize_thread_name_trims_and_rejects_empty() {
        assert_eq!(normalize_thread_name("   "), None);
        assert_eq!(
            normalize_thread_name("  my thread  "),
            Some("my thread".to_string())
        );
    }

    #[test]
    fn resume_command_prefers_name_over_id() {
        let thread_id =
            ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000").expect("valid thread id");
        let command = resume_command(Some("my-thread"), Some(thread_id));
        assert_eq!(command, Some("codex resume my-thread".to_string()));
    }

    #[test]
    fn resume_command_with_only_id() {
        let thread_id =
            ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000").expect("valid thread id");
        let command = resume_command(/*thread_name*/ None, Some(thread_id));
        assert_eq!(
            command,
            Some("codex resume 123e4567-e89b-12d3-a456-426614174000".to_string())
        );
    }

    #[test]
    fn resume_command_with_no_name_or_id() {
        let command = resume_command(/*thread_name*/ None, /*thread_id*/ None);
        assert_eq!(command, None);
    }

    #[test]
    fn resume_command_quotes_thread_name_when_needed() {
        let command = resume_command(Some("-starts-with-dash"), /*thread_id*/ None);
        assert_eq!(
            command,
            Some("codex resume -- -starts-with-dash".to_string())
        );

        let command = resume_command(Some("two words"), /*thread_id*/ None);
        assert_eq!(command, Some("codex resume 'two words'".to_string()));

        let command = resume_command(Some("quote'case"), /*thread_id*/ None);
        assert_eq!(command, Some("codex resume \"quote'case\"".to_string()));
    }
}
