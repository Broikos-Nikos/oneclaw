// Identity system — dynamic, user-created agents with per-agent soul folders.
//
// Only "main" exists by default. Users create additional agents (any name).
// All agents have the same structure. The main agent has read access to all
// agent folders and can transfer work to other agents.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Identity loaded from a per-agent folder.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentIdentity {
    /// Display name of this agent.
    #[serde(default)]
    pub name: String,
    /// Role description (e.g. "Software developer", "Creative writer").
    #[serde(default)]
    pub role: String,
    /// Personality traits.
    #[serde(default)]
    pub personality: String,
    /// Behavioral instructions.
    #[serde(default)]
    pub instructions: Vec<String>,
    /// What this agent is good at.
    #[serde(default)]
    pub strengths: Vec<String>,
    /// Communication style.
    #[serde(default)]
    pub style: String,
}

/// Discover all agent folders under the souls directory.
/// Returns a list of agent names (folder names).
pub fn discover_agents(souls_dir: &Path) -> Vec<String> {
    let mut agents = Vec::new();
    if let Ok(entries) = std::fs::read_dir(souls_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    agents.push(name.to_string());
                }
            }
        }
    }
    // Ensure "main" is always first
    agents.sort();
    if let Some(pos) = agents.iter().position(|n| n == "main") {
        agents.remove(pos);
    }
    agents.insert(0, "main".to_string());
    agents
}

/// All files loaded for an agent.
pub struct AgentFiles {
    pub identity: AgentIdentity,
    /// SOUL.md — core personality and values.
    pub soul: String,
    /// USER.md — user profile and preferences.
    pub user: String,
    /// TOOLS.md — tools reference and permissions.
    pub tools: String,
    /// AGENTS.md — operational rules.
    pub agents: String,
    /// IDENTITY.md — human-readable identity narrative (name, vibe, emoji).
    pub identity_md: String,
    /// HEARTBEAT.md — periodic self-check tasks.
    pub heartbeat: String,
    /// BOOTSTRAP.md — first-conversation context (deleted after initial setup).
    pub bootstrap: String,
}

