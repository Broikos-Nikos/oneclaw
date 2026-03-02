// OneClaw configuration system.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Top-level config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Provider configs keyed by agent name (e.g. "main", "developer").
    /// Each agent can have its own provider/model, or share with another.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

/// Provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// "anthropic", "openai", "ollama", or any OpenAI-compatible kind
    pub kind: String,
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
}

fn default_temperature() -> f64 { 0.7 }

/// Workspace configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_workspace_path")]
    pub path: String,
}

impl Default for WorkspaceConfig {
    fn default() -> Self { Self { path: default_workspace_path() } }
}

fn default_workspace_path() -> String { "~/.oneclaw/workspace".to_string() }

/// Agents configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default = "default_souls_dir")]
    pub souls_dir: String,
}

impl Default for AgentsConfig {
    fn default() -> Self { Self { souls_dir: default_souls_dir() } }
}

fn default_souls_dir() -> String { "~/.oneclaw/agents".to_string() }

/// Memory configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Backend: markdown (default) | none
    /// markdown — single MEMORY.md file in the workspace, injected at session start
    /// none     — memory disabled
    #[serde(default = "default_memory_backend")]
    pub backend: String,
}

fn default_memory_backend() -> String { "markdown".to_string() }

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: default_memory_backend(),
        }
    }
}

/// A lifecycle hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub name: String,
    /// "pre-tool" or "post-tool"
    pub phase: Option<String>,
    /// Tool name filter ("*" or specific tool name)
    #[serde(default)]
    pub tool_filter: Option<String>,
    /// Shell command to run
    pub command: String,
}

/// Daemon configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_interval_secs: u64,
}

fn default_heartbeat_secs() -> u64 { 60 }

impl Default for DaemonConfig {
    fn default() -> Self { Self { heartbeat_interval_secs: default_heartbeat_secs() } }
}

/// Channels configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub whatsapp: Option<WhatsAppConfig>,
}

/// Telegram channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default = "default_allowed_users")]
    pub allowed_users: Vec<String>,
}

fn default_allowed_users() -> Vec<String> { vec!["*".to_string()] }

/// WhatsApp channel configuration.
///
/// Supports two modes:
/// - **Web mode** (QR / pair-code): set `session_path`. Requires the `whatsapp-web` feature.
/// - **Cloud API mode**: set `access_token` + `phone_number_id`. Requires a Meta Business account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    // ── Cloud API fields ──────────────────────────────────────────────────
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub phone_number_id: Option<String>,
    #[serde(default)]
    pub verify_token: Option<String>,
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,

    // ── Web / QR mode fields ──────────────────────────────────────────────
    /// Path to the WhatsApp Web session database (enables QR/web mode).
    #[serde(default)]
    pub session_path: Option<String>,
    /// Phone number for pair-code linking (optional; leave unset to use QR).
    #[serde(default)]
    pub pair_phone: Option<String>,
    /// Custom pair code (optional; auto-generated when `pair_phone` is set).
    #[serde(default)]
    pub pair_code: Option<String>,

    // ── Shared fields ──────────────────────────────────────────────────────
    #[serde(default = "default_allowed_numbers")]
    pub allowed_numbers: Vec<String>,
}

fn default_webhook_port() -> u16 { 8443 }
fn default_allowed_numbers() -> Vec<String> { vec!["*".to_string()] }

// ─── Voice transcription ─────────────────────────────────────────────────────

/// Voice transcription configuration (Whisper API via Groq).
/// Used by WhatsApp Web channel to transcribe voice messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionConfig {
    /// Enable voice transcription for channels that support it.
    #[serde(default)]
    pub enabled: bool,
    /// API key for the transcription endpoint (falls back to GROQ_API_KEY env var).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Whisper API endpoint URL.
    #[serde(default = "default_transcription_api_url")]
    pub api_url: String,
    /// Whisper model name.
    #[serde(default = "default_transcription_model")]
    pub model: String,
    /// Language hint (ISO-639-1, e.g. "en").
    #[serde(default)]
    pub language: Option<String>,
    /// Maximum voice duration in seconds (longer messages are skipped).
    #[serde(default = "default_transcription_max_duration")]
    pub max_duration_secs: u64,
}

fn default_transcription_api_url() -> String {
    "https://api.groq.com/openai/v1/audio/transcriptions".to_string()
}
fn default_transcription_model() -> String { "whisper-large-v3-turbo".to_string() }
fn default_transcription_max_duration() -> u64 { 120 }

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            api_url: default_transcription_api_url(),
            model: default_transcription_model(),
            language: None,
            max_duration_secs: default_transcription_max_duration(),
        }
    }
}

