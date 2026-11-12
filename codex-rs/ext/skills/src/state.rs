//! Shared skill lifecycle state, including session resources and thread-owned caches.
//! Keeps discovery caches and live authorization checks separate from step preparation.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Mutex as AsyncMutex;

use codex_exec_server::EnvironmentAccessKey;
use codex_extension_api::CatalogRefreshBackoff;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionMetrics;
use codex_mcp::CODEX_APPS_MCP_SERVER_NAME;
use codex_mcp::McpResourceClient;
use codex_mcp::McpResourceServerCacheKey;
use codex_protocol::capabilities::SelectedCapabilityRoot;
use tokio::sync::OnceCell;

use crate::SkillsExtensionConfig;
use crate::aliases::SkillAliasState;
use crate::catalog::SkillAuthority;
use crate::catalog::SkillCatalog;
use crate::catalog::SkillPackageId;
use crate::catalog::SkillProviderError;
use crate::catalog::SkillProviderResult;
use crate::catalog::SkillReadResult;
use crate::catalog::SkillSourceKind;
use crate::catalog_observability::record_catalog;
use crate::catalog_state::CachedExecutorCatalog;
use crate::catalog_state::CachedExecutorDiscoveryCatalog;
use crate::catalog_state::CachedExecutorV2Catalog;
use crate::catalog_state::CloudResourceCache;
use crate::catalog_state::CloudSkillGeneration;
use crate::catalog_state::ExecutorCatalogCacheKey;
use crate::catalog_state::SkillCatalogState;
use crate::catalog_state::SkillReadCacheKey;
use crate::provider::SkillListQuery;
use crate::provider::SkillReadContext;
use crate::provider::SkillReadRequest;
use crate::shadow_selection_experiment::RecentSkillInvocations;
use crate::shadow_selection_experiment::ShadowSelectionTurnState;
use crate::shadow_selection_experiment::ShadowTaskContext;
use crate::sources::SkillProviders;
use crate::step_state::SkillStepCatalog;

#[cfg(test)]
#[path = "cloud_cache_tests.rs"]
mod cloud_cache_tests;

pub(crate) struct SkillsSessionState {
    pub(crate) mcp_resources: Option<Arc<McpResourceClient>>,
    pub(crate) extension_metrics: Option<Arc<dyn ExtensionMetrics>>,
}

/// Auth scope of the latest catalog accepted for this turn.
pub(crate) struct LatestCloudSkillsAuth(Option<McpResourceClientAuthKey>);

/// Thread-owned skill configuration and caches; consumers can only read catalog snapshots.
pub struct SkillsThreadState {
    /// Serializes discovery; true only when the last refresh published successfully.
    cloud_refresh_lock: AsyncMutex<bool>,
    config: Mutex<SkillsExtensionConfig>,
    cloud_skills_available: bool,
    catalog_state: Mutex<SkillCatalogState>,
    extension_metrics: Option<Arc<dyn ExtensionMetrics>>,
    aliases: SkillAliasState,
    shadow_selection_turn: Mutex<Option<ShadowSelectionTurn>>,
    pub(crate) executor_read_snapshot: Mutex<Option<ExecutorReadSnapshot>>,
    pub(crate) recent_skill_invocations: Arc<RecentSkillInvocations>,
    pub(crate) shadow_task_context: Arc<ShadowTaskContext>,
}

impl SkillsThreadState {
    pub(crate) fn new(
        config: SkillsExtensionConfig,
        cloud_skills_available: bool,
        extension_metrics: Option<Arc<dyn ExtensionMetrics>>,
    ) -> Self {
        Self {
            cloud_refresh_lock: AsyncMutex::new(false),
            config: Mutex::new(config),
            cloud_skills_available,
            catalog_state: Mutex::new(SkillCatalogState::default()),
            extension_metrics,
            aliases: SkillAliasState::default(),
            shadow_selection_turn: Mutex::new(None),
            executor_read_snapshot: Mutex::new(None),
            recent_skill_invocations: Arc::new(RecentSkillInvocations::default()),
            shadow_task_context: Arc::new(ShadowTaskContext::default()),
        }
    }

