use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use codex_model_provider_info::BundledProviderCatalogEntry;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::bundled_provider_catalog;
use codex_model_provider_info::bundled_provider_catalog_entry;
use toml::Table as TomlTable;
use toml::Value as TomlValue;

const CODEX_HOME_DIR: &str = ".codex";
const OPENCODE_HOME_DIR: &str = ".opencode";
const OPENCODE_STATE_DIR: &str = ".local/state/opencode";
const CONFIG_TOML_FILE: &str = "config.toml";
const AUTH_JSON_FILE: &str = "auth.json";
const MODEL_PROVIDER_KEY: &str = "model_provider";
const MODEL_KEY: &str = "model";
const MODEL_REASONING_EFFORT_KEY: &str = "model_reasoning_effort";
const MODEL_PROVIDERS_KEY: &str = "model_providers";
const OPENAI_PROVIDER_ID: &str = "openai";
const OPENAI_API_KEY_PROVIDER_ID: &str = "openai_api_key";
const OPENAI_API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";

#[derive(Clone, Debug, Eq, PartialEq)]
struct OpencodeApiAuth {
    provider_id: String,
    api_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OpencodeOauthAuth {
    provider_id: String,
    access_token: String,
    refresh_token: String,
    account_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OpencodeRecentModel {
    provider_id: String,
    model_id: String,
}

pub fn import_system_state(interpreter_home: &Path) -> io::Result<()> {
    let snapshot = SystemImportSnapshot::from_system();
    import_system_state_with_snapshot(interpreter_home, &snapshot)
}

fn import_system_state_with_snapshot(
    interpreter_home: &Path,
    snapshot: &SystemImportSnapshot,
) -> io::Result<()> {
    std::fs::create_dir_all(interpreter_home)?;
    import_auth_file(interpreter_home, snapshot)?;
    import_config(interpreter_home, snapshot)
}

struct SystemImportSnapshot {
    codex_home: Option<PathBuf>,
    present_env_keys: HashSet<String>,
    opencode_api_auth: Vec<OpencodeApiAuth>,
    opencode_oauth_auth: Vec<OpencodeOauthAuth>,
    opencode_recent_model: Option<OpencodeRecentModel>,
}

impl SystemImportSnapshot {
    fn from_system() -> Self {
        let opencode_home = default_opencode_home();
        let opencode_state_dir = default_opencode_state_dir();
        Self {
            codex_home: default_codex_home(),
            present_env_keys: std::env::vars_os()
                .filter_map(|(key, value)| (!value.is_empty()).then_some(key))
                .map(|key| key.to_string_lossy().to_string())
                .collect(),
            opencode_api_auth: opencode_home
                .as_ref()
                .map(|home| read_opencode_api_auth(&home.join("data").join(AUTH_JSON_FILE)))
                .transpose()
                .unwrap_or_default()
                .unwrap_or_default(),
            opencode_oauth_auth: opencode_home
                .as_ref()
                .map(|home| read_opencode_oauth_auth(&home.join("data").join(AUTH_JSON_FILE)))
                .transpose()
                .unwrap_or_default()
                .unwrap_or_default(),
            opencode_recent_model: opencode_state_dir
                .as_ref()
                .map(|dir| read_opencode_recent_model(&dir.join("model.json")))
                .transpose()
                .unwrap_or_default()
                .flatten(),
        }
    }

    fn env_var_present(&self, key: &str) -> bool {
        self.present_env_keys.contains(key)
    }
}

fn import_auth_file(interpreter_home: &Path, snapshot: &SystemImportSnapshot) -> io::Result<()> {
    let interpreter_auth_path = interpreter_home.join(AUTH_JSON_FILE);
    if interpreter_auth_path.exists() {
        return Ok(());
    }

    if let Some(codex_home) = snapshot.codex_home.as_deref() {
        let codex_auth_path = codex_home.join(AUTH_JSON_FILE);
        if codex_auth_path.exists() && !same_path(interpreter_home, codex_home) {
            std::fs::copy(codex_auth_path, interpreter_auth_path)?;
            return Ok(());
        }
    }

    if let Some(openai_oauth) = snapshot
        .opencode_oauth_auth
        .iter()
        .find(|auth| auth.provider_id == OPENAI_PROVIDER_ID)
    {
        let payload = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": openai_oauth.access_token,
                "access_token": openai_oauth.access_token,
                "refresh_token": openai_oauth.refresh_token,
                "account_id": openai_oauth.account_id,
            },
        });
        std::fs::write(
            interpreter_auth_path,
            serde_json::to_string_pretty(&payload).map_err(io::Error::other)?,
        )?;
    }

    Ok(())
}

