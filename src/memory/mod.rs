// Memory trait and SQLite backend.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod sqlite;

/// A memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub category: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub session_id: Option<String>,
    pub agent_type: Option<String>,
}

/// Memory trait — persistent storage for agent context.
#[async_trait]
pub trait Memory: Send + Sync {
    /// Store a new memory entry.
    async fn store(&self, category: &str, content: &str, session_id: Option<&str>, agent_type: Option<&str>) -> Result<String>;

    /// Recall memory entries, optionally filtered by category.
    async fn recall(&self, category: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Search memory entries by content.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Delete a memory entry by ID.
    async fn forget(&self, id: &str) -> Result<bool>;

    /// Clear all entries in a category.
    async fn clear(&self, category: Option<&str>) -> Result<u64>;
}