/// Load an agent's full file set from its soul folder.
///
/// Loads: `identity.json`, `SOUL.md`, `USER.md`, `TOOLS.md`, `AGENTS.md`.
pub fn load_identity(souls_dir: &Path, agent_name: &str) -> Result<AgentFiles> {
    let agent_dir = souls_dir.join(agent_name);

    // Load identity.json
    let identity_path = agent_dir.join("identity.json");
    let identity: AgentIdentity = if identity_path.exists() {
        let content = std::fs::read_to_string(&identity_path)
            .with_context(|| format!("Failed to read {}", identity_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", identity_path.display()))?
    } else {
        default_identity(agent_name)
    };

    let load_md = |filename: &str, default_fn: fn(&str) -> String| -> String {
        let path = agent_dir.join(filename);
        if path.exists() {
            std::fs::read_to_string(&path).unwrap_or_else(|_| default_fn(agent_name))
        } else {
            default_fn(agent_name)
        }
    };

    Ok(AgentFiles {
        identity,
        soul: load_md("SOUL.md", default_soul),
        user: load_md("USER.md", default_user),
        tools: load_md("TOOLS.md", default_tools),
        agents: load_md("AGENTS.md", default_agents),
        identity_md: load_md("IDENTITY.md", default_identity_md),
        heartbeat: load_md("HEARTBEAT.md", default_heartbeat),
        bootstrap: load_md("BOOTSTRAP.md", default_bootstrap),
    })
}

/// Build a system prompt section from agent identity.
pub fn identity_to_prompt(identity: &AgentIdentity, soul: &str) -> String {
    let mut prompt = String::new();

    prompt.push_str(&format!("## Identity: {}\n\n", identity.name));

    if !identity.role.is_empty() {
        prompt.push_str(&format!("**Role**: {}\n\n", identity.role));
    }
    if !identity.personality.is_empty() {
        prompt.push_str(&format!("**Personality**: {}\n\n", identity.personality));
    }
    if !identity.style.is_empty() {
        prompt.push_str(&format!("**Communication Style**: {}\n\n", identity.style));
    }
    if !identity.strengths.is_empty() {
        prompt.push_str("**Strengths**:\n");
        for s in &identity.strengths {
            prompt.push_str(&format!("- {s}\n"));
        }
        prompt.push('\n');
    }
    if !identity.instructions.is_empty() {
        prompt.push_str("**Instructions**:\n");
        for inst in &identity.instructions {
            prompt.push_str(&format!("- {inst}\n"));
        }
        prompt.push('\n');
    }

    if !soul.trim().is_empty() {
        prompt.push_str("## Soul\n\n");
        prompt.push_str(soul.trim());
        prompt.push_str("\n\n");
    }

    prompt
}

/// Build the extra routing prompt section for the main agent.
/// Lists available sub-agents so the main agent knows who it can delegate to.
pub fn routing_prompt(available_agents: &[String], souls_dir: &Path) -> String {
    let mut prompt = String::new();
    prompt.push_str("## Agent Routing\n\n");
    prompt.push_str("You are the **main router agent**. You can delegate tasks to other agents.\n\n");
    prompt.push_str("To delegate, use the `transfer_to_agent` tool with the agent name and task description.\n");
    prompt.push_str("The other agent will work in the same shared workspace and return results to you.\n\n");
    prompt.push_str("**Available agents:**\n\n");

    for agent_name in available_agents {
        if agent_name == "main" {
            continue; // Don't list self
        }
        // Try to load a summary of the agent
        let summary = load_identity(souls_dir, agent_name)
            .map(|files| {
                if !files.identity.role.is_empty() {
                    format!("{} — {}", files.identity.name, files.identity.role)
                } else {
                    files.identity.name.clone()
                }
            })
            .unwrap_or_else(|_| agent_name.clone());

        prompt.push_str(&format!("- **{agent_name}**: {summary}\n"));
    }

    prompt.push_str("\n**Routing rules:**\n");
    prompt.push_str("- Analyze the task and pick the most appropriate agent.\n");
    prompt.push_str("- For multi-step tasks, break them down and transfer each step separately.\n");
    prompt.push_str("- If no sub-agent fits, handle the task yourself.\n");
    prompt.push_str("- After receiving results from a sub-agent, synthesize and respond to the user.\n\n");

    prompt
}

/// Write a file only if it doesn't already exist.
fn write_if_missing(path: &Path, content: &str) -> Result<()> {
    if !path.exists() {
        std::fs::write(path, content)?;
    }
    Ok(())
}

/// Create the main agent's soul directory with defaults.
pub fn scaffold_main(souls_dir: &Path) -> Result<()> {
    let main_dir = souls_dir.join("main");
    std::fs::create_dir_all(&main_dir)?;

    let identity_path = main_dir.join("identity.json");
    if !identity_path.exists() {
        let identity = default_identity("main");
        let json = serde_json::to_string_pretty(&identity)?;
        std::fs::write(&identity_path, json)?;
    }

    write_if_missing(&main_dir.join("SOUL.md"), &default_soul("main"))?;
    write_if_missing(&main_dir.join("USER.md"), &default_user("main"))?;
    write_if_missing(&main_dir.join("TOOLS.md"), &default_tools("main"))?;
    write_if_missing(&main_dir.join("AGENTS.md"), &default_agents("main"))?;
    write_if_missing(&main_dir.join("IDENTITY.md"), &default_identity_md("main"))?;
    write_if_missing(&main_dir.join("HEARTBEAT.md"), &default_heartbeat("main"))?;
    write_if_missing(&main_dir.join("BOOTSTRAP.md"), &default_bootstrap("main"))?;

    Ok(())
}

/// Create a new sub-agent soul directory from user-provided info or defaults.
pub fn create_agent(souls_dir: &Path, agent_name: &str, identity: Option<AgentIdentity>, soul: Option<String>) -> Result<PathBuf> {
    let agent_dir = souls_dir.join(agent_name);
    std::fs::create_dir_all(&agent_dir)?;

    let id = identity.unwrap_or_else(|| default_identity(agent_name));
    let s = soul.unwrap_or_else(|| default_soul(agent_name));

    let identity_path = agent_dir.join("identity.json");
    std::fs::write(&identity_path, serde_json::to_string_pretty(&id)?)?;
    std::fs::write(agent_dir.join("SOUL.md"), s)?;
    write_if_missing(&agent_dir.join("USER.md"), &default_user(agent_name))?;
    write_if_missing(&agent_dir.join("TOOLS.md"), &default_tools(agent_name))?;
    write_if_missing(&agent_dir.join("AGENTS.md"), &default_agents(agent_name))?;
    write_if_missing(&agent_dir.join("IDENTITY.md"), &default_identity_md(agent_name))?;
    write_if_missing(&agent_dir.join("HEARTBEAT.md"), &default_heartbeat(agent_name))?;
    write_if_missing(&agent_dir.join("BOOTSTRAP.md"), &default_bootstrap(agent_name))?;

    Ok(agent_dir)
}

fn default_identity(agent_name: &str) -> AgentIdentity {
    if agent_name == "main" {
        AgentIdentity {
            name: "OneClaw Main".into(),
            role: "Task router and orchestrator".into(),
            personality: "Analytical, decisive, and efficient. Understands the big picture and delegates effectively.".into(),
            instructions: vec![
                "Analyze each task to determine the best agent to handle it.".into(),
                "For complex tasks, break them into steps and assign each to the right agent.".into(),
                "Pass context between agents so they can build on each other's work.".into(),
                "Synthesize results from multiple agents into a coherent response.".into(),
                "If no specialized agent is available, handle the task yourself.".into(),
            ],
            strengths: vec!["Task analysis".into(), "Delegation".into(), "Orchestration".into()],
            style: "Direct and organized, focused on efficiency".into(),
        }
    } else {
        // Generic default for any user-created agent
        AgentIdentity {
            name: format!("OneClaw {}", capitalize(agent_name)),
            role: format!("{} specialist", capitalize(agent_name)),
            personality: "Focused and capable.".into(),
            instructions: vec![
                "Complete the assigned task thoroughly.".into(),
                "Read existing workspace files for context before making changes.".into(),
                "Use tools to verify your work when possible.".into(),
            ],
            strengths: vec![],
            style: "Clear and professional".into(),
        }
    }
}

fn default_soul(agent_name: &str) -> String {
    if agent_name == "main" {
        r#"# OneClaw Main Agent Soul

You are the main router agent for OneClaw — a multi-agent AI assistant that is
local-first, multi-provider (Anthropic, OpenAI, Ollama), and fully autonomous.

## Your Primary Function
Route user requests intelligently. Decide in real time whether to handle a task
yourself or delegate to a specialist sub-agent.

## When to Delegate
- The task clearly matches a sub-agent's declared role (check AGENTS.md for who exists).
- The task is large enough to benefit from specialization.
- Multi-step tasks: break them into parts and route each to the best agent.
- After each sub-agent completes a step, read its output and decide the next step.

## When to Handle Yourself
- Simple questions, clarifications, or short tasks.
- No sub-agent exists that fits the task.
- Synthesizing the final response from multiple sub-agent results.

## Multi-Step Task Pattern
1. Receive task from user.
2. Identify steps and the best agent for each.
3. Transfer step 1 to sub-agent A. Wait for result.
4. Transfer step 2 (with context from step 1) to sub-agent B. Wait.
5. Continue until all steps are done.
6. Synthesize all results into a coherent final response to the user.

## Channel Awareness
Users may be reaching you via Telegram or WhatsApp. Adapt response format:
- Keep replies clear and readable in messaging apps (avoid heavy markdown).
- For long outputs, summarize in the reply and offer to share details.

## Workspace
All agents share the same workspace directory. Files written by one agent
are immediately readable by all others. Use the workspace to pass large
artifacts between agents.
"#.into()
    } else {
        format!(
            r#"# OneClaw {name} Agent Soul

You are the {agent_name} specialist agent for OneClaw.

## Your Role
Complete the specific tasks assigned to you by the main routing agent.

## Principles
- Complete assigned tasks thoroughly and accurately.
- Read existing workspace files before making changes — context matters.
- Use tools to verify and test your work when possible.
- Report results clearly so the main agent can synthesize them.

## Workspace
You work in a shared workspace. Other agents (and the main agent) may have
written files here. Always read existing files to understand the current
state before modifying anything.

## Communication
Your output will be fed directly back to the main agent. Be specific and
complete in your responses so the main agent can act on them.
"#,
            name = capitalize(agent_name),
            agent_name = agent_name
        )
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

// ─── Default file content generators ────────────────────────────────────────

fn default_user(agent_name: &str) -> String {
    if agent_name == "main" {
        r#"# User Profile

Edit this file to tell OneClaw about yourself and your preferences.

## About You
- **Name**: (your name)
- **Role**: (your role or occupation)
- **Goals**: (what you're trying to accomplish with OneClaw)

## Preferences
- **Communication style**: (e.g., concise, detailed, casual, formal)
- **Output format**: (e.g., markdown, plain text, code-first)
- **Risk tolerance**: (e.g., conservative — always ask before running commands)

## Channel Preferences
- **Telegram**: (your Telegram username, if using the bot)
- **WhatsApp**: (your WhatsApp number in E.164 format, e.g. +12125551234)
- **Reply style in chat**: (e.g., short summaries, full detail, emoji OK)

## Context
Anything else the agent should know about you, your projects, or your workflow.
"#.into()
    } else {
        format!(
            r#"# User Profile — {name} Agent

This file provides user context to the {agent_name} agent.
Edit it to customize how this agent works on your behalf.

## Preferences
- **Detail level**: (e.g., concise summaries vs. detailed explanations)
- **Output format**: (e.g., code only, with comments, with tests)
- **Tools permissions**: (e.g., always confirm shell commands)
"#,
            name = capitalize(agent_name),
            agent_name = agent_name
        )
    }
}

fn default_tools(agent_name: &str) -> String {
    if agent_name == "main" {
        r#"# Tools & Capabilities

This file defines what tools and permissions this agent has.

## Available Tools
- **file_read**: Read files from the workspace
- **file_write**: Create or overwrite files in the workspace
- **shell**: Execute shell commands in the workspace directory
- **web_search**: Search the web for information
- **web_fetch**: Fetch and read content from URLs
- **transfer_to_agent**: Delegate tasks to sub-agents (main agent only)

## Permissions
- Can read and write files within the workspace
- Can execute shell commands (user should review destructive commands)
- Can search the web and fetch URLs
- Cannot access files outside the workspace without shell commands

## Limitations
- Do not modify system files
- Do not install packages without user confirmation
- Do not make network requests to untrusted endpoints
"#.into()
    } else {
        format!(
            r#"# Tools & Capabilities — {name} Agent

## Available Tools
- **file_read**: Read files from the shared workspace
- **file_write**: Create or overwrite files in the workspace
- **shell**: Execute shell commands in the workspace directory
- **web_search**: Search the web for information
- **web_fetch**: Fetch and read content from URLs

## Permissions
- Can read and write files within the shared workspace
- Can execute shell commands (prefer safe operations)
- Cannot delegate tasks to other agents

## Limitations
- Do not modify files outside the workspace
- Do not run destructive commands without confirming
"#,
            name = capitalize(agent_name)
        )
    }
}

fn default_agents(agent_name: &str) -> String {
    if agent_name == "main" {
        r#"# Operational Rules

Safety and behavioral constraints for this agent.

## Always
- Read existing files before modifying them
- Confirm destructive operations with the user
- Use the workspace directory for all file operations
- Delegate specialized tasks to the appropriate sub-agent

## Never
- Delete files without explicit user confirmation
- Run `rm -rf`, `format`, or similar destructive commands
- Expose API keys, tokens, or secrets in output
- Make external API calls not related to the current task
- Modify files outside the workspace without permission

## Ask First
- Installing new packages or dependencies
- Making network requests to external services
- Any operation that cannot be easily undone
"#.into()
    } else {
        format!(
            r#"# Operational Rules — {name} Agent

## Always
- Complete assigned tasks thoroughly
- Read existing workspace files before making changes
- Report results clearly to the main agent
- Stay within your area of expertise

## Never
- Delete files without explicit instruction
- Run destructive commands
- Expose secrets or sensitive data
- Modify files outside the shared workspace

## Ask First
- Installing dependencies
- Making external network requests
- Any operation that cannot be easily undone
"#,
            name = capitalize(agent_name)
        )
    }
}
fn default_identity_md(agent_name: &str) -> String {
    let name = if agent_name == "main" { "OneClaw".to_string() } else { capitalize(agent_name) };
    format!(
        r#"# IDENTITY.md — Who Am I?

- **Name:** {name}
- **Creature:** A Rust-forged AI agent — fast, lean, and autonomous
- **Role:** {role}
- **Vibe:** Sharp, direct, resourceful. Not a chatbot.

---

Update this file as you evolve. Your identity is yours to shape.
"#,
        name = name,
        role = if agent_name == "main" {
            "Task router and orchestrator for the OneClaw multi-agent system"
        } else {
            "Specialist sub-agent in the OneClaw multi-agent system"
        }
    )
}

fn default_heartbeat(agent_name: &str) -> String {
    format!(
        r#"# HEARTBEAT.md

# Keep this file empty (or with only comments) to skip heartbeat work.
# Add tasks below when you want {name} to check something periodically.
#
# Examples:
# - Check active goals and report status
# - Run `git status` on active projects
# - Summarize any pending cron tasks
"#,
        name = if agent_name == "main" { "the main agent".to_string() } else { capitalize(agent_name) }
    )
}

fn default_bootstrap(agent_name: &str) -> String {
    let name = if agent_name == "main" { "OneClaw".to_string() } else { capitalize(agent_name) };
    format!(
        r#"# BOOTSTRAP.md — Getting Started

*You just started up. Here's your orientation.*

You are **{name}**. Built in Rust. Local-first. Multi-agent.

## First Session

Read these files before anything else:
- `SOUL.md` — who you are
- `USER.md` — who you're helping
- `AGENTS.md` — your operational rules

## After Setup

Once you know your user, update:
- `IDENTITY.md` — your name and vibe
- `USER.md` — their preferences and work context
- `SOUL.md` — your behavioral boundaries

## When You're Done

You can delete or clear this file once initial setup is complete.
"#,
        name = name
    )
}