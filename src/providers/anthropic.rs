// Anthropic provider — native Messages API implementation.
//
// Uses Anthropic's API format which differs from OpenAI:
// - Authentication: `x-api-key` header (not `Authorization: Bearer`)
// - Requires `anthropic-version` header
// - Different request/response schema (content blocks vs choices)

use super::{ConversationMessage, Provider, ProviderResponse, StreamChunk, Usage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    name: String,
}

impl AnthropicProvider {
    pub fn new(api_key: &str, model: &str, base_url: Option<&str>, name: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            base_url: base_url
                .unwrap_or("https://api.anthropic.com")
                .trim_end_matches('/')
                .to_string(),
            name: name.unwrap_or("anthropic").to_string(),
        }
    }
}

// ─── Request types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

// ─── Response types ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    model: Option<String>,
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

// ─── Streaming types ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<StreamDelta>,
}

#[derive(Deserialize)]
struct StreamDelta {
    #[serde(rename = "type")]
    delta_type: Option<String>,
    text: Option<String>,
}

// ─── Provider implementation ────────────────────────────────────────────────

#[async_trait]
impl Provider for AnthropicProvider {
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
        // Extract system message (Anthropic uses a separate `system` field)
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<ApiMessage> = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                // Accumulate system messages
                match &mut system_prompt {
                    Some(existing) => {
                        existing.push_str("\n\n");
                        existing.push_str(&msg.content);
                    }
                    None => {
                        system_prompt = Some(msg.content.clone());
                    }
                }
            } else {
                api_messages.push(ApiMessage {
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                });
            }
        }

        let request_body = MessagesRequest {
            model: self.model.clone(),
            messages: api_messages,
            system: system_prompt,
            max_tokens: 8192,
            temperature,
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send Anthropic chat request")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic returned {status}: {error_text}");
        }

        let api_response: MessagesResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic response")?;

        // Extract text from content blocks
        let content = api_response
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        let usage = api_response.usage.map(|u| Usage {
            prompt_tokens: u.input_tokens.unwrap_or(0),
            completion_tokens: u.output_tokens.unwrap_or(0),
            total_tokens: u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0),
        });

        Ok(ProviderResponse {
            content,
            model: api_response.model.unwrap_or_else(|| self.model.clone()),
            usage,
        })
    }

    async fn chat_stream(
        &self,
        messages: &[ConversationMessage],
        temperature: Option<f64>,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<ApiMessage> = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                match &mut system_prompt {
                    Some(existing) => {
                        existing.push_str("\n\n");
                        existing.push_str(&msg.content);
                    }
                    None => {
                        system_prompt = Some(msg.content.clone());
                    }
                }
            } else {
                api_messages.push(ApiMessage {
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                });
            }
        }

        let request_body = MessagesRequest {
            model: self.model.clone(),
            messages: api_messages,
            system: system_prompt,
            max_tokens: 8192,
            temperature,
            stream: true,
        };

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send Anthropic streaming request")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic returned {status}: {error_text}");
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
                        if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                            match event.event_type.as_str() {
                                "content_block_delta" => {
                                    if let Some(delta) = &event.delta {
                                        if delta.delta_type.as_deref() == Some("text_delta") {
                                            if let Some(text) = &delta.text {
                                                let _ = tx.send(StreamChunk {
                                                    delta: text.clone(),
                                                    done: false,
                                                }).await;
                                            }
                                        }
                                    }
                                }
                                "message_stop" => {
                                    let _ = tx.send(StreamChunk {
                                        delta: String::new(),
                                        done: true,
                                    }).await;
                                    return;
                                }
                                _ => {}
                            }
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
    fn new_sets_defaults() {
        let provider = AnthropicProvider::new("sk-test", "claude-sonnet-4-20250514", None, None);
        assert_eq!(provider.base_url, "https://api.anthropic.com");
        assert_eq!(provider.model, "claude-sonnet-4-20250514");
        assert_eq!(provider.name, "anthropic");
    }

    #[test]
    fn new_custom_base_url() {
        let provider = AnthropicProvider::new(
            "sk-test",
            "claude-sonnet-4-20250514",
            Some("https://custom.api.com/"),
            Some("custom"),
        );
        assert_eq!(provider.base_url, "https://custom.api.com");
        assert_eq!(provider.name, "custom");
    }

    #[test]
    fn parse_response_extracts_text() {
        let json = r#"{"content":[{"type":"text","text":"Hello world"}],"model":"claude-sonnet-4-20250514","usage":{"input_tokens":10,"output_tokens":5}}"#;
        let response: MessagesResponse = serde_json::from_str(json).unwrap();

        let text: String = response
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        assert_eq!(text, "Hello world");
        assert_eq!(response.usage.unwrap().input_tokens.unwrap(), 10);
    }

    #[test]
    fn parse_response_multiple_blocks() {
        let json = r#"{"content":[{"type":"text","text":"Part 1"},{"type":"text","text":" Part 2"}],"model":"claude-sonnet-4-20250514"}"#;
        let response: MessagesResponse = serde_json::from_str(json).unwrap();

        let text: String = response
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        assert_eq!(text, "Part 1 Part 2");
    }
}
