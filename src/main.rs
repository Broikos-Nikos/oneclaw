#![warn(clippy::all)]
#![forbid(unsafe_code)]
#![allow(dead_code)]

//! OneClaw — Multi-agent AI assistant with router architecture.
//! Fork of ZeroClaw. Local-first, multi-provider, fully autonomous.

mod agent;
mod channels;
mod config;
mod coordination;
mod cron;
mod daemon;
mod doctor;
mod goals;
mod health;
mod heartbeat;
mod hooks;
mod identity;
mod memory;
mod providers;
mod router;
mod service;
mod tools;
mod update;

use crate::agent::{Agent, TurnResult};
use crate::config::Config;
use crate::cron::{CronStore, TaskKind};
use crate::goals::{GoalStatus, GoalStore};
use crate::memory::MemoryBackend;
use crate::service::ServiceAction;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::str::FromStr;
use std::sync::Arc;

// ─── CLI ─────────────────────────────────────────────────────────────────────

/// OneClaw — Multi-agent AI assistant. Local-first, multi-provider.
#[derive(Parser, Debug)]
#[command(name = "oneclaw")]
#[command(version)]
#[command(about = "Multi-agent AI assistant with local Ollama, Anthropic, and OpenAI support.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the AI agent (interactive or single-shot)
    #[command(long_about = "\
Start the AI agent loop.

The main agent receives your message and decides whether to handle it
or delegate to a sub-agent.

Examples:
  oneclaw agent                           # interactive session
  oneclaw agent -m \"Build a REST API\"     # single-shot
  oneclaw agent --agent developer -m \"Fix the bug\"
  oneclaw agent --memory none             # disable MEMORY.md injection")]
    Agent {
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short = 'a', long)]
        agent: Option<String>,
        /// Memory: pass "none" to disable MEMORY.md injection
        #[arg(long)]
        memory: Option<String>,
    },

    /// Initialize workspace, config, and main agent
    Onboard {
        #[arg(long)]
        defaults: bool,
    },

    /// Start the long-running autonomous daemon (channels + heartbeat + cron)
    #[command(long_about = "\
Start the long-running autonomous daemon.

Launches all configured channels (Telegram/WhatsApp), the heartbeat
monitor, and the cron scheduler. Use 'oneclaw service install' to
register it as an OS service.

Examples:
  oneclaw daemon")]
    Daemon,

    /// Run diagnostics (config, providers, channels, connectivity)
    #[command(name = "doctor")]
    Doctor,

    /// Check for updates / self-update
    Update {
        /// Check for updates without installing
        #[arg(long, conflicts_with = "force")]
        check: bool,
        /// Force reinstall even if already at latest
        #[arg(long)]
        force: bool,
    },

    /// Manage cron-scheduled tasks
    #[command(subcommand)]
    Cron(CronCommands),

    /// Manage long-term goals
    #[command(subcommand)]
    Goal(GoalCommands),

    /// Manage memory entries
    #[command(subcommand)]
    Memory(MemoryCommands),

    /// Manage OS service (install/uninstall/status)
    #[command(subcommand)]
    Service(ServiceCommands),

    /// Create a new sub-agent
    CreateAgent {
        name: String,
        #[arg(long)]
        role: Option<String>,
    },

    /// List all configured agents
    ListAgents,

    /// Show current configuration and status
    Status,

    /// Show health check results
    Health,

    /// Print sample configuration
    SampleConfig,
}

#[derive(Subcommand, Debug)]
enum CronCommands {
    /// List all scheduled tasks
    List,
    /// Add a cron task (cron expression)
    Add {
        /// Task name
        name: String,
        /// Message to send to the agent
        message: String,
        /// Cron expression (5-field, e.g. "0 9 * * 1-5")
        #[arg(long)]
        cron: Option<String>,
        /// Interval in seconds
        #[arg(long)]
        every: Option<u64>,
        /// Fire once at RFC3339 timestamp
        #[arg(long)]
        at: Option<String>,
    },
    /// Enable a cron task by ID
    Enable { id: String },
    /// Disable a cron task by ID
    Disable { id: String },
    /// Remove a cron task
    Remove { id: String },
}

#[derive(Subcommand, Debug)]
enum GoalCommands {
    /// List goals
    List {
        #[arg(long)]
        status: Option<String>,
    },
    /// Add a goal
    Add {
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, default_value = "3")]
        priority: u8,
    },
    /// Complete a goal
    Complete { id: String },
    /// Cancel a goal
    Cancel { id: String },
    /// Delete a goal
    Delete { id: String },
}

