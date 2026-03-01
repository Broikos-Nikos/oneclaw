// Multi-agent orchestrator with dynamic agent names.
//
// The main agent uses its LLM intelligence to decide routing.
// Sub-agents are discovered from config/soul folders — any name the user creates.

use crate::agent::{Agent, TurnResult};
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

        let result = main_agent.turn(message).await?;

        match result {
            TurnResult::Response(response) => Ok(("main".to_string(), response)),
            TurnResult::Transfer { target_agent, task } => {
                // If the main agent wants to delegate, run the sub-agent
                if let Some(sub) = agents.get_mut(&target_agent) {
                    let sub_result = sub.turn(&task).await?;
                    match sub_result {
                        TurnResult::Response(resp) => Ok((target_agent, resp)),
                        _ => Ok((target_agent, "[Sub-agent delegation not supported]".into())),
                    }
                } else {
                    anyhow::bail!("Agent '{}' not found", target_agent)
                }
            }
        }
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

        let result = agent.turn(message).await?;
        match result {
            TurnResult::Response(resp) => Ok(resp),
            TurnResult::Transfer { .. } => Ok("[Agent attempted delegation]".into()),
        }
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

            let result = agent.turn(&message).await?;
            let output = match result {
                TurnResult::Response(resp) => resp,
                TurnResult::Transfer { .. } => "[Agent attempted delegation]".into(),
            };

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
