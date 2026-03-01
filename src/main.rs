#![warn(clippy::all)]
#![forbid(unsafe_code)]

//! OneClaw — Multi-agent AI assistant with router architecture.
//! Fork of ZeroClaw, streamlined for multi-agent orchestration.
//!
//! Only the "main" agent is created by default. Users create sub-agents
//! by adding soul folders and provider configs. All agents share the same
//! workspace and have the same structure — the main agent just has
//! routing/transfer capabilities.

mod agent;
mod config;
mod identity;
mod memory;
mod providers;
mod router;
mod tools;

use crate::agent::Agent;
use crate::config::Config;
use crate::router::Orchestrator;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::sync::Arc;

/// OneClaw — Multi-agent AI assistant with router architecture.
#[derive(Parser, Debug)]
#[command(name = "oneclaw")]
#[command(version)]
#[command(about = "Multi-agent AI assistant. One main agent routes tasks to user-created sub-agents.", long_about = None)]
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
or delegate to a sub-agent. Use --agent to force a specific agent.

Examples:
  oneclaw agent                           # interactive session
  oneclaw agent -m \"Build a REST API\"     # single-shot
  oneclaw agent --agent developer -m \"Fix the bug\"")]
    Agent {
        /// Single message mode
        #[arg(short, long)]
        message: Option<String>,

        /// Force a specific agent by name (bypasses router)
        #[arg(short = 'a', long)]
        agent: Option<String>,
    },

    /// Initialize workspace, config, and main agent
    #[command(long_about = "\
Set up OneClaw for first use.

Creates the config file, workspace directory, and the main agent's
soul folder. Sub-agents can be created later with 'oneclaw create-agent'.

Examples:
  oneclaw onboard              # guided setup
  oneclaw onboard --defaults   # use all defaults")]
    Onboard {
        /// Skip prompts and use all defaults
        #[arg(long)]
        defaults: bool,
    },

    /// Create a new sub-agent
    #[command(long_about = "\
Create a new sub-agent with its own soul folder and optional provider config.

The agent gets a folder in ~/.oneclaw/agents/<name>/ with identity.json
and SOUL.md that you can customize. If no provider is configured for it,
it falls back to the main agent's provider.

Examples:
  oneclaw create-agent developer
  oneclaw create-agent creative --role \"Writer and content creator\"
  oneclaw create-agent researcher")]
    CreateAgent {
        /// Agent name (lowercase, no spaces)
        name: String,

        /// Role description
        #[arg(long)]
        role: Option<String>,
    },

    /// List all configured agents
    ListAgents,

    /// Show current configuration and status
    Status,

    /// Print sample configuration
    SampleConfig,
}

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
        Commands::Agent { message, agent } => cmd_agent(message, agent).await,
        Commands::Onboard { defaults } => cmd_onboard(defaults),
        Commands::CreateAgent { name, role } => cmd_create_agent(&name, role.as_deref()),
        Commands::ListAgents => cmd_list_agents(),
        Commands::Status => cmd_status(),
        Commands::SampleConfig => {
            println!("{}", Config::sample_toml());
            Ok(())
        }
    }
}