#[derive(Subcommand, Debug)]
enum MemoryCommands {
    /// List memory entries
    List {
        #[arg(long)]
        category: Option<String>,
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Search memory
    Search {
        query: String,
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Delete a memory entry by ID
    Forget { id: String },
    /// Clear all memories (or by category)
    Clear {
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Show memory statistics
    Stats,
}

#[derive(Subcommand, Debug)]
enum ServiceCommands {
    /// Install as OS service (systemd/launchd/task scheduler)
    Install,
    /// Uninstall OS service
    Uninstall,
    /// Show service status
    Status,
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Agent { message, agent, memory } => cmd_agent(message, agent, memory).await,
        Commands::Onboard { defaults } => cmd_onboard(defaults),
        Commands::Daemon => cmd_daemon().await,
        Commands::Doctor => cmd_doctor().await,
        Commands::Update { check, force } => cmd_update(check, force).await,
        Commands::Cron(sc) => cmd_cron(sc),
        Commands::Goal(sc) => cmd_goal(sc),
        Commands::Memory(sc) => cmd_memory(sc).await,
        Commands::Service(sc) => cmd_service(sc),
        Commands::CreateAgent { name, role } => cmd_create_agent(&name, role.as_deref()),
        Commands::ListAgents => cmd_list_agents(),
        Commands::Status => cmd_status(),
        Commands::Health => cmd_health(),
        Commands::SampleConfig => { println!("{}", Config::sample_toml()); Ok(()) }
    }
}

// ─── Agent ────────────────────────────────────────────────────────────────────

/// Run a message through the main agent with full delegation.
///
/// When the main agent issues `transfer_to_agent`, this function:
/// 1. Builds the requested sub-agent and runs it on the task.
/// 2. Feeds the sub-agent's result back to the main agent via `continue_with_result`.
/// 3. Loops until the main agent produces a final text response.
///
/// The main agent preserves conversation history across calls so the user
/// gets a continuous interactive session.
async fn run_with_delegation(main_agent: &mut Agent, input: &str, config: &Config) -> Result<String> {
    let workspace_dir = config.workspace_dir();
    let souls_dir = config.souls_dir();
    let available_agents = identity::discover_agents(&souls_dir);

    let mut current_result = main_agent.turn(input).await?;

    loop {
        match current_result {
            TurnResult::Response(text) => return Ok(text),
            TurnResult::Transfer { target_agent, task } => {
                println!(
                    "{}",
                    console::style(format!("  → [{target_agent}] working on task...")).dim()
                );

                // Build and run the sub-agent on the delegated task.
                let sub_tools = tools::core_tools(&workspace_dir);
                let sub_result = match Agent::from_config(
                    config,
                    &target_agent,
                    false,
                    sub_tools,
                    &available_agents,
                ) {
                    Ok(mut sub_agent) => {
                        match sub_agent.turn(&task).await {
                            Ok(TurnResult::Response(r)) => {
                                println!(
                                    "{}",
                                    console::style(format!("  ✓ [{target_agent}] completed")).dim()
                                );
                                r
                            }
                            Ok(TurnResult::Transfer { target_agent: t2, task: t2_task }) => {
                                // Sub-agents should not re-delegate; handle gracefully.
                                format!(
                                    "[{target_agent} tried to re-delegate to {t2}: {t2_task}]"
                                )
                            }
                            Err(e) => format!("[{target_agent} error: {e}]"),
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "  ✗ Cannot build agent '{}': {}",
                            target_agent, e
                        );
                        format!("[Failed to create agent '{target_agent}': {e}]")
                    }
                };

                // Feed result back to the main agent and continue the loop.
                current_result = main_agent
                    .continue_with_result(&target_agent, &sub_result)
                    .await?;
            }
        }
    }
}

async fn cmd_agent(
    message: Option<String>,
    agent_name: Option<String>,
    memory_override: Option<String>,
) -> Result<()> {
    let config = Config::load()?;
    let souls_dir = config.souls_dir();
    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir)?;

    let available_agents = identity::discover_agents(&souls_dir);
    if available_agents.is_empty() {
        eprintln!("❌ No agents found. Run 'oneclaw onboard' first.");
        return Ok(());
    }
    let main_name = agent_name.as_deref().unwrap_or("main");
    let is_main = main_name == "main";

    let mut agent = Agent::from_config(&config, main_name, is_main, tools::core_tools(&workspace_dir), &available_agents)?;

    // Build memory if configured
    let memory_backend_str = memory_override
        .as_deref()
        .unwrap_or(&config.memory.backend);
    let memory_backend = MemoryBackend::from_str(memory_backend_str).unwrap_or_default();
    let memory = memory::build_memory(&memory_backend, &workspace_dir).await.unwrap_or(None);

    // Inject recent memories into first turn if any
    let memory_context = if let Some(ref mem) = memory {
        let entries = mem.recall(None, 100).await.unwrap_or_default();
        memory::entries_to_prompt(&entries)
    } else {
        String::new()
    };

    // Start communication channels if configured (background)
    let (mut chan_rx, _) = channels::start_channels(&config).await
        .unwrap_or_else(|_| {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            (rx, vec![])
        });

    match message {
        Some(msg) => {
            // Single-shot mode
            let input = if memory_context.is_empty() {
                msg.clone()
            } else {
                format!("{memory_context}\n{msg}")
            };
            let response = run_with_delegation(&mut agent, &input, &config).await?;
            println!("{response}");

            // Store conversation in memory
            if let Some(ref mem) = memory {
                let _ = mem.store("conversation", &format!("User: {msg}\nAgent: {response}"), None, Some(main_name)).await;
            }
        }
        None => {
            // ── Parallel interactive mode ──────────────────────────────────────
            // Main agent stays responsive while sub-agents run concurrently.
            // Transfers to the same already-running agent are queued (backlinked).
            // Transfers to a different agent spawn a new parallel worker.
            println!(
                "{}",
                console::style(format!(
                    "OneClaw [{main_name}] — type 'exit' to quit, 'tasks' to see running agents"
                ))
                .dim()
            );

            // Readline runs in its own OS thread so the async runtime stays free.
            let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<Option<String>>(16);
            {
                let tx = input_tx;
                std::thread::spawn(move || {
                    let mut rl = rustyline::DefaultEditor::new().expect("readline init");
                    loop {
                        match rl.readline("\u{25b6} ") {
                            Ok(line) => {
                                let trimmed = line.trim().to_string();
                                if !trimmed.is_empty() {
                                    let _ = rl.add_history_entry(&trimmed);
                                }
                                if tx.blocking_send(Some(trimmed)).is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                let _ = tx.blocking_send(None);
                                break;
                            }
                        }
                    }
                });
            }

            // Channel for completed background agent results.
            let (result_tx, mut result_rx) =
                tokio::sync::mpsc::channel::<agent::task_manager::TaskResult>(64);

            let mut task_manager = agent::task_manager::TaskManager::new();
            let mut first_turn = true;
            let main_name_owned = main_name.to_string();

            loop {
                tokio::select! {
                    // ── Background agent finished ──────────────────────────────
                    Some(result) = result_rx.recv() => {
                        println!(
                            "\n{} {} finished:\n{}\n",
                            console::style("\u{2705}").green(),
                            console::style(format!("[{}]", result.agent_name)).bold(),
                            result.output,
                        );
                        // Let the main agent know what the sub-agent produced.
                        agent.inject_result(&result.agent_name, &result.output);

                        // Spawn the next queued (backlinked) task if any.
                        if let Some((next_agent, next_task)) = task_manager.finish(&result.task_id) {
                            let new_id = task_manager.register(&next_agent, &next_task);
                            println!(
                                "{} Running queued task for [{}]...\n",
                                console::style("\u{2192}").dim(),
                                console::style(&next_agent).bold(),
                            );
                            let cfg = config.clone();
                            let tx  = result_tx.clone();
                            let na  = next_agent.clone();
                            let nt  = next_task.clone();
                            tokio::spawn(async move {
                                let output =
                                    agent::run_agent_task(&cfg, &na, &nt, false, 0)
                                        .await
                                        .unwrap_or_else(|e| format!("[{na} error: {e}]"));
                                let _ = tx.send(agent::task_manager::TaskResult {
                                    task_id: new_id,
                                    agent_name: na,
                                    description: nt,
                                    output,
                                }).await;
                            });
                        }
                        // finish() always removes the task from the active list.
                    }

                    // ── User typed a line ──────────────────────────────────────
                    Some(maybe_line) = input_rx.recv() => {
                        let Some(line) = maybe_line else { break; };
                        if line.is_empty() { continue; }
                        if line == "exit" || line == "quit" { break; }

                        // Built-in: list running agents.
                        if line == "tasks" || line == "status" {
                            let active = task_manager.active();
                            if active.is_empty() {
                                println!("{}", console::style("No agents running.").dim());
                            } else {
                                for t in active {
                                    let queued = if t.queued_count() > 0 {
                                        format!("  (+{} queued)", t.queued_count())
                                    } else {
                                        String::new()
                                    };
                                    println!(
                                        "  {} [{}] {}{}",
                                        console::style("\u{26a1}").cyan(),
                                        console::style(&t.agent_name).bold(),
                                        t.description.chars().take(60).collect::<String>(),
                                        queued,
                                    );
                                }
                            }
                            println!();
                            continue;
                        }

                        let input = if first_turn && !memory_context.is_empty() {
                            first_turn = false;
                            format!("{memory_context}\n{line}")
                        } else {
                            first_turn = false;
                            line.clone()
                        };

                        match agent.turn(&input).await? {
                            TurnResult::Response(text) => {
                                println!("\n{text}\n");
                                if let Some(ref mem) = memory {
                                    let _ = mem.store(
                                        "conversation",
                                        &format!("User: {line}\nAgent: {text}"),
                                        None,
                                        Some(&main_name_owned),
                                    ).await;
                                }
                            }
                            TurnResult::Transfer { target_agent, task } => {
                                let existing = task_manager
                                    .find_for_agent(&target_agent)
                                    .map(|id| id.to_string());
                                if let Some(existing_id) = existing {
                                    // Same agent already running — queue the follow-up.
                                    task_manager.enqueue(&existing_id, &task);
                                    println!(
                                        "\n{} Follow-up queued for [{}] — runs after current task\n",
                                        console::style("\u{21a9}").yellow().bold(),
                                        console::style(&target_agent).bold(),
                                    );
                                } else {
                                    // Spawn a new parallel agent.
                                    let task_id = task_manager.register(&target_agent, &task);
                                    let cfg = config.clone();
                                    let tx  = result_tx.clone();
                                    let ta  = target_agent.clone();
                                    let t   = task.clone();
                                    tokio::spawn(async move {
                                        let output =
                                            agent::run_agent_task(&cfg, &ta, &t, false, 0)
                                                .await
                                                .unwrap_or_else(|e| format!("[{ta} error: {e}]"));
                                        let _ = tx.send(agent::task_manager::TaskResult {
                                            task_id,
                                            agent_name: ta,
                                            description: t,
                                            output,
                                        }).await;
                                    });
                                    println!(
                                        "\n{} [{}] agent spawned — running in background\n",
                                        console::style("\u{26a1}").cyan().bold(),
                                        console::style(&target_agent).bold(),
                                    );
                                }
                            }
                        }
                    }

                    // ── Incoming channel message (Telegram / WhatsApp / etc.) ──
                    Some(chan_msg) = chan_rx.recv() => {
                        let prefix = format!("[{}] {}", chan_msg.channel, chan_msg.sender);
                        match agent.turn(&format!("{prefix}: {}", chan_msg.content)).await? {
                            TurnResult::Response(text) => {
                                println!("{}: {text}\n", console::style(&prefix).dim());
                            }
                            TurnResult::Transfer { target_agent, task } => {
                                let existing = task_manager
                                    .find_for_agent(&target_agent)
                                    .map(|id| id.to_string());
                                if let Some(existing_id) = existing {
                                    task_manager.enqueue(&existing_id, &task);
                                } else {
                                    let task_id = task_manager.register(&target_agent, &task);
                                    let cfg = config.clone();
                                    let tx  = result_tx.clone();
                                    let ta  = target_agent.clone();
                                    let t   = task.clone();
                                    tokio::spawn(async move {
                                        let output =
                                            agent::run_agent_task(&cfg, &ta, &t, false, 0)
                                                .await
                                                .unwrap_or_else(|e| format!("[{ta} error: {e}]"));
                                        let _ = tx.send(agent::task_manager::TaskResult {
                                            task_id,
                                            agent_name: ta,
                                            description: t,
                                            output,
                                        }).await;
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// ─── Onboard ──────────────────────────────────────────────────────────────────

fn cmd_onboard(use_defaults: bool) -> Result<()> {
    println!("{}", console::style("OneClaw Setup").bold().cyan());
    println!();

    let config_path = Config::default_path();
    if config_path.exists() && !use_defaults {
        let ok = dialoguer::Confirm::new()
            .with_prompt("Config already exists. Reconfigure?")
            .default(false)
            .interact()?;
        if !ok { return Ok(()); }
    }

    let config = if use_defaults {
        let mut c = Config::default();
        c.providers.insert("main".to_string(), config::ProviderConfig {
            kind: "ollama".to_string(),
            api_key: String::new(),
            model: "llama3.2".to_string(),
            base_url: None,
            temperature: 0.7,
        });
        c
    } else {
        println!("This will set up the main agent.\n");

        // Provider selection
        let provider_options = &[
            "anthropic         — Claude (cloud, API key required)",
            "openai            — GPT-4o / o3-mini (cloud, API key required)",
            "ollama            — local Ollama server (no API key needed)",
            "compatible        — any OpenAI-compatible endpoint (custom base_url)",
        ];
        let provider_idx = dialoguer::Select::new()
            .with_prompt("LLM provider")
            .items(provider_options)
            .default(0)
            .interact()?;
        let kind = match provider_idx {
            0 => "anthropic",
            1 => "openai",
            2 => "ollama",
            _ => "compatible",
        }.to_string();

        let (api_key, models, base_url_override): (String, Vec<&str>, Option<String>) =
            match kind.as_str() {
                "anthropic" => {
                    let key: String = dialoguer::Password::new()
                        .with_prompt("Anthropic API key")
                        .interact()?;
                    (key, vec!["claude-sonnet-4-20250514", "claude-opus-4-20250514", "(custom)"], None)
                }
                "openai" => {
                    let key: String = dialoguer::Password::new()
                        .with_prompt("OpenAI API key")
                        .interact()?;
                    (key, vec!["gpt-4o", "gpt-4.1", "gpt-4.1-mini", "gpt-4.1-nano", "o4-mini", "o3-mini", "(custom)"], None)
                }
                "ollama" => {
                    let url: String = dialoguer::Input::new()
                        .with_prompt("Ollama base URL")
                        .default("http://localhost:11434".into())
                        .interact_text()?;
                    let models_list: Vec<&str> = vec![
                        "llama3.2", "llama3.1:8b", "mistral", "deepseek-r1:8b",
                        "qwen2.5:7b", "phi4", "gemma3:9b", "(custom)",
                    ];
                    (String::new(), models_list, Some(url))
                }
                _ => {
                    // compatible — custom OpenAI-compatible endpoint
                    let url: String = dialoguer::Input::new()
                        .with_prompt("Endpoint base URL (e.g. http://localhost:8080/v1)")
                        .interact_text()?;
                    let key: String = dialoguer::Input::new()
                        .with_prompt("API key (leave empty if none required)")
                        .allow_empty(true)
                        .interact_text()?;
                    let model_name: String = dialoguer::Input::new()
                        .with_prompt("Model name")
                        .interact_text()?;
                    // Already have model — skip model selection by using a sentinel
                    (key, vec![], Some(format!("_custom_model_{model_name}_{url}")))
                }
            };

        // For compatible with pre-filled model, skip the selector
        let (model, base_url) = if kind == "compatible" {
            if let Some(ref enc) = base_url_override {
                // Decode _custom_model_<model>_<url> sentinel
                let stripped = enc.strip_prefix("_custom_model_").unwrap_or("");
                let (model_part, url_part) = stripped
                    .split_once('_')
                    .unwrap_or((stripped, "http://localhost:8080/v1"));
                (model_part.to_string(), Some(url_part.to_string()))
            } else {
                let m: String = dialoguer::Input::new()
                    .with_prompt("Model name")
                    .interact_text()?;
                let u: String = dialoguer::Input::new()
                    .with_prompt("Endpoint base URL")
                    .interact_text()?;
                (m, Some(u))
            }
        } else {
            let model_idx = dialoguer::Select::new()
                .with_prompt("Model")
                .items(&models)
                .default(0)
                .interact()?;
            let m: String = if models[model_idx] == "(custom)" {
                dialoguer::Input::new().with_prompt("Model name").interact_text()?
            } else {
                models[model_idx].to_string()
            };
            (m, base_url_override)
        };

        // ── Channels ─────────────────────────────────────────────────────────
        let channel_items = &["Telegram", "WhatsApp"];
        let channel_selections = dialoguer::MultiSelect::new()
            .with_prompt("Which channels to configure? (space to toggle, enter to confirm)")
            .items(channel_items)
            .interact()?;

        // Telegram
        let telegram = if channel_selections.contains(&0) {
            println!();
            println!("  Telegram Setup");
            println!("  1. Open Telegram and message @BotFather, run /newbot");
            println!("  2. Copy the token and paste it below.");
            println!();
            let token: String = dialoguer::Password::new()
                .with_prompt("  Bot token (from @BotFather)")
                .interact()?;
            if token.trim().is_empty() {
                println!("  → Skipped");
                None
            } else {
                let users_str: String = dialoguer::Input::new()
                    .with_prompt("  Allowed usernames (comma-separated, or * for all)")
                    .default("*".into())
                    .interact_text()?;
                let allowed_users = if users_str.trim() == "*" {
                    vec!["*".to_string()]
                } else {
                    users_str.split(',').map(|s| s.trim().to_string()).collect()
                };
                Some(config::TelegramConfig { bot_token: token.trim().to_string(), allowed_users })
            }
        } else {
            None
        };

        // WhatsApp
        let whatsapp = if channel_selections.contains(&1) {
            println!();
            println!("  WhatsApp Setup");
            println!();
            let wa_mode_options = &[
                "QR / Web  — scan QR in WhatsApp > Linked Devices (no Meta account needed)",
                "Cloud API — Meta Business webhook (requires Meta app + public URL)",
            ];
            let wa_mode = dialoguer::Select::new()
                .with_prompt("  WhatsApp mode")
                .items(wa_mode_options)
                .default(0)
                .interact()?;

            if wa_mode == 0 {
                // QR / Web mode
                println!("  → Build oneclaw with --features whatsapp-web to enable QR mode.");
                println!("  → Start the daemon and scan the QR in WhatsApp > Linked Devices.");
                println!();
                let session_path: String = dialoguer::Input::new()
                    .with_prompt("  Session database path")
                    .default("~/.oneclaw/state/whatsapp-web/session.db".into())
                    .interact_text()?;
                if session_path.trim().is_empty() {
                    println!("  → Skipped — session path required");
                    None
                } else {
                    let pair_phone: String = dialoguer::Input::new()
                        .with_prompt("  Pair phone (optional, digits only; leave empty to use QR flow)")
                        .allow_empty(true)
                        .interact_text()?;
                    let users_str: String = dialoguer::Input::new()
                        .with_prompt("  Allowed phone numbers (comma-separated +E.164, or * for all)")
                        .default("*".into())
                        .interact_text()?;
                    let allowed_numbers = if users_str.trim() == "*" {
                        vec!["*".to_string()]
                    } else {
                        users_str.split(',').map(|s| s.trim().to_string()).collect()
                    };
                    Some(config::WhatsAppConfig {
                        session_path: Some(session_path.trim().to_string()),
                        pair_phone: (!pair_phone.trim().is_empty()).then(|| pair_phone.trim().to_string()),
                        pair_code: None,
                        access_token: None,
                        phone_number_id: None,
                        verify_token: None,
                        webhook_port: 8443,
                        allowed_numbers,
                    })
                }
            } else {
                // Cloud API mode
                println!("  1. Go to developers.facebook.com and create a WhatsApp app");
                println!("  2. Add the WhatsApp product and get your phone number ID");
                println!("  3. Generate an access token (System User)");
                println!("  4. Configure webhook URL to: https://your-domain/webhook");
                println!();
                let token: String = dialoguer::Input::new()
                    .with_prompt("  Access token (from Meta Developers)")
                    .interact_text()?;
                if token.trim().is_empty() {
                    println!("  → Skipped");
                    None
                } else {
                    let phone_id: String = dialoguer::Input::new()
                        .with_prompt("  Phone number ID (from WhatsApp app settings)")
                        .interact_text()?;
                    if phone_id.trim().is_empty() {
                        println!("  → Skipped — phone number ID required");
                        None
                    } else {
                        let verify: String = dialoguer::Input::new()
                            .with_prompt("  Webhook verify token")
                            .default("oneclaw-verify".into())
                            .interact_text()?;
                        let users_str: String = dialoguer::Input::new()
                            .with_prompt("  Allowed numbers (comma-separated +E.164, or * for all)")
                            .default("*".into())
                            .interact_text()?;
                        let allowed_numbers = if users_str.trim() == "*" {
                            vec!["*".to_string()]
                        } else {
                            users_str.split(',').map(|s| s.trim().to_string()).collect()
                        };
                        Some(config::WhatsAppConfig {
                            session_path: None,
                            pair_phone: None,
                            pair_code: None,
                            access_token: Some(token.trim().to_string()),
                            phone_number_id: Some(phone_id.trim().to_string()),
                            verify_token: Some(verify.trim().to_string()),
                            webhook_port: 8443,
                            allowed_numbers,
                        })
                    }
                }
            }
        } else {
            None
        };

        let mut c = Config::default();
        c.providers.insert("main".to_string(), config::ProviderConfig {
            kind,
            api_key,
            model,
            base_url,
            temperature: 0.7,
        });
        c.channels.telegram = telegram;
        c.channels.whatsapp = whatsapp;
        c
    };

    // Save config
    config.save()?;
    println!("✅ Config saved: {}", Config::default_path().display());

    // Scaffold workspace and main agent
    let workspace = config.workspace_dir();
    std::fs::create_dir_all(&workspace)?;
    println!("✅ Workspace: {}", workspace.display());

    let souls_dir = config.souls_dir();
    identity::scaffold_main(&souls_dir)?;
    println!("✅ Main agent (router) created: {}", souls_dir.join("main").display());

    // Init data dir
    std::fs::create_dir_all(&Config::data_dir())?;

    println!();
    println!("{}", console::style("Setup complete!").bold().green());
    println!();
    println!("Next steps:");
    println!("  oneclaw agent                          # start interactive chat");
    println!("  oneclaw create-agent developer \\");
    println!("    --role \"Software engineer\"           # add a specialist sub-agent");
    println!("  oneclaw list-agents                    # see all agents");
    println!("  oneclaw daemon                         # start background daemon (Telegram/WhatsApp)");
    println!();
    if config.channels.telegram.is_some() {
        println!("  → Telegram: start a chat with your bot and message it.");
        println!("    Run 'oneclaw daemon' to receive messages.");
    }
    if config.channels.whatsapp.is_some() {
        let wa = config.channels.whatsapp.as_ref().unwrap();
        if wa.session_path.is_some() {
            println!("  → WhatsApp Web: run 'oneclaw daemon' and scan the QR code shown in the terminal.");
            println!("    Open WhatsApp > Linked Devices > Link a Device.");
        } else {
            println!("  → WhatsApp Cloud API: point your Meta webhook to http://<your-ip>:8443/webhook");
            println!("    Use ngrok or Cloudflare Tunnel to expose the port publicly.");
            println!("    Run 'oneclaw daemon' to start the webhook listener.");
        }
    }
    println!();
    println!("Tip: edit {}  to customize the main agent's personality.", souls_dir.join("main").join("SOUL.md").display());
    Ok(())
}

// ─── Daemon ───────────────────────────────────────────────────────────────────

async fn cmd_daemon() -> Result<()> {
    let config = Arc::new(Config::load()?);
    let daemon_cfg = daemon::DaemonConfig {
        heartbeat_interval_secs: config.daemon.heartbeat_interval_secs,
    };
    daemon::run(config, daemon_cfg).await
}

// ─── Doctor ───────────────────────────────────────────────────────────────────

async fn cmd_doctor() -> Result<()> {
    let config = Config::load()?;
    let report = doctor::run(&config).await?;
    health::print_report(&report);
    doctor::print_diagnostics(&config);
    Ok(())
}

// ─── Update ───────────────────────────────────────────────────────────────────

async fn cmd_update(check: bool, force: bool) -> Result<()> {
    if check {
        update::check().await
    } else {
        update::update(force).await
    }
}

// ─── Cron ─────────────────────────────────────────────────────────────────────

fn cmd_cron(sc: CronCommands) -> Result<()> {
    let store = CronStore::new(&Config::data_dir().join("cron.db"))?;

    match sc {
        CronCommands::List => {
            let tasks = store.list()?;
            cron::print_tasks(&tasks);
        }
        CronCommands::Add { name, message, cron: cron_expr, every, at } => {
            let (kind, schedule) = if let Some(expr) = cron_expr {
                (TaskKind::Cron, expr)
            } else if let Some(secs) = every {
                (TaskKind::Interval, secs.to_string())
            } else if let Some(ts) = at {
                (TaskKind::Once, ts)
            } else {
                anyhow::bail!("Specify --cron, --every, or --at");
            };
            let task = store.add(&name, &message, kind, &schedule)?;
            println!("✅ Task created: {}", task.id);
        }
        CronCommands::Enable { id } => {
            let ok = store.set_enabled(&id, true)?;
            println!("{}", if ok { "✅ Enabled" } else { "❌ Task not found" });
        }
        CronCommands::Disable { id } => {
            let ok = store.set_enabled(&id, false)?;
            println!("{}", if ok { "✅ Disabled" } else { "❌ Task not found" });
        }
        CronCommands::Remove { id } => {
            let ok = store.delete(&id)?;
            println!("{}", if ok { "✅ Removed" } else { "❌ Task not found" });
        }
    }
    Ok(())
}

// ─── Goals ────────────────────────────────────────────────────────────────────

fn cmd_goal(sc: GoalCommands) -> Result<()> {
    let store = GoalStore::new(&Config::data_dir().join("goals.db"))?;

    match sc {
        GoalCommands::List { status } => {
            let filter = status.and_then(|s| s.parse::<GoalStatus>().ok());
            let goals = store.list(filter)?;
            goals::print_goals(&goals);
        }
        GoalCommands::Add { title, description, priority } => {
            let goal = store.add(&title, &description, priority)?;
            println!("✅ Goal created: {}", goal.id);
        }
        GoalCommands::Complete { id } => {
            let ok = store.update_status(&id, GoalStatus::Completed)?;
            println!("{}", if ok { "✅ Completed" } else { "❌ Goal not found" });
        }
        GoalCommands::Cancel { id } => {
            let ok = store.update_status(&id, GoalStatus::Cancelled)?;
            println!("{}", if ok { "✅ Cancelled" } else { "❌ Goal not found" });
        }
        GoalCommands::Delete { id } => {
            let ok = store.delete(&id)?;
            println!("{}", if ok { "✅ Deleted" } else { "❌ Goal not found" });
        }
    }
    Ok(())
}

// ─── Memory ───────────────────────────────────────────────────────────────────

async fn cmd_memory(sc: MemoryCommands) -> Result<()> {
    let config = Config::load()?;
    let backend = MemoryBackend::from_str(&config.memory.backend).unwrap_or_default();
    let workspace_dir = config.workspace_dir();
    let mem = memory::build_memory(&backend, &workspace_dir).await?;

    let Some(mem) = mem else {
        println!("Memory is disabled. Enable it in config.toml: [memory] backend = \"markdown\".");
        return Ok(());
    };

    match sc {
        MemoryCommands::List { category, limit } => {
            let entries = mem.recall(category.as_deref(), limit).await?;
            if entries.is_empty() {
                println!("No memories found.");
            } else {
                for e in &entries {
                    println!("{} [{}] {}: {}", e.id, e.category, e.created_at.format("%Y-%m-%d %H:%M"), e.content);
                }
            }
        }
        MemoryCommands::Search { query, limit } => {
            let entries = mem.search(&query, limit).await?;
            if entries.is_empty() {
                println!("No results.");
            } else {
                for e in &entries {
                    println!("{} [{}] {}", e.id, e.category, e.content);
                }
            }
        }
        MemoryCommands::Forget { id } => {
            let ok = mem.forget(&id).await?;
            println!("{}", if ok { "✅ Deleted" } else { "❌ Entry not found" });
        }
        MemoryCommands::Clear { category, yes } => {
            let confirmed = yes || dialoguer::Confirm::new()
                .with_prompt(format!("Clear {}?", category.as_deref().unwrap_or("ALL memories")))
                .default(false)
                .interact()?;
            if confirmed {
                let n = mem.clear(category.as_deref()).await?;
                println!("✅ Cleared {n} entries");
            }
        }
        MemoryCommands::Stats => {
            let stats = mem.stats().await?;
            println!("MEMORY.md lines: {}", stats.total);
        }
    }
    Ok(())
}

// ─── Service ──────────────────────────────────────────────────────────────────

fn cmd_service(sc: ServiceCommands) -> Result<()> {
    let bin = std::env::current_exe().context("Cannot determine binary path")?;
    match sc {
        ServiceCommands::Install => service::manage(ServiceAction::Install, &bin),
        ServiceCommands::Uninstall => service::manage(ServiceAction::Uninstall, &bin),
        ServiceCommands::Status => service::manage(ServiceAction::Status, &bin),
    }
}

// ─── Create Agent ─────────────────────────────────────────────────────────────

fn cmd_create_agent(name: &str, role: Option<&str>) -> Result<()> {
    if name.is_empty() || name.contains(' ') {
        anyhow::bail!("Agent name must be lowercase with no spaces");
    }
    let config = Config::load()?;
    let souls_dir = config.souls_dir();

    let identity = if let Some(r) = role {
        let id = identity::AgentIdentity {
            name: name.to_string(),
            role: r.to_string(),
            ..Default::default()
        };
        Some(id)
    } else { None };

    let dir = identity::create_agent(&souls_dir, name, identity, None)?;
    println!("✅ Agent '{}' created: {}", name, dir.display());
    println!("   Edit {} to customize personality.", dir.join("SOUL.md").display());
    println!("   Edit {} to describe tools/permissions.", dir.join("TOOLS.md").display());
    println!("   Add a [providers.{}] section in config.toml to use a different model.", name);
    Ok(())
}

// ─── List Agents ─────────────────────────────────────────────────────────────

fn cmd_list_agents() -> Result<()> {
    let config = Config::load()?;
    let souls_dir = config.souls_dir();
    let agents = identity::discover_agents(&souls_dir);

    println!("{}", console::style("Agents:").bold());
    for name in &agents {
        let files = identity::load_identity(&souls_dir, name).ok();
        let role = files.as_ref().map(|f| f.identity.role.as_str()).unwrap_or("(unknown)");
        let has_provider = config.providers.contains_key(name);
        let provider_tag = if has_provider {
            let pc = &config.providers[name];
            format!(" [{} / {}]", pc.kind, pc.model)
        } else if name != "main" {
            " [falls back to main]".to_string()
        } else { String::new() };
        let main_tag = if name == "main" { " (router)" } else { "" };
        println!("  • {}{main_tag}: {}{provider_tag}", name, role);
    }
    Ok(())
}

// ─── Status ──────────────────────────────────────────────────────────────────

fn cmd_status() -> Result<()> {
    let config_path = Config::default_path();
    println!("{}", console::style("OneClaw Status").bold().cyan());
    println!();

    let config = if config_path.exists() {
        Config::from_file(&config_path)?
    } else {
        println!("⚠️  No config found at {}", config_path.display());
        println!("   Run 'oneclaw onboard' to get started.");
        return Ok(());
    };

    println!("Config    : {}", config_path.display());
    println!("Workspace : {}", config.workspace_dir().display());
    println!("Agents    : {}", config.souls_dir().display());
    println!("Memory    : {}", config.memory.backend);
    println!("Heartbeat : {}s", config.daemon.heartbeat_interval_secs);
    println!();

    println!("Providers:");
    for (name, p) in &config.providers {
        println!("  [{name}] {} / {}", p.kind, p.model);
    }

    println!();
    println!("Agents:");
    let souls_dir = config.souls_dir();
    let agents = identity::discover_agents(&souls_dir);
    for name in &agents {
        println!("  • {name}");
    }

    println!();
    println!("Channels:");
    println!("  telegram : {}", if config.channels.telegram.is_some() { "configured" } else { "not configured" });
    println!("  whatsapp : {}", if config.channels.whatsapp.is_some() { "configured" } else { "not configured" });

    Ok(())
}

// ─── Health ──────────────────────────────────────────────────────────────────

fn cmd_health() -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let report = health::run_health_checks(&config)?;
    health::print_report(&report);
    Ok(())
}