    pub(crate) fn config(&self) -> SkillsExtensionConfig {
        self.config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set_config(&self, config: SkillsExtensionConfig) {
        let mut previous = self
            .config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if previous.cloud_skill_enabled != config.cloud_skill_enabled {
            let mut catalogs = self
                .catalog_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            catalogs.cloud_generation = None;
            catalogs.cloud_retry = None;
            catalogs.cloud_resources = CloudResourceCache::default();
        }
        *previous = config;
    }

    pub(crate) fn cloud_skills_available(&self) -> bool {
        self.cloud_skills_available
    }

    pub(crate) fn cloud_skill_enabled(&self) -> bool {
        self.cloud_skills_available && self.config().cloud_skill_enabled
    }

    pub(crate) fn prepare_step_catalog(
        &self,
        cloud: Option<SkillCatalog>,
        executor: SkillCatalog,
        discovery_v2: bool,
    ) -> SkillStepCatalog {
        let mut step = SkillStepCatalog::new(cloud, executor, discovery_v2, Default::default());
        step.aliases = self.aliases.capture(
            step.merged
                .entries
                .iter()
                .flat_map(super::catalog::SkillCatalogEntry::source_locations)
                .filter(|source| source.enabled && source.prompt_visible)
                .filter_map(|source| source.alias_root.map(|root| (source.authority.kind, root))),
        );
        step
    }

    /// Returns the executor catalog, reusing matching discovery caches.
    #[tracing::instrument(
        name = "skills.executor.refresh_executor_catalog",
        level = "info",
        skip_all,
        fields(root_count = query.executor_roots.len())
    )]
    pub(crate) async fn refresh_executor_catalog(
        &self,
        providers: &SkillProviders,
        query: SkillListQuery<'_>,
    ) -> SkillCatalog {
        if let Some(snapshots) = &query.executor_capability_discovery_v2 {
            let snapshots = snapshots.clone();
            let (cached, already_observed) = {
                let state = self
                    .catalog_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let cached = state.executor_v2_cache.as_ref();
                let already_observed =
                    cached.is_some_and(|cached| cached.same_discovery(&snapshots));
                (
                    cached
                        .filter(|_| {
                            already_observed
                                && snapshots.iter().all(|snapshot| snapshot.result.is_ok())
                        })
                        .map(|cached| cached.catalog.clone()),
                    already_observed,
                )
            };
            match cached {
                Some(catalog) => catalog,
                None => {
                    let catalog = providers.list_executor_for_turn(query).await;
                    // Failed snapshots are rebuilt for callers, but each is still one observation.
                    if !already_observed {
                        record_catalog(self.extension_metrics.as_deref(), "executor", &catalog);
                    }
                    self.catalog_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .executor_v2_cache = Some(CachedExecutorV2Catalog {
                        snapshots,
                        catalog: catalog.clone(),
                    });
                    catalog
                }
            }
        } else if query.executor_capability_discovery.is_some() {
            self.executor_discovery_catalog_snapshot(providers, query)
                .await
        } else {
            if let Some(cached) = self
                .catalog_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .executor_cache
                .as_ref()
                .filter(|cached| cached.key.matches(&query))
            {
                return cached.catalog.clone();
            }

            let key = ExecutorCatalogCacheKey {
                turn_id: query.turn_id.clone(),
                roots: query
                    .executor_roots
                    .iter()
                    .map(|root| (root.root.clone(), root.fs.cache_key()))
                    .collect(),
                include_bundled_skills: query.include_bundled_skills,
            };
            let catalog = if query.executor_roots.is_empty() {
                SkillCatalog::default()
            } else {
                providers.list_executor_for_turn(query).await
            };
            record_catalog(self.extension_metrics.as_deref(), "executor", &catalog);
            self.catalog_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .executor_cache = Some(CachedExecutorCatalog {
                key,
                catalog: catalog.clone(),
            });
            catalog
        }
    }

    /// Reuses matching successful discovery; otherwise returns the result of this request.
    async fn executor_discovery_catalog_snapshot(
        &self,
        providers: &SkillProviders,
        query: SkillListQuery<'_>,
    ) -> SkillCatalog {
        let discovery = query.executor_capability_discovery.clone();
        let discovery_failed = discovery.as_ref().is_some_and(|discovery| {
            discovery.roots().iter().any(|root| {
                root.result
                    .as_ref()
                    .ok()
                    .is_none_or(|discovered| discovered.error.is_some())
            })
        });
        let already_observed = {
            let state = self
                .catalog_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let cached = state
                .executor_discovery_cache
                .as_ref()
                .filter(|cached| cached.matches(discovery.as_ref(), query.include_bundled_skills));
            if !discovery_failed && let Some(cached) = cached {
                return cached.catalog.clone();
            }
            cached.is_some()
        };

        let include_bundled_skills = query.include_bundled_skills;
        let discovered = providers.list_executor_for_turn(query).await;
        if !already_observed {
            record_catalog(self.extension_metrics.as_deref(), "executor", &discovered);
        }
        let mut state = self
            .catalog_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Failed discovery is returned to this caller but is never reused by another request.
        state.executor_discovery_cache =
            discovery.map(|discovery| CachedExecutorDiscoveryCatalog {
                discovery,
                include_bundled_skills,
                catalog: discovered.clone(),
            });
        discovered
    }

    /// Returns the currently authorized cached Cloud catalog without refreshing it.
    /// Preparation decides whether Cloud is enabled using its captured config.
    pub(crate) fn available_cloud_catalog(&self) -> Option<SkillCatalog> {
        self.catalog_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cloud_generation
            .as_ref()
            .filter(|cache| cache.is_auth_current())
            .and_then(|cache| cache.catalog.clone())
    }

    /// Reuses discovery within a turn; backs off failures.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "cloud discovery must remain serialized through fetching and catalog publication"
    )]
    #[tracing::instrument(name = "skills.cloud.refresh_cloud_catalog", level = "info", skip_all)]
    pub(crate) async fn refresh_cloud_catalog(
        &self,
        providers: &SkillProviders,
        query: SkillListQuery<'_>,
        turn_store: &ExtensionData,
    ) -> SkillProviderResult<()> {
        // Wait without holding config/catalog state or the MCP runtime permit.
        let (mut refresh_succeeded, waited) = match self.cloud_refresh_lock.try_lock() {
            Ok(guard) => (guard, false),
            Err(_) => (self.cloud_refresh_lock.lock().await, true),
        };
        let (cache, resource_cache_key) = {
            let config = self
                .config
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !config.cloud_skill_enabled || !self.cloud_skills_available {
                return Ok(());
            }
            let mut catalogs = self
                .catalog_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Check this turn's discovery using the provider's auth scope.
            let cache = catalogs.prepare_cloud_generation(
                providers.cloud_mcp_resources(query.mcp_resources.as_deref()),
            );
            if cache.is_auth_current()
                && cache.catalog.is_some()
                && turn_store
                    .get::<LatestCloudSkillsAuth>()
                    .is_some_and(|previous| previous.0 == cache.auth_cache_key)
            {
                return Ok(());
            }
            let resource_cache_key = cache.current_resource_cache_key();
            if catalogs.cloud_retry.as_ref().is_some_and(|retry| {
                retry.is_pending(cache.auth_cache_key.as_ref(), resource_cache_key.as_ref())
            }) {
                return Ok(());
            }
            turn_store.remove::<LatestCloudSkillsAuth>();
            if cache.is_auth_current()
                && cache.catalog.as_ref().is_some_and(|catalog| {
                    (waited && *refresh_succeeded)
                        || (!catalog.is_partial && catalog.warnings.is_empty())
                })
                && cache.resource_cache_key == resource_cache_key
            {
                catalogs.cloud_retry = None;
                turn_store.insert(LatestCloudSkillsAuth(cache.auth_cache_key.clone()));
                return Ok(());
            }
            // Waiting callers must not reuse a failed or cancelled refresh.
            *refresh_succeeded = false;
            (cache, resource_cache_key)
        };
        let mut result = providers.list_cloud_for_turn(query).await;
        if let Ok(catalog) = &mut result {
            catalog.entries.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        }
        let mut catalogs = self
            .catalog_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !cache.is_auth_current()
            || cache.current_resource_cache_key() != resource_cache_key
            || !catalogs
                .cloud_generation
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &cache))
        {
            // Ignore superseded results, including errors.
            return Ok(());
        }
        let mut catalog = match result {
            Ok(catalog) => catalog,
            Err(error) => {
                catalogs.cloud_retry = Some(CatalogRefreshBackoff::after_failure(
                    cache.auth_cache_key.clone(),
                    resource_cache_key,
                    catalogs.cloud_retry.as_ref(),
                ));
                return Err(error);
            }
        };
        catalogs.cloud_retry = None;
        if catalog.is_partial
            && let Some(previous) = cache
                .catalog
                .as_ref()
                .filter(|previous| !previous.is_partial)
        {
            catalog = previous.clone();
            catalog.is_partial = true;
        } else {
            catalogs.cloud_resources = CloudResourceCache::default();
        }
        record_catalog(self.extension_metrics.as_deref(), "cloud", &catalog);
        let generation = Arc::new(CloudSkillGeneration {
            auth_cache_key: cache.auth_cache_key.clone(),
            resource_cache_key,
            mcp_resources: cache.mcp_resources.clone(),
            catalog: Some(catalog),
        });
        catalogs.cloud_generation = Some(Arc::clone(&generation));
        turn_store.insert(LatestCloudSkillsAuth(generation.auth_cache_key.clone()));
        *refresh_succeeded = true;
        Ok(())
    }

    pub(crate) async fn read_skill(
        &self,
        providers: &SkillProviders,
        request: SkillReadRequest<'_>,
    ) -> SkillProviderResult<SkillReadResult> {
        if request.authority.kind != SkillSourceKind::Cloud {
            return providers.read(request).await;
        }

        let SkillReadContext::Cloud { mcp_resources } = &request.context else {
            return Err(SkillProviderError::new(
                "cloud skill reads require a cloud context",
            ));
        };
        let cache_key = SkillReadCacheKey::from(&request);
        let cache = {
            let config = self
                .config
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !config.cloud_skill_enabled || !self.cloud_skills_available {
                return Err(SkillProviderError::new("cloud skills are disabled"));
            }
            let mut catalogs = self
                .catalog_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let cache = catalogs
                .prepare_cloud_generation(providers.cloud_mcp_resources(mcp_resources.as_deref()));
            if !cache.is_auth_current() {
                return Err(SkillProviderError::new(
                    "cloud skill cache changed before read",
                ));
            }
            if let Some(result) = catalogs.cloud_resources.get(&cache_key) {
                return Ok(result);
            }
            cache
        };

        let result = providers.read(request).await?;
        let mut catalogs = self
            .catalog_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !cache.is_auth_current()
            || !catalogs
                .cloud_generation
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &cache))
        {
            return Err(SkillProviderError::new(
                "cloud skill cache changed during read",
            ));
        }
        if result.resource != cache_key.resource {
            return Ok(result);
        }
        Ok(catalogs.cloud_resources.insert(cache_key, result))
    }

    pub(crate) fn replace_shadow_selection_turn(
        &self,
        turn_id: String,
        state: Option<ShadowSelectionTurnState>,
    ) {
        *self
            .shadow_selection_turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            state.map(|state| ShadowSelectionTurn {
                turn_id,
                state: Arc::new(state),
            });
    }

    pub(crate) fn shadow_selection_turn(
        &self,
        turn_id: &str,
    ) -> Option<Arc<ShadowSelectionTurnState>> {
        self.shadow_selection_turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|turn| turn.turn_id == turn_id)
            .map(|turn| Arc::clone(&turn.state))
    }

    /// Returns catalogs for stable selected roots.
    ///
    /// Successful catalogs, including empty or warning-bearing catalogs, remain cached until
    /// this thread state is dropped. Catalogs backed by failed discovery are not cached, so
    /// later steps can recover. There is no filesystem watcher because selected roots are stable.
    #[tracing::instrument(
        name = "skills.executor.catalog_snapshot",
        level = "info",
        skip_all,
        fields(root_count = query.executor_roots.len())
    )]
    pub(crate) async fn executor_catalog_snapshot(
        &self,
        providers: &SkillProviders,
        mut query: SkillListQuery,
    ) -> SkillCatalog {
        if query.executor_capability_discovery.is_some() {
            return self
                .executor_discovery_catalog_snapshot(providers, query)
                .await;
        }
        let roots = std::mem::take(&mut query.executor_roots);
        let mut catalog = SkillCatalog::default();
        for root in roots {
            query.executor_roots = vec![root.clone()];
            catalog.extend(
                self.executor_root_catalog(providers, root, query.clone())
                    .await,
            );
        }
        catalog
    }

    async fn executor_discovery_catalog_snapshot(
        &self,
        providers: &SkillProviders,
        query: SkillListQuery,
    ) -> SkillCatalog {
        let discovery_failed = query
            .executor_capability_discovery
            .as_ref()
            .is_some_and(|discovery| discovery.roots().iter().any(|root| root.result.is_err()));
        let sandbox_contexts = query
            .executor_capability_discovery
            .as_ref()
            .map(|discovery| discovery.sandbox_contexts().clone())
            .unwrap_or_default();
        if let Some(cached) = self
            .executor_discovery_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|cached| {
                cached.roots == query.executor_roots && cached.sandbox_contexts == sandbox_contexts
            })
        {
            return cached.catalog.clone();
        }
        let roots = query.executor_roots.clone();
        let discovered = providers.list_executor_for_turn(query).await;
        if discovery_failed {
            return discovered;
        }
        let mut cache = self
            .executor_discovery_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = cache
            .as_ref()
            .filter(|cached| cached.roots == roots && cached.sandbox_contexts == sandbox_contexts)
        {
            return cached.catalog.clone();
        }
        *cache = Some(CachedExecutorDiscoveryCatalog {
            roots,
            sandbox_contexts,
            catalog: discovered.clone(),
        });
        discovered
    }

    #[tracing::instrument(
        name = "skills.orchestrator.catalog_snapshot",
        level = "info",
        skip_all
    )]
    pub(crate) async fn orchestrator_catalog_snapshot(
        &self,
        providers: &SkillProviders,
        query: SkillListQuery,
    ) -> SkillCatalog {
        if !query.include_orchestrator_skills {
            return SkillCatalog::default();
        }

        let cache = self.orchestrator_cache(query.mcp_resources.as_deref());
        cache
            .catalog
            .get_or_init(|| async {
                providers
                    .list_orchestrator_for_turn(query)
                    .await
                    .unwrap_or_else(|err| SkillCatalog {
                        warnings: vec![err.message],
                        ..Default::default()
                    })
            })
            .await
            .clone()
    }

    pub(crate) async fn read_skill(
        &self,
        providers: &SkillProviders,
        request: SkillReadRequest<'_>,
    ) -> SkillProviderResult<SkillReadResult> {
        if request.authority.kind != SkillSourceKind::Orchestrator {
            return providers.read(request).await;
        }

        let cache = self.orchestrator_cache(request.mcp_resources.as_deref());
        let cache_key = SkillReadCacheKey::from(&request);
        if let Some(result) = cache
            .resources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&cache_key)
        {
            return Ok(result);
        }

        let result = providers.read(request).await?;
        if result.resource != cache_key.resource {
            return Ok(result);
        }

        Ok(cache
            .resources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(cache_key, result))
    }

    fn orchestrator_cache(
        &self,
        mcp_resources: Option<&McpResourceClient>,
    ) -> Arc<OrchestratorGenerationCache> {
        let mut cache = self
            .orchestrator_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cache_key =
            mcp_resources.and_then(|client| client.server_cache_key(CODEX_APPS_MCP_SERVER_NAME));
        if let Some(cache) = cache
            .as_ref()
            .filter(|cache| cache.mcp_cache_key == cache_key)
        {
            return Arc::clone(cache);
        }

        let next_cache = Arc::new(OrchestratorGenerationCache {
            mcp_cache_key: cache_key,
            catalog: OnceCell::new(),
            resources: Mutex::new(OrchestratorResourceCache::default()),
        });
        *cache = Some(Arc::clone(&next_cache));
        next_cache
    }

    #[tracing::instrument(name = "skills.executor.catalog_root", level = "info", skip_all)]
    async fn executor_root_catalog(
        &self,
        providers: &SkillProviders,
        root: SelectedCapabilityRoot,
        query: SkillListQuery,
    ) -> SkillCatalog {
        if let Some(cached) = self
            .executor_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|cached| cached.root == root)
        {
            return cached.catalog.clone();
        }

        let discovered = providers.list_executor_for_turn(query).await;
        let mut cache = self
            .executor_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = cache.iter().find(|cached| cached.root == root) {
            return cached.catalog.clone();
        }
        cache.push(CachedExecutorCatalog {
            root,
            catalog: discovered.clone(),
        });
        discovered
    }
}

