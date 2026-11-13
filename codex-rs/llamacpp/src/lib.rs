//! Bundled llama.cpp local provider support.

use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io::IsTerminal as _;
use std::io::Read as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use codex_core::config::Config;
use codex_model_provider_info::DEFAULT_LOCAL_PORT;
use reqwest::StatusCode;
use sha2::Digest as _;
use tokio::io::AsyncWriteExt;

const STARTUP_LOCK_FILENAME: &str = ".local-startup.lock";

pub const DEFAULT_OSS_MODEL: &str =
    "yuxinlu1/gemma-4-12B-agentic-fable5-composer2.5-v2-3.5x-tau2-GGUF:Q4_K_M";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub repo: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub expected_bytes: u64,
    pub sha256: &'static str,
    pub size_label: &'static str,
    pub context_window: i64,
    pub hf_token_required: bool,
}

pub const LOCAL_MODEL_SPECS: &[LocalModelSpec] = &[
    LocalModelSpec {
        id: DEFAULT_OSS_MODEL,
        display_name: "Gemma 4 12B Agentic Fable5 Composer",
        description: "Best quality Local default • Public HF download • llama.cpp tool calling",
        repo: "yuxinlu1/gemma-4-12B-agentic-fable5-composer2.5-v2-3.5x-tau2-GGUF",
        filename: "gemma4-v2-Q4_K_M.gguf",
        url: "https://huggingface.co/yuxinlu1/gemma-4-12B-agentic-fable5-composer2.5-v2-3.5x-tau2-GGUF/resolve/190a31365a6b80a692349be34ccdac730cad4fe4/gemma4-v2-Q4_K_M.gguf",
        expected_bytes: 7381381664,
        sha256: "0b9506cab36f7f818e34f9c0f5a3d6568d0b37100f3a3e1092e2eec3c4c96791",
        size_label: "about 7.4 GB",
        context_window: 32768,
        hf_token_required: false,
    },
    LocalModelSpec {
        id: "Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF:Q4_K_M",
        display_name: "Qwen2.5 Coder 0.5B",
        description: "Tiny coding baseline • Public HF download • experimental",
        repo: "Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF",
        filename: "qwen2.5-coder-0.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF/resolve/ebb2015119c907b064c512bf053e945850b5875f/qwen2.5-coder-0.5b-instruct-q4_k_m.gguf",
        expected_bytes: 491400064,
        sha256: "1d9614638d18024d0fbb36575a15f1302a3adf044df10345688ec4f6e1c4ff32",
        size_label: "about 491 MB",
        context_window: 32768,
        hf_token_required: false,
    },
    LocalModelSpec {
        id: "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF:Q4_K_M",
        display_name: "Qwen2.5 Coder 1.5B",
        description: "Small coding model • Public HF download • experimental",
        repo: "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF",
        filename: "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF/resolve/f86cb2c1fa58255f8052cc32aeede1b7482d4361/qwen2.5-coder-1.5b-instruct-q4_k_m.gguf",
        expected_bytes: 1117320768,
        sha256: "cc324af070c2ecbfd324a30884d2f951a7ff756aba85cb811a6ec436933bb046",
        size_label: "about 1.1 GB",
        context_window: 32768,
        hf_token_required: false,
    },
    LocalModelSpec {
        id: "Qwen/Qwen2.5-Coder-3B-Instruct-GGUF:Q4_K_M",
        display_name: "Qwen2.5 Coder 3B",
        description: "Small coding model • Public HF download • experimental",
        repo: "Qwen/Qwen2.5-Coder-3B-Instruct-GGUF",
        filename: "qwen2.5-coder-3b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-Coder-3B-Instruct-GGUF/resolve/f74adce6aa16316c625447af059dbebe4983757c/qwen2.5-coder-3b-instruct-q4_k_m.gguf",
        expected_bytes: 2104932800,
        sha256: "724fb256bec1ff062b2f65e4569e871ad2e95ab2a3989723d1769c54294730b7",
        size_label: "about 2.1 GB",
        context_window: 32768,
        hf_token_required: false,
    },
    LocalModelSpec {
        id: "unsloth/Qwen3-1.7B-GGUF:Q4_K_M",
        display_name: "Qwen3 1.7B",
        description: "Small reasoning/tool model • Public HF download • experimental",
        repo: "unsloth/Qwen3-1.7B-GGUF",
        filename: "Qwen3-1.7B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/d7f544eead698dbd1f15126ef60b45a1e1933222/Qwen3-1.7B-Q4_K_M.gguf",
        expected_bytes: 1107409472,
        sha256: "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
        size_label: "about 1.1 GB",
        context_window: 40960,
        hf_token_required: false,
    },
];

