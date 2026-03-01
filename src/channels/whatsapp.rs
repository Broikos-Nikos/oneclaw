//! WhatsApp Business Cloud API channel integration for OneClaw.
//!
//! Receives messages via a built-in webhook HTTP server (push-based).
//! Sends messages via the WhatsApp Cloud API (graph.facebook.com).
//! Adapted from ZeroClaw's WhatsApp channel implementation.

use super::traits::{Channel, ChannelMessage, SendMessage};
use async_trait::async_trait;
use std::time::Duration;

/// WhatsApp Cloud API channel.
///
/// # Webhook Architecture
///
/// Unlike Telegram (which uses long-polling), WhatsApp uses webhooks:
/// Meta sends HTTP POST requests to your server when messages arrive.
///
/// OneClaw runs a lightweight HTTP server on `webhook_port` (default 8443)
/// that handles:
/// - `GET /webhook` — Meta verification challenge
/// - `POST /webhook` — Incoming message payloads
///
/// You must expose this port publicly (via ngrok, Cloudflare Tunnel, etc.)
/// and configure the URL in Meta's WhatsApp Business dashboard.
pub struct WhatsAppChannel {
    access_token: String,
    phone_number_id: String,
    verify_token: String,
    allowed_numbers: Vec<String>,
    webhook_port: u16,
    client: reqwest::Client,
}

impl WhatsAppChannel {
    /// Create a new WhatsAppChannel.
    pub fn new(
        access_token: String,
        phone_number_id: String,
        verify_token: String,
        allowed_numbers: Vec<String>,
        webhook_port: u16,
    ) -> Self {
        Self {
            access_token,
            phone_number_id,
            verify_token,
            allowed_numbers,
            webhook_port,
            client: reqwest::Client::new(),
        }
    }

    /// Check if a phone number is allowed (E.164 format: +1234567890).
    fn is_number_allowed(&self, phone: &str) -> bool {
        self.allowed_numbers.iter().any(|n| n == "*" || n == phone)
    }

    /// Normalize phone number to E.164 format (+prefix).
    fn normalize_phone(phone: &str) -> String {
        if phone.starts_with('+') {
            phone.to_string()
        } else {
            format!("+{phone}")
        }
    }