/// Build a reqwest HTTP client for a named service.
/// Oneclaw does not configure per-service proxies, so this returns a plain client.
pub fn build_runtime_proxy_client(_service_key: &str) -> reqwest::Client {
    reqwest::Client::new()
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::default_path();
        if config_path.exists() {
            Self::from_file(&config_path)
        } else {
            Ok(Self::default())
        }
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        toml::from_str(&content).with_context(|| "Failed to parse config TOML")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn default_path() -> PathBuf {
        let dirs = directories::ProjectDirs::from("", "", "oneclaw")
            .expect("Failed to determine config directory");
        dirs.config_dir().join("config.toml")
    }

    pub fn workspace_dir(&self) -> PathBuf {
        PathBuf::from(shellexpand::tilde(&self.workspace.path).to_string())
    }

    pub fn souls_dir(&self) -> PathBuf {
        PathBuf::from(shellexpand::tilde(&self.agents.souls_dir).to_string())
    }

    /// Data directory for databases (cron, goals, coordination, memory).
    pub fn data_dir() -> PathBuf {
        let dirs = directories::ProjectDirs::from("", "", "oneclaw")
            .expect("Failed to determine data directory");
        dirs.data_local_dir().to_path_buf()
    }

    pub fn provider_for(&self, agent_name: &str) -> Option<&ProviderConfig> {
        self.providers.get(agent_name).or_else(|| self.providers.get("main"))
    }

    pub fn sample_toml() -> String {
        r#"# OneClaw Configuration
#
# Agents are dynamic — create as many as you need.
# Each agent name maps to a soul folder in ~/.oneclaw/agents/<name>/
# and a provider config here.
#
# The "main" agent is required. It routes tasks to sub-agents.
# Sub-agents without a specific provider config fall back to "main".

# ── Anthropic (direct API) ─────────────────────────────────────
[providers.main]
kind = "anthropic"
api_key = "sk-ant-YOUR_KEY_HERE"
model = "claude-sonnet-4-20250514"
temperature = 0.7

# ── OpenAI (direct API) ────────────────────────────────────────
# [providers.main]
# kind = "openai"
# api_key = "sk-YOUR_KEY_HERE"
# model = "gpt-4o"
# temperature = 0.7

# ── Ollama (local, no API key) ─────────────────────────────────
# [providers.main]
# kind = "ollama"
# model = "llama3.2"
# base_url = "http://localhost:11434"    # default, can omit
# temperature = 0.7

# ── OpenAI-compatible custom endpoint ─────────────────────────
# [providers.main]
# kind = "compatible"
# api_key = "YOUR_KEY_OR_EMPTY"
# model = "your-model-name"
# base_url = "http://your-server/v1"    # REQUIRED for compatible kind
# temperature = 0.7

# ── Sub-agent example ─────────────────────────────────────────
# [providers.developer]
# kind = "anthropic"
# api_key = "sk-ant-YOUR_KEY_HERE"
# model = "claude-sonnet-4-20250514"
# temperature = 0.3

# Each sub-agent without its own [providers.<name>] falls back to [providers.main].

[workspace]
path = "~/.oneclaw/workspace"

[agents]
souls_dir = "~/.oneclaw/agents"

# ── Memory ─────────────────────────────────────────────────────
# backend: markdown (default) | none
#   markdown -- single MEMORY.md file in workspace, injected at session start
#   none     -- memory disabled
[memory]
backend = "markdown"

# ── Daemon ─────────────────────────────────────────────────────
[daemon]
heartbeat_interval_secs = 60

# ── Communication Channels ─────────────────────────────────────

# Telegram — create a bot via @BotFather, paste the token.
# Long-polling only, no public URL required.
# [channels.telegram]
# bot_token     = "123456:ABCdef-YOUR_TOKEN"
# allowed_users = ["*"]  # or ["your_username", "numeric_id"]

# WhatsApp — two modes:
#
# Mode A: Web / QR scan (no Meta account needed, requires --features whatsapp-web)
# [channels.whatsapp]
# session_path    = "~/.oneclaw/state/whatsapp-web/session.db"
# pair_phone      = ""            # optional: digits-only phone for pair-code flow
# allowed_numbers = ["*"]
#
# Mode B: Business Cloud API (requires Meta Business account + public webhook URL)
# [channels.whatsapp]
# access_token    = "YOUR_META_ACCESS_TOKEN"
# phone_number_id = "YOUR_PHONE_NUMBER_ID"
# verify_token    = "oneclaw-verify"
# allowed_numbers = ["*"]    # or ["+12125551234"]
# webhook_port    = 8443

# ── Hooks ─────────────────────────────────────────────────────
# Run shell commands before/after tool execution.
# [[hooks]]
# name        = "log-tool"
# phase       = "pre-tool"
# tool_filter = "*"
# command     = "echo \"Tool: $ONECLAW_TOOL\" >> ~/.oneclaw/tool.log"
"#
        .to_string()
    }
}
