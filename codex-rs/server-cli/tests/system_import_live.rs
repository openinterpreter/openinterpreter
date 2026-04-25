#![cfg(not(target_os = "windows"))]

use codex_server_cli::system_import::import_system_state;
use tempfile::TempDir;
use toml::Table as TomlTable;
use toml::Value as TomlValue;

const CONFIG_TOML_FILE: &str = "config.toml";
const AUTH_JSON_FILE: &str = "auth.json";
const MODEL_KEY: &str = "model";
const MODEL_PROVIDER_KEY: &str = "model_provider";
const MODEL_PROVIDERS_KEY: &str = "model_providers";

#[test]
#[ignore = "live local machine import smoke"]
fn live_system_import_populates_ready_providers_for_this_machine() -> anyhow::Result<()> {
    let interpreter_home = TempDir::new()?;

    import_system_state(interpreter_home.path())?;

    let config = read_toml_table(&interpreter_home.path().join(CONFIG_TOML_FILE))?;
    let providers = match get_table(&config, MODEL_PROVIDERS_KEY) {
        Some(providers) => providers,
        None => panic!("model providers"),
    };

    assert!(providers.contains_key("openai_api_key"));
    assert!(providers.contains_key("groq"));
    assert!(
        providers.len() >= 2,
        "expected a few imported providers, found only {:?}",
        providers.keys().collect::<Vec<_>>()
    );
    assert!(
        matches!(
            get_string(&config, MODEL_PROVIDER_KEY),
            Some("openai_api_key" | "groq")
        ),
        "expected imported default provider from this machine, found {:?}",
        get_string(&config, MODEL_PROVIDER_KEY)
    );
    assert!(
        get_string(&config, MODEL_KEY).is_some(),
        "expected imported default model from system state"
    );
    assert!(
        interpreter_home.path().join(AUTH_JSON_FILE).exists(),
        "expected imported auth.json from existing system auth"
    );
    Ok(())
}

fn read_toml_table(path: &std::path::Path) -> anyhow::Result<TomlTable> {
    let contents = std::fs::read_to_string(path)?;
    Ok(toml::from_str::<TomlTable>(&contents)?)
}

fn get_table<'a>(root: &'a TomlTable, key: &str) -> Option<&'a TomlTable> {
    root.get(key).and_then(TomlValue::as_table)
}

fn get_string<'a>(root: &'a TomlTable, key: &str) -> Option<&'a str> {
    root.get(key).and_then(TomlValue::as_str)
}
