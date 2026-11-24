//! Bounded lazy V2 responses, cached per environment, executor session, cwd, and sandbox.
//! Reports recovery from retryable failures so Core can refresh dependent MCP projections.

use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use tokio::sync::Mutex;
use tokio::sync::OnceCell;

use crate::DiscoverV2CapabilitiesRequest;
use crate::DiscoverV2CapabilitiesResponse;
use crate::Environment;

const MIN_CACHED_DISCOVERY_CONTEXTS: usize = 16;

/// Shared by turn and idle consumers; Core replaces its current cache at each turn start.
#[derive(Default)]
pub struct ExecutorCapabilityDiscoveryV2Cache {
    entries: Mutex<Vec<Arc<CachedEnvironmentDiscovery>>>,
    capacity: AtomicUsize,
    recovered_discovery: AtomicBool,
}

struct CachedEnvironmentDiscovery {
    request: DiscoverV2CapabilitiesRequest,
    environment: Weak<Environment>,
    session_id: Option<String>,
    response: OnceCell<Result<Arc<DiscoverV2CapabilitiesResponse>, String>>,
    retryable: Arc<AtomicBool>,
}

impl ExecutorCapabilityDiscoveryV2Cache {
    /// Fits a complete environment selection; never shrinks until the cache is replaced.
    pub fn ensure_capacity(&self, minimum: usize) {
        self.capacity.fetch_max(minimum, Ordering::Relaxed);
    }

    /// Reports whether a failed discovery recovered since the last observation.
    pub fn take_recovered_discovery(&self) -> bool {
        self.recovered_discovery.swap(false, Ordering::AcqRel)
    }

    /// Reuses successes and non-retryable failures using V1's error classification.
    /// Retryable failures and cancelled loads leave the cell available for retry.
    pub async fn discover(
        &self,
        environment_id: &str,
        environment: &Arc<Environment>,
        request: DiscoverV2CapabilitiesRequest,
    ) -> Result<Arc<DiscoverV2CapabilitiesResponse>, String> {
        // Resolve startup/replacement before lookup; a resumed session keeps its cache entries.
        let session_id = environment.executor_session_id().await.map_err(|_| {
            tracing::warn!(environment_id, "executor discovery V2 session unavailable");
            "executor discovery V2 failed".to_string()
        })?;
        let entry = {
            let mut entries = self.entries.lock().await;
            let environment = Arc::downgrade(environment);
            if let Some(entry) = entries.iter().find(|entry| {
                entry.environment.ptr_eq(&environment)
                    && entry.session_id == session_id
                    && entry.request == request
            }) {
                Arc::clone(entry)
            } else {
                // Recovery belongs to the environment/request, not one executor session.
                let retryable = entries
                    .iter()
                    .find(|entry| {
                        entry.environment.ptr_eq(&environment) && entry.request == request
                    })
                    .map(|entry| Arc::clone(&entry.retryable))
                    .unwrap_or_default();
                let entry = Arc::new(CachedEnvironmentDiscovery {
                    request: request.clone(),
                    environment,
                    session_id,
                    response: OnceCell::new(),
                    retryable,
                });
                let capacity = self
                    .capacity
                    .load(Ordering::Relaxed)
                    .max(MIN_CACHED_DISCOVERY_CONTEXTS);
                if entries.len() == capacity {
                    // Evict the oldest context (FIFO). In-flight requests retain their own Arc.
                    entries.remove(0);
                }
                entries.push(Arc::clone(&entry));
                entry
            }
        };
        let response = entry
            .response
            .get_or_try_init(|| async {
                match environment.discover_capabilities_v2(request).await {
                    Ok(response) => Ok(Ok(Arc::new(response))),
                    Err(error) => {
                        let retryable = crate::client::can_retry_connection_attempt(&error);
                        tracing::warn!(environment_id, "executor discovery V2 failed");
                        let error = "executor discovery V2 failed".to_string();
                        if retryable {
                            entry.retryable.store(true, Ordering::Release);
                            Err(error)
                        } else {
                            Ok(Err(error))
                        }
                    }
                }
            })
            .await?
            .clone();
        if response.is_ok() && entry.retryable.swap(false, Ordering::AcqRel) {
            self.recovered_discovery.store(true, Ordering::Release);
        }
        response
    }
}
