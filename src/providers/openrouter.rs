// OpenRouter / OpenAI-compatible provider implementation.

use super::{ConversationMessage, Provider, ProviderResponse, StreamChunk, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct OpenRouterProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    name: String,
}

impl OpenRouterProvider {
    pub fn new(api_key: &str, model: &str, base_url: Option<&str>, name: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            base_url: base_url
                .unwrap_or("https://openrouter.ai/api/v1")
                .to_string(),
            name: name.unwrap_or("openrouter").to_string(),
        }
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
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

#[async_trait]
impl Provider for OpenRouterProvider {
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
            .header("HTTP-Referer", "https://github.com/oneclaw/oneclaw")
            .header("X-Title", "OneClaw")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send chat request")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Provider returned {status}: {error_text}");
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .context("Failed to parse chat response")?;

        let content = chat_response
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.clone())
            .unwrap_or_default();

        let usage = chat_response.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
        });

        Ok(ProviderResponse {
            content,
            model: chat_response.model.unwrap_or_else(|| self.model.clone()),
            usage,
        })
    }

    async fn chat_stream(
        &self,
        messages: &[ConversationMessage],
        temperature: Option<f64>,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
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
            .header("HTTP-Referer", "https://github.com/oneclaw/oneclaw")
            .header("X-Title", "OneClaw")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send streaming chat request")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Provider returned {status}: {error_text}");
        }

        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(_) => break,
                };

                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Process SSE lines
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            let _ = tx.send(StreamChunk {
                                delta: String::new(),
                                done: true,
                            }).await;
                            return;
                        }

                        if let Ok(parsed) = serde_json::from_str::<ChatResponse>(data) {
                            if let Some(choice) = parsed.choices.first() {
                                if let Some(delta) = &choice.delta {
                                    if let Some(content) = &delta.content {
                                        let _ = tx.send(StreamChunk {
                                            delta: content.clone(),
                                            done: false,
                                        }).await;
                                    }
                                }
                                if choice.finish_reason.is_some() {
                                    let _ = tx.send(StreamChunk {
                                        delta: String::new(),
                                        done: true,
                                    }).await;
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}
