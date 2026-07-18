use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thiserror::Error;

pub const KIMI_CODE_PROVIDER_ID: &str = "kimi-for-coding";

const DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";
const KIMI_CODE_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_CODE_DEVICE_ID_FILE: &str = "device_id";
const KIMI_CODE_CREDENTIALS_DIR: &str = "credentials";
const KIMI_CODE_CREDENTIALS_FILE: &str = "kimi-code.json";
const MIN_REFRESH_THRESHOLD_SECS: u64 = 300;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct KimiCodeDeviceAuthorization {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: Option<u64>,
    pub interval_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct KimiCodeToken {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    scope: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Debug, Error)]
pub enum KimiCodeAuthError {
    #[error("Kimi Code browser login is unavailable: missing saved credentials")]
    MissingCredentials,
    #[error("Kimi Code browser login is required again")]
    ReauthenticationRequired,
    #[error("failed to create Kimi Code credential directory: {0}")]
    CredentialsDir(#[source] io::Error),
    #[error("failed to read Kimi Code credentials: {0}")]
    CredentialsRead(#[source] io::Error),
    #[error("failed to write Kimi Code credentials: {0}")]
    CredentialsWrite(#[source] io::Error),
    #[error("failed to parse Kimi Code credentials: {0}")]
    CredentialsParse(#[source] serde_json::Error),
    #[error("failed to generate Kimi Code device id: {0}")]
    DeviceId(#[source] io::Error),
    #[error("Kimi Code device authorization failed: {0}")]
    DeviceAuthorization(String),
    #[error("Kimi Code token exchange failed: {0}")]
    TokenExchange(String),
    #[error("Kimi Code token refresh failed: {0}")]
    TokenRefresh(String),
    #[error("failed to open browser for Kimi Code login: {0}")]
    Browser(#[source] io::Error),
    #[error("network error during Kimi Code login: {0}")]
    Network(#[from] reqwest::Error),
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorizationResponse {
    user_code: String,
    device_code: String,
    verification_uri: Option<String>,
    verification_uri_complete: String,
    expires_in: Option<u64>,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    scope: Option<String>,
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: Option<String>,
    error_description: Option<String>,
}

pub async fn request_device_authorization(
    codex_home: &Path,
) -> Result<KimiCodeDeviceAuthorization, KimiCodeAuthError> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "{}/api/oauth/device_authorization",
            oauth_host().trim_end_matches('/')
        ))
        .headers(common_headers(codex_home)?)
        .form(&[("client_id", KIMI_CODE_CLIENT_ID)])
        .send()
        .await?;
    let status = response.status();
    let body = response.bytes().await?;
    if !status.is_success() {
        return Err(KimiCodeAuthError::DeviceAuthorization(format_oauth_error(
            &body,
            status.as_u16(),
        )));
    }
    let payload: DeviceAuthorizationResponse =
        serde_json::from_slice(&body).map_err(KimiCodeAuthError::CredentialsParse)?;
    Ok(KimiCodeDeviceAuthorization {
        user_code: payload.user_code,
        device_code: payload.device_code,
        verification_uri: payload.verification_uri.unwrap_or_default(),
        verification_uri_complete: payload.verification_uri_complete,
        expires_in: payload.expires_in,
        interval_seconds: payload.interval.unwrap_or(5).max(1),
    })
}

pub fn open_verification_url(verification_url: &str) -> Result<(), KimiCodeAuthError> {
    webbrowser::open(verification_url).map_err(KimiCodeAuthError::Browser)?;
    Ok(())
}

pub async fn complete_device_authorization(
    codex_home: &Path,
    authorization: &KimiCodeDeviceAuthorization,
) -> Result<(), KimiCodeAuthError> {
    let client = reqwest::Client::new();
    loop {
        let response = client
            .post(format!(
                "{}/api/oauth/token",
                oauth_host().trim_end_matches('/')
            ))
            .headers(common_headers(codex_home)?)
            .form(&[
                ("client_id", KIMI_CODE_CLIENT_ID),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", authorization.device_code.as_str()),
            ])
            .send()
            .await?;
        let status = response.status();
        let body = response.bytes().await?;
        if status.is_success() {
            let token: TokenResponse =
                serde_json::from_slice(&body).map_err(KimiCodeAuthError::CredentialsParse)?;
            save_token(codex_home, token_to_stored(token))?;
            return Ok(());
        }

        let error =
            serde_json::from_slice::<OAuthErrorResponse>(&body).unwrap_or(OAuthErrorResponse {
                error: None,
                error_description: None,
            });
        if matches!(error.error.as_deref(), Some("expired_token")) {
            return Err(KimiCodeAuthError::ReauthenticationRequired);
        }
        tokio::time::sleep(Duration::from_secs(authorization.interval_seconds)).await;
    }
}

pub async fn resolve_access_token(codex_home: &Path) -> Result<String, KimiCodeAuthError> {
    let mut token = load_token(codex_home)?;
    if token.access_token.trim().is_empty() || token.refresh_token.trim().is_empty() {
        return Err(KimiCodeAuthError::ReauthenticationRequired);
    }
    if token_needs_refresh(&token) {
        token = refresh_token(codex_home, &token.refresh_token).await?;
        save_token(codex_home, token.clone())?;
    }
    Ok(token.access_token)
}

pub async fn ensure_access_token(
    codex_home: &Path,
    open_browser: bool,
) -> Result<String, KimiCodeAuthError> {
    match resolve_access_token(codex_home).await {
        Ok(access_token) => Ok(access_token),
        Err(
            KimiCodeAuthError::MissingCredentials | KimiCodeAuthError::ReauthenticationRequired,
        ) => {
            let authorization = request_device_authorization(codex_home).await?;
            if open_browser {
                let _ = open_verification_url(&authorization.verification_uri_complete);
            }
            complete_device_authorization(codex_home, &authorization).await?;
            resolve_access_token(codex_home).await
        }
        Err(err) => Err(err),
    }
}

fn oauth_host() -> String {
    std::env::var("KIMI_CODE_OAUTH_HOST")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("KIMI_OAUTH_HOST")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_OAUTH_HOST.to_string())
}

fn credentials_dir(codex_home: &Path) -> PathBuf {
    codex_home.join(KIMI_CODE_CREDENTIALS_DIR)
}

fn credentials_path(codex_home: &Path) -> PathBuf {
    credentials_dir(codex_home).join(KIMI_CODE_CREDENTIALS_FILE)
}

fn device_id_path(codex_home: &Path) -> PathBuf {
    codex_home.join(KIMI_CODE_DEVICE_ID_FILE)
}

fn load_token(codex_home: &Path) -> Result<KimiCodeToken, KimiCodeAuthError> {
    let path = credentials_path(codex_home);
    let raw = fs::read_to_string(path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            KimiCodeAuthError::MissingCredentials
        } else {
            KimiCodeAuthError::CredentialsRead(err)
        }
    })?;
    serde_json::from_str(&raw).map_err(KimiCodeAuthError::CredentialsParse)
}

fn save_token(codex_home: &Path, token: KimiCodeToken) -> Result<(), KimiCodeAuthError> {
    create_credentials_dir(&credentials_dir(codex_home))
        .map_err(KimiCodeAuthError::CredentialsDir)?;
    let path = credentials_path(codex_home);
    let temp_path = path.with_extension("json.tmp");
    let contents = serde_json::to_vec(&token).map_err(KimiCodeAuthError::CredentialsParse)?;
    write_private_file(&temp_path, &contents).map_err(KimiCodeAuthError::CredentialsWrite)?;
    fs::rename(temp_path, path).map_err(KimiCodeAuthError::CredentialsWrite)?;
    Ok(())
}

/// Creates `dir` and any missing parents, restricted to the current user
/// (`0o700`) on Unix. Mirrors the owner-only handling used for `auth.json`, and
/// also tightens a directory that an earlier version may have left world- or
/// group-readable.
fn create_credentials_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `DirBuilder` does not change the mode of a directory that already
        // exists, so repair it explicitly.
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes `contents` to `path`, creating the file readable and writable only by
/// the current user (`0o600`) on Unix. The Kimi credential file holds long-lived
/// access and refresh tokens in plaintext, so it must not be world-readable.
fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    // Remove any stale or pre-planted file at this path (a regular file left by
    // a crashed write, or a symlink) so the create below always makes a fresh
    // file with the mode we request and cannot be tricked into following a
    // symlink to another location.
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let mut options = fs::OpenOptions::new();
    // `create_new` fails closed if the path reappears between the remove above
    // and this open, rather than writing through whatever was planted there.
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Guarantee the exact mode regardless of the process umask.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(contents)?;
    file.flush()?;
    Ok(())
}

fn token_to_stored(token: TokenResponse) -> KimiCodeToken {
    let expires_at = current_timestamp_secs().saturating_add(token.expires_in);
    KimiCodeToken {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at,
        scope: token.scope.unwrap_or_default(),
        token_type: token.token_type.unwrap_or_else(|| "Bearer".to_string()),
        expires_in: token.expires_in,
    }
}

fn token_needs_refresh(token: &KimiCodeToken) -> bool {
    if token.expires_at <= current_timestamp_secs() {
        return true;
    }
    let lifetime_threshold = token.expires_in / 2;
    let refresh_threshold = lifetime_threshold.max(MIN_REFRESH_THRESHOLD_SECS);
    token.expires_at.saturating_sub(current_timestamp_secs()) <= refresh_threshold
}

async fn refresh_token(
    codex_home: &Path,
    refresh_token: &str,
) -> Result<KimiCodeToken, KimiCodeAuthError> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "{}/api/oauth/token",
            oauth_host().trim_end_matches('/')
        ))
        .headers(common_headers(codex_home)?)
        .form(&[
            ("client_id", KIMI_CODE_CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;
    let status = response.status();
    let body = response.bytes().await?;
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(KimiCodeAuthError::ReauthenticationRequired);
    }
    if !status.is_success() {
        return Err(KimiCodeAuthError::TokenRefresh(format_oauth_error(
            &body,
            status.as_u16(),
        )));
    }
    let token: TokenResponse =
        serde_json::from_slice(&body).map_err(KimiCodeAuthError::CredentialsParse)?;
    Ok(token_to_stored(token))
}

fn common_headers(codex_home: &Path) -> Result<HeaderMap, KimiCodeAuthError> {
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("X-Msh-Platform", "kimi_cli".to_string()),
        ("X-Msh-Version", env!("CARGO_PKG_VERSION").to_string()),
        ("X-Msh-Device-Name", device_name()),
        ("X-Msh-Device-Model", device_model()),
        ("X-Msh-Os-Version", os_info::get().version().to_string()),
        ("X-Msh-Device-Id", device_id(codex_home)?),
    ] {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|err| KimiCodeAuthError::TokenExchange(err.to_string()))?,
            HeaderValue::from_str(&ascii_header_value(&value))
                .map_err(|err| KimiCodeAuthError::TokenExchange(err.to_string()))?,
        );
    }
    Ok(headers)
}

