// Coordination state — tracks and synchronizes multi-agent work.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl std::fmt::Display for WorkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkStatus::Pending => write!(f, "pending"),
            WorkStatus::Running => write!(f, "running"),
            WorkStatus::Done => write!(f, "done"),
            WorkStatus::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for WorkStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(WorkStatus::Pending),
            "running" => Ok(WorkStatus::Running),
            "done" => Ok(WorkStatus::Done),
            "failed" => Ok(WorkStatus::Failed),
            other => anyhow::bail!("Unknown status: {other}"),
        }
    }
}

/// A work item representing a delegated task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: String,
    pub session_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub task: String,
    pub result: Option<String>,
    pub status: WorkStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct CoordinationStore {
    conn: Mutex<Connection>,
}

impl CoordinationStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open coordination DB: {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS work_items (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                from_agent TEXT NOT NULL,
                to_agent TEXT NOT NULL,
                task TEXT NOT NULL,
                result TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_work_session ON work_items(session_id);
            CREATE INDEX IF NOT EXISTS idx_work_to_agent ON work_items(to_agent, status);",
        )
        .context("Failed to create coordination schema")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Record a new delegation.
    pub fn delegate(
        &self,
        session_id: &str,
        from_agent: &str,
        to_agent: &str,
        task: &str,
    ) -> Result<WorkItem> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO work_items (id,session_id,from_agent,to_agent,task,status,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,'pending',?6,?7)",
            params![id, session_id, from_agent, to_agent, task, now_str, now_str],
        )?;
        Ok(WorkItem {
            id, session_id: session_id.to_string(), from_agent: from_agent.to_string(),
            to_agent: to_agent.to_string(), task: task.to_string(), result: None,
            status: WorkStatus::Pending, created_at: now, updated_at: now,
        })
    }

    /// Complete a work item with a result.
    pub fn complete(&self, id: &str, result: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE work_items SET status='done', result=?1, updated_at=?2 WHERE id=?3",
            params![result, now, id],
        )?;
        Ok(())
    }

    /// Mark a work item as failed.
    pub fn fail(&self, id: &str, reason: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE work_items SET status='failed', result=?1, updated_at=?2 WHERE id=?3",
            params![reason, now, id],
        )?;
        Ok(())
    }

    /// Get all work items for a session.
    pub fn session_work(&self, session_id: &str) -> Result<Vec<WorkItem>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id,session_id,from_agent,to_agent,task,result,status,created_at,updated_at FROM work_items WHERE session_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?,
            ))
        })?;
        let mut items = Vec::new();
        for row in rows.flatten() {
            let (id, sid, from, to, task, result, status_str, created_str, updated_str) = row;
            items.push(WorkItem {
                id, session_id: sid, from_agent: from, to_agent: to, task, result,
                status: status_str.parse()?,
                created_at: created_str.parse().unwrap_or_else(|_| Utc::now()),
                updated_at: updated_str.parse().unwrap_or_else(|_| Utc::now()),
            });
        }
        Ok(items)
    }

    /// Build a prompt section summarizing current session coordination.
    pub fn session_prompt(&self, session_id: &str) -> String {
        match self.session_work(session_id) {
            Ok(items) if !items.is_empty() => {
                let mut s = "## Coordination History (this session)\n\n".to_string();
                for item in &items {
                    s.push_str(&format!(
                        "- {} → {}: {} [{}]\n",
                        item.from_agent, item.to_agent, item.task, item.status
                    ));
                }
                s.push('\n');
                s
            }
            _ => String::new(),
        }
    }
}