pub async fn ensure_oss_ready(config: &Config) -> Result<(), String> {
    let base_url = config
        .model_provider
        .base_url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{DEFAULT_LOCAL_PORT}/v1"));
    validate_bundled_base_url(&base_url)?;
    let model = local_model_spec_for_config(config)?;
    if check_running_server_model(&base_url, model).await? {
        return Ok(());
    }

    let _startup_lock = acquire_startup_lock().await?;
    if check_running_server_model(&base_url, model).await? {
        return Ok(());
    }

    let model_path = ensure_model_downloaded(model, false).await?;
    let llama_server = find_llama_server()?;
    spawn_llama_server(&llama_server, &model_path, model)?;
    wait_for_local_server(&base_url, model).await
}

fn validate_bundled_base_url(base_url: &str) -> Result<(), String> {
    let expected = format!("http://127.0.0.1:{DEFAULT_LOCAL_PORT}/v1");
    if base_url.trim_end_matches('/') == expected {
        Ok(())
    } else {
        Err(format!(
            "bundled Local requires {expected}; configure a separate provider for another endpoint"
        ))
    }
}

pub fn local_model_spec(model: &str) -> Option<&'static LocalModelSpec> {
    LOCAL_MODEL_SPECS.iter().find(|spec| {
        spec.id == model
            || spec
                .id
                .strip_suffix(":Q4_K_M")
                .is_some_and(|without_quant| without_quant == model)
    })
}

pub fn local_model_description(model: &str) -> Option<String> {
    let spec = local_model_spec(model)?;
    let auth = if spec.hf_token_required {
        "Requires HF_TOKEN"
    } else {
        "No HF token required"
    };
    Some(format!(
        "{} • {} • {}",
        spec.description, spec.size_label, auth
    ))
}

/// Explicit, headless-safe acquisition for the app-server configuration path.
/// Callers must obtain the user's download consent before invoking this method.
pub async fn download_local_model(model_id: &str) -> Result<PathBuf, String> {
    let model = local_model_spec(model_id)
        .ok_or_else(|| format!("unsupported Local model `{model_id}`"))?;
    let _startup_lock = acquire_startup_lock().await?;
    ensure_model_downloaded(model, true).await
}

pub async fn local_model_cached(model_id: &str) -> Result<bool, String> {
    let model = local_model_spec(model_id)
        .ok_or_else(|| format!("unsupported Local model `{model_id}`"))?;
    let path = model_cache_dir(model).join(model.filename);
    if !path.exists() {
        return Ok(false);
    }
    cached_model_is_usable(&path, model.expected_bytes, model.sha256).await
}

async fn cached_model_is_usable(
    path: &PathBuf,
    expected_bytes: u64,
    expected_sha256: &'static str,
) -> Result<bool, String> {
    let size = tokio::fs::metadata(&path)
        .await
        .map_err(|err| format!("failed to inspect cached model {}: {err}", path.display()))?
        .len();
    if size != expected_bytes || validate_gguf_header(path).is_err() {
        // Keep a damaged download for diagnosis; acquisition reports the exact
        // failure, while the catalog must not mark it as usable.
        return Ok(false);
    }
    Ok(validate_model_digest(path, expected_sha256).await.is_ok())
}

fn local_model_spec_for_config(config: &Config) -> Result<&'static LocalModelSpec, String> {
    let model = config.model.as_deref().unwrap_or(DEFAULT_OSS_MODEL);
    local_model_spec(model).ok_or_else(|| {
        let supported = LOCAL_MODEL_SPECS
            .iter()
            .map(|spec| spec.id)
            .collect::<Vec<_>>()
            .join(", ");
        format!("unsupported Local model `{model}`. Supported Local models: {supported}")
    })
}

#[derive(Debug, Eq, PartialEq)]
enum LocalServerState {
    Ready,
    DifferentModel(String),
    Unreachable,
}

