// OneClaw configuration system.
// Agents are dynamic — users create them. Only "main" is required.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Top-level config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Provider configs keyed by agent name (e.g. "main", "developer", "creative").
    /// Each agent can have its own provider/model, or share with another.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
}

/// Provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
}

fn default_temperature() -> f64 {
    0.7
}

/// Workspace configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_workspace_path")]
    pub path: String,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            path: default_workspace_path(),
        }
    }
}

fn default_workspace_path() -> String {
    "~/.oneclaw/workspace".to_string()
}

/// Agents configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default = "default_souls_dir")]
    pub souls_dir: String,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            souls_dir: default_souls_dir(),
        }
    }
}

fn default_souls_dir() -> String {
    "~/.oneclaw/agents".to_string()
}

impl Config {
    /// Load config from the default path.
    pub fn load() -> Result<Self> {
        let config_path = Self::default_path();
        if config_path.exists() {
            Self::from_file(&config_path)
        } else {
            Ok(Self::default_config())
        }
    }

    /// Load config from a specific file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse config TOML")?;
        Ok(config)
    }

    /// Save config to the default path.
    pub fn save(&self) -> Result<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Default config file path.
    pub fn default_path() -> PathBuf {
        let dirs = directories::ProjectDirs::from("", "", "oneclaw")
            .expect("Failed to determine config directory");
        dirs.config_dir().join("config.toml")
    }

    /// Resolve workspace path.
    pub fn workspace_dir(&self) -> PathBuf {
        let expanded = shellexpand::tilde(&self.workspace.path);
        PathBuf::from(expanded.to_string())
    }

    /// Resolve souls directory path.
    pub fn souls_dir(&self) -> PathBuf {
        let expanded = shellexpand::tilde(&self.agents.souls_dir);
        PathBuf::from(expanded.to_string())
    }

    /// Get provider config for a named agent.
    /// Falls back to "main" if no specific config exists for the requested agent.
    pub fn provider_for(&self, agent_name: &str) -> Option<&ProviderConfig> {
        self.providers
            .get(agent_name)
            .or_else(|| self.providers.get("main"))
    }

    /// Generate a default config (no providers configured).
    pub fn default_config() -> Self {
        Config {
            providers: HashMap::new(),
            workspace: WorkspaceConfig::default(),
            agents: AgentsConfig::default(),
        }
    }

    /// Generate a sample config TOML string.
    pub fn sample_toml() -> String {
        r#"# OneClaw Configuration
#
# Agents are dynamic — create as many as you need.
# Each agent name maps to a soul folder in ~/.oneclaw/agents/<name>/
# and a provider config here.
#
# The "main" agent is required. It routes tasks to sub-agents.
# Sub-agents without a specific provider config fall back to "main".

[providers.main]
kind = "openrouter"
api_key = "sk-or-v1-YOUR_KEY_HERE"
model = "anthropic/claude-sonnet-4-20250514"
temperature = 0.7

# Example: a developer agent with a different model
# [providers.developer]
# kind = "openrouter"
# api_key = "sk-or-v1-YOUR_KEY_HERE"
# model = "anthropic/claude-sonnet-4-20250514"
# temperature = 0.3

# Example: a creative agent with higher temperature
# [providers.creative]
# kind = "openrouter"
# api_key = "sk-or-v1-YOUR_KEY_HERE"
# model = "anthropic/claude-sonnet-4-20250514"
# temperature = 0.9

# Example: a cheap/fast agent for simple tasks
# [providers.quick]
# kind = "openrouter"
# api_key = "sk-or-v1-YOUR_KEY_HERE"
# model = "meta-llama/llama-3.3-70b-instruct"
# temperature = 0.5

[workspace]
path = "~/.oneclaw/workspace"

[agents]
souls_dir = "~/.oneclaw/agents"
"#
        .to_string()
    }
}
