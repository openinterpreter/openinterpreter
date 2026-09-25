//! Provider and authentication dimensions that partition model catalogs.

use super::*;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use codex_model_provider_info::WireApi;
use pretty_assertions::assert_eq;
use std::path::Path;
use tempfile::tempdir;

fn kimi_code_provider() -> ModelProviderInfo {
    ModelProviderInfo {
        name: "Kimi For Coding".to_string(),
        base_url: Some("https://api.kimi.com/coding/v1".to_string()),
        wire_api: WireApi::Chat,
        requires_openai_auth: false,
        ..Default::default()
    }
}

fn write_kimi_credentials(
    codex_home: &Path,
    access_token: &str,
    refresh_token: &str,
    expires_at: u64,
) {
    let credentials_dir = codex_home.join("credentials");
    std::fs::create_dir_all(&credentials_dir).expect("create credentials directory");
    let credentials = serde_json::json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
        "expires_at": expires_at,
        "scope": "scope",
        "token_type": "Bearer",
        "expires_in": 600,
    });
    std::fs::write(
        credentials_dir.join("kimi-code.json"),
        serde_json::to_vec(&credentials).expect("serialize credentials"),
    )
    .expect("write credentials");
}

fn chatgpt_auth(email: &str, user: &str, account: &str, plan: &str, signature: &str) -> CodexAuth {
    let claims = serde_json::json!({
        "email": email,
        "https://api.openai.com/auth": {"chatgpt_user_id": user}
    });
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    CodexAuth::from_external_chatgpt_tokens(
        &format!("e30.{payload}.{signature}"),
        account,
        Some(plan),
    )
    .unwrap()
}

#[test]
fn cache_identity_tracks_account_email_user_plan_and_auth_mode_but_not_token_refresh() {
    let mut provider = ModelProviderInfo::create_openai_provider(/*base_url*/ None);
    // The endpoint residency test temporarily sets the process default to US.
    provider.http_headers = Some(std::collections::HashMap::from([(
        codex_login::default_client::RESIDENCY_HEADER_NAME.to_string(),
        "us".into(),
    )]));
    let initial = chatgpt_auth("one@example.com", "user", "account", "team", "first");
    let key = identity(&provider, Some(&initial)).unwrap();
    let refreshed = chatgpt_auth("one@example.com", "user", "account", "team", "second");
    assert_eq!(identity(&provider, Some(&refreshed)).unwrap(), key);
    for auth in [
        chatgpt_auth("two@example.com", "user", "account", "team", "first"),
        chatgpt_auth("one@example.com", "other", "account", "team", "first"),
        chatgpt_auth("one@example.com", "user", "other", "team", "first"),
        chatgpt_auth("one@example.com", "user", "account", "plus", "first"),
        CodexAuth::from_api_key("api-key"),
    ] {
        assert_ne!(identity(&provider, Some(&auth)).unwrap(), key);
    }
    assert_ne!(identity(&provider, /*auth*/ None).unwrap(), key);
}

#[test]
fn cache_identity_tracks_provider_routing_and_effective_api_credentials() {
    let auth = CodexAuth::from_api_key("first-key");
    let provider = ModelProviderInfo::create_openai_provider(Some("https://one.example/v1".into()));
    let key = identity(&provider, Some(&auth)).unwrap();
    let other_key = CodexAuth::from_api_key("second-key");
    assert_ne!(identity(&provider, Some(&other_key)).unwrap(), key);
    let mut other_provider = provider.clone();
    other_provider.base_url = Some("https://two.example/v1".into());
    assert_ne!(identity(&other_provider, Some(&auth)).unwrap(), key);
    other_provider = provider.clone();
    other_provider.experimental_bearer_token = Some("provider-key".into());
    assert_ne!(identity(&other_provider, Some(&auth)).unwrap(), key);
    other_provider = provider;
    other_provider.http_headers = Some(std::collections::HashMap::from([(
        "openai-project".into(),
        "another-project".into(),
    )]));
    assert_ne!(identity(&other_provider, Some(&auth)).unwrap(), key);
}

#[test]
fn cache_identity_distinguishes_auth_requirements_and_unknown_plans() {
    let mut provider =
        ModelProviderInfo::create_openai_provider(Some("https://example.com/v1".into()));
    let first = chatgpt_auth(
        "one@example.com",
        "user",
        "account",
        "future-plan-one",
        "first",
    );
    let second = chatgpt_auth(
        "one@example.com",
        "user",
        "account",
        "future-plan-two",
        "first",
    );
    let key = identity(&provider, Some(&first)).unwrap();
    assert_ne!(key, identity(&provider, Some(&second)).unwrap());
    provider.requires_openai_auth = false;
    assert_ne!(key, identity(&provider, Some(&first)).unwrap());
}

#[test]
fn cache_identity_tracks_catalog_url() {
    let auth = CodexAuth::from_api_key("test-key");
    let mut provider =
        ModelProviderInfo::create_openai_provider(Some("https://gateway.example/v1".into()));
    let bundled = identity(&provider, Some(&auth)).unwrap();
    provider.model_catalog_url = Some("https://gateway.example/catalog-one".into());
    let first = identity(&provider, Some(&auth)).unwrap();
    provider.model_catalog_url = Some("https://gateway.example/catalog-two".into());
    let second = identity(&provider, Some(&auth)).unwrap();
    assert_ne!(bundled, first);
    assert_ne!(first, second);
}

#[test]
fn cache_identity_distinguishes_provider_credential_fingerprints() {
    let provider = kimi_code_provider();
    let first = identity_with_credential_fingerprint(&provider, None, Some("account-one"))
        .expect("first identity");
    let second = identity_with_credential_fingerprint(&provider, None, Some("account-two"))
        .expect("second identity");

    assert_ne!(first, second);
}

#[test]
fn cache_identity_does_not_reuse_saved_kimi_credentials_when_file_is_missing() {
    let provider = kimi_code_provider();
    let codex_home = tempdir().expect("tempdir");
    let missing = identity_with_codex_home(&provider, None, Some(codex_home.path()))
        .expect("identity without saved credentials");

    write_kimi_credentials(
        codex_home.path(),
        "saved-access-token",
        "saved-refresh-token",
        123,
    );
    let saved = identity_with_codex_home(&provider, None, Some(codex_home.path()))
        .expect("identity with saved credentials");

    assert_ne!(missing, saved);
}

#[test]
fn cache_identity_changes_after_kimi_token_file_refresh_without_exposing_tokens() {
    let provider = kimi_code_provider();
    let codex_home = tempdir().expect("tempdir");
    write_kimi_credentials(
        codex_home.path(),
        "access-token-before-refresh",
        "refresh-token-before-refresh",
        123,
    );
    let before_refresh = identity_with_codex_home(&provider, None, Some(codex_home.path()))
        .expect("identity before refresh");

    write_kimi_credentials(
        codex_home.path(),
        "access-token-after-refresh",
        "refresh-token-after-refresh",
        456,
    );
    let after_refresh = identity_with_codex_home(&provider, None, Some(codex_home.path()))
        .expect("identity after refresh");

    assert_ne!(before_refresh, after_refresh);
    for secret in [
        "access-token-before-refresh",
        "refresh-token-before-refresh",
        "access-token-after-refresh",
        "refresh-token-after-refresh",
    ] {
        assert!(!before_refresh.contains(secret));
        assert!(!after_refresh.contains(secret));
    }
}