/// Run the agent — either single-shot or interactive.
async fn cmd_agent(message: Option<String>, force_agent: Option<String>) -> Result<()> {
    let config = Config::load()?;

    if config.providers.is_empty() {
        eprintln!("❌ No providers configured. Run 'oneclaw onboard' or add providers to your config.");
        eprintln!("   Config location: {}", Config::default_path().display());
        std::process::exit(1);
    }

    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir)?;

    let souls_dir = config.souls_dir();
    let available_agents = identity::discover_agents(&souls_dir);

    let config = Arc::new(config);

    // Build agents
    let mut agents: HashMap<String, Agent> = HashMap::new();

    for agent_name in &available_agents {
        let is_main = agent_name == "main";
        let tools = tools::core_tools(&workspace_dir);
        match Agent::from_config(&config, agent_name, is_main, tools, &available_agents) {
            Ok(agent) => {
                agents.insert(agent_name.clone(), agent);
            }
            Err(e) => {
                if is_main {
                    eprintln!("❌ Failed to initialize main agent: {e}");
                    std::process::exit(1);
                }
                tracing::warn!("Failed to initialize agent '{}': {e}", agent_name);
            }
        }
    }

    let _orchestrator = Orchestrator::new(config.clone());

    if let Some(msg) = message {
        // Single-shot mode
        let (agent_name, response) = if let Some(ref forced) = force_agent {
            if !agents.contains_key(forced.as_str()) {
                eprintln!("❌ Agent '{}' not found. Available: {}", forced, available_agents.join(", "));
                std::process::exit(1);
            }
            let agent = agents.get_mut(forced.as_str()).unwrap();
            let resp = agent.turn(&msg).await?;
            (forced.clone(), resp)
        } else {
            // Route through main agent
            let main = agents.get_mut("main")
                .ok_or_else(|| anyhow::anyhow!("Main agent not found"))?;
            let resp = main.turn(&msg).await?;
            ("main".to_string(), resp)
        };

        let label = console::style(format!("[{}]", agent_name)).cyan().bold();
        println!("{label} {response}");
    } else {
        // Interactive mode
        println!("{}", console::style("🦀 OneClaw — Multi-Agent AI Assistant").bold().cyan());
        println!("Messages go through the main agent, which routes to sub-agents as needed.");
        if available_agents.len() > 1 {
            let subs: Vec<_> = available_agents.iter().filter(|a| a.as_str() != "main").collect();
            println!("Sub-agents: {}", subs.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
        } else {
            println!("No sub-agents configured. Use 'oneclaw create-agent <name>' to add one.");
        }
        println!("Commands: /clear, /status, /agent <name>, /agents, /quit\n");

        let mut rl = rustyline::DefaultEditor::new()?;
        let mut force_name: Option<String> = force_agent;

        loop {
            let prompt = if let Some(ref name) = force_name {
                format!("oneclaw [{}]> ", name)
            } else {
                "oneclaw> ".to_string()
            };

            let line = match rl.readline(&prompt) {
                Ok(line) => line,
                Err(rustyline::error::ReadlineError::Interrupted | rustyline::error::ReadlineError::Eof) => break,
                Err(e) => {
                    eprintln!("Error: {e}");
                    break;
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let _ = rl.add_history_entry(trimmed);

            // Handle commands
            if trimmed.starts_with('/') {
                match trimmed {
                    "/quit" | "/exit" | "/q" => break,
                    "/clear" => {
                        for agent in agents.values_mut() {
                            agent.clear_history();
                        }
                        println!("{}", console::style("History cleared.").dim());
                        continue;
                    }
                    "/agents" | "/status" => {
                        println!("\n{}", console::style("Agents:").bold());
                        for name in &available_agents {
                            let marker = if force_name.as_ref() == Some(name) { " ← forced" } else { "" };
                            let main_tag = if name == "main" { " (router)" } else { "" };
                            println!("  • {name}{main_tag}{marker}");
                        }
                        println!("  Routing: {}", if force_name.is_some() { "bypassed" } else { "main agent decides" });
                        println!();
                        continue;
                    }
                    _ if trimmed.starts_with("/agent ") => {
                        let name = trimmed.strip_prefix("/agent ").unwrap().trim();
                        if name == "auto" || name == "main" {
                            force_name = None;
                            println!("{}", console::style("Routing: main agent decides").dim());
                        } else if agents.contains_key(name) {
                            force_name = Some(name.to_string());
                            println!("{}", console::style(format!("Forced agent: {name}")).dim());
                        } else {
                            eprintln!("Unknown agent: {name}. Available: {}", available_agents.join(", "));
                        }
                        continue;
                    }
                    _ => {
                        eprintln!("Unknown command: {trimmed}");
                        continue;
                    }
                }
            }

            // Route the message
            let target = force_name.as_deref().unwrap_or("main");
            let agent = agents.get_mut(target)
                .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", target))?;

            match agent.turn(trimmed).await {
                Ok(response) => {
                    let label = console::style(format!("[{}]", target)).cyan().bold();
                    println!("\n{label}\n{response}\n");
                }
                Err(e) => {
                    eprintln!("{} {e}", console::style("Error:").red().bold());
                }
            }
        }

        println!("\n{}", console::style("Goodbye! 🦀").dim());
    }

    Ok(())
}

/// Onboard — set up config, workspace, and main agent soul.
fn cmd_onboard(defaults: bool) -> Result<()> {
    println!("{}", console::style("🦀 OneClaw Setup").bold().cyan());
    println!();

    let config = if defaults {
        Config::default_config()
    } else {
        println!("This will set up the main agent. You can add sub-agents later.\n");

        let api_key: String = dialoguer::Input::new()
            .with_prompt("OpenRouter API key")
            .interact_text()?;

        let model: String = dialoguer::Input::new()
            .with_prompt("Model for main agent")
            .default("anthropic/claude-sonnet-4-20250514".into())
            .interact_text()?;

        let mut providers = HashMap::new();
        providers.insert(
            "main".to_string(),
            config::ProviderConfig {
                kind: "openrouter".into(),
                api_key,
                model,
                base_url: None,
                temperature: 0.7,
            },
        );

        Config {
            providers,
            workspace: config::WorkspaceConfig::default(),
            agents: config::AgentsConfig::default(),
        }
    };

    config.save()?;
    println!("✅ Config saved to: {}", Config::default_path().display());

    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir)?;
    println!("✅ Workspace created: {}", workspace_dir.display());

    let souls_dir = config.souls_dir();
    identity::scaffold_main(&souls_dir)?;
    println!("✅ Main agent created: {}/main/", souls_dir.display());

    println!("\n{}", console::style("Setup complete! 🎉").green().bold());
    println!("\nNext steps:");
    println!("  oneclaw create-agent developer    # create a developer sub-agent");
    println!("  oneclaw create-agent creative     # create a creative sub-agent");
    println!("  oneclaw agent                     # start chatting\n");

    Ok(())
}

/// Create a new sub-agent.
fn cmd_create_agent(name: &str, role: Option<&str>) -> Result<()> {
    let config = Config::load()?;
    let souls_dir = config.souls_dir();

    let agent_dir = souls_dir.join(name);
    if agent_dir.exists() {
        eprintln!("Agent '{}' already exists at: {}", name, agent_dir.display());
        std::process::exit(1);
    }

    let custom_identity = role.map(|r| identity::AgentIdentity {
        name: format!("OneClaw {}", capitalize(name)),
        role: r.to_string(),
        ..Default::default()
    });

    let path = identity::create_agent(&souls_dir, name, custom_identity, None)?;

    println!("{}", console::style(format!("✅ Agent '{}' created!", name)).green().bold());
    println!("   Soul folder: {}", path.display());
    println!("   Edit identity.json and SOUL.md to customize behavior.");
    println!();
    println!("   To give it a dedicated model, add to your config:");
    println!("   [providers.{}]", name);
    println!("   kind = \"openrouter\"");
    println!("   api_key = \"sk-...\"");
    println!("   model = \"...\"");
    println!();
    println!("   Without a dedicated provider, it uses the main agent's model.");

    Ok(())
}

/// List all agents.
fn cmd_list_agents() -> Result<()> {
    let config = Config::load()?;
    let souls_dir = config.souls_dir();
    let agents = identity::discover_agents(&souls_dir);

    println!("{}", console::style("Agents:").bold());
    for name in &agents {
        let (id, _) = identity::load_identity(&souls_dir, name)
            .unwrap_or_default();
        let has_provider = config.providers.contains_key(name);
        let provider_tag = if has_provider {
            let pc = &config.providers[name];
            format!(" [{}]", pc.model)
        } else if name != "main" {
            " [falls back to main]".to_string()
        } else {
            String::new()
        };
        let main_tag = if name == "main" { " (router)" } else { "" };

        println!("  • {}{main_tag}: {}{provider_tag}", name, id.role);
    }

    Ok(())
}

/// Show status.
fn cmd_status() -> Result<()> {
    let config_path = Config::default_path();
    println!("{}", console::style("OneClaw Status").bold().cyan());
    println!();

    if config_path.exists() {
        println!("Config: {}", config_path.display());
        let config = Config::load()?;
        println!("Workspace: {}", config.workspace_dir().display());
        println!("Souls: {}", config.souls_dir().display());
        println!();
        cmd_list_agents()?;
    } else {
        println!("Config: {} (not found)", config_path.display());
        println!("\nRun 'oneclaw onboard' to set up.");
    }

    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
