// Memory system.
//
// Agents have two forms of memory:
//   1. Session memory  — the live conversation history held in Agent::history.
//   2. MEMORY.md       — a persistent markdown file in the workspace directory.
//                        Injected into the first prompt turn each session.
//
// Backends:
//   markdown (default) — single MEMORY.md file in the workspace
//   none               — disabled, injects nothing

pub mod markdown;
pub mod none;

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

/// Memory trait — persistent storage backed by MEMORY.md.
#[async_trait]
pub trait Memory: Send + Sync {
    /// Append content to MEMORY.md.
    async fn store(&self, category: &str, content: &str, session_id: Option<&str>, agent_type: Option<&str>) -> Result<String>;

    /// Return the full contents of MEMORY.md as a single entry (or empty vec).
    async fn recall(&self, category: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Search MEMORY.md for a keyword.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Truncate MEMORY.md.
    async fn forget(&self, id: &str) -> Result<bool>;

    /// Truncate MEMORY.md (category arg ignored).
    async fn clear(&self, category: Option<&str>) -> Result<u64>;

    /// Basic stats.
    async fn stats(&self) -> Result<MemoryStats> {
        let all = self.recall(None, 1).await?;
        let lines = all.first().map(|e| e.content.lines().count()).unwrap_or(0);
        Ok(MemoryStats { total: lines as u64 })
    }
}

#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub total: u64,
}

/// Which memory backend to use.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    #[default]
    Markdown,
    None,
}

impl std::str::FromStr for MemoryBackend {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "markdown" | "md" | "file" => Ok(MemoryBackend::Markdown),
            "none" | "off" => Ok(MemoryBackend::None),
            other => anyhow::bail!("Unknown memory backend: {other}. Options: markdown, none"),
        }
    }
}

impl std::fmt::Display for MemoryBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryBackend::Markdown => write!(f, "markdown"),
            MemoryBackend::None => write!(f, "none"),
        }
    }
}

/// Build the memory backend.
///
/// `workspace_dir` is the agent workspace; MEMORY.md lives there.
/// Returns `None` when backend is `None` (disabled).
pub async fn build_memory(
    backend: &MemoryBackend,
    workspace_dir: &Path,
) -> Result<Option<Box<dyn Memory>>> {
    match backend {
        MemoryBackend::None => Ok(None),
        MemoryBackend::Markdown => {
            let path = workspace_dir.join("MEMORY.md");
            Ok(Some(Box::new(markdown::MarkdownMemory::new(&path).await?)))
        }
    }
}

/// Inject the MEMORY.md content into the system prompt.
pub fn entries_to_prompt(entries: &[MemoryEntry]) -> String {
    match entries.first() {
        Some(e) if !e.content.trim().is_empty() => {
            format!("## MEMORY.md\n\n{}\n\n", e.content.trim())
        }
        _ => String::new(),
    }
}
