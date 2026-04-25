use std::ffi::OsStr;

pub(crate) const OPEN_INTERPRETER_BRAND_ENV_VAR: &str = "OPEN_INTERPRETER_BRAND";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProductBranding {
    pub(crate) is_open_interpreter: bool,
    pub(crate) display_name: &'static str,
    pub(crate) welcome_suffix: &'static str,
    pub(crate) auth_intro_primary: &'static str,
    pub(crate) auth_intro_secondary: &'static str,
    pub(crate) api_key_intro: &'static str,
}

impl ProductBranding {
    pub(crate) fn current() -> Self {
        Self::for_open_interpreter(is_open_interpreter_brand(
            std::env::var_os(OPEN_INTERPRETER_BRAND_ENV_VAR).as_deref(),
        ))
    }

    pub(crate) fn for_open_interpreter(is_open_interpreter: bool) -> Self {
        if is_open_interpreter {
            return Self {
                is_open_interpreter: true,
                display_name: "Open Interpreter",
                welcome_suffix: ".",
                auth_intro_primary: "Welcome to Open Interpreter.",
                auth_intro_secondary: "Choose a provider to get started.",
                api_key_intro: "Use your own API key with Open Interpreter",
            };
        }

        Self {
            is_open_interpreter: false,
            display_name: "Codex",
            welcome_suffix: ", OpenAI's command-line coding agent",
            auth_intro_primary: "Sign in with ChatGPT to use Codex as part of your paid plan",
            auth_intro_secondary: "or connect an API key for usage-based billing",
            api_key_intro: "Use your own OpenAI API key for usage-based billing",
        }
    }

    pub(crate) fn session_header_name(self) -> &'static str {
        if self.is_open_interpreter {
            self.display_name
        } else {
            "OpenAI Codex"
        }
    }

    pub(crate) fn agent_name(self) -> &'static str {
        if self.is_open_interpreter {
            "Interpreter"
        } else {
            "Codex"
        }
    }

    pub(crate) fn agent_name_lowercase(self) -> &'static str {
        if self.is_open_interpreter {
            "interpreter"
        } else {
            "codex"
        }
    }

    pub(crate) fn command_name(self) -> &'static str {
        if self.is_open_interpreter {
            "interpreter"
        } else {
            "codex"
        }
    }
}

fn is_open_interpreter_brand(value: Option<&OsStr>) -> bool {
    match value {
        None => true,
        Some(value) => !value.is_empty() && value != OsStr::new("0"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn codex_branding_is_default() {
        let branding = ProductBranding::for_open_interpreter(/*is_open_interpreter*/ false);

        assert!(!branding.is_open_interpreter);
        assert_eq!(branding.display_name, "Codex");
    }

    #[test]
    fn open_interpreter_branding_is_default_and_zero_disables_it() {
        assert!(is_open_interpreter_brand(None));
        assert!(is_open_interpreter_brand(Some(OsStr::new("1"))));
        assert!(!is_open_interpreter_brand(Some(OsStr::new(""))));
        assert!(!is_open_interpreter_brand(Some(OsStr::new("0"))));
    }

    #[test]
    fn session_header_name_matches_branding() {
        assert_eq!(
            ProductBranding::for_open_interpreter(/*is_open_interpreter*/ false)
                .session_header_name(),
            "OpenAI Codex"
        );
        assert_eq!(
            ProductBranding::for_open_interpreter(/*is_open_interpreter*/ true)
                .session_header_name(),
            "Open Interpreter"
        );
    }

    #[test]
    fn agent_and_command_names_match_branding() {
        let codex = ProductBranding::for_open_interpreter(/*is_open_interpreter*/ false);
        assert_eq!(codex.agent_name(), "Codex");
        assert_eq!(codex.agent_name_lowercase(), "codex");
        assert_eq!(codex.command_name(), "codex");

        let interpreter = ProductBranding::for_open_interpreter(/*is_open_interpreter*/ true);
        assert_eq!(interpreter.agent_name(), "Interpreter");
        assert_eq!(interpreter.agent_name_lowercase(), "interpreter");
        assert_eq!(interpreter.command_name(), "interpreter");
    }
}
