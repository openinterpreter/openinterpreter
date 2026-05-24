const LOW_MEMORY_ENV: &str = "INTERPRETER_TUI_LOW_MEMORY";
const DROP_COMMITTED_HISTORY_ENV: &str = "INTERPRETER_TUI_DROP_COMMITTED_HISTORY";
const ACTIVE_EXEC_OUTPUT_MAX_BYTES_ENV: &str = "INTERPRETER_TUI_ACTIVE_EXEC_OUTPUT_MAX_BYTES";
const DEFAULT_ACTIVE_EXEC_OUTPUT_MAX_BYTES: usize = 65_536;

pub(crate) fn retain_committed_history() -> bool {
    retain_committed_history_from_env(env_value)
}

pub(crate) fn active_exec_output_max_bytes(marker_len: usize) -> Option<usize> {
    active_exec_output_max_bytes_from_env(marker_len, env_value)
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn retain_committed_history_from_env(get: impl Fn(&str) -> Option<String>) -> bool {
    if let Some(value) = get(DROP_COMMITTED_HISTORY_ENV) {
        return !flag_value_enabled(&value);
    }
    !low_memory_enabled_from_env(get)
}

fn active_exec_output_max_bytes_from_env(
    marker_len: usize,
    get: impl Fn(&str) -> Option<String>,
) -> Option<usize> {
    if let Some(value) = get(ACTIVE_EXEC_OUTPUT_MAX_BYTES_ENV) {
        return parse_output_max_bytes(&value, marker_len);
    }
    low_memory_enabled_from_env(get).then_some(DEFAULT_ACTIVE_EXEC_OUTPUT_MAX_BYTES)
}

fn low_memory_enabled_from_env(get: impl Fn(&str) -> Option<String>) -> bool {
    get(LOW_MEMORY_ENV)
        .as_deref()
        .map(flag_value_enabled)
        .unwrap_or(true)
}

fn parse_output_max_bytes(value: &str, marker_len: usize) -> Option<usize> {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|max_bytes| *max_bytes > marker_len)
}

fn flag_value_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env<'a>(values: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let values = values
            .iter()
            .map(|(key, value)| (*key, *value))
            .collect::<HashMap<_, _>>();
        move |name| values.get(name).map(|value| (*value).to_string())
    }

    #[test]
    fn low_memory_is_enabled_by_default() {
        assert!(!retain_committed_history_from_env(env(&[])));
        assert_eq!(
            active_exec_output_max_bytes_from_env(20, env(&[])),
            Some(DEFAULT_ACTIVE_EXEC_OUTPUT_MAX_BYTES)
        );
    }

    #[test]
    fn low_memory_can_be_disabled() {
        let env = env(&[(LOW_MEMORY_ENV, "0")]);
        assert!(retain_committed_history_from_env(&env));
        assert_eq!(active_exec_output_max_bytes_from_env(20, &env), None);
    }

    #[test]
    fn granular_drop_history_override_wins() {
        let env = env(&[(LOW_MEMORY_ENV, "0"), (DROP_COMMITTED_HISTORY_ENV, "1")]);
        assert!(!retain_committed_history_from_env(env));
    }

    #[test]
    fn granular_exec_output_cap_override_wins() {
        let env = env(&[(ACTIVE_EXEC_OUTPUT_MAX_BYTES_ENV, "128")]);
        assert_eq!(active_exec_output_max_bytes_from_env(20, env), Some(128));
    }

    #[test]
    fn granular_exec_output_cap_can_disable_default_cap() {
        let env = env(&[(ACTIVE_EXEC_OUTPUT_MAX_BYTES_ENV, "0")]);
        assert_eq!(active_exec_output_max_bytes_from_env(20, env), None);
    }

    #[test]
    fn flag_value_enabled_accepts_common_true_values() {
        for value in ["1", "true", "TRUE", "yes", "on"] {
            assert!(flag_value_enabled(value), "{value}");
        }
    }

    #[test]
    fn flag_value_enabled_rejects_other_values() {
        for value in ["", "0", "false", "no", "off", "anything"] {
            assert!(!flag_value_enabled(value), "{value}");
        }
    }
}