fn import_config(interpreter_home: &Path, snapshot: &SystemImportSnapshot) -> io::Result<()> {
    let config_path = interpreter_home.join(CONFIG_TOML_FILE);
    let mut interpreter_config = read_toml_table(&config_path)?;
    let codex_config = snapshot
        .codex_home
        .as_deref()
        .filter(|codex_home| !same_path(interpreter_home, codex_home))
        .map(|codex_home| read_toml_table(&codex_home.join(CONFIG_TOML_FILE)))
        .transpose()?;
    let codex_auth_mode = snapshot
        .codex_home
        .as_deref()
        .map(|codex_home| read_auth_mode(&codex_home.join(AUTH_JSON_FILE)))
        .transpose()?
        .flatten();

    let mut changed = false;
    changed |= import_codex_provider_entries(&mut interpreter_config, codex_config.as_ref());
    changed |= import_env_provider_entries(&mut interpreter_config, snapshot);
    changed |= import_opencode_provider_entries(&mut interpreter_config, snapshot);
    changed |= import_codex_defaults(
        &mut interpreter_config,
        codex_config.as_ref(),
        codex_auth_mode.as_deref(),
    );
    changed |= import_opencode_defaults(&mut interpreter_config, snapshot);

    if changed {
        let serialized = toml::to_string_pretty(&interpreter_config).map_err(io::Error::other)?;
        std::fs::write(config_path, serialized)?;
    }

    Ok(())
}

fn import_codex_provider_entries(
    interpreter_config: &mut TomlTable,
    codex_config: Option<&TomlTable>,
) -> bool {
    let Some(codex_config) = codex_config else {
        return false;
    };
    let Some(codex_providers) = get_table(codex_config, MODEL_PROVIDERS_KEY) else {
        return false;
    };

    let interpreter_providers = ensure_table(interpreter_config, MODEL_PROVIDERS_KEY);
    let mut changed = false;
    for (provider_id, provider_value) in codex_providers {
        if !interpreter_providers.contains_key(provider_id) {
            interpreter_providers.insert(provider_id.clone(), provider_value.clone());
            changed = true;
        }
    }
    changed
}

fn import_env_provider_entries(
    interpreter_config: &mut TomlTable,
    snapshot: &SystemImportSnapshot,
) -> bool {
    let interpreter_providers = ensure_table(interpreter_config, MODEL_PROVIDERS_KEY);
    let mut changed = false;

    if snapshot.env_var_present(OPENAI_API_KEY_ENV_VAR)
        && !interpreter_providers.contains_key(OPENAI_API_KEY_PROVIDER_ID)
    {
        interpreter_providers.insert(
            OPENAI_API_KEY_PROVIDER_ID.to_string(),
            openai_api_key_provider_value(),
        );
        changed = true;
    }

    for entry in bundled_provider_catalog() {
        let Some(env_key) = entry.env_key.as_deref() else {
            continue;
        };
        if !snapshot.env_var_present(env_key) || interpreter_providers.contains_key(&entry.id) {
            continue;
        }

        interpreter_providers.insert(entry.id.clone(), bundled_provider_value(entry));
        changed = true;
    }

    changed
}

