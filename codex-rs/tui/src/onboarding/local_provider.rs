use std::io;
use std::process::Stdio;
use std::time::Duration;

use codex_model_provider_info::DEFAULT_LMSTUDIO_PORT;
use codex_model_provider_info::DEFAULT_OLLAMA_PORT;
use codex_model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use codex_model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use tokio::process::Command;

pub(crate) fn is_local_provider(provider_id: &str) -> bool {
    matches!(
        provider_id,
        LMSTUDIO_OSS_PROVIDER_ID | OLLAMA_OSS_PROVIDER_ID
    )
}

pub(crate) fn can_start_local_provider(provider_id: &str) -> bool {
    provider_id == OLLAMA_OSS_PROVIDER_ID
}

pub(crate) fn not_running_message(provider_name: &str, provider_id: &str) -> Option<String> {
    match provider_id {
        OLLAMA_OSS_PROVIDER_ID => Some(format!(
            "{provider_name} is not running on localhost:{DEFAULT_OLLAMA_PORT}."
        )),
        LMSTUDIO_OSS_PROVIDER_ID => Some(format!(
            "{provider_name} is not running on localhost:{DEFAULT_LMSTUDIO_PORT}."
        )),
        _ => None,
    }
}

pub(crate) fn start_hint(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        OLLAMA_OSS_PROVIDER_ID => Some("Press S to start `ollama serve`."),
        LMSTUDIO_OSS_PROVIDER_ID => Some("Start the local server in LM Studio, then try again."),
        _ => None,
    }
}

pub(crate) fn no_models_message(provider_name: &str, provider_id: &str) -> Option<String> {
    match provider_id {
        OLLAMA_OSS_PROVIDER_ID | LMSTUDIO_OSS_PROVIDER_ID => Some(format!(
            "{provider_name} did not report any local models. Start or load one, or enter a model name manually."
        )),
        _ => None,
    }
}

pub(crate) async fn is_local_provider_running(provider_id: &str) -> io::Result<bool> {
    let port = match provider_id {
        OLLAMA_OSS_PROVIDER_ID => DEFAULT_OLLAMA_PORT,
        LMSTUDIO_OSS_PROVIDER_ID => DEFAULT_LMSTUDIO_PORT,
        _ => return Ok(false),
    };

    match tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    {
        Ok(Ok(_stream)) => Ok(true),
        Ok(Err(_)) | Err(_) => Ok(false),
    }
}

pub(crate) async fn start_local_provider(provider_id: &str) -> io::Result<()> {
    match provider_id {
        OLLAMA_OSS_PROVIDER_ID => {
            let mut command = Command::new("ollama");
            command
                .arg("serve")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command.spawn()?;
            Ok(())
        }
        _ => Err(io::Error::other(
            "provider cannot be started from onboarding",
        )),
    }
}

pub(crate) async fn wait_for_local_provider_running(
    provider_id: &str,
    timeout: Duration,
) -> io::Result<bool> {
    let start = tokio::time::Instant::now();
    loop {
        if is_local_provider_running(provider_id).await? {
            return Ok(true);
        }
        if start.elapsed() >= timeout {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
