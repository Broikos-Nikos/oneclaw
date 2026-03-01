// System prompt builder.

use crate::identity::{self, AgentFiles};
use crate::tools::Tool;
use chrono::Local;
use std::fmt::Write;
use std::path::Path;

/// Build the complete system prompt for an agent.
pub fn build_system_prompt(
    agent_files: &AgentFiles,
    tools: &[Box<dyn Tool>],
    workspace_dir: &Path,
    model_name: &str,
) -> String {
    let mut prompt = String::new();

    // Identity + Soul section
    prompt.push_str(&identity::identity_to_prompt(&agent_files.identity, &agent_files.soul));

    // IDENTITY.md — human-readable identity narrative
    let id_md = agent_files.identity_md.trim();
    if !id_md.is_empty() {
        prompt.push_str("## Project Context\n\n");
        prompt.push_str("### IDENTITY.md\n\n");
        prompt.push_str(id_md);
        prompt.push_str("\n\n");
    }

    // BOOTSTRAP.md — first-conversation context (if file has real content)
    let bootstrap = agent_files.bootstrap.trim();
    if !bootstrap.is_empty() {
        prompt.push_str("### BOOTSTRAP.md\n\n");
        prompt.push_str(bootstrap);
        prompt.push_str("\n\n");
    }

    // User context section (USER.md)
    let user_trimmed = agent_files.user.trim();
    if !user_trimmed.is_empty() {
        prompt.push_str("## User Context\n\n");
        prompt.push_str(user_trimmed);
        prompt.push_str("\n\n");
    }

    // Tools section — dynamic from Tool trait + TOOLS.md static content
    prompt.push_str("## Available Tools\n\n");
    prompt.push_str("You can use tools by including XML-style tool calls in your response.\n");
    prompt.push_str("Format:\n```\n<tool_call name=\"tool_name\">\n{\"param\": \"value\"}\n</tool_call>\n```\n\n");
    prompt.push_str("Available tools:\n\n");
    for tool in tools {
        let _ = writeln!(
            prompt,
            "- **{}**: {}\n  Parameters: `{}`\n",
            tool.name(),
            tool.description(),
            tool.parameters_schema()
        );
    }

    // Static tools/permissions from TOOLS.md
    let tools_trimmed = agent_files.tools.trim();
    if !tools_trimmed.is_empty() {
        prompt.push_str(tools_trimmed);
        prompt.push_str("\n\n");
    }

    // Safety/rules section from AGENTS.md
    let agents_trimmed = agent_files.agents.trim();
    if !agents_trimmed.is_empty() {
        prompt.push_str(agents_trimmed);
        prompt.push_str("\n\n");
    }

    // HEARTBEAT.md — periodic task checklist (skip if only comments)
    let heartbeat = agent_files.heartbeat.trim();
    let heartbeat_has_tasks = heartbeat.lines().any(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty());
    if heartbeat_has_tasks {
        prompt.push_str("### HEARTBEAT.md\n\n");
        prompt.push_str(heartbeat);
        prompt.push_str("\n\n");
    }

    // Workspace section
    let _ = writeln!(
        prompt,
        "## Workspace\n\nWorking directory: `{}`\n",
        workspace_dir.display()
    );

    // Runtime section
    let host = hostname::get()
        .map_or_else(|_| "unknown".into(), |h| h.to_string_lossy().to_string());
    let _ = writeln!(
        prompt,
        "## Runtime\n\nHost: {host} | OS: {} | Model: {model_name}\n",
        std::env::consts::OS
    );

    // DateTime section
    let now = Local::now();
    let _ = writeln!(
        prompt,
        "## Current Date & Time\n\n{} ({})\n",
        now.format("%Y-%m-%d %H:%M:%S"),
        now.format("%Z")
    );

    prompt
}
