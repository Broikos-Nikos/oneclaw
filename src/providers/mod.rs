// Provider trait — abstraction over LLM backends.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod anthropic;  // native Anthropic Messages API
pub mod openai;     // native OpenAI chat completions
pub mod ollama;     // local Ollama server
pub mod compatible; // generic OpenAI-compatible endpoint (custom base_url required)

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}

/// Streaming chunk from a provider.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub delta: String,
    pub done: bool,
}

/// Complete (non-streaming) response from a provider.
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Provider trait — implemented by each LLM backend.
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn model(&self) -> &str;

    /// Send a conversation and get a complete response.
    async fn chat(
        &self,
        messages: &[ConversationMessage],
        temperature: Option<f64>,
    ) -> Result<ProviderResponse>;

    /// Send a conversation and stream the response in chunks.
    async fn chat_stream(
        &self,
        messages: &[ConversationMessage],
        temperature: Option<f64>,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>>;
}