    /// Parse an incoming webhook payload from Meta and extract messages.
    pub fn parse_webhook_payload(&self, payload: &serde_json::Value) -> Vec<ChannelMessage> {
        let mut messages = Vec::new();

        // WhatsApp Cloud API webhook structure:
        // { "object": "whatsapp_business_account", "entry": [...] }
        let Some(entries) = payload.get("entry").and_then(|e| e.as_array()) else {
            return messages;
        };

        for entry in entries {
            let Some(changes) = entry.get("changes").and_then(|c| c.as_array()) else {
                continue;
            };

            for change in changes {
                let Some(value) = change.get("value") else {
                    continue;
                };

                let Some(msgs) = value.get("messages").and_then(|m| m.as_array()) else {
                    continue;
                };

                for msg in msgs {
                    let Some(from) = msg.get("from").and_then(|f| f.as_str()) else {
                        continue;
                    };

                    let normalized_from = Self::normalize_phone(from);

                    if !self.is_number_allowed(&normalized_from) {
                        tracing::warn!(
                            "WhatsApp: ignoring message from unauthorized number: {normalized_from}. \
                            Add to channels.whatsapp.allowed_numbers in config.toml."
                        );
                        continue;
                    }

                    // Extract text content (text messages only for now)
                    let content = if let Some(text_obj) = msg.get("text") {
                        text_obj
                            .get("body")
                            .and_then(|b| b.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        tracing::debug!("WhatsApp: skipping non-text message from {from}");
                        continue;
                    };

                    if content.is_empty() {
                        continue;
                    }

                    let timestamp = msg
                        .get("timestamp")
                        .and_then(|t| t.as_str())
                        .and_then(|t| t.parse::<u64>().ok())
                        .unwrap_or_else(|| {
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        });

                    let msg_id = msg
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("unknown");

                    messages.push(ChannelMessage {
                        id: format!("whatsapp_{msg_id}"),
                        reply_target: normalized_from.clone(),
                        sender: normalized_from,
                        content,
                        channel: "whatsapp".to_string(),
                        timestamp,
                    });
                }
            }
        }

        messages
    }

    /// Handle the webhook verification challenge from Meta.
    fn handle_verification_challenge(
        &self,
        query: &str,
    ) -> Result<String, &'static str> {
        // Parse query params: hub.mode, hub.verify_token, hub.challenge
        let params: Vec<(String, String)> = query
            .split('&')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                let key = parts.next()?.to_string();
                let value = parts.next().unwrap_or("").to_string();
                Some((key, value))
            })
            .collect();

        let mode = params
            .iter()
            .find(|(k, _)| k == "hub.mode")
            .map(|(_, v)| v.as_str());
        let token = params
            .iter()
            .find(|(k, _)| k == "hub.verify_token")
            .map(|(_, v)| v.as_str());
        let challenge = params
            .iter()
            .find(|(k, _)| k == "hub.challenge")
            .map(|(_, v)| v.as_str());

        if mode == Some("subscribe") && token == Some(&self.verify_token) {
            if let Some(challenge) = challenge {
                return Ok(challenge.to_string());
            }
        }

        Err("Verification failed")
    }

    /// Run the webhook HTTP server. Parses incoming webhook POSTs and
    /// sends parsed messages through the channel.
    async fn run_webhook_server(
        &self,
        tx: tokio::sync::mpsc::Sender<ChannelMessage>,
    ) -> anyhow::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let addr = format!("0.0.0.0:{}", self.webhook_port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(
            "WhatsApp webhook server listening on http://{addr}/webhook"
        );

        loop {
            let (mut stream, peer) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!("WhatsApp webhook: accept error: {e}");
                    continue;
                }
            };

            tracing::debug!("WhatsApp webhook: connection from {peer}");

            // Read the HTTP request (up to 64KB)
            let mut buf = vec![0u8; 65536];
            let n = match tokio::time::timeout(
                Duration::from_secs(10),
                stream.read(&mut buf),
            )
            .await
            {
                Ok(Ok(n)) if n > 0 => n,
                _ => continue,
            };

            let request = String::from_utf8_lossy(&buf[..n]);

            // Parse request line
            let first_line = request.lines().next().unwrap_or("");
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if parts.len() < 2 {
                let _ = stream
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                    .await;
                continue;
            }

            let method = parts[0];
            let path = parts[1];

            // Route
            if path.starts_with("/webhook") {
                match method {
                    "GET" => {
                        // Verification challenge
                        let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
                        match self.handle_verification_challenge(query) {
                            Ok(challenge) => {
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                                    challenge.len(),
                                    challenge
                                );
                                let _ = stream.write_all(resp.as_bytes()).await;
                            }
                            Err(_) => {
                                let _ = stream
                                    .write_all(
                                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n",
                                    )
                                    .await;
                            }
                        }
                    }
                    "POST" => {
                        // Extract body (after \r\n\r\n)
                        let body = request
                            .split_once("\r\n\r\n")
                            .map(|(_, b)| b)
                            .unwrap_or("");

                        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(body) {
                            let messages = self.parse_webhook_payload(&payload);
                            for msg in messages {
                                if tx.send(msg).await.is_err() {
                                    return Ok(());
                                }
                            }
                        }

                        let _ = stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                            .await;
                    }
                    _ => {
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n",
                            )
                            .await;
                    }
                }
            } else {
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        }
    }
}

#[async_trait]
impl Channel for WhatsAppChannel {
    fn name(&self) -> &str {
        "whatsapp"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let url = format!(
            "https://graph.facebook.com/v18.0/{}/messages",
            self.phone_number_id
        );

        // Normalize recipient (remove leading + for API)
        let to = message
            .recipient
            .strip_prefix('+')
            .unwrap_or(&message.recipient);

        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "text",
            "text": {
                "preview_url": false,
                "body": message.content
            }
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_body = resp.text().await.unwrap_or_default();
            tracing::error!("WhatsApp send failed: {status} — {error_body}");
            anyhow::bail!("WhatsApp API error: {status}");
        }

