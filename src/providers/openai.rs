// OpenAI provider — native OpenAI API (chat completions).
//
// Uses the same API format as the OpenAI-compatible provider but defaults to
// api.openai.com and uses the `openai` name for diagnostics.

use super::{ConversationMessage, Provider, ProviderResponse, StreamChunk, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct OpenAIProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    name: String,
}

impl OpenAIProvider {
    pub fn new(api_key: &str, model: &str, base_url: Option<&str>, name: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            base_url: base_url
                .unwrap_or("https://api.openai.com/v1")
                .trim_end_matches('/')
                .to_string(),
            name: name.unwrap_or("openai").to_string(),
        }
    }
}

// ─── Request / Response types ────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    stream: bool,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    model: Option<String>,
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: Option<ChoiceMessage>,
    delta: Option<ChoiceDelta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChoiceDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ApiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

// ─── Provider implementation ─────────────────────────────────────────────────

#[async_trait]
impl Provider for OpenAIProvider {
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
        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request_body = ChatRequest {
            model: self.model.clone(),
            messages: chat_messages,
            temperature,
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {status}: {body}");
        }

        let data: ChatResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        let content = data
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .and_then(|m| m.content)
            .unwrap_or_default();

        let model = data.model.unwrap_or_else(|| self.model.clone());
        let usage = data.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
        });

        Ok(ProviderResponse { content, model, usage })
    }

    async fn chat_stream(
        &self,
        messages: &[ConversationMessage],
        temperature: Option<f64>,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        use futures_util::StreamExt;

        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request_body = ChatRequest {
            model: self.model.clone(),
            messages: chat_messages,
            temperature,
            stream: true,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send streaming request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {status}: {body}");
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamChunk>(128);
        let mut stream = response.bytes_stream();

        tokio::spawn(async move {
            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(_) => break,
                };
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines() {
                    let data = line.strip_prefix("data: ").unwrap_or(line);
                    if data == "[DONE]" {
                        let _ = tx.send(StreamChunk { delta: String::new(), done: true }).await;
                        return;
                    }
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(delta) = val
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("content"))
                            .and_then(|s| s.as_str())
                        {
                            let _ = tx
                                .send(StreamChunk {
                                    delta: delta.to_string(),
                                    done: false,
                                })
                                .await;
                        }
                    }
                }
            }
            let _ = tx.send(StreamChunk { delta: String::new(), done: true }).await;
        });

        Ok(rx)
    }
}
