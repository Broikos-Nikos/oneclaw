// Health check system.

use crate::config::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Warn,
    Error,
}

impl std::fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckStatus::Ok => write!(f, "OK"),
            CheckStatus::Warn => write!(f, "WARN"),
            CheckStatus::Error => write!(f, "ERROR"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
}

impl Check {
    pub fn ok(name: &str, message: impl Into<String>) -> Self {
        Self { name: name.to_string(), status: CheckStatus::Ok, message: message.into() }
    }
    pub fn warn(name: &str, message: impl Into<String>) -> Self {
        Self { name: name.to_string(), status: CheckStatus::Warn, message: message.into() }
    }
    pub fn error(name: &str, message: impl Into<String>) -> Self {
        Self { name: name.to_string(), status: CheckStatus::Error, message: message.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub checks: Vec<Check>,
    pub overall: CheckStatus,
}

impl HealthReport {
    pub fn is_healthy(&self) -> bool {
        self.overall != CheckStatus::Error
    }
}

/// Run all health checks and return a report.
pub fn run_health_checks(config: &Config) -> Result<HealthReport> {
    let mut checks = Vec::new();

    // Config file check
    let config_path = Config::default_path();
    if config_path.exists() {
        checks.push(Check::ok("config", format!("Found at {}", config_path.display())));
    } else {
        checks.push(Check::warn("config", "Config file not found — run 'oneclaw onboard'"));
    }

    // Provider check
    if config.providers.is_empty() {
        checks.push(Check::error("providers", "No providers configured"));
    } else {
        for (name, provider) in &config.providers {
            // Ollama does not require an API key — local server.
            let needs_key = provider.kind != "ollama";
            if needs_key && provider.api_key.is_empty() {
                checks.push(Check::error(
                    &format!("provider.{name}"),
                    format!("API key is empty (kind={})", provider.kind),
                ));
            } else if provider.model.is_empty() {
                checks.push(Check::error(
                    &format!("provider.{name}"),
                    "Model not set",
                ));
            } else {
                checks.push(Check::ok(
                    &format!("provider.{name}"),
                    format!("{} / {}", provider.kind, provider.model),
                ));
            }
        }
    }

    // Workspace check
    let workspace = config.workspace_dir();
    if workspace.exists() {
        checks.push(Check::ok("workspace", format!("{}", workspace.display())));
    } else {
        checks.push(Check::warn("workspace", format!("{} (not yet created)", workspace.display())));
    }

    // Agents check
    let souls = config.souls_dir();
    let main_soul = souls.join("main");
    if main_soul.exists() {
        checks.push(Check::ok("agents.main", "Soul folder found"));
    } else {
        checks.push(Check::warn("agents.main", "Main agent soul not found — run 'oneclaw onboard'"));
    }

    // Channels check
    if config.channels.telegram.is_some() {
        checks.push(Check::ok("channels.telegram", "Configured"));
    }
    if config.channels.whatsapp.is_some() {
        checks.push(Check::ok("channels.whatsapp", "Configured"));
    }
    if config.channels.telegram.is_none() && config.channels.whatsapp.is_none() {
        checks.push(Check::warn("channels", "No channels configured (CLI only mode)"));
    }

    // Overall status
    let overall = if checks.iter().any(|c| c.status == CheckStatus::Error) {
        CheckStatus::Error
    } else if checks.iter().any(|c| c.status == CheckStatus::Warn) {
        CheckStatus::Warn
    } else {
        CheckStatus::Ok
    };

    Ok(HealthReport { checks, overall })
}

/// Print health report to stdout.
pub fn print_report(report: &HealthReport) {
    let icon = match report.overall {
        CheckStatus::Ok => "✅",
        CheckStatus::Warn => "⚠️ ",
        CheckStatus::Error => "❌",
    };
    println!("{icon} Health: {}\n", report.overall);
    for check in &report.checks {
        let symbol = match check.status {
            CheckStatus::Ok => "✓",
            CheckStatus::Warn => "!",
            CheckStatus::Error => "✗",
        };
        println!("  [{symbol}] {} — {}", check.name, check.message);
    }
}
