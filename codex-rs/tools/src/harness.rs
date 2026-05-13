#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub enum Harness {
    #[default]
    Native,
    ClaudeCode,
    KimiCli,
    QwenCode,
    Minimal,
    Other(String),
}

impl Harness {
    pub fn from_config_name(name: Option<&str>) -> Self {
        match name {
            None | Some("") => Self::Native,
            Some("claude-code") => Self::ClaudeCode,
            Some("kimi-cli") => Self::KimiCli,
            Some("qwen-code") => Self::QwenCode,
            Some("minimal") => Self::Minimal,
            Some(other) => Self::Other(other.to_string()),
        }
    }

    pub fn is_claude_code(&self) -> bool {
        matches!(self, Self::ClaudeCode)
    }

    pub fn is_kimi_cli(&self) -> bool {
        matches!(self, Self::KimiCli)
    }

    pub fn is_qwen_code(&self) -> bool {
        matches!(self, Self::QwenCode)
    }

    pub fn is_minimal(&self) -> bool {
        matches!(self, Self::Minimal)
    }
}

#[cfg(test)]
mod tests {
    use super::Harness;
    use pretty_assertions::assert_eq;

    #[test]
    fn from_config_name_parses_known_harnesses() {
        assert_eq!(Harness::from_config_name(None), Harness::Native);
        assert_eq!(
            Harness::from_config_name(Some("claude-code")),
            Harness::ClaudeCode
        );
        assert_eq!(
            Harness::from_config_name(Some("kimi-cli")),
            Harness::KimiCli
        );
        assert_eq!(
            Harness::from_config_name(Some("qwen-code")),
            Harness::QwenCode
        );
        assert_eq!(Harness::from_config_name(Some("minimal")), Harness::Minimal);
    }
}
