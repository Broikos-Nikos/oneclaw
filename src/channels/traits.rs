//! Channel trait and shared message types for OneClaw.
//!
//! Adapted from ZeroClaw's trait-driven channel architecture, simplified
//! for OneClaw's multi-agent model.

use async_trait::async_trait;

/// A message received from a channel.
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    /// Unique message identifier (e.g. "telegram_-100200300_42").
    pub id: String,
    /// Sender identity (username or numeric ID).
    pub sender: String,
    /// Target for replies (e.g. chat_id or chat_id:thread_id).
    pub reply_target: String,
    /// Message text content.
    pub content: String,
    /// Channel name (e.g. "telegram", "whatsapp").
    pub channel: String,
    /// Unix timestamp.
    pub timestamp: u64,
}

/// Message to send through a channel.
#[derive(Debug, Clone)]
pub struct SendMessage {
    pub content: String,
    pub recipient: String,
}

impl SendMessage {
    /// Create a new message with content and recipient.
    pub fn new(content: impl Into<String>, recipient: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            recipient: recipient.into(),
        }
    }
}

/// Core channel trait — implement for any messaging platform.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Human-readable channel name.
    fn name(&self) -> &str;

    /// Send a message through this channel.
    async fn send(&self, message: &SendMessage) -> anyhow::Result<()>;

    /// Start listening for incoming messages (long-running).
    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()>;

    /// Check if channel is healthy.
    async fn health_check(&self) -> bool {
        true
    }

    /// Signal that the bot is processing a response.
    async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }

    /// Stop any active typing indicator.
    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
