// Markdown memory backend — single MEMORY.md file in the workspace.
//
// All agent memory is stored as plain markdown in one file.
// The full file content is injected into the first prompt turn each session.

use super::{Memory, MemoryEntry};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::path::{Path, PathBuf};
use tokio::fs;

pub struct MarkdownMemory {
    path: PathBuf,
}

impl MarkdownMemory {
    pub async fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        Ok(Self { path: path.to_path_buf() })
    }
}

#[async_trait]
impl Memory for MarkdownMemory {
    async fn store(
        &self,
        _category: &str,
        content: &str,
        _session_id: Option<&str>,
        _agent_type: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().format("%Y-%m-%d %H:%M").to_string();
        let line = format!("\n- [{}] {}\n", now, content);
        let mut current = fs::read_to_string(&self.path).await.unwrap_or_default();
        current.push_str(&line);
        fs::write(&self.path, &current).await?;
        Ok(id)
    }

    async fn recall(&self, _category: Option<&str>, _limit: usize) -> Result<Vec<MemoryEntry>> {
        let content = match fs::read_to_string(&self.path).await {
            Ok(c) if !c.trim().is_empty() => c,
            _ => return Ok(vec![]),
        };
        Ok(vec![MemoryEntry {
            id: "memory.md".to_string(),
            category: "memory".to_string(),
            content,
            created_at: Utc::now(),
            session_id: None,
            agent_type: None,
        }])
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let entries = self.recall(None, limit).await?;
        let q = query.to_lowercase();
        Ok(entries.into_iter().filter(|e| e.content.to_lowercase().contains(&q)).collect())
    }

    async fn forget(&self, _id: &str) -> Result<bool> {
        fs::write(&self.path, "").await?;
        Ok(true)
    }

    async fn clear(&self, _category: Option<&str>) -> Result<u64> {
        fs::write(&self.path, "").await?;
        Ok(1)
    }
}

// NOTE: The old per-entry-file parse/write helpers are removed.
// The following dead code was previously impl blocks that stored entries —
// replaced by the single-file approach above.

impl MarkdownMemory {
    pub fn path(&self) -> &Path {
        &self.path
    }
}
