#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub enum Harness {
    #[default]
    Native,
    ClaudeCode,
    ClaudeCodeBare,
    DeepSeekTui,
    KimiCode,
    KimiCli,
    ZCode,
    LittleCoder,
    Local,
    MiniSweAgent,
    OpenCode,
    Pi,
    QwenCode,
    SweAgent,
    Terminus2,
    Minimal,
    Other(String),
}

impl Harness {
    pub fn from_config_name(name: Option<&str>) -> Self {
        match name {
            None | Some("") => Self::Native,
            Some("claude-code") => Self::ClaudeCode,
            Some("claude-code-bare") => Self::ClaudeCodeBare,
            Some("deepseek-tui") => Self::DeepSeekTui,
            Some("kimi-code") => Self::KimiCode,
            Some("kimi-cli") => Self::KimiCli,
            Some("zcode") => Self::ZCode,
            Some("little-coder") => Self::LittleCoder,
            Some("local") => Self::Local,
            Some("mini-swe-agent") => Self::MiniSweAgent,
            Some("opencode") => Self::OpenCode,
            Some("pi") => Self::Pi,
            Some("qwen-code") => Self::QwenCode,
            Some("swe-agent") => Self::SweAgent,
            Some("terminus-2") => Self::Terminus2,
            Some("minimal") => Self::Minimal,
            Some(other) => Self::Other(other.to_string()),
        }
    }

    pub fn is_claude_code(&self) -> bool {
        matches!(self, Self::ClaudeCode | Self::ClaudeCodeBare)
    }

    pub fn is_claude_code_bare(&self) -> bool {
        matches!(self, Self::ClaudeCodeBare)
    }

    pub fn is_kimi_cli(&self) -> bool {
        matches!(self, Self::KimiCli)
    }

    pub fn is_kimi_code(&self) -> bool {
        matches!(self, Self::KimiCode)
    }

    pub fn is_zcode(&self) -> bool {
        matches!(self, Self::ZCode)
    }

    pub fn is_little_coder(&self) -> bool {
        matches!(self, Self::LittleCoder)
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    pub fn is_opencode(&self) -> bool {
        matches!(self, Self::OpenCode)
    }

    pub fn is_pi(&self) -> bool {
        matches!(self, Self::Pi)
    }

    pub fn is_mini_swe_agent(&self) -> bool {
        matches!(self, Self::MiniSweAgent)
    }

    pub fn is_deepseek_tui(&self) -> bool {
        matches!(self, Self::DeepSeekTui)
    }

    pub fn is_qwen_code(&self) -> bool {
        matches!(self, Self::QwenCode)
    }

    pub fn is_swe_agent(&self) -> bool {
        matches!(self, Self::SweAgent)
    }

    pub fn is_terminus_2(&self) -> bool {
        matches!(self, Self::Terminus2)
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
        assert_eq!(Harness::from_config_name(/*name*/ None), Harness::Native);
        assert_eq!(
            Harness::from_config_name(Some("claude-code")),
            Harness::ClaudeCode
        );
        assert_eq!(
            Harness::from_config_name(Some("claude-code-bare")),
            Harness::ClaudeCodeBare
        );
        assert_eq!(
            Harness::from_config_name(Some("deepseek-tui")),
            Harness::DeepSeekTui
        );
        assert_eq!(
            Harness::from_config_name(Some("kimi-cli")),
            Harness::KimiCli
        );
        assert_eq!(
            Harness::from_config_name(Some("kimi-code")),
            Harness::KimiCode
        );
        assert_eq!(Harness::from_config_name(Some("zcode")), Harness::ZCode);
        assert_eq!(
            Harness::from_config_name(Some("little-coder")),
            Harness::LittleCoder
        );
        assert_eq!(
            Harness::from_config_name(Some("mini-swe-agent")),
            Harness::MiniSweAgent
        );
        assert_eq!(
            Harness::from_config_name(Some("opencode")),
            Harness::OpenCode
        );
        assert_eq!(Harness::from_config_name(Some("pi")), Harness::Pi);
        assert_eq!(Harness::from_config_name(Some("local")), Harness::Local);
        assert_eq!(
            Harness::from_config_name(Some("qwen-code")),
            Harness::QwenCode
        );
        assert_eq!(
            Harness::from_config_name(Some("swe-agent")),
            Harness::SweAgent
        );
        assert_eq!(
            Harness::from_config_name(Some("terminus-2")),
            Harness::Terminus2
        );
        assert_eq!(Harness::from_config_name(Some("minimal")), Harness::Minimal);
    }
}
