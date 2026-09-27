//! Consolidation progress used to select a warmed memory pipeline.

use super::MemoryStore;

impl MemoryStore {
    /// Largest number of distinct source threads included in a successful consolidation.
    /// Kept across ordinary pruning, and cleared by an explicit memory reset.
    pub async fn max_consolidated_thread_count(&self) -> anyhow::Result<u32> {
        let count: i64 = sqlx::query_scalar(
            "SELECT max_thread_count FROM consolidation_progress WHERE singleton = 1",
        )
        .fetch_one(self.pool.as_ref())
        .await?;
        Ok(u32::try_from(count)?)
    }
}
