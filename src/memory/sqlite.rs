use super::{Memory, MemoryEntry};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct SqliteMemory {
    conn: Mutex<Connection>,
}

impl SqliteMemory {
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open SQLite database: {}", db_path.display()))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                session_id TEXT,
                agent_type TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_memory_category ON memory(category);
            CREATE INDEX IF NOT EXISTS idx_memory_created ON memory(created_at);
            CREATE INDEX IF NOT EXISTS idx_memory_agent ON memory(agent_type);",
        )
        .context("Failed to initialize memory schema")?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

#[async_trait]
impl Memory for SqliteMemory {
    async fn store(
        &self,
        category: &str,
        content: &str,
        session_id: Option<&str>,
        agent_type: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memory (id, category, content, created_at, session_id, agent_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, category, content, now, session_id, agent_type],
        )?;
        Ok(id)
    }

    async fn recall(&self, category: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut entries = Vec::new();

        if let Some(cat) = category {
            let mut stmt = conn.prepare(
                "SELECT id, category, content, created_at, session_id, agent_type FROM memory WHERE category = ?1 ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![cat, limit], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get::<_, String>(3)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    session_id: row.get(4)?,
                    agent_type: row.get(5)?,
                })
            })?;
            for row in rows {
                entries.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, category, content, created_at, session_id, agent_type FROM memory ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(rusqlite::params![limit], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get::<_, String>(3)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    session_id: row.get(4)?,
                    agent_type: row.get(5)?,
                })
            })?;
            for row in rows {
                entries.push(row?);
            }
        }

        Ok(entries)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{query}%");
        let mut stmt = conn.prepare(
            "SELECT id, category, content, created_at, session_id, agent_type FROM memory WHERE content LIKE ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![pattern, limit], |row| {
            Ok(MemoryEntry {
                id: row.get(0)?,
                category: row.get(1)?,
                content: row.get(2)?,
                created_at: row.get::<_, String>(3)?
                    .parse()
                    .unwrap_or_else(|_| Utc::now()),
                session_id: row.get(4)?,
                agent_type: row.get(5)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    async fn forget(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute("DELETE FROM memory WHERE id = ?1", rusqlite::params![id])?;
        Ok(count > 0)
    }

    async fn clear(&self, category: Option<&str>) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count = if let Some(cat) = category {
            conn.execute("DELETE FROM memory WHERE category = ?1", rusqlite::params![cat])? as u64
        } else {
            conn.execute("DELETE FROM memory", [])? as u64
        };
        Ok(count)
    }
}
