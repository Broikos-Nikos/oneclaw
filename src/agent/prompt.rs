// System prompt builder.

use crate::identity::{self, AgentIdentity};
use crate::tools::Tool;
use chrono::Local;
use std::fmt::Write;
use std::path::Path;

/// Build the complete system prompt for an agent.
pub fn build_system_prompt(
    identity: &AgentIdentity,
    soul: &str,
    tools: &[Box<dyn Tool>],
    workspace_dir: &Path,
    model_name: &str,
) -> String {
    let mut prompt = String::new();

    // Identity section
    prompt.push_str(&identity::identity_to_prompt(identity, soul));

    // Tools section
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

    // Safety section
    prompt.push_str("## Safety\n\n");
    prompt.push_str("- Do not exfiltrate private data.\n");
    prompt.push_str("- Do not run destructive commands without confirming.\n");
    prompt.push_str("- Prefer safe operations over destructive ones.\n");
    prompt.push_str("- When in doubt, ask before acting.\n\n");

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