fn import_opencode_provider_entries(
    interpreter_config: &mut TomlTable,
    snapshot: &SystemImportSnapshot,
) -> bool {
    let interpreter_providers = ensure_table(interpreter_config, MODEL_PROVIDERS_KEY);
    let mut changed = false;

    for auth in &snapshot.opencode_api_auth {
        let provider_id = translated_opencode_provider_id(auth.provider_id.as_str());
        if interpreter_providers.contains_key(provider_id) {
            continue;
        }

        let Some(provider_value) =
            provider_value_from_imported_api_key(provider_id, auth.api_key.as_str())
        else {
            continue;
        };

        interpreter_providers.insert(provider_id.to_string(), provider_value);
        changed = true;
    }

    for auth in &snapshot.opencode_oauth_auth {
        let provider_id = translated_opencode_provider_id(auth.provider_id.as_str());
        if provider_id == OPENAI_PROVIDER_ID || interpreter_providers.contains_key(provider_id) {
            continue;
        }

        let Some(entry) = bundled_provider_catalog_entry(provider_id) else {
            continue;
        };

        interpreter_providers.insert(
            provider_id.to_string(),
            bundled_provider_value_with_token(entry, auth.access_token.as_str()),
        );
        changed = true;
    }

    changed
}

fn import_codex_defaults(
    interpreter_config: &mut TomlTable,
    codex_config: Option<&TomlTable>,
    codex_auth_mode: Option<&str>,
) -> bool {
    let Some(codex_config) = codex_config else {
        return false;
    };
    if interpreter_config.contains_key(MODEL_PROVIDER_KEY) {
        return false;
    }

    let Some(codex_provider_id) =
        get_string(codex_config, MODEL_PROVIDER_KEY).map(std::string::ToString::to_string)
    else {
        return false;
    };

    let provider_id = translated_provider_id(codex_provider_id.as_str(), codex_auth_mode);
    let mut changed = false;
    if provider_id == OPENAI_API_KEY_PROVIDER_ID {
        let interpreter_providers = ensure_table(interpreter_config, MODEL_PROVIDERS_KEY);
        if !interpreter_providers.contains_key(OPENAI_API_KEY_PROVIDER_ID) {
            interpreter_providers.insert(
                OPENAI_API_KEY_PROVIDER_ID.to_string(),
                openai_api_key_provider_value(),
            );
            changed = true;
        }
    }
    let provider_available = provider_id == OPENAI_PROVIDER_ID
        || get_table(interpreter_config, MODEL_PROVIDERS_KEY)
            .is_some_and(|providers| providers.contains_key(provider_id));
    if !provider_available {
        return changed;
    }

    interpreter_config.insert(
        MODEL_PROVIDER_KEY.to_string(),
        TomlValue::String(provider_id.to_string()),
    );
    if let Some(model) = get_string(codex_config, MODEL_KEY) {
        interpreter_config.insert(MODEL_KEY.to_string(), TomlValue::String(model.to_string()));
    }
    if let Some(reasoning_effort) = get_string(codex_config, MODEL_REASONING_EFFORT_KEY) {
        interpreter_config.insert(
            MODEL_REASONING_EFFORT_KEY.to_string(),
            TomlValue::String(reasoning_effort.to_string()),
        );
    }
    true
}

fn import_opencode_defaults(
    interpreter_config: &mut TomlTable,
    snapshot: &SystemImportSnapshot,
) -> bool {
    if interpreter_config.contains_key(MODEL_PROVIDER_KEY) {
        return false;
    }

    let Some(recent_model) = snapshot.opencode_recent_model.as_ref() else {
        return false;
    };
    let provider_id = if recent_model.provider_id == OPENAI_PROVIDER_ID
        && snapshot
            .opencode_oauth_auth
            .iter()
            .any(|auth| auth.provider_id == OPENAI_PROVIDER_ID)
    {
        OPENAI_PROVIDER_ID
    } else {
        translated_opencode_provider_id(recent_model.provider_id.as_str())
    };
    let provider_available = provider_id == OPENAI_PROVIDER_ID
        || get_table(interpreter_config, MODEL_PROVIDERS_KEY)
            .is_some_and(|providers| providers.contains_key(provider_id));
    if !provider_available {
        return false;
    }

    interpreter_config.insert(
        MODEL_PROVIDER_KEY.to_string(),
        TomlValue::String(provider_id.to_string()),
    );
    interpreter_config.insert(
        MODEL_KEY.to_string(),
        TomlValue::String(recent_model.model_id.clone()),
    );
    true
}

