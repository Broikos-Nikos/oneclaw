// Vector memory backend — SQLite-backed with basic cosine similarity via embeddings.
//
// Uses Ollama's embedding API (or any OpenAI-compatible /embeddings endpoint)
// to generate embeddings, stored as JSON arrays in SQLite.
// Falls back to keyword search if embeddings are unavailable.

use super::{Memory, MemoryEntry};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

pub struct VectorMemory {
    conn: Mutex<Connection>,
    embed_url: String,
    embed_model: String,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct EmbedRequest {
    model: String,
    prompt: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embedding: Vec<f32>,
}

// OpenAI-compatible embedding endpoint
#[derive(Serialize)]
struct OaiEmbedRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct OaiEmbedResponse {
    data: Vec<OaiEmbedData>,
}
#[derive(Deserialize)]
struct OaiEmbedData {
    embedding: Vec<f32>,
}

impl VectorMemory {
    /// `embed_url` — base URL for the embedding endpoint (e.g. http://localhost:11434)
    /// `embed_model` — model name (e.g. nomic-embed-text)
    pub fn new(db_path: &Path, embed_url: &str, embed_model: &str) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open vector memory DB: {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vector_memory (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding TEXT,          -- JSON array of f32
                created_at TEXT NOT NULL,
                session_id TEXT,
                agent_type TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_vmem_category ON vector_memory(category);",
        )
        .context("Failed to create vector memory schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
            embed_url: embed_url.trim_end_matches('/').to_string(),
            embed_model: embed_model.to_string(),
            client: reqwest::Client::new(),
        })
    }

    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        // Try Ollama-style first, then OpenAI-compatible
        let ollama_url = format!("{}/api/embeddings", self.embed_url);
        if let Ok(resp) = self.client
            .post(&ollama_url)
            .json(&EmbedRequest { model: self.embed_model.clone(), prompt: text.to_string() })
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<EmbedResponse>().await {
                    return Some(body.embedding);
                }
            }
        }

        // Fall back to OpenAI-compatible
        let oai_url = format!("{}/v1/embeddings", self.embed_url);
        if let Ok(resp) = self.client
            .post(&oai_url)
            .json(&OaiEmbedRequest { model: self.embed_model.clone(), input: text.to_string() })
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<OaiEmbedResponse>().await {
                    if let Some(d) = body.data.into_iter().next() {
                        return Some(d.embedding);
                    }
                }
            }
        }

        None
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
    }
}

#[async_trait]
impl Memory for VectorMemory {
    async fn store(
        &self,
        category: &str,
        content: &str,
        session_id: Option<&str>,
        agent_type: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let embedding = self.embed(content).await;
        let embedding_json = embedding
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO vector_memory (id,category,content,embedding,created_at,session_id,agent_type) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, category, content, embedding_json, now, session_id, agent_type],
        )?;
        Ok(id)
    }

    async fn recall(&self, category: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut entries = Vec::new();
        if let Some(cat) = category {
            let mut stmt = conn.prepare(
                "SELECT id,category,content,created_at,session_id,agent_type FROM vector_memory WHERE category=?1 ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![cat, limit], row_to_entry)?;
            for r in rows.flatten() { entries.push(r); }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id,category,content,created_at,session_id,agent_type FROM vector_memory ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], row_to_entry)?;
            for r in rows.flatten() { entries.push(r); }
        }
        Ok(entries)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        // Try vector search first
        let query_embedding = self.embed(query).await;

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id,category,content,created_at,session_id,agent_type,embedding FROM vector_memory",
        )?;

        struct Row { entry: MemoryEntry, embedding: Option<String> }
        let rows: Vec<Row> = stmt.query_map([], |row| {
            Ok(Row {
                entry: MemoryEntry {
                    id: row.get(0)?,
                    category: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get::<_, String>(3)?.parse().unwrap_or_else(|_| Utc::now()),
                    session_id: row.get(4)?,
                    agent_type: row.get(5)?,
                },
                embedding: row.get(6)?,
            })
        })?.filter_map(|r| r.ok()).collect();

        if let Some(ref qe) = query_embedding {
            // Vector search
            let mut scored: Vec<(f32, MemoryEntry)> = rows.into_iter()
                .filter_map(|r| {
                    let emb: Vec<f32> = r.embedding
                        .and_then(|s| serde_json::from_str(&s).ok())?;
                    let score = Self::cosine_similarity(qe, &emb);
                    Some((score, r.entry))
                })
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            Ok(scored.into_iter().take(limit).map(|(_, e)| e).collect())
        } else {
            // Keyword fallback
            let query_lower = query.to_lowercase();
            let mut entries: Vec<MemoryEntry> = rows.into_iter()
                .filter(|r| r.entry.content.to_lowercase().contains(&query_lower))
                .map(|r| r.entry)
                .collect();
            entries.truncate(limit);
            Ok(entries)
        }
    }

    async fn forget(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM vector_memory WHERE id=?1", params![id])?;
        Ok(n > 0)
    }

    async fn clear(&self, category: Option<&str>) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let n = if let Some(cat) = category {
            conn.execute("DELETE FROM vector_memory WHERE category=?1", params![cat])? as u64
        } else {
            conn.execute("DELETE FROM vector_memory", [])? as u64
        };
        Ok(n)
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    Ok(MemoryEntry {
        id: row.get(0)?,
        category: row.get(1)?,
        content: row.get(2)?,
        created_at: row.get::<_, String>(3)?.parse().unwrap_or_else(|_| Utc::now()),
        session_id: row.get(4)?,
        agent_type: row.get(5)?,
    })
}
