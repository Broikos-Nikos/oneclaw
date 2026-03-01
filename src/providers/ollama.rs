// Ollama provider — local LLM via Ollama's API.
//
// Compatible with Ollama's /api/chat endpoint which uses the messages format.
// Also compatible with OpenAI's /v1/chat/completions for any OpenAI-compat server.

use super::{ConversationMessage, Provider, ProviderResponse, StreamChunk, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct OllamaProvider {
    client: reqwest::Client,
    model: String,
    base_url: String,
    name: String,
}

impl OllamaProvider {
    pub fn new(model: &str, base_url: Option<&str>, name: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            model: model.to_string(),
            base_url: base_url
                .unwrap_or("http://localhost:11434")
                .trim_end_matches('/')
                .to_string(),
            name: name.unwrap_or("ollama").to_string(),
        }
    }
}

// ─── Ollama /api/chat types ─────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f64,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaStreamChunk {
    message: Option<OllamaMessage>,
    done: bool,
}

// ─── Provider implementation ────────────────────────────────────────────────

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(
        &self,
        messages: &[ConversationMessage],
        temperature: Option<f64>,
    ) -> Result<ProviderResponse> {
        let api_messages: Vec<OllamaMessage> = messages
            .iter()
            .map(|m| OllamaMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let body = OllamaChatRequest {
            model: self.model.clone(),
            messages: api_messages,
            stream: false,
            options: temperature.map(|t| OllamaOptions { temperature: t }),
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .context("Failed to send Ollama chat request")?;

        let status = response.status();
        if !status.is_success() {
            let err = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned {status}: {err}");
        }

        let resp: OllamaChatResponse = response
            .json()
            .await
            .context("Failed to parse Ollama response")?;

        let usage = Usage {
            prompt_tokens: resp.prompt_eval_count.unwrap_or(0),
            completion_tokens: resp.eval_count.unwrap_or(0),
            total_tokens: resp.prompt_eval_count.unwrap_or(0) + resp.eval_count.unwrap_or(0),
        };

        Ok(ProviderResponse {
            content: resp.message.content,
            model: self.model.clone(),
            usage: Some(usage),
        })
    }

    async fn chat_stream(
        &self,
        messages: &[ConversationMessage],
        temperature: Option<f64>,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let api_messages: Vec<OllamaMessage> = messages
            .iter()
            .map(|m| OllamaMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let body = OllamaChatRequest {
            model: self.model.clone(),
            messages: api_messages,
            stream: true,
            options: temperature.map(|t| OllamaOptions { temperature: t }),
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .context("Failed to send Ollama streaming request")?;

        let status = response.status();
        if !status.is_success() {
            let err = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned {status}: {err}");
        }

        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buf = String::new();

            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(_) => break,
                };
                buf.push_str(&String::from_utf8_lossy(&bytes));

                // Ollama sends newline-delimited JSON
                while let Some(nl) = buf.find('\n') {
                    let line = buf[..nl].trim().to_string();
                    buf = buf[nl + 1..].to_string();

                    if line.is_empty() { continue; }

                    if let Ok(chunk) = serde_json::from_str::<OllamaStreamChunk>(&line) {
                        if let Some(msg) = &chunk.message {
                            if !msg.content.is_empty() {
                                let _ = tx.send(StreamChunk {
                                    delta: msg.content.clone(),
                                    done: false,
                                }).await;
                            }
                        }
                        if chunk.done {
                            let _ = tx.send(StreamChunk { delta: String::new(), done: true }).await;
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults() {
        let p = OllamaProvider::new("llama3.2", None, None);
        assert_eq!(p.base_url, "http://localhost:11434");
        assert_eq!(p.model, "llama3.2");
        assert_eq!(p.name, "ollama");
    }

    #[test]
    fn new_custom() {
        let p = OllamaProvider::new("mistral", Some("http://myserver:11434/"), Some("local"));
        assert_eq!(p.base_url, "http://myserver:11434");
        assert_eq!(p.name, "local");
    }
}
