// Hooks — pre/post tool lifecycle hooks.
//
// Hooks let users run custom logic before or after any tool execution.
// They are defined as shell scripts or inline commands in config.

use crate::config::HookConfig;
use tracing::{debug, warn};

/// Phase of hook execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPhase {
    PreTool,
    PostTool,
}

impl std::fmt::Display for HookPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookPhase::PreTool => write!(f, "pre-tool"),
            HookPhase::PostTool => write!(f, "post-tool"),
        }
    }
}

/// Run hooks for a given phase and tool name.
/// Hooks are run synchronously and failures are warned, not fatal.
pub async fn run_hooks(
    hooks: &[HookConfig],
    phase: HookPhase,
    tool_name: &str,
    workspace: &std::path::Path,
) {
    for hook in hooks {
        // Filter by phase and optional tool filter
        if hook.phase.as_deref() != Some(&phase.to_string()) && hook.phase.is_some() {
            continue;
        }
        if let Some(ref filter) = hook.tool_filter {
            if filter != "*" && filter != tool_name {
                continue;
            }
        }

        debug!("Running {phase} hook '{}' for tool '{tool_name}'", hook.name);

        let result = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&hook.command)
            .current_dir(workspace)
            .env("ONECLAW_HOOK_PHASE", phase.to_string())
            .env("ONECLAW_TOOL", tool_name)
            .output()
            .await;

        match result {
            Ok(out) if out.status.success() => {
                debug!("Hook '{}' succeeded", hook.name);
            }
            Ok(out) => {
                warn!(
                    "Hook '{}' failed (exit {}): {}",
                    hook.name,
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            Err(e) => {
                warn!("Hook '{}' could not be run: {e}", hook.name);
            }
        }
    }
}
