//! Keychain labels isolate account-user identities within the fixed plugin-service scope.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::Digest as _;
use sha2::Sha256;

/// An opaque account-user namespace. App-server authenticates and selects the identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserVerificationKeyNamespace {
    pub(crate) label: String,
}

impl UserVerificationKeyNamespace {
    pub fn new(account_user_id: &str) -> Self {
        let identity = URL_SAFE_NO_PAD.encode(Sha256::digest(account_user_id.as_bytes()));
        Self {
            label: format!("com.openai.codex.user-verification.plugin-service.v1.{identity}"),
        }
    }
}

#[cfg(test)]
#[path = "key_namespace_tests.rs"]
mod tests;
