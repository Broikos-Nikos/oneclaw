// Markdown memory backend — stores entries as .md files in a directory.

use super::{Memory, MemoryEntry};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use std::path::{Path, PathBuf};
use tokio::fs;

pub struct MarkdownMemory {
    dir: PathBuf,
}

impl MarkdownMemory {
    pub async fn new(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir)
            .await
            .with_context(|| format!("Failed to create markdown memory dir: {}", dir.display()))?;
        Ok(Self { dir: dir.to_path_buf() })
    }

    fn entry_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.md"))
    }

    fn parse_entry(content: &str, id: &str) -> Option<MemoryEntry> {
        // Format:
        // ---
        // id: <uuid>
        // category: <cat>
        // created_at: <rfc3339>
        // session_id: <optional>
        // agent_type: <optional>
        // ---
        // <content>
        let content = content.trim();
        if !content.starts_with("---") {
            return None;
        }
        let rest = content.strip_prefix("---")?;
        let (frontmatter, body) = rest.split_once("\n---")?;

        let mut category = String::new();
        let mut created_at = Utc::now();
        let mut session_id: Option<String> = None;
        let mut agent_type: Option<String> = None;

        for line in frontmatter.trim().lines() {
            if let Some(val) = line.strip_prefix("category: ") {
                category = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("created_at: ") {
                created_at = val.trim().parse().unwrap_or(Utc::now());
            } else if let Some(val) = line.strip_prefix("session_id: ") {
                let v = val.trim();
                if v != "null" && !v.is_empty() {
                    session_id = Some(v.to_string());
                }
            } else if let Some(val) = line.strip_prefix("agent_type: ") {
                let v = val.trim();
                if v != "null" && !v.is_empty() {
                    agent_type = Some(v.to_string());
                }
            }
        }

        Some(MemoryEntry {
            id: id.to_string(),
            category,
            content: body.trim().to_string(),
            created_at,
            session_id,
            agent_type,
        })
    }

    fn entry_to_md(entry: &MemoryEntry) -> String {
        format!(
            "---\nid: {}\ncategory: {}\ncreated_at: {}\nsession_id: {}\nagent_type: {}\n---\n{}\n",
            entry.id,
            entry.category,
            entry.created_at.to_rfc3339(),
            entry.session_id.as_deref().unwrap_or("null"),
            entry.agent_type.as_deref().unwrap_or("null"),
            entry.content,
        )
    }
}

#[async_trait]
impl Memory for MarkdownMemory {
    async fn store(
        &self,
        category: &str,
        content: &str,
        session_id: Option<&str>,
        agent_type: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let entry = MemoryEntry {
            id: id.clone(),
            category: category.to_string(),
            content: content.to_string(),
            created_at: Utc::now(),
            session_id: session_id.map(String::from),
            agent_type: agent_type.map(String::from),
        };
        let md = Self::entry_to_md(&entry);
        fs::write(self.entry_path(&id), md).await?;
        Ok(id)
    }

    async fn recall(&self, category: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut entries = self.all_entries().await?;
        if let Some(cat) = category {
            entries.retain(|e| e.category == cat);
        }
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        entries.truncate(limit);
        Ok(entries)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let query_lower = query.to_lowercase();
        let mut entries = self.all_entries().await?;
        entries.retain(|e| e.content.to_lowercase().contains(&query_lower));
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        entries.truncate(limit);
        Ok(entries)
    }

    async fn forget(&self, id: &str) -> Result<bool> {
        let path = self.entry_path(id);
        if path.exists() {
            fs::remove_file(path).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn clear(&self, category: Option<&str>) -> Result<u64> {
        let entries = self.all_entries().await?;
        let mut count = 0u64;
        for entry in entries {
            if category.map_or(true, |c| c == entry.category) {
                if fs::remove_file(self.entry_path(&entry.id)).await.is_ok() {
                    count += 1;
                }
            }
        }
        Ok(count)
    }
}

impl MarkdownMemory {
    async fn all_entries(&self) -> Result<Vec<MemoryEntry>> {
        let mut entries = Vec::new();
        let mut dir = fs::read_dir(&self.dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(content) = fs::read_to_string(&path).await {
                        if let Some(mem) = Self::parse_entry(&content, stem) {
                            entries.push(mem);
                        }
                    }
                }
            }
        }
        Ok(entries)
    }
}
