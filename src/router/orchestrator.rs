// Multi-agent orchestrator with dynamic agent names.
//
// The main agent uses its LLM intelligence to decide routing.
// Sub-agents are discovered from config/soul folders — any name the user creates.

use crate::agent::Agent;
use crate::config::Config;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

/// Result from a delegated step.
#[derive(Debug, Clone)]
pub struct StepResult {
    pub agent_name: String,
    pub task_description: String,
    pub output: String,
}

/// Multi-agent orchestrator.
#[allow(dead_code)]
pub struct Orchestrator {
    config: Arc<Config>,
}

impl Orchestrator {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Route a message through the main agent.
    /// The main agent decides whether to handle it or transfer to a sub-agent.
    /// Returns (agent_name_that_responded, response_text).
    pub async fn route(
        &self,
        message: &str,
        agents: &mut HashMap<String, Agent>,
    ) -> Result<(String, String)> {
        let main_agent = agents
            .get_mut("main")
            .ok_or_else(|| anyhow::anyhow!("Main agent not found"))?;

        let response = main_agent.turn(message).await?;

        // Check if the main agent called transfer_to_agent tool
        // If so, the tool itself handles the delegation and returns results
        // The main agent then synthesizes the response
        Ok(("main".to_string(), response))
    }

    /// Execute a task directly on a named agent.
    pub async fn execute_on(
        &self,
        agent_name: &str,
        message: &str,
        agents: &mut HashMap<String, Agent>,
    ) -> Result<String> {
        let agent = agents
            .get_mut(agent_name)
            .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_name))?;

        agent.turn(message).await
    }

    /// Execute a planned sequence of steps across different agents.
    /// Each step gets context from previous steps.
    pub async fn execute_plan(
        &self,
        steps: Vec<(String, String)>, // (agent_name, task_description)
        agents: &mut HashMap<String, Agent>,
    ) -> Result<Vec<StepResult>> {
        let mut results = Vec::new();
        let mut context = String::new();

        for (i, (agent_name, description)) in steps.iter().enumerate() {
            tracing::info!(
                "Orchestrator: Step {}/{} → {}: {}",
                i + 1,
                steps.len(),
                agent_name,
                description
            );

            // Build message with context from previous steps
            let message = if context.is_empty() {
                description.clone()
            } else {
                format!(
                    "Previous work completed:\n{context}\n\nYour task now:\n{description}"
                )
            };

            let agent = agents
                .get_mut(agent_name.as_str())
                .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found", agent_name))?;

            let output = agent.turn(&message).await?;

            // Accumulate context
            context.push_str(&format!(
                "\n--- Step {} ({}) ---\n{}\n",
                i + 1,
                agent_name,
                &output
            ));

            results.push(StepResult {
                agent_name: agent_name.clone(),
                task_description: description.clone(),
                output,
            });
        }

        Ok(results)
    }
}
