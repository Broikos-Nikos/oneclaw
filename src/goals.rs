// Goal tracking — persistent long-term goals for the agent.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GoalStatus {
    Active,
    Completed,
    Cancelled,
}

impl std::fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalStatus::Active => write!(f, "active"),
            GoalStatus::Completed => write!(f, "completed"),
            GoalStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for GoalStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "active" => Ok(GoalStatus::Active),
            "completed" => Ok(GoalStatus::Completed),
            "cancelled" => Ok(GoalStatus::Cancelled),
            other => anyhow::bail!("Unknown status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: GoalStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub priority: u8, // 1 = highest, 5 = lowest
}

pub struct GoalStore {
    conn: Mutex<Connection>,
}

impl GoalStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open goals DB: {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 3
            );
            CREATE INDEX IF NOT EXISTS idx_goals_status ON goals(status);",
        )
        .context("Failed to create goals schema")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn add(&self, title: &str, description: &str, priority: u8) -> Result<Goal> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO goals (id, title, description, status, created_at, updated_at, priority) VALUES (?1,?2,?3,'active',?4,?5,?6)",
            params![id, title, description, now_str, now_str, priority],
        )?;
        Ok(Goal {
            id,
            title: title.to_string(),
            description: description.to_string(),
            status: GoalStatus::Active,
            created_at: now,
            updated_at: now,
            priority,
        })
    }

    pub fn list(&self, status_filter: Option<GoalStatus>) -> Result<Vec<Goal>> {
        let conn = self.conn.lock().unwrap();
        let mut goals = Vec::new();
        let sql = match &status_filter {
            Some(_) => "SELECT id,title,description,status,created_at,updated_at,priority FROM goals WHERE status=?1 ORDER BY priority ASC, created_at DESC",
            None => "SELECT id,title,description,status,created_at,updated_at,priority FROM goals ORDER BY priority ASC, created_at DESC",
        };
        if let Some(status) = &status_filter {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![status.to_string()], |row| {
                let status_str: String = row.get(3)?;
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                    status_str, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, u8>(6)?))
            })?;
            for row in rows {
                let (id, title, description, status_str, created_str, updated_str, priority) = row?;
                goals.push(Goal {
                    id, title, description,
                    status: status_str.parse()?,
                    created_at: created_str.parse().unwrap_or_else(|_| Utc::now()),
                    updated_at: updated_str.parse().unwrap_or_else(|_| Utc::now()),
                    priority,
                });
            }
        } else {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, u8>(6)?))
            })?;
            for row in rows {
                let (id, title, description, status_str, created_str, updated_str, priority) = row?;
                goals.push(Goal {
                    id, title, description,
                    status: status_str.parse()?,
                    created_at: created_str.parse().unwrap_or_else(|_| Utc::now()),
                    updated_at: updated_str.parse().unwrap_or_else(|_| Utc::now()),
                    priority,
                });
            }
        }
        Ok(goals)
    }

    pub fn update_status(&self, id: &str, status: GoalStatus) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "UPDATE goals SET status=?1, updated_at=?2 WHERE id=?3",
            params![status.to_string(), now, id],
        )?;
        Ok(count > 0)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute("DELETE FROM goals WHERE id=?1", params![id])?;
        Ok(count > 0)
    }

    /// Build a system prompt section listing active goals.
    pub fn active_goals_prompt(&self) -> String {
        match self.list(Some(GoalStatus::Active)) {
            Ok(goals) if !goals.is_empty() => {
                let mut s = "## Active Goals\n\n".to_string();
                for g in &goals {
                    s.push_str(&format!(
                        "- [P{}] **{}**: {}\n",
                        g.priority, g.title, g.description
                    ));
                }
                s.push('\n');
                s
            }
            _ => String::new(),
        }
    }
}

/// Print goals table to stdout.
pub fn print_goals(goals: &[Goal]) {
    if goals.is_empty() {
        println!("No goals found.");
        return;
    }
    println!("{:<36} {:<8} {:<4} {}", "ID", "STATUS", "PRI", "TITLE");
    println!("{}", "-".repeat(80));
    for g in goals {
        println!("{:<36} {:<8} {:<4} {}", g.id, g.status, g.priority, g.title);
        if !g.description.is_empty() {
            println!("    {}", g.description);
        }
    }
}
