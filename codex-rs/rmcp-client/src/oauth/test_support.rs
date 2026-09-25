use std::path::Path;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::OnceLock;
use std::sync::PoisonError;

use tempfile::tempdir;

/// Build the actual client's Stop-policy route pool before measuring protocol deadlines.
/// Client construction loads platform/custom CA state and can be slow on developer hosts.
/// Use a separate local endpoint so warmup cannot affect the test server's request assertions;
/// the same production HTTP capability still applies all proxy and certificate policy.
pub(crate) async fn warm_http_client(
    client: &dyn codex_exec_server::HttpClient,
) -> anyhow::Result<()> {
    let server = wiremock::MockServer::start().await;
    client
        .http_request(codex_exec_server::HttpRequestParams {
            method: "GET".to_string(),
            url: server.uri(),
            headers: Vec::new(),
            body: None,
            timeout_ms: None,
            redirect_policy: codex_exec_server::HttpRedirectPolicy::Stop,
            request_id: "test-client-warmup".to_string(),
            stream_response: false,
        })
        .await?;
    Ok(())
}

/// Serializes tests that mutate process-wide CODEX_HOME.
///
/// Keep OAuth tests on this one guard instead of defining per-module helpers; otherwise
/// concurrently running test modules can point File/Secrets storage at different homes.
pub(crate) struct TempCodexHome {
    _guard: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
}

impl TempCodexHome {
    pub(crate) fn new() -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let dir = tempdir().expect("create CODEX_HOME temp dir");
        unsafe {
            std::env::set_var("CODEX_HOME", dir.path());
        }
        Self {
            _guard: guard,
            _dir: dir,
        }
    }

    pub(super) fn path(&self) -> &Path {
        self._dir.path()
    }
}

impl Drop for TempCodexHome {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("CODEX_HOME");
        }
    }
}