fn translated_provider_id<'a>(provider_id: &'a str, codex_auth_mode: Option<&str>) -> &'a str {
    if provider_id == OPENAI_PROVIDER_ID && codex_auth_mode == Some("apikey") {
        OPENAI_API_KEY_PROVIDER_ID
    } else {
        provider_id
    }
}

fn translated_opencode_provider_id(provider_id: &str) -> &str {
    if provider_id == OPENAI_PROVIDER_ID {
        OPENAI_API_KEY_PROVIDER_ID
    } else {
        provider_id
    }
}

fn openai_api_key_provider_value() -> TomlValue {
    let mut provider = TomlTable::new();
    provider.insert(
        "name".to_string(),
        TomlValue::String("OpenAI (API key)".to_string()),
    );
    provider.insert(
        "base_url".to_string(),
        TomlValue::String("https://api.openai.com/v1".to_string()),
    );
    provider.insert(
        "wire_api".to_string(),
        TomlValue::String(WireApi::Responses.to_string()),
    );
    provider.insert(
        "requires_openai_auth".to_string(),
        TomlValue::Boolean(false),
    );
    provider.insert(
        "env_key".to_string(),
        TomlValue::String(OPENAI_API_KEY_ENV_VAR.to_string()),
    );
    TomlValue::Table(provider)
}

fn openai_api_key_provider_value_with_token(api_key: &str) -> TomlValue {
    let mut provider = match openai_api_key_provider_value() {
        TomlValue::Table(provider) => provider,
        _ => unreachable!("openai api key provider must serialize as a table"),
    };
    provider.remove("env_key");
    provider.insert(
        "experimental_bearer_token".to_string(),
        TomlValue::String(api_key.to_string()),
    );
    TomlValue::Table(provider)
}

fn bundled_provider_value(entry: &BundledProviderCatalogEntry) -> TomlValue {
    let mut provider = TomlTable::new();
    provider.insert("name".to_string(), TomlValue::String(entry.name.clone()));
    provider.insert(
        "base_url".to_string(),
        TomlValue::String(entry.base_url.clone()),
    );
    provider.insert(
        "wire_api".to_string(),
        TomlValue::String(entry.wire_api.to_string()),
    );
    if let Some(env_key) = entry.env_key.as_ref() {
        provider.insert("env_key".to_string(), TomlValue::String(env_key.clone()));
    }
    TomlValue::Table(provider)
}

fn bundled_provider_value_with_token(
    entry: &BundledProviderCatalogEntry,
    token: &str,
) -> TomlValue {
    let mut provider = match bundled_provider_value(entry) {
        TomlValue::Table(provider) => provider,
        _ => unreachable!("bundled provider must serialize as a table"),
    };
    provider.remove("env_key");
    provider.insert(
        "experimental_bearer_token".to_string(),
        TomlValue::String(token.to_string()),
    );
    TomlValue::Table(provider)
}

fn provider_value_from_imported_api_key(provider_id: &str, api_key: &str) -> Option<TomlValue> {
    if provider_id == OPENAI_API_KEY_PROVIDER_ID {
        return Some(openai_api_key_provider_value_with_token(api_key));
    }

    bundled_provider_catalog_entry(provider_id)
        .map(|entry| bundled_provider_value_with_token(entry, api_key))
}

fn read_toml_table(path: &Path) -> io::Result<TomlTable> {
    match std::fs::read_to_string(path) {
        Ok(contents) => toml::from_str::<TomlTable>(&contents).map_err(io::Error::other),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(TomlTable::new()),
        Err(err) => Err(err),
    }
}