        Ok(())
    }

    async fn listen(
        &self,
        tx: tokio::sync::mpsc::Sender<ChannelMessage>,
    ) -> anyhow::Result<()> {
        self.run_webhook_server(tx).await
    }

    async fn health_check(&self) -> bool {
        let url = format!(
            "https://graph.facebook.com/v18.0/{}",
            self.phone_number_id
        );
        self.client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel() -> WhatsAppChannel {
        WhatsAppChannel::new(
            "test-token".into(),
            "123456789".into(),
            "verify-me".into(),
            vec!["+1234567890".into()],
            8443,
        )
    }

    #[test]
    fn whatsapp_channel_name() {
        let ch = make_channel();
        assert_eq!(ch.name(), "whatsapp");
    }

    #[test]
    fn whatsapp_number_allowed_exact() {
        let ch = make_channel();
        assert!(ch.is_number_allowed("+1234567890"));
        assert!(!ch.is_number_allowed("+9876543210"));
    }

    #[test]
    fn whatsapp_number_allowed_wildcard() {
        let ch = WhatsAppChannel::new(
            "tok".into(),
            "123".into(),
            "ver".into(),
            vec!["*".into()],
            8443,
        );
        assert!(ch.is_number_allowed("+1234567890"));
        assert!(ch.is_number_allowed("+9999999999"));
    }

    #[test]
    fn whatsapp_number_denied_empty() {
        let ch = WhatsAppChannel::new(
            "tok".into(),
            "123".into(),
            "ver".into(),
            vec![],
            8443,
        );
        assert!(!ch.is_number_allowed("+1234567890"));
    }

    #[test]
    fn whatsapp_normalize_phone() {
        assert_eq!(WhatsAppChannel::normalize_phone("1234567890"), "+1234567890");
        assert_eq!(WhatsAppChannel::normalize_phone("+1234567890"), "+1234567890");
    }

    #[test]
    fn whatsapp_parse_empty_payload() {
        let ch = make_channel();
        let payload = serde_json::json!({});
        let msgs = ch.parse_webhook_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn whatsapp_parse_valid_text_message() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "123",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "15551234567",
                            "phone_number_id": "123456789"
                        },
                        "messages": [{
                            "from": "1234567890",
                            "id": "wamid.xxx",
                            "timestamp": "1699999999",
                            "type": "text",
                            "text": {
                                "body": "Hello OneClaw!"
                            }
                        }]
                    },
                    "field": "messages"
                }]
            }]
        });

        let msgs = ch.parse_webhook_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].sender, "+1234567890");
        assert_eq!(msgs[0].content, "Hello OneClaw!");
        assert_eq!(msgs[0].channel, "whatsapp");
        assert_eq!(msgs[0].timestamp, 1_699_999_999);
        assert_eq!(msgs[0].id, "whatsapp_wamid.xxx");
    }

    #[test]
    fn whatsapp_parse_unauthorized_number() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "9999999999",
                            "timestamp": "1699999999",
                            "type": "text",
                            "text": { "body": "Spam" }
                        }]
                    }
                }]
            }]
        });

        let msgs = ch.parse_webhook_payload(&payload);
        assert!(msgs.is_empty(), "Unauthorized numbers should be filtered");
    }

    #[test]
    fn whatsapp_parse_non_text_message_skipped() {
        let ch = WhatsAppChannel::new(
            "tok".into(),
            "123".into(),
            "ver".into(),
            vec!["*".into()],
            8443,
        );
        let payload = serde_json::json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "1234567890",
                            "timestamp": "1699999999",
                            "type": "image",
                            "image": { "id": "img123" }
                        }]
                    }
                }]
            }]
        });

        let msgs = ch.parse_webhook_payload(&payload);
        assert!(msgs.is_empty(), "Non-text messages should be skipped");
    }

    #[test]
    fn whatsapp_parse_multiple_messages() {
        let ch = WhatsAppChannel::new(
            "tok".into(),
            "123".into(),
            "ver".into(),
            vec!["*".into()],
            8443,
        );
        let payload = serde_json::json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [
                            {
                                "from": "1111111111",
                                "id": "wamid.1",
                                "timestamp": "1700000001",
                                "type": "text",
                                "text": { "body": "First" }
                            },
                            {
                                "from": "2222222222",
                                "id": "wamid.2",
                                "timestamp": "1700000002",
                                "type": "text",
                                "text": { "body": "Second" }
                            }
                        ]
                    }
                }]
            }]
        });

        let msgs = ch.parse_webhook_payload(&payload);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "First");
        assert_eq!(msgs[1].content, "Second");
    }

    #[test]
    fn whatsapp_verification_challenge_success() {
        let ch = make_channel();
        let query = "hub.mode=subscribe&hub.verify_token=verify-me&hub.challenge=abc123";
        let result = ch.handle_verification_challenge(query);
        assert_eq!(result, Ok("abc123".to_string()));
    }

    #[test]
    fn whatsapp_verification_challenge_wrong_token() {
        let ch = make_channel();
        let query = "hub.mode=subscribe&hub.verify_token=wrong-token&hub.challenge=abc123";
        let result = ch.handle_verification_challenge(query);
        assert!(result.is_err());
    }

    #[test]
    fn whatsapp_verification_challenge_wrong_mode() {
        let ch = make_channel();
        let query = "hub.mode=unsubscribe&hub.verify_token=verify-me&hub.challenge=abc123";
        let result = ch.handle_verification_challenge(query);
        assert!(result.is_err());
    }
}
