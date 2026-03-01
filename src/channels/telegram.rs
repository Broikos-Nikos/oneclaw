//! Telegram Bot API channel integration for OneClaw.
//!
//! Uses long-polling via `getUpdates` — no webhook server required.
//! Adapted from ZeroClaw's Telegram channel, simplified for OneClaw.

use super::traits::{Channel, ChannelMessage, SendMessage};
use async_trait::async_trait;
use std::fmt::Write;
use std::time::Duration;
use tokio::sync::Mutex;

/// Maximum Telegram message length (UTF-8 characters).
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

/// Telegram Bot API channel.
pub struct TelegramChannel {
    bot_token: String,
    allowed_users: Vec<String>,
    client: reqwest::Client,
    api_base: String,
    bot_username: Mutex<Option<String>>,
    typing_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TelegramChannel {
    /// Create a new TelegramChannel.
    pub fn new(bot_token: String, allowed_users: Vec<String>) -> Self {
        let normalized = Self::normalize_allowed_users(allowed_users);
        Self {
            bot_token,
            allowed_users: normalized,
            client: reqwest::Client::new(),
            api_base: "https://api.telegram.org".to_string(),
            bot_username: Mutex::new(None),
            typing_handle: Mutex::new(None),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{method}", self.api_base, self.bot_token)
    }

    fn normalize_identity(value: &str) -> String {
        value.trim().trim_start_matches('@').to_string()
    }

    fn normalize_allowed_users(allowed_users: Vec<String>) -> Vec<String> {
        allowed_users
            .into_iter()
            .map(|entry| Self::normalize_identity(&entry))
            .filter(|entry| !entry.is_empty())
            .collect()
    }

    fn is_user_allowed(&self, username: &str) -> bool {
        let identity = Self::normalize_identity(username);
        self.allowed_users
            .iter()
            .any(|u| u == "*" || u == &identity)
    }

    fn is_any_user_allowed<'a, I>(&self, identities: I) -> bool
    where
        I: IntoIterator<Item = &'a str>,
    {
        identities.into_iter().any(|id| self.is_user_allowed(id))
    }

    /// Extract sender username and display identity from a Telegram message object.
    fn extract_sender_info(
        message: &serde_json::Value,
    ) -> (String, Option<String>, String) {
        let username = message
            .get("from")
            .and_then(|from| from.get("username"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let sender_id = message
            .get("from")
            .and_then(|from| from.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let sender_identity = if username == "unknown" {
            sender_id.clone().unwrap_or_else(|| "unknown".to_string())
        } else {
            username.clone()
        };

        (username, sender_id, sender_identity)
    }

    /// Parse a Telegram update into a ChannelMessage.
    fn parse_update_message(&self, update: &serde_json::Value) -> Option<ChannelMessage> {
        let message = update.get("message")?;
        let text = message.get("text").and_then(serde_json::Value::as_str)?;

        let (username, sender_id, sender_identity) = Self::extract_sender_info(message);
        let mut identities = vec![username.as_str()];
        if let Some(id) = sender_id.as_deref() {
            identities.push(id);
        }

        if !self.is_any_user_allowed(identities.iter().copied()) {
            return None;
        }

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())?;

        let message_id = message
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        // Extract thread/topic ID for forum support
        let thread_id = message
            .get("message_thread_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let reply_target = if let Some(ref tid) = thread_id {
            format!("{}:{}", chat_id, tid)
        } else {
            chat_id.clone()
        };

        Some(ChannelMessage {
            id: format!("telegram_{chat_id}_{message_id}"),
            sender: sender_identity,
            reply_target,
            content: text.to_string(),
            channel: "telegram".to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    }

    /// Fetch bot username from Telegram API.
    async fn fetch_bot_username(&self) -> anyhow::Result<String> {
        let resp = self.client.get(self.api_url("getMe")).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to fetch bot info: {}", resp.status());
        }
        let data: serde_json::Value = resp.json().await?;
        let username = data
            .get("result")
            .and_then(|r| r.get("username"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| anyhow::anyhow!("Bot username not found in response"))?;
        Ok(username.to_string())
    }

    /// Parse reply_target into (chat_id, optional thread_id).
    fn parse_reply_target(reply_target: &str) -> (String, Option<String>) {
        if let Some((chat_id, thread_id)) = reply_target.split_once(':') {
            (chat_id.to_string(), Some(thread_id.to_string()))
        } else {
            (reply_target.to_string(), None)
        }
    }

    fn build_typing_action_body(reply_target: &str) -> serde_json::Value {
        let (chat_id, thread_id) = Self::parse_reply_target(reply_target);
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "action": "typing"
        });
        if let Some(thread_id) = thread_id {
            body["message_thread_id"] = serde_json::Value::String(thread_id);
        }
        body
    }

    /// Convert Markdown to Telegram HTML format.
    fn markdown_to_telegram_html(text: &str) -> String {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut result_lines: Vec<String> = Vec::new();

        for line in &lines {
            let trimmed_line = line.trim_start();

            if trimmed_line.starts_with("```") {
                result_lines.push(trimmed_line.to_string());
                continue;
            }

            let mut line_out = String::new();

            // Handle headers: ## Title → <b>Title</b>
            let stripped = line.trim_start_matches('#');
            let header_level = line.len() - stripped.len();
            if header_level > 0 && line.starts_with('#') && stripped.starts_with(' ') {
                let title = Self::escape_html(stripped.trim());
                result_lines.push(format!("<b>{title}</b>"));
                continue;
            }

            let mut i = 0;
            let bytes = line.as_bytes();
            let len = bytes.len();

            while i < len {
                // Bold: **text**
                if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'*' {
                    if let Some(end) = line[i + 2..].find("**") {
                        let inner = Self::escape_html(&line[i + 2..i + 2 + end]);
                        write!(line_out, "<b>{inner}</b>").unwrap();
                        i += 4 + end;
                        continue;
                    }
                }

                // Italic: *text*
                if bytes[i] == b'*' && (i == 0 || bytes[i - 1] != b'*') {
                    if let Some(end) = line[i + 1..].find('*') {
                        if end > 0 {
                            let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                            write!(line_out, "<i>{inner}</i>").unwrap();
                            i += 2 + end;
                            continue;
                        }
                    }
                }

                // Inline code: `code`
                if bytes[i] == b'`' && (i == 0 || bytes[i - 1] != b'`') {
                    if let Some(end) = line[i + 1..].find('`') {
                        let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                        write!(line_out, "<code>{inner}</code>").unwrap();
                        i += 2 + end;
                        continue;
                    }
                }

                // Default: escape HTML entities
                let ch = line[i..].chars().next().unwrap();
                match ch {
                    '<' => line_out.push_str("&lt;"),
                    '>' => line_out.push_str("&gt;"),
                    '&' => line_out.push_str("&amp;"),
                    '"' => line_out.push_str("&quot;"),
                    _ => line_out.push(ch),
                }
                i += ch.len_utf8();
            }
            result_lines.push(line_out);
        }

        // Second pass: handle ``` code blocks across lines
        let joined = result_lines.join("\n");
        let mut final_out = String::with_capacity(joined.len());
        let mut in_code_block = false;
        let mut code_buf = String::new();

        for line in joined.split('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_code_block {
                    in_code_block = false;
                    let escaped = code_buf.trim_end_matches('\n');
                    writeln!(final_out, "<pre>{escaped}</pre>").unwrap();
                    code_buf.clear();
                } else {
                    in_code_block = true;
                    code_buf.clear();
                }
            } else if in_code_block {
                code_buf.push_str(line);
                code_buf.push('\n');
            } else {
                final_out.push_str(line);
                final_out.push('\n');
            }
        }

        if in_code_block && !code_buf.is_empty() {
            writeln!(final_out, "<pre>{}</pre>", code_buf.trim_end()).unwrap();
        }

        final_out.trim_end_matches('\n').to_string()
    }

    fn escape_html(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// Split a message into chunks that fit within Telegram's limit.
    fn split_message(message: &str) -> Vec<String> {
        if message.len() <= TELEGRAM_MAX_MESSAGE_LENGTH {
            return vec![message.to_string()];
        }

        let mut chunks = Vec::new();
        let mut remaining = message;

        while !remaining.is_empty() {
            if remaining.len() <= TELEGRAM_MAX_MESSAGE_LENGTH {
                chunks.push(remaining.to_string());
                break;
            }

            // Try to split at newline
            let split_at = remaining[..TELEGRAM_MAX_MESSAGE_LENGTH]
                .rfind('\n')
                .or_else(|| remaining[..TELEGRAM_MAX_MESSAGE_LENGTH].rfind(' '))
                .unwrap_or(TELEGRAM_MAX_MESSAGE_LENGTH);

            chunks.push(remaining[..split_at].to_string());
            remaining = &remaining[split_at..].trim_start();
        }

        chunks
    }

    /// Send text in chunks, with markdown-to-HTML conversion.
    async fn send_text_chunks(
        &self,
        message: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let chunks = Self::split_message(message);

        for (index, chunk) in chunks.iter().enumerate() {
            let text = if chunks.len() > 1 {
                if index == 0 {
                    format!("{chunk}\n\n(continues...)")
                } else if index == chunks.len() - 1 {
                    format!("(continued)\n\n{chunk}")
                } else {
                    format!("(continued)\n\n{chunk}\n\n(continues...)")
                }
            } else {
                chunk.to_string()
            };

            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": Self::markdown_to_telegram_html(&text),
                "parse_mode": "HTML"
            });

            if let Some(tid) = thread_id {
                body["message_thread_id"] = serde_json::Value::String(tid.to_string());
            }

            let resp = self
                .client
                .post(self.api_url("sendMessage"))
                .json(&body)
                .send()
                .await?;

            if resp.status().is_success() {
                if index < chunks.len() - 1 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                continue;
            }

            // HTML failed — retry without parse_mode
            let mut plain_body = serde_json::json!({
                "chat_id": chat_id,
                "text": text,
            });

            if let Some(tid) = thread_id {
                plain_body["message_thread_id"] = serde_json::Value::String(tid.to_string());
            }

            let plain_resp = self
                .client
                .post(self.api_url("sendMessage"))
                .json(&plain_body)
                .send()
                .await?;

            if !plain_resp.status().is_success() {
                let status = plain_resp.status();
                let err = plain_resp.text().await.unwrap_or_default();
                let sanitized = Self::sanitize_error(&err);
                anyhow::bail!("Telegram sendMessage failed ({status}): {sanitized}");
            }

            if index < chunks.len() - 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        Ok(())
    }

    /// Redact bot token from error messages.
    fn sanitize_error(input: &str) -> String {
        let mut sanitized = input.to_string();
        let mut search_from = 0usize;

        while let Some(rel) = sanitized[search_from..].find("/bot") {
            let marker_start = search_from + rel;
            let token_start = marker_start + "/bot".len();
            let Some(next_slash_rel) = sanitized[token_start..].find('/') else {
                break;
            };
            let token_end = token_start + next_slash_rel;
            let should_redact = sanitized[token_start..token_end].contains(':')
                && sanitized[token_start..token_end]
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-'));
            if should_redact {
                sanitized.replace_range(token_start..token_end, "[REDACTED]");
                search_from = token_start + "[REDACTED]".len();
            } else {
                search_from = token_start;
            }
        }

        sanitized
    }

    fn log_poll_transport_error(sanitized: &str, consecutive_failures: u32) {
        if consecutive_failures >= 6 && consecutive_failures % 6 == 0 {
            tracing::warn!(
                "Telegram poll transport error persists (consecutive={}): {}",
                consecutive_failures,
                sanitized
            );
        } else {
            tracing::debug!(
                "Telegram poll transport error (consecutive={}): {}",
                consecutive_failures,
                sanitized
            );
        }
    }

    /// Handle messages from unauthorized users.
    async fn handle_unauthorized_message(&self, update: &serde_json::Value) {
        let Some(message) = update.get("message") else {
            return;
        };
        let Some(_text) = message.get("text").and_then(serde_json::Value::as_str) else {
            return;
        };

        let username = message
            .get("from")
            .and_then(|from| from.get("username"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");

        let sender_id = message
            .get("from")
            .and_then(|from| from.get("id"))
            .and_then(serde_json::Value::as_i64);

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());

        let Some(chat_id) = chat_id else {
            return;
        };

        tracing::warn!(
            "Telegram: ignoring message from unauthorized user: username={username}, sender_id={}.",
            sender_id.map(|id| id.to_string()).unwrap_or_else(|| "unknown".to_string())
        );

        let suggested_identity = sender_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| username.to_string());

        let _ = self
            .send(&SendMessage::new(
                format!(
                    "🔐 Unauthorized. Add your ID to allowed_users in config.toml:\n`{suggested_identity}`"
                ),
                &chat_id,
            ))
            .await;
    }

    /// Register bot commands with Telegram.
    async fn register_commands(&self) -> anyhow::Result<()> {
        let url = self.api_url("setMyCommands");
        let body = serde_json::json!({
            "commands": [
                {
                    "command": "new",
                    "description": "Start a new conversation"
                },
            ]
        });

        let resp = self.client.post(&url).json(&body).send().await?;
        if resp.status().is_success() {
            tracing::info!("Telegram bot commands registered successfully");
        } else {
            let status = resp.status();
            tracing::warn!("setMyCommands failed: status={status}");
        }

        Ok(())
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let (chat_id, thread_id) = match message.recipient.split_once(':') {
            Some((chat, thread)) => (chat, Some(thread)),
            None => (message.recipient.as_str(), None),
        };

        self.send_text_chunks(&message.content, chat_id, thread_id)
            .await
    }

    async fn listen(
        &self,
        tx: tokio::sync::mpsc::Sender<ChannelMessage>,
    ) -> anyhow::Result<()> {
        let mut offset: i64 = 0;
        let mut consecutive_poll_transport_failures = 0u32;

        // Register commands
        if let Err(e) = self.register_commands().await {
            tracing::warn!("Failed to register Telegram bot commands: {e}");
        }

        // Fetch bot username
        match self.fetch_bot_username().await {
            Ok(username) => {
                tracing::info!("Telegram bot: @{username}");
                *self.bot_username.lock().await = Some(username);
            }
            Err(e) => {
                tracing::warn!("Failed to fetch bot username: {e}");
            }
        }

        tracing::info!("Telegram channel listening for messages...");

        // Startup probe: claim the getUpdates slot
        loop {
            let url = self.api_url("getUpdates");
            let probe = serde_json::json!({
                "offset": offset,
                "timeout": 0,
                "allowed_updates": ["message"]
            });

            match self.client.post(&url).json(&probe).send().await {
                Err(e) => {
                    let sanitized = Self::sanitize_error(&e.to_string());
                    tracing::warn!("Telegram startup probe error: {sanitized}; retrying in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Err(e) => {
                        let sanitized = Self::sanitize_error(&e.to_string());
                        tracing::warn!(
                            "Telegram startup probe parse error: {sanitized}; retrying in 5s"
                        );
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    Ok(data) => {
                        let ok = data
                            .get("ok")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        if ok {
                            if let Some(results) =
                                data.get("result").and_then(serde_json::Value::as_array)
                            {
                                for update in results {
                                    if let Some(uid) = update
                                        .get("update_id")
                                        .and_then(serde_json::Value::as_i64)
                                    {
                                        offset = uid + 1;
                                    }
                                }
                            }
                            break;
                        }

                        let error_code = data
                            .get("error_code")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or_default();

                        if error_code == 409 {
                            tracing::debug!("Startup probe: slot busy (409), retrying in 5s");
                        } else {
                            let desc = data
                                .get("description")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("unknown");
                            tracing::warn!(
                                "Startup probe: API error {error_code}: {desc}; retrying in 5s"
                            );
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                },
            }
        }

        tracing::debug!("Startup probe succeeded; entering main long-poll loop.");

        // Main polling loop
        loop {
            let url = self.api_url("getUpdates");
            let body = serde_json::json!({
                "offset": offset,
                "timeout": 30,
                "allowed_updates": ["message"]
            });

            let resp = match self.client.post(&url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    let sanitized = Self::sanitize_error(&e.to_string());
                    consecutive_poll_transport_failures =
                        consecutive_poll_transport_failures.saturating_add(1);
                    Self::log_poll_transport_error(
                        &sanitized,
                        consecutive_poll_transport_failures,
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            consecutive_poll_transport_failures = 0;

            let data: serde_json::Value = match resp.json().await {
                Ok(d) => d,
                Err(e) => {
                    let sanitized = Self::sanitize_error(&e.to_string());
                    tracing::warn!("Telegram parse error: {sanitized}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let ok = data
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);

            if !ok {
                let error_code = data
                    .get("error_code")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default();
                let description = data
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown Telegram API error");

                if error_code == 409 {
                    tracing::warn!(
                        "Telegram polling conflict (409): {description}. \
                        Ensure only one `oneclaw` process is using this bot token."
                    );
                    tokio::time::sleep(Duration::from_secs(35)).await;
                } else {
                    tracing::warn!(
                        "Telegram getUpdates API error (code={error_code}): {description}"
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                continue;
            }

            if let Some(results) = data.get("result").and_then(serde_json::Value::as_array) {
                for update in results {
                    if let Some(uid) =
                        update.get("update_id").and_then(serde_json::Value::as_i64)
                    {
                        offset = uid + 1;
                    }

                    let msg = if let Some(m) = self.parse_update_message(update) {
                        m
                    } else {
                        self.handle_unauthorized_message(update).await;
                        continue;
                    };

                    // Send typing indicator
                    let typing_body = Self::build_typing_action_body(&msg.reply_target);
                    let _ = self
                        .client
                        .post(self.api_url("sendChatAction"))
                        .json(&typing_body)
                        .send()
                        .await;

                    if tx.send(msg).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        let timeout_duration = Duration::from_secs(5);
        match tokio::time::timeout(
            timeout_duration,
            self.client.get(self.api_url("getMe")).send(),
        )
        .await
        {
            Ok(Ok(resp)) => resp.status().is_success(),
            Ok(Err(e)) => {
                let sanitized = Self::sanitize_error(&e.to_string());
                tracing::debug!("Telegram health check failed: {sanitized}");
                false
            }
            Err(_) => {
                tracing::debug!("Telegram health check timed out after 5s");
                false
            }
        }
    }

    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        self.stop_typing(recipient).await?;

        let client = self.client.clone();
        let url = self.api_url("sendChatAction");
        let chat_id = recipient.to_string();

        let handle = tokio::spawn(async move {
            loop {
                let body = serde_json::json!({
                    "chat_id": &chat_id,
                    "action": "typing"
                });
                let _ = client.post(&url).json(&body).send().await;
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        });

        let mut guard = self.typing_handle.lock().await;
        *guard = Some(handle);
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        let mut guard = self.typing_handle.lock().await;
        if let Some(handle) = guard.take() {
            handle.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_channel_name() {
        let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
        assert_eq!(ch.name(), "telegram");
    }

    #[test]
    fn telegram_api_url() {
        let ch = TelegramChannel::new("123:ABC".into(), vec![]);
        assert_eq!(
            ch.api_url("getMe"),
            "https://api.telegram.org/bot123:ABC/getMe"
        );
    }

    #[test]
    fn telegram_user_allowed_wildcard() {
        let ch = TelegramChannel::new("t".into(), vec!["*".into()]);
        assert!(ch.is_user_allowed("anyone"));
    }

    #[test]
    fn telegram_user_allowed_specific() {
        let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "bob".into()]);
        assert!(ch.is_user_allowed("alice"));
        assert!(!ch.is_user_allowed("eve"));
    }

    #[test]
    fn telegram_user_allowed_with_at_prefix() {
        let ch = TelegramChannel::new("t".into(), vec!["@alice".into()]);
        assert!(ch.is_user_allowed("alice"));
    }

    #[test]
    fn telegram_user_denied_empty() {
        let ch = TelegramChannel::new("t".into(), vec![]);
        assert!(!ch.is_user_allowed("anyone"));
    }

    #[test]
    fn telegram_user_allowed_by_numeric_id() {
        let ch = TelegramChannel::new("t".into(), vec!["123456789".into()]);
        assert!(ch.is_any_user_allowed(["unknown", "123456789"]));
    }

    #[test]
    fn sanitize_error_redacts_bot_token() {
        let input =
            "error sending request for url (https://api.telegram.org/bot123456:ABCdef/getUpdates)";
        let sanitized = TelegramChannel::sanitize_error(input);
        assert!(!sanitized.contains("123456:ABCdef"));
        assert!(sanitized.contains("/bot[REDACTED]/getUpdates"));
    }

    #[test]
    fn sanitize_error_preserves_non_token_path() {
        let input = "error sending request for url (https://example.com/bot/getUpdates)";
        let sanitized = TelegramChannel::sanitize_error(input);
        assert_eq!(sanitized, input);
    }

    #[test]
    fn split_message_short() {
        let msg = "Hello, world!";
        let chunks = TelegramChannel::split_message(msg);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], msg);
    }

    #[test]
    fn split_message_exact_limit() {
        let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
        let chunks = TelegramChannel::split_message(&msg);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn split_message_over_limit() {
        let msg = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 100);
        let chunks = TelegramChannel::split_message(&msg);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
    }

    #[test]
    fn markdown_to_html_bold() {
        let html = TelegramChannel::markdown_to_telegram_html("**bold text**");
        assert_eq!(html, "<b>bold text</b>");
    }

    #[test]
    fn markdown_to_html_italic() {
        let html = TelegramChannel::markdown_to_telegram_html("*italic text*");
        assert_eq!(html, "<i>italic text</i>");
    }

    #[test]
    fn markdown_to_html_code() {
        let html = TelegramChannel::markdown_to_telegram_html("`inline code`");
        assert_eq!(html, "<code>inline code</code>");
    }

    #[test]
    fn markdown_to_html_header() {
        let html = TelegramChannel::markdown_to_telegram_html("## My Header");
        assert_eq!(html, "<b>My Header</b>");
    }

    #[test]
    fn markdown_to_html_code_block() {
        let html =
            TelegramChannel::markdown_to_telegram_html("```rust\nlet x = 1;\n```");
        assert_eq!(html, "<pre>let x = 1;</pre>");
    }

    #[test]
    fn markdown_to_html_escapes_special_chars() {
        let html = TelegramChannel::markdown_to_telegram_html("a < b & c > d");
        assert_eq!(html, "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn parse_reply_target_simple() {
        let (chat, thread) = TelegramChannel::parse_reply_target("12345");
        assert_eq!(chat, "12345");
        assert!(thread.is_none());
    }

    #[test]
    fn parse_reply_target_with_thread() {
        let (chat, thread) = TelegramChannel::parse_reply_target("-100200300:789");
        assert_eq!(chat, "-100200300");
        assert_eq!(thread.as_deref(), Some("789"));
    }

    #[test]
    fn parse_update_message_basic() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()]);
        let update = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 33,
                "text": "hello",
                "from": {
                    "id": 555,
                    "username": "alice"
                },
                "chat": {
                    "id": -100_200_300
                }
            }
        });

        let msg = ch.parse_update_message(&update).expect("should parse");
        assert_eq!(msg.sender, "alice");
        assert_eq!(msg.reply_target, "-100200300");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel, "telegram");
    }

    #[test]
    fn parse_update_message_rejects_unauthorized() {
        let ch = TelegramChannel::new("token".into(), vec!["alice".into()]);
        let update = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 1,
                "text": "hello",
                "from": {
                    "id": 999,
                    "username": "eve"
                },
                "chat": {
                    "id": 123
                }
            }
        });

        assert!(ch.parse_update_message(&update).is_none());
    }

    #[test]
    fn parse_update_message_with_thread_id() {
        let ch = TelegramChannel::new("token".into(), vec!["*".into()]);
        let update = serde_json::json!({
            "update_id": 3,
            "message": {
                "message_id": 42,
                "text": "hello from topic",
                "from": { "id": 555, "username": "alice" },
                "chat": { "id": -100_200_300 },
                "message_thread_id": 789
            }
        });

        let msg = ch.parse_update_message(&update).expect("should parse");
        assert_eq!(msg.reply_target, "-100200300:789");
    }

    #[test]
    fn parse_update_message_numeric_id_without_username() {
        let ch = TelegramChannel::new("token".into(), vec!["555".into()]);
        let update = serde_json::json!({
            "update_id": 2,
            "message": {
                "message_id": 9,
                "text": "ping",
                "from": { "id": 555 },
                "chat": { "id": 12345 }
            }
        });

        let msg = ch.parse_update_message(&update).expect("should parse");
        assert_eq!(msg.sender, "555");
    }

    #[test]
    fn build_typing_body_with_thread() {
        let body = TelegramChannel::build_typing_action_body("-100200300:789");
        assert_eq!(
            body.get("chat_id").and_then(serde_json::Value::as_str),
            Some("-100200300")
        );
        assert_eq!(
            body.get("message_thread_id")
                .and_then(serde_json::Value::as_str),
            Some("789")
        );
    }

    #[test]
    fn build_typing_body_without_thread() {
        let body = TelegramChannel::build_typing_action_body("12345");
        assert_eq!(
            body.get("chat_id").and_then(serde_json::Value::as_str),
            Some("12345")
        );
        assert!(body.get("message_thread_id").is_none());
    }
}
