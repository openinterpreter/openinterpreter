use crate::home::INTERPRETER_HOME_ENV_VAR;
use crate::home::OPEN_INTERPRETER_HOME_ENV_VAR;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupModelPreview {
    pub model_display: String,
    pub reasoning_effort: Option<ReasoningEffortConfig>,
}

impl StartupModelPreview {
    pub fn resolve(
        explicit_model: Option<&str>,
        explicit_profile: Option<&str>,
    ) -> StartupModelPreview {
        let Some(config) = load_startup_config() else {
            return StartupModelPreview {
                model_display: explicit_model.unwrap_or("default").to_string(),
                reasoning_effort: None,
            };
        };

        let active_profile = explicit_profile
            .filter(|profile| !profile.is_empty())
            .or(config.profile.as_deref());
        let profile = active_profile.and_then(|name| config.profiles.get(name));
        let model_display = explicit_model
            .filter(|model| !model.is_empty())
            .map(str::to_string)
            .or_else(|| profile.and_then(|profile| profile.model.clone()))
            .or(config.model)
            .unwrap_or_else(|| "default".to_string());
        let reasoning_effort = profile
            .and_then(|profile| profile.model_reasoning_effort)
            .or(config.model_reasoning_effort);

        StartupModelPreview {
            model_display,
            reasoning_effort,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct StartupConfigToml {
    model: Option<String>,
    model_reasoning_effort: Option<ReasoningEffortConfig>,
    profile: Option<String>,
    #[serde(default)]
    profiles: HashMap<String, StartupConfigProfile>,
}

#[derive(Debug, Default, Deserialize)]
struct StartupConfigProfile {
    model: Option<String>,
    model_reasoning_effort: Option<ReasoningEffortConfig>,
}

fn load_startup_config() -> Option<StartupConfigToml> {
    let codex_home = std::env::var_os(INTERPRETER_HOME_ENV_VAR)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var_os(OPEN_INTERPRETER_HOME_ENV_VAR).filter(|value| !value.is_empty())
        })?;
    load_startup_config_from_path(PathBuf::from(codex_home).join("config.toml"))
}

fn load_startup_config_from_path(path: PathBuf) -> Option<StartupConfigToml> {
    let text = fs::read_to_string(path).ok()?;
    toml::from_str::<StartupConfigToml>(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn preview_prefers_explicit_model() {
        let preview = StartupModelPreview::resolve(Some("groq/compound-beta"), None);

        assert_eq!(
            preview,
            StartupModelPreview {
                model_display: "groq/compound-beta".to_string(),
                reasoning_effort: None,
            }
        );
    }

    #[test]
    fn preview_reads_profile_model_from_config() {
        let home = TempDir::new().expect("temp dir");
        let path = home.path().join("config.toml");
        fs::write(
            &path,
            r#"
model = "root-model"

[profiles.fast]
model = "profile-model"
model_reasoning_effort = "high"
"#,
        )
        .expect("write config");
        let config = load_startup_config_from_path(path).expect("config");
        let preview = StartupModelPreview {
            model_display: config
                .profiles
                .get("fast")
                .and_then(|profile| profile.model.clone())
                .expect("profile model"),
            reasoning_effort: config
                .profiles
                .get("fast")
                .and_then(|profile| profile.model_reasoning_effort),
        };

        assert_eq!(
            preview,
            StartupModelPreview {
                model_display: "profile-model".to_string(),
                reasoning_effort: Some(ReasoningEffortConfig::High),
            }
        );
    }

    #[test]
    fn preview_uses_root_model_when_no_profile_selected() {
        let home = TempDir::new().expect("temp dir");
        let path = home.path().join("config.toml");
        fs::write(
            &path,
            r#"
model = "root-model"
model_reasoning_effort = "medium"
"#,
        )
        .expect("write config");
        let config = load_startup_config_from_path(path).expect("config");
        let preview = StartupModelPreview {
            model_display: config.model.expect("root model"),
            reasoning_effort: config.model_reasoning_effort,
        };

        assert_eq!(
            preview,
            StartupModelPreview {
                model_display: "root-model".to_string(),
                reasoning_effort: Some(ReasoningEffortConfig::Medium),
            }
        );
    }
}
