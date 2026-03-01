// Heartbeat — periodic health monitor for daemon mode.

use crate::config::Config;
use crate::health;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{info, warn};

/// Spawn the heartbeat task. Returns a sender to signal shutdown.
/// The heartbeat runs every `interval` seconds and logs health status.
pub fn spawn(config: Arc<Config>, interval_secs: u64, mut shutdown: watch::Receiver<bool>) {
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs);
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // skip immediate first tick

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    match health::run_health_checks(&config) {
                        Ok(report) => {
                            if report.is_healthy() {
                                info!("Heartbeat: healthy ({} checks passed)", report.checks.len());
                            } else {
                                let errors: Vec<_> = report.checks.iter()
                                    .filter(|c| matches!(c.status, health::CheckStatus::Error))
                                    .map(|c| c.name.as_str())
                                    .collect();
                                warn!("Heartbeat: unhealthy — failed: {:?}", errors);
                            }
                        }
                        Err(e) => {
                            warn!("Heartbeat: health check error: {e}");
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Heartbeat: shutting down");
                        return;
                    }
                }
            }
        }
    });
}
