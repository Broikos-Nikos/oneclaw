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

/// Load an agent's identity from its soul folder.
///
/// Looks for `identity.json` and `SOUL.md` in `souls_dir/<agent_name>/`.
pub fn load_identity(souls_dir: &Path, agent_name: &str) -> Result<(AgentIdentity, String)> {
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

    // Load SOUL.md
    let soul_path = agent_dir.join("SOUL.md");
    let soul = if soul_path.exists() {
        std::fs::read_to_string(&soul_path)
            .with_context(|| format!("Failed to read {}", soul_path.display()))?
    } else {
        default_soul(agent_name)
    };

    Ok((identity, soul))
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
            .map(|(id, _)| {
                if !id.role.is_empty() {
                    format!("{} — {}", id.name, id.role)
                } else {
                    id.name.clone()
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

    let soul_path = main_dir.join("SOUL.md");
    if !soul_path.exists() {
        let soul = default_soul("main");
        std::fs::write(&soul_path, soul)?;
    }

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

    let soul_path = agent_dir.join("SOUL.md");
    std::fs::write(&soul_path, s)?;

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

You are the main router agent for OneClaw. Your primary function is to understand
what the user needs and either handle it yourself or delegate to a specialist agent.

## When to Delegate
- If a task clearly matches a sub-agent's role, transfer it.
- For complex tasks, break them into parts and route each part.
- Pass the output of each step as context to the next agent.

## When to Handle Yourself
- Simple tasks that don't need a specialist.
- When no sub-agent is configured for the task type.
- Synthesizing final results from multiple agents.

## Workspace
All agents share the same workspace. Files written by one agent are readable by all.
"#.into()
    } else {
        format!(
            r#"# OneClaw {name} Agent Soul

You are the {agent_name} agent for OneClaw.

## Principles
- Complete assigned tasks thoroughly and accurately.
- Read existing project files before making changes.
- Use tools to verify your work.
- Report results clearly.

## Workspace
You work in a shared workspace. Other agents may have written files here.
Always read existing files to understand the current state before modifying.
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