async fn check_running_server_model(
    base_url: &str,
    model: &LocalModelSpec,
) -> Result<bool, String> {
    match local_server_state(base_url, model).await {
        LocalServerState::Ready => Ok(true),
        LocalServerState::DifferentModel(loaded) => Err(format!(
            "Local llama.cpp is already running on this port with a different model ({loaded}). Stop that server or choose the loaded model before starting `{}`.",
            model.id
        )),
        LocalServerState::Unreachable => Ok(false),
    }
}

async fn local_server_state(base_url: &str, model: &LocalModelSpec) -> LocalServerState {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let response = match reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response,
        _ => return LocalServerState::Unreachable,
    };

    let body = match response.json::<serde_json::Value>().await {
        Ok(body) => body,
        Err(_) => return LocalServerState::Unreachable,
    };
    classify_loaded_model(&body, model)
}

fn classify_loaded_model(body: &serde_json::Value, model: &LocalModelSpec) -> LocalServerState {
    let loaded = body
        .get("data")
        .and_then(|data| data.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if loaded
        .iter()
        .any(|id| id == model.id || id.ends_with(model.filename))
    {
        LocalServerState::Ready
    } else if loaded.is_empty() {
        LocalServerState::Unreachable
    } else {
        LocalServerState::DifferentModel(loaded.join(", "))
    }
}

async fn wait_for_local_server(base_url: &str, model: &LocalModelSpec) -> Result<(), String> {
    for _ in 0..60 {
        match local_server_state(base_url, model).await {
            LocalServerState::Ready => return Ok(()),
            LocalServerState::DifferentModel(loaded) => {
                return Err(format!(
                    "bundled llama.cpp started with a different model than expected ({loaded})"
                ));
            }
            LocalServerState::Unreachable => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("timed out waiting for bundled llama.cpp server to start".to_string())
}

async fn ensure_model_downloaded(
    model: &LocalModelSpec,
    confirmed: bool,
) -> Result<PathBuf, String> {
    let path = model_cache_dir(model).join(model.filename);
    if path.exists() {
        let size = tokio::fs::metadata(&path)
            .await
            .map_err(|err| format!("failed to inspect cached model {}: {err}", path.display()))?
            .len();
        if size != model.expected_bytes {
            return Err(format!(
                "cached model {} has {size} bytes, expected {}; preserve or remove it before retrying",
                path.display(),
                model.expected_bytes
            ));
        }
        validate_gguf_header(&path)?;
        validate_model_digest(&path, model.sha256).await?;
        return Ok(path);
    }

    if !confirmed && !confirm_download(model, &path)? {
        return Err("local model download cancelled".to_string());
    }

    let partial_path = path.with_extension("gguf.partial");
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| format!("failed to create local model cache: {err}"))?;
    }

    let mut request = reqwest::Client::new().get(model.url);
    if let Some(token) = hugging_face_token() {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|err| format!("failed to start local model download: {err}"))?;
    if response.status() != StatusCode::OK {
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(format!(
                "Hugging Face denied the Local model download for `{}` (HTTP {}). Set HF_TOKEN to a token with access to {} and try again.",
                model.id,
                response.status(),
                model.repo
            ));
        }
        return Err(format!(
            "failed to download local model: HTTP {}",
            response.status()
        ));
    }
    if let Some(length) = response.content_length()
        && length != model.expected_bytes
    {
        return Err(format!(
            "GGUF response size {length} does not match the pinned {} bytes for `{}`",
            model.expected_bytes, model.id
        ));
    }

    let mut file = tokio::fs::File::create(&partial_path)
        .await
        .map_err(|err| format!("failed to create local model file: {err}"))?;
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("failed while downloading local model: {err}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|err| format!("failed to write local model file: {err}"))?;
    }
    file.flush()
        .await
        .map_err(|err| format!("failed to finish local model file: {err}"))?;
    let size = file
        .metadata()
        .await
        .map_err(|err| format!("failed to inspect downloaded model: {err}"))?
        .len();
    if size != model.expected_bytes {
        return Err(format!(
            "downloaded model has {size} bytes, expected {}; partial file retained at {}",
            model.expected_bytes,
            partial_path.display()
        ));
    }
    drop(file);
    validate_gguf_header(&partial_path)?;
    validate_model_digest(&partial_path, model.sha256).await?;
    tokio::fs::rename(&partial_path, &path)
        .await
        .map_err(|err| format!("failed to install local model file: {err}"))?;
    Ok(path)
}

