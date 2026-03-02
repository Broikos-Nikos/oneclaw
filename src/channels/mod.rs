//! Channel subsystem for messaging platform integrations.
//!
//! Each channel implements the [`Channel`] trait defined in [`traits`],
//! which provides a uniform interface for sending/receiving messages.
//! Channels are started based on the runtime configuration.

pub mod telegram;
pub mod traits;
pub mod transcription;
pub mod whatsapp;
#[cfg(feature = "whatsapp-web")]
pub mod whatsapp_storage;
#[cfg(feature = "whatsapp-web")]
pub mod whatsapp_web;

pub use traits::{Channel, ChannelMessage, SendMessage};
#[cfg(feature = "whatsapp-web")]
pub use whatsapp_web::WhatsAppWebChannel;

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
        // Mode A: WhatsApp Web (QR / pair-code) — requires --features whatsapp-web
        #[cfg(feature = "whatsapp-web")]
        if let Some(ref session_path) = wa_config.session_path {
            let session_path = shellexpand::tilde(session_path).into_owned();
            let wa = whatsapp_web::WhatsAppWebChannel::new(
                session_path,
                wa_config.pair_phone.clone(),
                wa_config.pair_code.clone(),
                wa_config.allowed_numbers.clone(),
            );
            let wa = Arc::new(wa);
            channels.push(wa.clone());
            let listener_tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = wa.listen(listener_tx).await {
                    tracing::error!("WhatsApp Web channel listener exited with error: {e}");
                }
            });
            tracing::info!("WhatsApp Web channel started (QR / pair-code mode)");
        }

        // Mode B: WhatsApp Cloud API (webhook)
        #[cfg(not(feature = "whatsapp-web"))]
        let _session_path_unused = &wa_config.session_path;

        if wa_config.session_path.is_none() {
            if let (Some(token), Some(phone_id)) = (&wa_config.access_token, &wa_config.phone_number_id) {
                if !token.is_empty() && !phone_id.is_empty() {
                    let wa = whatsapp::WhatsAppChannel::new(
                        token.clone(),
                        phone_id.clone(),
                        wa_config.verify_token.clone().unwrap_or_else(|| "oneclaw-verify".to_string()),
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
                    tracing::info!("WhatsApp channel started (Cloud API webhook on port {})", wa_config.webhook_port);
                }
            }
        }
    }

    Ok((rx, channels))
}

