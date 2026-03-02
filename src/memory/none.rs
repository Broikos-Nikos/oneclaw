// NoneMemory — a no-op memory backend for when memory is disabled.

use super::{Memory, MemoryEntry, MemoryStats};
use anyhow::Result;
use async_trait::async_trait;

/// Memory backend that stores nothing and recalls nothing.
/// Used when `memory.backend = "none"` in config.
pub struct NoneMemory;

impl NoneMemory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoneMemory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Memory for NoneMemory {
    async fn store(
        &self,
        _category: &str,
        _content: &str,
        _session_id: Option<&str>,
        _agent_type: Option<&str>,
    ) -> Result<String> {
        Ok(String::new())
    }

    async fn recall(&self, _category: Option<&str>, _limit: usize) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn forget(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }

    async fn clear(&self, _category: Option<&str>) -> Result<u64> {
        Ok(0)
    }

    async fn stats(&self) -> Result<MemoryStats> {
        Ok(MemoryStats { total: 0 })
    }
}
