use codex_core::config::Config;
use codex_model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use codex_model_provider_info::OLLAMA_OSS_PROVIDER_ID;

const DEFAULT_LMSTUDIO_OSS_MODEL: &str = "openai/gpt-oss-20b";
const DEFAULT_OLLAMA_OSS_MODEL: &str = "gpt-oss:20b";

pub(crate) fn default_model_for_oss_provider(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        LMSTUDIO_OSS_PROVIDER_ID => Some(DEFAULT_LMSTUDIO_OSS_MODEL),
        OLLAMA_OSS_PROVIDER_ID => Some(DEFAULT_OLLAMA_OSS_MODEL),
        _ => None,
    }
}

#[cfg(feature = "oss-local-bootstrap")]
pub(crate) async fn ensure_oss_provider_ready(
    provider_id: &str,
    config: &Config,
) -> std::io::Result<()> {
    codex_utils_oss::ensure_oss_provider_ready(provider_id, config).await
}

#[cfg(not(feature = "oss-local-bootstrap"))]
pub(crate) async fn ensure_oss_provider_ready(
    _provider_id: &str,
    _config: &Config,
) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn defaults_lmstudio_model() {
        assert_eq!(
            default_model_for_oss_provider(LMSTUDIO_OSS_PROVIDER_ID),
            Some(DEFAULT_LMSTUDIO_OSS_MODEL)
        );
    }

    #[test]
    fn defaults_ollama_model() {
        assert_eq!(
            default_model_for_oss_provider(OLLAMA_OSS_PROVIDER_ID),
            Some(DEFAULT_OLLAMA_OSS_MODEL)
        );
    }

    #[test]
    fn ignores_unknown_provider() {
        assert_eq!(default_model_for_oss_provider("unknown"), None);
    }
}
