// Memory trait and all backends.
//
// Backends:
//   sqlite   (default) — SQLite, keyword search
//   markdown           — one Markdown file per entry
//   vector             — SQLite + Ollama embeddings for semantic search
//   none               — disabled, stores nothing

pub mod markdown;
pub mod none;
pub mod sqlite;
pub mod vector;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

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

    /// Search memory entries by content (keyword or vector similarity).
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Delete a memory entry by ID.
    async fn forget(&self, id: &str) -> Result<bool>;

    /// Clear all entries in a category (or all if None).
    async fn clear(&self, category: Option<&str>) -> Result<u64>;

    /// Statistics summary.
    async fn stats(&self) -> Result<MemoryStats> {
        let all = self.recall(None, 10_000).await?;
        let mut categories = std::collections::HashMap::new();
        for e in &all {
            *categories.entry(e.category.clone()).or_insert(0u64) += 1;
        }
        Ok(MemoryStats { total: all.len() as u64, by_category: categories })
    }
}

#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub total: u64,
    pub by_category: std::collections::HashMap<String, u64>,
}

/// Which memory backend to use.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    #[default]
    Sqlite,
    Markdown,
    Vector,
    None,
}

impl std::str::FromStr for MemoryBackend {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "sqlite" | "sql" => Ok(MemoryBackend::Sqlite),
            "markdown" | "md" => Ok(MemoryBackend::Markdown),
            "vector" | "vec" => Ok(MemoryBackend::Vector),
            "none" | "off" => Ok(MemoryBackend::None),
            other => anyhow::bail!("Unknown memory backend: {other}. Options: sqlite, markdown, vector, none"),
        }
    }
}

impl std::fmt::Display for MemoryBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryBackend::Sqlite => write!(f, "sqlite"),
            MemoryBackend::Markdown => write!(f, "markdown"),
            MemoryBackend::Vector => write!(f, "vector"),
            MemoryBackend::None => write!(f, "none"),
        }
    }
}

/// Build the appropriate memory backend from config.
///
/// Returns `None` when backend is `None` (disabled).
pub async fn build_memory(
    backend: &MemoryBackend,
    data_dir: &Path,
    embed_url: &str,
    embed_model: &str,
) -> Result<Option<Box<dyn Memory>>> {
    match backend {
        MemoryBackend::None => Ok(None),
        MemoryBackend::Sqlite => {
            let db = data_dir.join("memory.db");
            Ok(Some(Box::new(sqlite::SqliteMemory::new(&db)?)))
        }
        MemoryBackend::Markdown => {
            let dir = data_dir.join("memory");
            Ok(Some(Box::new(markdown::MarkdownMemory::new(&dir).await?)))
        }
        MemoryBackend::Vector => {
            let db = data_dir.join("vector_memory.db");
            Ok(Some(Box::new(vector::VectorMemory::new(&db, embed_url, embed_model)?)))
        }
    }
}

/// Format a memory entry for display in the system prompt.
pub fn entries_to_prompt(entries: &[MemoryEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut s = "## Recalled Memories\n\n".to_string();
    for e in entries {
        s.push_str(&format!(
            "- [{}] {}: {}\n",
            e.category,
            e.created_at.format("%Y-%m-%d"),
            e.content
        ));
    }
    s.push('\n');
    s
}