fn read_auth_mode(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let auth =
                serde_json::from_str::<serde_json::Value>(&contents).map_err(io::Error::other)?;
            Ok(auth
                .get("auth_mode")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn read_opencode_api_auth(path: &Path) -> io::Result<Vec<OpencodeApiAuth>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let auth =
                serde_json::from_str::<serde_json::Value>(&contents).map_err(io::Error::other)?;
            let providers = auth
                .as_object()
                .into_iter()
                .flat_map(|providers| providers.iter())
                .filter_map(|(provider_id, info)| {
                    let info = info.as_object()?;
                    if info.get("type").and_then(serde_json::Value::as_str) != Some("api") {
                        return None;
                    }
                    let api_key = info.get("key").and_then(serde_json::Value::as_str)?;
                    (!api_key.trim().is_empty()).then_some(OpencodeApiAuth {
                        provider_id: provider_id.clone(),
                        api_key: api_key.to_string(),
                    })
                })
                .collect();
            Ok(providers)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

fn read_opencode_oauth_auth(path: &Path) -> io::Result<Vec<OpencodeOauthAuth>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let auth =
                serde_json::from_str::<serde_json::Value>(&contents).map_err(io::Error::other)?;
            let providers = auth
                .as_object()
                .into_iter()
                .flat_map(|providers| providers.iter())
                .filter_map(|(provider_id, info)| {
                    let info = info.as_object()?;
                    if info.get("type").and_then(serde_json::Value::as_str) != Some("oauth") {
                        return None;
                    }
                    let access_token = info.get("access").and_then(serde_json::Value::as_str)?;
                    let refresh_token = info.get("refresh").and_then(serde_json::Value::as_str)?;
                    if access_token.trim().is_empty() || refresh_token.trim().is_empty() {
                        return None;
                    }
                    Some(OpencodeOauthAuth {
                        provider_id: provider_id.clone(),
                        access_token: access_token.to_string(),
                        refresh_token: refresh_token.to_string(),
                        account_id: info
                            .get("accountId")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect();
            Ok(providers)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

fn read_opencode_recent_model(path: &Path) -> io::Result<Option<OpencodeRecentModel>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let model_state =
                serde_json::from_str::<serde_json::Value>(&contents).map_err(io::Error::other)?;
            let recent = model_state
                .get("recent")
                .and_then(serde_json::Value::as_array)
                .and_then(|recent| recent.first())
                .and_then(serde_json::Value::as_object);
            Ok(recent.and_then(|recent| {
                let provider_id = recent
                    .get("providerID")
                    .and_then(serde_json::Value::as_str)?;
                let model_id = recent.get("modelID").and_then(serde_json::Value::as_str)?;
                Some(OpencodeRecentModel {
                    provider_id: provider_id.to_string(),
                    model_id: model_id.to_string(),
                })
            }))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn ensure_table<'a>(root: &'a mut TomlTable, key: &str) -> &'a mut TomlTable {
    if !root.contains_key(key) {
        root.insert(key.to_string(), TomlValue::Table(TomlTable::new()));
    }

    match root.get_mut(key).and_then(TomlValue::as_table_mut) {
        Some(table) => table,
        None => panic!("existing key should remain a TOML table"),
    }
}

fn get_table<'a>(root: &'a TomlTable, key: &str) -> Option<&'a TomlTable> {
    root.get(key).and_then(TomlValue::as_table)
}

fn get_string<'a>(root: &'a TomlTable, key: &str) -> Option<&'a str> {
    root.get(key).and_then(TomlValue::as_str)
}

fn default_codex_home() -> Option<PathBuf> {
    home_dir().map(|home_dir| home_dir.join(CODEX_HOME_DIR))
}

fn default_opencode_home() -> Option<PathBuf> {
    home_dir().map(|home_dir| home_dir.join(OPENCODE_HOME_DIR))
}