fn device_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "open-interpreter".to_string())
}

fn device_model() -> String {
    let info = os_info::get();
    let os = info.os_type().to_string();
    let version = info.version().to_string();
    let arch = std::env::consts::ARCH;
    if version.is_empty() {
        format!("{os} {arch}")
    } else {
        format!("{os} {version} {arch}")
    }
}

fn ascii_header_value(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .filter(char::is_ascii)
        .collect::<String>()
        .trim()
        .to_string();
    if sanitized.is_empty() {
        sanitized = "unknown".to_string();
    }
    sanitized
}

fn device_id(codex_home: &Path) -> Result<String, KimiCodeAuthError> {
    let path = device_id_path(codex_home);
    if let Ok(device_id) = fs::read_to_string(&path) {
        let trimmed = device_id.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let device_id = generate_device_id();
    fs::write(path, &device_id).map_err(KimiCodeAuthError::DeviceId)?;
    Ok(device_id)
}

fn generate_device_id() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn current_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn format_oauth_error(body: &[u8], status: u16) -> String {
    if let Ok(error) = serde_json::from_slice::<OAuthErrorResponse>(body) {
        if let Some(description) = error.error_description.filter(|value| !value.is_empty()) {
            return description;
        }
        if let Some(code) = error.error.filter(|value| !value.is_empty()) {
            return format!("{code} (HTTP {status})");
        }
    }
    String::from_utf8_lossy(body).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn refresh_threshold_uses_half_lifetime_or_minimum_floor() {
        let token = KimiCodeToken {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: current_timestamp_secs().saturating_add(400),
            scope: String::new(),
            token_type: "Bearer".to_string(),
            expires_in: 600,
        };
        assert_eq!(token_needs_refresh(&token), false);

        let expiring = KimiCodeToken {
            expires_at: current_timestamp_secs().saturating_add(299),
            ..token
        };
        assert_eq!(token_needs_refresh(&expiring), true);
    }

    #[test]
    fn save_and_load_token_round_trip() {
        let temp_dir = tempdir().expect("tempdir");
        let token = KimiCodeToken {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: 123,
            scope: "scope".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 456,
        };
        save_token(temp_dir.path(), token.clone()).expect("save token");
        let loaded = load_token(temp_dir.path()).expect("load token");
        assert_eq!(loaded, token);
    }
}
