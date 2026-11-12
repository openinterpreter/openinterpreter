use codex_api::AuthError;
use codex_api::AuthProvider;
use codex_http_client::CHATGPT_ACCOUNT_ID_HEADER;
use codex_login::auth::is_fedramp_api_key;
use http::HeaderMap;
use http::HeaderValue;

/// Bearer-token auth provider for OpenAI-compatible model-provider requests.
#[derive(Clone, Default)]
pub struct BearerAuthProvider {
    pub token: Option<String>,
    pub account_id: Option<String>,
    pub is_fedramp_account: bool,
    pub token_header_name: Option<&'static str>,
    pub use_bearer_prefix: bool,
}

impl BearerAuthProvider {
    pub fn new(token: String) -> Self {
        let is_fedramp_account = is_fedramp_api_key(&token);
        Self {
            token: Some(token),
            account_id: None,
            is_fedramp_account: false,
            token_header_name: None,
            use_bearer_prefix: true,
        }
    }

    pub fn for_test(token: Option<&str>, account_id: Option<&str>) -> Self {
        Self {
            token: token.map(str::to_string),
            account_id: account_id.map(str::to_string),
            is_fedramp_account: false,
            token_header_name: None,
            use_bearer_prefix: true,
        }
    }
}

impl AuthProvider for BearerAuthProvider {
    fn add_auth_headers(&self, headers: &mut HeaderMap) -> Result<(), AuthError> {
        if let Some(token) = self.token.as_ref() {
            let header_value = if self.use_bearer_prefix {
                format!("Bearer {token}")
            } else {
                token.clone()
            };
            if let Ok(header) = HeaderValue::from_str(&header_value) {
                let header_name = self.token_header_name.unwrap_or("authorization");
                let _ = headers.insert(header_name, header);
            }
        }
        if let Some(account_id) = self.account_id.as_ref()
            && let Ok(header) = HeaderValue::from_str(account_id)
        {
            let _ = headers.insert(CHATGPT_ACCOUNT_ID_HEADER, header);
        }
        if self.is_fedramp_account {
            let _ = headers.insert("X-OpenAI-Fedramp", HeaderValue::from_static("true"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn bearer_auth_provider_reports_when_auth_header_will_attach() {
        let auth = BearerAuthProvider {
            token: Some("access-token".to_string()),
            account_id: None,
            is_fedramp_account: false,
            token_header_name: None,
            use_bearer_prefix: true,
        };

        assert_eq!(
            codex_api::auth_header_telemetry(&auth),
            codex_api::AuthHeaderTelemetry {
                attached: true,
                name: Some("authorization"),
            }
        );
    }

    #[test]
    fn bearer_auth_provider_adds_auth_headers() {
        let auth = BearerAuthProvider::for_test(Some("access-token"), Some("workspace-123"));
        let mut headers = HeaderMap::new();

        auth.add_auth_headers(&mut headers).unwrap();

        assert_eq!(
            headers
                .get(http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer access-token")
        );
        assert_eq!(
            headers
                .get("ChatGPT-Account-ID")
                .and_then(|value| value.to_str().ok()),
            Some("workspace-123")
        );
    }

    #[test]
    fn bearer_auth_provider_adds_fedramp_routing_header_for_fedramp_accounts() {
        let auth = BearerAuthProvider {
            token: Some("access-token".to_string()),
            account_id: Some("workspace-123".to_string()),
            is_fedramp_account: true,
            token_header_name: None,
            use_bearer_prefix: true,
        };
        let mut headers = HeaderMap::new();

        auth.add_auth_headers(&mut headers).unwrap();

        assert_eq!(
            headers
                .get("X-OpenAI-Fedramp")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
    }

    #[test]
    fn bearer_auth_provider_supports_custom_header_without_bearer_prefix() {
        let auth = BearerAuthProvider {
            token: Some("anthropic-token".to_string()),
            account_id: None,
            is_fedramp_account: false,
            token_header_name: Some("x-api-key"),
            use_bearer_prefix: false,
        };
        let mut headers = HeaderMap::new();

        auth.add_auth_headers(&mut headers).unwrap();

        assert_eq!(
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("anthropic-token")
        );
        assert_eq!(
            codex_api::auth_header_telemetry(&auth),
            codex_api::AuthHeaderTelemetry {
                attached: true,
                name: Some("x-api-key"),
            }
        );
    }
}