fn default_opencode_state_dir() -> Option<PathBuf> {
    home_dir().map(|home_dir| home_dir.join(OPENCODE_STATE_DIR))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

fn same_path(left: &Path, right: &Path) -> bool {
    path_or_self(left) == path_or_self(right)
}

fn path_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn imports_env_backed_provider_entries() {
        let temp = TempDir::new().expect("temp dir");
        let snapshot = SystemImportSnapshot {
            codex_home: None,
            present_env_keys: HashSet::from([
                "OPENROUTER_API_KEY".to_string(),
                OPENAI_API_KEY_ENV_VAR.to_string(),
            ]),
            opencode_api_auth: Vec::new(),
            opencode_oauth_auth: Vec::new(),
            opencode_recent_model: None,
        };

        import_system_state_with_snapshot(temp.path(), &snapshot).expect("import env providers");

        let config = read_toml_table(&temp.path().join(CONFIG_TOML_FILE)).expect("read config");
        let providers = get_table(&config, MODEL_PROVIDERS_KEY).expect("model providers");
        assert_eq!(
            providers
                .get(OPENAI_API_KEY_PROVIDER_ID)
                .and_then(TomlValue::as_table)
                .and_then(|provider| get_string(provider, "env_key")),
            Some(OPENAI_API_KEY_ENV_VAR)
        );
        assert_eq!(
            providers
                .get("openrouter")
                .and_then(TomlValue::as_table)
                .and_then(|provider| get_string(provider, "env_key")),
            Some("OPENROUTER_API_KEY")
        );
    }

    #[test]
    fn imports_codex_defaults_and_auth_mode() {
        let temp_home = TempDir::new().expect("interpreter temp dir");
        let codex_home = temp_home.path().join(CODEX_HOME_DIR);
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        std::fs::write(
            codex_home.join(CONFIG_TOML_FILE),
            r#"
model_provider = "openai"
model = "gpt-5.4"
model_reasoning_effort = "high"

[model_providers.groq]
name = "Groq"
base_url = "https://api.groq.com/openai/v1"
env_key = "GROQ_API_KEY"
"#,
        )
        .expect("write codex config");
        std::fs::write(
            codex_home.join(AUTH_JSON_FILE),
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"secret"}"#,
        )
        .expect("write codex auth");

        let snapshot = SystemImportSnapshot {
            codex_home: Some(codex_home),
            present_env_keys: HashSet::new(),
            opencode_api_auth: Vec::new(),
            opencode_oauth_auth: Vec::new(),
            opencode_recent_model: None,
        };
        import_system_state_with_snapshot(&temp_home.path().join(".openinterpreter"), &snapshot)
            .expect("import codex");

        let interpreter_config = read_toml_table(
            &temp_home
                .path()
                .join(".openinterpreter")
                .join(CONFIG_TOML_FILE),
        )
        .expect("read interpreter config");
        assert_eq!(
            get_string(&interpreter_config, MODEL_PROVIDER_KEY),
            Some(OPENAI_API_KEY_PROVIDER_ID)
        );
        assert_eq!(get_string(&interpreter_config, MODEL_KEY), Some("gpt-5.4"));
        assert_eq!(
            get_string(&interpreter_config, MODEL_REASONING_EFFORT_KEY),
            Some("high")
        );
        assert!(
            get_table(&interpreter_config, MODEL_PROVIDERS_KEY)
                .expect("providers table")
                .contains_key("groq")
        );
        assert!(
            temp_home
                .path()
                .join(".openinterpreter")
                .join(AUTH_JSON_FILE)
                .exists()
        );
    }

    #[test]
    fn imports_opencode_api_auth_and_recent_model() {
        let temp = TempDir::new().expect("temp dir");
        let snapshot = SystemImportSnapshot {
            codex_home: None,
            present_env_keys: HashSet::new(),
            opencode_api_auth: vec![
                OpencodeApiAuth {
                    provider_id: "groq".to_string(),
                    api_key: "gsk-test".to_string(),
                },
                OpencodeApiAuth {
                    provider_id: OPENAI_PROVIDER_ID.to_string(),
                    api_key: "sk-opencode".to_string(),
                },
            ],
            opencode_oauth_auth: vec![OpencodeOauthAuth {
                provider_id: "github-copilot".to_string(),
                access_token: "gho_test".to_string(),
                refresh_token: "ghr_test".to_string(),
                account_id: None,
            }],
            opencode_recent_model: Some(OpencodeRecentModel {
                provider_id: "groq".to_string(),
                model_id: "qwen/qwen3-32b".to_string(),
            }),
        };

        import_system_state_with_snapshot(temp.path(), &snapshot).expect("import opencode");

        let config = read_toml_table(&temp.path().join(CONFIG_TOML_FILE)).expect("read config");
        let providers = get_table(&config, MODEL_PROVIDERS_KEY).expect("model providers");
        assert_eq!(get_string(&config, MODEL_PROVIDER_KEY), Some("groq"));
        assert_eq!(get_string(&config, MODEL_KEY), Some("qwen/qwen3-32b"));
        assert_eq!(
            providers
                .get("groq")
                .and_then(TomlValue::as_table)
                .and_then(|provider| get_string(provider, "experimental_bearer_token")),
            Some("gsk-test")
        );
        assert_eq!(
            providers
                .get(OPENAI_API_KEY_PROVIDER_ID)
                .and_then(TomlValue::as_table)
                .and_then(|provider| get_string(provider, "experimental_bearer_token")),
            Some("sk-opencode")
        );
        assert_eq!(
            providers
                .get("github-copilot")
                .and_then(TomlValue::as_table)
                .and_then(|provider| get_string(provider, "experimental_bearer_token")),
            Some("gho_test")
        );
    }

    #[test]
    fn imports_opencode_openai_oauth_into_auth_file_and_defaults() {
        let temp = TempDir::new().expect("temp dir");
        let snapshot = SystemImportSnapshot {
            codex_home: None,
            present_env_keys: HashSet::new(),
            opencode_api_auth: Vec::new(),
            opencode_oauth_auth: vec![OpencodeOauthAuth {
                provider_id: OPENAI_PROVIDER_ID.to_string(),
                access_token: "header.payload.signature".to_string(),
                refresh_token: "refresh-token".to_string(),
                account_id: Some("acct_123".to_string()),
            }],
            opencode_recent_model: Some(OpencodeRecentModel {
                provider_id: OPENAI_PROVIDER_ID.to_string(),
                model_id: "gpt-5.4".to_string(),
            }),
        };

        import_system_state_with_snapshot(temp.path(), &snapshot)
            .expect("import opencode openai oauth");

        let config = read_toml_table(&temp.path().join(CONFIG_TOML_FILE)).expect("read config");
        assert_eq!(
            get_string(&config, MODEL_PROVIDER_KEY),
            Some(OPENAI_PROVIDER_ID)
        );
        assert_eq!(get_string(&config, MODEL_KEY), Some("gpt-5.4"));

        let auth = std::fs::read_to_string(temp.path().join(AUTH_JSON_FILE)).expect("read auth");
        let auth = serde_json::from_str::<serde_json::Value>(&auth).expect("parse auth json");
        assert_eq!(
            auth.get("auth_mode").and_then(serde_json::Value::as_str),
            Some("chatgpt")
        );
        assert_eq!(
            auth.get("tokens")
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(serde_json::Value::as_str),
            Some("header.payload.signature")
        );
        assert_eq!(
            auth.get("tokens")
                .and_then(|tokens| tokens.get("refresh_token"))
                .and_then(serde_json::Value::as_str),
            Some("refresh-token")
        );
        assert_eq!(
            auth.get("tokens")
                .and_then(|tokens| tokens.get("account_id"))
                .and_then(serde_json::Value::as_str),
            Some("acct_123")
        );
    }
}
