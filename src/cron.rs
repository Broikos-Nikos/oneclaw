// Cron — scheduled task system.
//
// Tasks are stored in SQLite and executed by the cron runner on a schedule.
// Supports cron expressions (5-field), intervals (every N seconds), and one-shots.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Cron,      // 5-field cron expression
    Interval,  // every N seconds
    Once,      // fire once at a timestamp
}

impl std::fmt::Display for TaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskKind::Cron => write!(f, "cron"),
            TaskKind::Interval => write!(f, "interval"),
            TaskKind::Once => write!(f, "once"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronTask {
    pub id: String,
    pub name: String,
    pub message: String,   // message to send to the agent
    pub kind: TaskKind,
    pub schedule: String,  // cron expression, interval seconds, or RFC3339 timestamp
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub run_count: u64,
}

pub struct CronStore {
    conn: Mutex<Connection>,
}

impl CronStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open cron DB: {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cron_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                message TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'cron',
                schedule TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                last_run TEXT,
                run_count INTEGER NOT NULL DEFAULT 0
            );",
        )
        .context("Failed to create cron schema")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn add(&self, name: &str, message: &str, kind: TaskKind, schedule: &str) -> Result<CronTask> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cron_tasks (id,name,message,kind,schedule,enabled,created_at,run_count) VALUES (?1,?2,?3,?4,?5,1,?6,0)",
            params![id, name, message, kind.to_string(), schedule, now_str],
        )?;
        Ok(CronTask { id, name: name.to_string(), message: message.to_string(), kind,
            schedule: schedule.to_string(), enabled: true, created_at: now,
            last_run: None, run_count: 0 })
    }

    pub fn list(&self) -> Result<Vec<CronTask>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id,name,message,kind,schedule,enabled,created_at,last_run,run_count FROM cron_tasks ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                row.get::<_, String>(4)?, row.get::<_, bool>(5)?,
                row.get::<_, String>(6)?, row.get::<_, Option<String>>(7)?,
                row.get::<_, u64>(8)?,
            ))
        })?;
        let mut tasks = Vec::new();
        for row in rows {
            let (id, name, message, kind_str, schedule, enabled, created_str, last_str, run_count) = row?;
            let kind = match kind_str.as_str() {
                "interval" => TaskKind::Interval,
                "once" => TaskKind::Once,
                _ => TaskKind::Cron,
            };
            tasks.push(CronTask {
                id, name, message, kind, schedule, enabled,
                created_at: created_str.parse().unwrap_or_else(|_| Utc::now()),
                last_run: last_str.and_then(|s| s.parse().ok()),
                run_count,
            });
        }
        Ok(tasks)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("UPDATE cron_tasks SET enabled=?1 WHERE id=?2", params![enabled, id])?;
        Ok(n > 0)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM cron_tasks WHERE id=?1", params![id])?;
        Ok(n > 0)
    }

    pub fn record_run(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE cron_tasks SET last_run=?1, run_count=run_count+1 WHERE id=?2",
            params![now, id],
        )?;
        Ok(())
    }
}

/// Spawn the cron runner as a background task.
/// For each enabled task, it checks if the schedule has fired since last_run.
pub fn spawn_runner(
    store: Arc<CronStore>,
    message_tx: tokio::sync::mpsc::Sender<String>,
    mut shutdown: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        tick.tick().await;

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let now = Utc::now();
                    let tasks = match store.list() {
                        Ok(t) => t,
                        Err(e) => { warn!("Cron: failed to list tasks: {e}"); continue; }
                    };

                    for task in tasks.into_iter().filter(|t| t.enabled) {
                        let should_fire = match task.kind {
                            TaskKind::Cron => should_fire_cron(&task, now),
                            TaskKind::Interval => should_fire_interval(&task, now),
                            TaskKind::Once => should_fire_once(&task, now),
                        };

                        if should_fire {
                            info!("Cron: firing task '{}' — {}", task.name, task.message);
                            if message_tx.send(task.message.clone()).await.is_err() {
                                warn!("Cron: message channel closed");
                                return;
                            }
                            let _ = store.record_run(&task.id);
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Cron runner: shutting down");
                        return;
                    }
                }
            }
        }
    });
}

fn should_fire_cron(task: &CronTask, now: DateTime<Utc>) -> bool {
    // Parse cron, check if it has fired since last_run (or since created_at if never run)
    let Ok(schedule) = Schedule::from_str(&format!("0 {}", task.schedule)) else {
        warn!("Invalid cron expression for '{}': {}", task.name, task.schedule);
        return false;
    };
    let since = task.last_run.unwrap_or(task.created_at);
    schedule.after(&since).next().map(|t| t <= now).unwrap_or(false)
}

fn should_fire_interval(task: &CronTask, now: DateTime<Utc>) -> bool {
    let Ok(secs) = task.schedule.parse::<i64>() else { return false };
    let since = task.last_run.unwrap_or(task.created_at);
    (now - since).num_seconds() >= secs
}

fn should_fire_once(task: &CronTask, now: DateTime<Utc>) -> bool {
    if task.last_run.is_some() { return false; } // already ran
    let Ok(target) = task.schedule.parse::<DateTime<Utc>>() else { return false };
    now >= target
}

pub fn print_tasks(tasks: &[CronTask]) {
    if tasks.is_empty() {
        println!("No scheduled tasks.");
        return;
    }
    println!("{:<36} {:<10} {:<8} {}", "ID", "KIND", "ENABLED", "NAME / SCHEDULE");
    println!("{}", "-".repeat(80));
    for t in tasks {
        println!("{:<36} {:<10} {:<8} {} [{}]",
            t.id, t.kind, if t.enabled { "yes" } else { "no" }, t.name, t.schedule);
        println!("   msg: {}", t.message);
        if let Some(lr) = t.last_run {
            println!("   last: {} (runs: {})", lr.format("%Y-%m-%d %H:%M UTC"), t.run_count);
        }
    }
}
