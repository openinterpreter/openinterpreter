use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Declarative description of an environment-backed credential family.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CredentialProviderConfig {
    pub env: Vec<String>,
    pub patterns: Vec<String>,
    /// URL prefixes authorized for injection. Bare loopback hosts imply HTTP; other bare hosts
    /// imply HTTPS.
    pub url_prefixes: Vec<String>,
    /// Environment variable containing an additional URL prefix or hostname.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_prefix_from_env: Option<String>,
    pub auth: Vec<CredentialAuthMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

impl CredentialProviderConfig {
    /// Validate a complete provider definition using the broker's compilation rules.
    pub fn validate(&self, id: &str) -> anyhow::Result<()> {
        super::configured::ConfiguredCredentialProvider::compile(id, self)?;
        Ok(())
    }
}

/// Authentication formats supported by declarative credential providers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAuthMethod {
    Bearer,
    Token,
    Basic,
    Header,
}
