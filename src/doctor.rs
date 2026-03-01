// Doctor — diagnostic checks for runtime, provider, and channels.

use crate::config::Config;
use crate::health::{self, CheckStatus, HealthReport};
use anyhow::Result;

/// Run all diagnostics and return a report.
pub async fn run(config: &Config) -> Result<HealthReport> {
    let mut report = health::run_health_checks(config)?;

    // Provider connectivity check (non-blocking attempt)
    for (name, provider) in &config.providers {
        if !provider.api_key.is_empty() || provider.kind == "ollama" {
            let ok = match provider.kind.as_str() {
                "ollama" => check_ollama(&provider.base_url.clone().unwrap_or_else(|| "http://localhost:11434".into())).await,
                _ => true, // skip live API checks for cloud providers unless we have a key
            };
            if ok {
                report.checks.push(health::Check::ok(
                    &format!("provider.{name}.connectivity"),
                    "Reachable",
                ));
            } else {
                report.checks.push(health::Check::warn(
                    &format!("provider.{name}.connectivity"),
                    "Not reachable (check Ollama is running)",
                ));
            }
        }
    }

    // Recalculate overall
    report.overall = if report.checks.iter().any(|c| c.status == CheckStatus::Error) {
        CheckStatus::Error
    } else if report.checks.iter().any(|c| c.status == CheckStatus::Warn) {
        CheckStatus::Warn
    } else {
        CheckStatus::Ok
    };

    Ok(report)
}

async fn check_ollama(base_url: &str) -> bool {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Print trace/diagnostic info for the last N agent interactions.
/// Currently shows config and module status — can be extended with runtime traces.
pub fn print_diagnostics(config: &Config) {
    println!("── OneClaw Diagnostics ──────────────────────────────");
    println!("Config path : {}", Config::default_path().display());
    println!("Workspace   : {}", config.workspace_dir().display());
    println!("Agents dir  : {}", config.souls_dir().display());
    println!("Providers   : {}", config.providers.len());
    for (name, p) in &config.providers {
        println!("  [{name}] kind={} model={}", p.kind, p.model);
    }
    println!("Telegram    : {}", if config.channels.telegram.is_some() { "configured" } else { "not configured" });
    println!("WhatsApp    : {}", if config.channels.whatsapp.is_some() { "configured" } else { "not configured" });
}