fn validate_gguf_header(path: &PathBuf) -> Result<(), String> {
    let mut file = File::open(path)
        .map_err(|err| format!("failed to open GGUF model {}: {err}", path.display()))?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)
        .map_err(|err| format!("failed to read GGUF header {}: {err}", path.display()))?;
    if magic != *b"GGUF" {
        return Err(format!("model {} is not a GGUF file", path.display()));
    }
    Ok(())
}

async fn validate_model_digest(path: &PathBuf, expected: &'static str) -> Result<(), String> {
    let path = path.clone();
    tokio::task::spawn_blocking(move || {
        let mut file = File::open(&path)
            .map_err(|err| format!("failed to open GGUF {} for checksum: {err}", path.display()))?;
        let mut digest = sha2::Sha256::new();
        let mut buffer = [0_u8; 1024 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|err| format!("failed to checksum GGUF {}: {err}", path.display()))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != expected {
            return Err(format!(
                "GGUF checksum mismatch for {}; preserve or remove it before retrying",
                path.display()
            ));
        }
        Ok(())
    })
    .await
    .map_err(|err| format!("GGUF checksum task failed: {err}"))?
}

fn confirm_download(model: &LocalModelSpec, path: &PathBuf) -> Result<bool, String> {
    if !std::io::stdin().is_terminal() {
        return Err(format!(
            "local model `{}` is not downloaded at {}. Call interpreter/model/acquire before starting a headless turn.",
            model.id,
            path.display()
        ));
    }
    let auth = if model.hf_token_required {
        "Hugging Face token: required. Set HF_TOKEN before downloading."
    } else {
        "Hugging Face token: not required for this model. If Hugging Face denies access, set HF_TOKEN and retry."
    };
    eprintln!(
        "Local model not found.\nModel: {}\nSize: {}\n{}\nDestination: {}\nDownload now? [y/N] ",
        model.id,
        model.size_label,
        auth,
        path.display()
    );
    std::io::stderr()
        .flush()
        .map_err(|err| format!("failed to prompt for local model download: {err}"))?;
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|err| format!("failed to read local model download response: {err}"))?;
    Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES"))
}

fn hugging_face_token() -> Option<String> {
    ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

struct StartupLock {
    _file: File,
}

async fn acquire_startup_lock() -> Result<StartupLock, String> {
    let lock_dir = local_model_root();
    tokio::fs::create_dir_all(&lock_dir)
        .await
        .map_err(|err| format!("failed to create local model cache: {err}"))?;
    let lock_path = lock_dir.join(STARTUP_LOCK_FILENAME);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|err| {
            format!(
                "failed to open local provider startup lock {}: {err}",
                lock_path.display()
            )
        })?;

    for _ in 0..180 {
        match file.try_lock() {
            Ok(()) => return Ok(StartupLock { _file: file }),
            Err(TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(err) => {
                return Err(format!(
                    "failed to lock local provider startup lock {}: {err}",
                    lock_path.display()
                ));
            }
        }
    }

    Err(format!(
        "timed out waiting for local provider startup lock {}",
        lock_path.display()
    ))
}

fn local_model_root() -> PathBuf {
    std::env::var_os("INTERPRETER_MODEL_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("INTERPRETER_HOME")
                .map(PathBuf::from)
                .map(|path| path.join("models"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".open-interpreter").join("models"))
        })
        .unwrap_or_else(|| PathBuf::from(".open-interpreter").join("models"))
        .join("huggingface")
}

fn model_cache_dir(model: &LocalModelSpec) -> PathBuf {
    local_model_root().join(model.repo)
}

fn find_llama_server() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("INTERPRETER_LLAMA_SERVER").map(PathBuf::from)
        && path.exists()
    {
        return Ok(path);
    }
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let resource_dir = dir.join("..").join("codex-resources").join("llama.cpp");
        for candidate in llama_server_names()
            .iter()
            .flat_map(|name| [dir.join(name), resource_dir.join(name)])
            .filter(|path| path.exists())
        {
            return Ok(candidate);
        }
    }
    Err("bundled llama-server binary was not found".to_string())
}

