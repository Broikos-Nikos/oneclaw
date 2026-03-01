//! Channel subsystem for messaging platform integrations.
//!
//! Each channel implements the [`Channel`] trait defined in [`traits`],
//! which provides a uniform interface for sending/receiving messages.
//! Channels are started based on the runtime configuration.

pub mod telegram;
pub mod traits;
pub mod whatsapp;

pub use traits::{Channel, ChannelMessage, SendMessage};

use crate::config::Config;
use std::sync::Arc;

/// Start all configured channels.
///
/// Returns a receiver for incoming messages from all channels,
/// and a list of channel handles (boxed trait objects) for sending replies.
pub async fn start_channels(
    config: &Config,
) -> anyhow::Result<(
    tokio::sync::mpsc::Receiver<ChannelMessage>,
    Vec<Arc<dyn Channel>>,
)> {
    let (tx, rx) = tokio::sync::mpsc::channel::<ChannelMessage>(256);
    let mut channels: Vec<Arc<dyn Channel>> = Vec::new();

    // Telegram
    if let Some(ref tg_config) = config.channels.telegram {
        if !tg_config.bot_token.is_empty() {
            let tg = telegram::TelegramChannel::new(
                tg_config.bot_token.clone(),
                tg_config.allowed_users.clone(),
            );
            let tg = Arc::new(tg);
            channels.push(tg.clone());

            let listener_tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = tg.listen(listener_tx).await {
                    tracing::error!("Telegram channel listener exited with error: {e}");
                }
            });

            tracing::info!("Telegram channel started");
        }
    }

    // WhatsApp
    if let Some(ref wa_config) = config.channels.whatsapp {
        if !wa_config.access_token.is_empty() && !wa_config.phone_number_id.is_empty() {
            let wa = whatsapp::WhatsAppChannel::new(
                wa_config.access_token.clone(),
                wa_config.phone_number_id.clone(),
                wa_config.verify_token.clone(),
                wa_config.allowed_numbers.clone(),
                wa_config.webhook_port,
            );
            let wa = Arc::new(wa);
            channels.push(wa.clone());

            let listener_tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = wa.listen(listener_tx).await {
                    tracing::error!("WhatsApp channel listener exited with error: {e}");
                }
            });

            tracing::info!("WhatsApp channel started (webhook on port {})", wa_config.webhook_port);
        }
    }

    Ok((rx, channels))
}

