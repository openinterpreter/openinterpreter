//! Creates isolated v2 memory state on demand while sharing the source thread catalog.
//! Reset and thread deletion always cover every existing version.

use super::MemoryStore;
use super::StateRuntime;
use codex_protocol::MemoryVersion;
use codex_protocol::ThreadId;
use std::sync::Arc;

impl StateRuntime {
    pub async fn memories_for_version(
        &self,
        version: MemoryVersion,
    ) -> anyhow::Result<MemoryStore> {
        match version {
            MemoryVersion::V1 => Ok(self.memories.clone()),
            MemoryVersion::V2 => self
                .memories_v2
                .get_or_try_init(|| async {
                    let pool = self.sqlite.open_memories_v2_db().await?;
                    Ok(MemoryStore::new(Arc::new(pool), Arc::clone(&self.pool)))
                })
                .await
                .cloned(),
        }
    }

    pub async fn clear_all_memory_data(&self) -> anyhow::Result<()> {
        self.memories.clear_memory_data().await?;
        if tokio::fs::try_exists(self.sqlite.memories_v2_db_path()).await? {
            self.memories_for_version(MemoryVersion::V2)
                .await?
                .clear_memory_data()
                .await?;
        }
        Ok(())
    }

    pub(super) async fn delete_versioned_thread_memory(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<()> {
        self.memories.delete_thread_memory(thread_id).await?;
        if tokio::fs::try_exists(self.sqlite.memories_v2_db_path()).await? {
            self.memories_for_version(MemoryVersion::V2)
                .await?
                .delete_thread_memory(thread_id)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "memory_versions_tests.rs"]
mod tests;