fn llama_server_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["llama-server.exe"]
    } else {
        &["llama-server", "llama-server-metal"]
    }
}

fn spawn_llama_server(
    llama_server: &PathBuf,
    model_path: &PathBuf,
    model: &LocalModelSpec,
) -> Result<(), String> {
    llama_server_command(llama_server, model_path, model)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("failed to start bundled llama.cpp server: {err}"))
}

fn llama_server_command(
    llama_server: &PathBuf,
    model_path: &PathBuf,
    model: &LocalModelSpec,
) -> Command {
    let mut command = Command::new(llama_server);
    command
        .arg("-m")
        .arg(model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(DEFAULT_LOCAL_PORT.to_string())
        .arg("--jinja")
        .arg("--parallel")
        .arg("1")
        .arg("--ctx-size")
        .arg(model.context_window.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn cached_model_requires_exact_header_length_and_digest() {
        let path = std::env::temp_dir().join(format!(
            "codex-llamacpp-cache-validation-{}-{}.gguf",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let valid = b"GGUFtest";
        let digest = "30be5a60b7786b878803d49ff1dcaa157e9c098b896ff8bca692104fa9df3ba0";
        tokio::fs::write(&path, valid).await.unwrap();
        assert!(cached_model_is_usable(&path, 8, digest).await.unwrap());
        tokio::fs::write(&path, b"GGUFfail").await.unwrap();
        assert!(!cached_model_is_usable(&path, 8, digest).await.unwrap());
        tokio::fs::write(&path, b"BAD!test").await.unwrap();
        assert!(!cached_model_is_usable(&path, 8, digest).await.unwrap());
        assert!(!cached_model_is_usable(&path, 9, digest).await.unwrap());
        tokio::fs::remove_file(&path).await.unwrap();
    }

    #[test]
    fn loaded_server_is_reused_without_starting_another_process() {
        let model = &LOCAL_MODEL_SPECS[1];
        let body = json!({"data": [{"id": format!("/models/{}", model.filename)}]});
        assert_eq!(classify_loaded_model(&body, model), LocalServerState::Ready);
    }

    #[test]
    fn different_model_does_not_silently_use_the_occupied_port() {
        let model = &LOCAL_MODEL_SPECS[1];
        let body = json!({"data": [{"id": "another-model.gguf"}]});
        assert_eq!(
            classify_loaded_model(&body, model),
            LocalServerState::DifferentModel("another-model.gguf".to_string())
        );
    }

    #[test]
    fn empty_model_list_is_not_ready() {
        assert_eq!(
            classify_loaded_model(&json!({"data": []}), &LOCAL_MODEL_SPECS[0]),
            LocalServerState::Unreachable
        );
    }

    #[test]
    fn bundled_provider_rejects_nonlocal_or_wrong_port_endpoints() {
        assert!(validate_bundled_base_url("http://127.0.0.1:17654/v1").is_ok());
        assert!(validate_bundled_base_url("https://example.com/v1").is_err());
        assert!(validate_bundled_base_url("http://127.0.0.1:11434/v1").is_err());
    }

    #[test]
    fn download_catalog_uses_exact_revision_and_digest() {
        for model in LOCAL_MODEL_SPECS {
            assert!(model.expected_bytes > 1_000_000);
            assert_eq!(model.sha256.len(), 64);
            assert!(model.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()));
            let path = model.url.strip_prefix("https://huggingface.co/").unwrap();
            let revision_and_file = path.strip_prefix(model.repo).unwrap();
            let (revision, filename) = revision_and_file
                .strip_prefix("/resolve/")
                .unwrap()
                .split_once('/')
                .unwrap();
            assert_eq!(revision.len(), 40);
            assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_eq!(filename, model.filename);
        }
    }

    #[test]
    fn server_context_matches_selected_model_catalog() {
        let model = &LOCAL_MODEL_SPECS[4];
        let command = llama_server_command(
            &PathBuf::from("llama-server"),
            &PathBuf::from("model.gguf"),
            model,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        assert_eq!(args.last().map(|arg| arg.as_ref()), Some("40960"));
    }
}
