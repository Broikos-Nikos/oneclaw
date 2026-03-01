// Daemon — long-running autonomous runtime.
//
// Launches: channels (Telegram/WhatsApp), heartbeat monitor, cron scheduler.
// Handles graceful shutdown on SIGTERM/SIGINT.

use crate::channels::{Channel, SendMessage};
use crate::config::Config;
use crate::cron::CronStore;
use crate::heartbeat;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::info;

pub struct DaemonConfig {
    pub heartbeat_interval_secs: u64,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self { heartbeat_interval_secs: 60 }
    }
}

/// Run the daemon. Blocks until SIGTERM/SIGINT/ctrl-c.
pub async fn run(config: Arc<Config>, daemon_config: DaemonConfig) -> Result<()> {
    info!("OneClaw daemon starting...");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (cron_msg_tx, mut cron_msg_rx) = mpsc::channel::<String>(32);

    // ── Heartbeat ───────────────────────────────────────────────────────
    heartbeat::spawn(config.clone(), daemon_config.heartbeat_interval_secs, shutdown_rx.clone());
    info!("Heartbeat started ({}s interval)", daemon_config.heartbeat_interval_secs);

    // ── Cron runner ─────────────────────────────────────────────────────
    let data_dir = Config::data_dir();
    let cron_store = Arc::new(CronStore::new(&data_dir.join("cron.db"))?);
    crate::cron::spawn_runner(cron_store.clone(), cron_msg_tx, shutdown_rx.clone());
    info!("Cron scheduler started");

    // ── Channels ────────────────────────────────────────────────────────
    let (mut chan_rx, channel_senders) =
        crate::channels::start_channels(&config).await?;
    let channel_senders: Arc<Vec<Arc<dyn Channel>>> = Arc::new(channel_senders);
    info!(
        "Channels started ({} active)",
        channel_senders.len()
    );

    // ── Channel message handler ──────────────────────────────────────────
    // For each incoming message: run the agent with full delegation, then
    // send the reply back to the user via the same channel.
    {
        let cfg = config.clone();
        let senders = channel_senders.clone();
        let mut srv = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(msg) = chan_rx.recv() => {
                        info!(
                            "Channel message from {} ({}): {}",
                            msg.sender, msg.channel, msg.content
                        );
                        let cfg2 = cfg.clone();
                        let senders2 = senders.clone();
                        let channel_name = msg.channel.clone();
                        let reply_target = msg.reply_target.clone();
                        let content = msg.content.clone();

                        tokio::spawn(async move {
                            let response = crate::agent::run_agent_task(
                                &cfg2, "main", &content, true, 0,
                            )
                            .await;

                            match response {
                                Ok(reply) => {
                                    // Find the originating channel and reply
                                    if let Some(ch) = senders2.iter().find(|c| c.name() == channel_name) {
                                        let m = SendMessage::new(reply, reply_target);
                                        if let Err(e) = ch.send(&m).await {
                                            tracing::error!(
                                                "Failed to send reply via {channel_name}: {e}"
                                            );
                                        }
                                    } else {
                                        tracing::warn!(
                                            "No active sender found for channel '{channel_name}'"
                                        );
                                    }
                                }
                                Err(e) => tracing::warn!("Agent error for channel message: {e}"),
                            }
                        });
                    }
                    _ = srv.changed() => {
                        if *srv.borrow() { break; }
                    }
                }
            }
        });
    }

    // ── Cron message handler ─────────────────────────────────────────────
    // Cron tasks fire agent messages without a reply channel (just log).
    {
        let cfg = config.clone();
        let mut srv = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(msg) = cron_msg_rx.recv() => {
                        info!("Cron task firing: {msg}");
                        let cfg2 = cfg.clone();
                        let m = msg.clone();
                        tokio::spawn(async move {
                            match crate::agent::run_agent_task(&cfg2, "main", &m, true, 0).await {
                                Ok(r) => info!(
                                    "Cron agent response ({} chars): {}",
                                    r.len(),
                                    &r[..r.len().min(200)]
                                ),
                                Err(e) => tracing::warn!("Cron agent error: {e}"),
                            }
                        });
                    }
                    _ = srv.changed() => {
                        if *srv.borrow() { break; }
                    }
                }
            }
        });
    }

    // ── Shutdown signal ──────────────────────────────────────────────────
    wait_for_signal().await;
    info!("OneClaw daemon shutting down...");
    let _ = shutdown_tx.send(true);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    info!("OneClaw daemon stopped.");
    Ok(())
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT handler");
    tokio::select! {
        _ = sigterm.recv() => info!("SIGTERM received"),
        _ = sigint.recv() => info!("SIGINT received"),
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl-c");
    info!("Ctrl+C received");
}
