//! Compatibility with servers that advertise cyber programs without accepting them for API keys.
//! Remove when supported servers all forward API-key programs. Both callers pass raw catalogs.

use super::AppServerSession;
use super::thread_cwd_from_config;
use crate::legacy_core::config::Config;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigReadParams;
use codex_app_server_protocol::ConfigReadResponse;
use codex_app_server_protocol::ConfigRequirements;
use codex_app_server_protocol::RequestId;
use codex_config::ConfigLayerSource;
use codex_features::Feature;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ModelPreset;
use std::sync::Arc;

impl AppServerSession {
    pub(crate) fn constrain_api_key_models(
        &self,
        config: &Config,
        thread_id: Option<ThreadId>,
        requirements: Option<&ConfigRequirements>,
        mut models: Vec<ModelPreset>,
    ) -> impl Future<Output = Vec<ModelPreset>> + Send + 'static {
        let cli_override = config
            .config_layer_stack
            .origins()
            .get("features.api_key_cyber_access_programs")
            .is_some_and(|origin| matches!(origin.name, ConfigLayerSource::SessionFlags))
            .then(|| config.features.enabled(Feature::ApiKeyCyberAccessPrograms));
        let startup_enabled = requirements
            .and_then(|r| r.feature_requirements.as_ref())
            .and_then(|features| features.get(Feature::ApiKeyCyberAccessPrograms.key()))
            .copied()
            .or(cli_override);
        let cwd = thread_cwd_from_config(
            config,
            self.thread_params_mode(),
            self.remote_cwd_override(),
        );
        let handle = self.request_handle();
        let lookup = Arc::clone(&self.daybreak_api_key_compat);
        async move {
            // Serialize the reserved ID. This semaphore is never closed.
            let _permit = lookup.acquire().await;
            const ID: &str = "tui-daybreak-forwarding";
            let forwarding_enabled = if let Some(thread_id) = thread_id {
                let (tx, rx) = tokio::sync::oneshot::channel();
                crate::experimental_features::fetch(handle, Some(thread_id), ID, tx);
                rx.await.ok().and_then(Result::ok).is_some_and(|features| {
                    features.iter().any(|feature| {
                        feature.name == Feature::ApiKeyCyberAccessPrograms.key() && feature.enabled
                    })
                })
            } else if let Some(enabled) = startup_enabled {
                enabled
            } else {
                tokio::time::timeout(
                    std::time::Duration::from_secs(/*secs*/ 5),
                    handle.request_typed::<ConfigReadResponse>(ClientRequest::ConfigRead {
                        request_id: RequestId::String(ID.into()),
                        params: ConfigReadParams {
                            include_layers: false,
                            cwd,
                        },
                    }),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|response| {
                    response
                        .config
                        .additional
                        .get("features")?
                        .get(Feature::ApiKeyCyberAccessPrograms.key())?
                        .as_bool()
                })
                .unwrap_or(false)
            };
            if !forwarding_enabled {
                for model in &mut models {
                    model.available_access_programs = None;
                }
            }
            models
        }
    }
}