/// One bounded executor resource, retained for continuations until replacement or thread drop.
/// Interleaved resources may evict it; misses reread and validate the content-bound cursor.
pub(crate) struct ExecutorReadSnapshot {
    pub(crate) authority: SkillAuthority,
    pub(crate) package: SkillPackageId,
    // The key binds cached contents to their source filesystem and callback permissions.
    pub(crate) access: EnvironmentAccessKey,
    pub(crate) result: Arc<SkillReadResult>,
}

struct ShadowSelectionTurn {
    turn_id: String,
    state: Arc<ShadowSelectionTurnState>,
}

struct CachedExecutorCatalog {
    root: SelectedCapabilityRoot,
    catalog: SkillCatalog,
}

struct CachedExecutorDiscoveryCatalog {
    roots: Vec<SelectedCapabilityRoot>,
    sandbox_contexts: HashMap<String, FileSystemSandboxContext>,
    catalog: SkillCatalog,
}

struct OrchestratorGenerationCache {
    mcp_cache_key: Option<McpResourceServerCacheKey>,
    catalog: OnceCell<SkillCatalog>,
    resources: Mutex<OrchestratorResourceCache>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SkillReadCacheKey {
    authority: SkillAuthority,
    package: SkillPackageId,
    resource: SkillResourceId,
}

impl From<&SkillReadRequest<'_>> for SkillReadCacheKey {
    fn from(request: &SkillReadRequest<'_>) -> Self {
        Self {
            authority: request.authority.clone(),
            package: request.package.clone(),
            resource: request.resource.clone(),
        }
    }
}

#[derive(Default)]
struct OrchestratorResourceCache {
    entries: HashMap<SkillReadCacheKey, SkillReadResult>,
    contents_bytes: usize,
}

impl OrchestratorResourceCache {
    fn get(&self, key: &SkillReadCacheKey) -> Option<SkillReadResult> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: SkillReadCacheKey, result: SkillReadResult) -> SkillReadResult {
        if let Some(cached) = self.entries.get(&key) {
            return cached.clone();
        }

        let contents_bytes = result.contents.len();
        let Some(next_contents_bytes) = self.contents_bytes.checked_add(contents_bytes) else {
            return result;
        };
        if self.entries.len() >= MAX_CACHED_ORCHESTRATOR_RESOURCES
            || next_contents_bytes > MAX_CACHED_ORCHESTRATOR_CONTENT_BYTES
        {
            return result;
        }

        self.contents_bytes = next_contents_bytes;
        self.entries.insert(key, result.clone());
        result
    }
}

#[derive(Default)]
pub(crate) struct EmittedCatalogBudgetWarnings(Mutex<HashSet<String>>);

impl EmittedCatalogBudgetWarnings {
    pub(crate) fn insert(&self, warning: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(warning.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HostSkillsCatalogInWorldState;
