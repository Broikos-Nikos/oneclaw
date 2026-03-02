// Agent module — core agent struct, prompt builder, and conversation loop.
// All agents have the same structure. The main agent additionally gets a
// routing prompt and a transfer_to_agent tool.

pub mod dispatcher;
pub mod prompt;
pub mod task_manager;

use crate::config::Config;
use crate::identity;
use crate::providers::ConversationMessage;
use crate::tools::Tool;
use anyhow::Result;

/// Run a message through any agent with full recursive delegation support.
///
/// Creates a fresh agent instance for the given name, runs the task, and
/// if the agent delegates to another agent (via `transfer_to_agent`), builds
/// that sub-agent, runs it, feeds results back, and loops until done.
///
/// `depth` prevents infinite delegation loops (max 4 levels).
/// Use `is_main = true` only for the "main" agent.
pub async fn run_agent_task(
    config: &Config,
    agent_name: &str,
    task: &str,
    is_main: bool,
    depth: u32,
) -> anyhow::Result<String> {
    const MAX_DEPTH: u32 = 4;
    if depth >= MAX_DEPTH {
        tracing::warn!("Max delegation depth ({MAX_DEPTH}) reached at agent '{agent_name}'");
        return Ok(format!("[Max delegation depth reached — unable to delegate further]"));
    }

    let workspace_dir = config.workspace_dir();
    let souls_dir = config.souls_dir();
    let available = crate::identity::discover_agents(&souls_dir);
    let tools = crate::tools::core_tools(&workspace_dir);

    let mut agent = Agent::from_config(config, agent_name, is_main, tools, &available)?;
    let mut current = agent.turn(task).await?;

    loop {
        match current {
            TurnResult::Response(text) => return Ok(text),
            TurnResult::Transfer { target_agent, task: subtask } => {
                tracing::info!("[{agent_name}] delegating to [{target_agent}]: {subtask}");
                // Recursively run the sub-agent (depth+1 prevents infinite loops)
                let sub_result = Box::pin(run_agent_task(
                    config,
                    &target_agent,
                    &subtask,
                    false,
                    depth + 1,
                ))
                .await
                .unwrap_or_else(|e| format!("[{target_agent} error: {e}]"));
                // Feed sub-agent result back so main agent can continue
                current = agent.continue_with_result(&target_agent, &sub_result).await?;
            }
        }
    }
}

/// Result of a single agent turn — either a final response or a transfer request.
#[derive(Debug)]
pub enum TurnResult {
    /// Agent produced a final response.
    Response(String),
    /// Agent wants to transfer a task to another agent.
    Transfer {
        target_agent: String,
        task: String,
    },
}

/// An Agent instance — holds provider, tools, history, and identity.
pub struct Agent {
    /// Name of this agent (e.g. "main", "developer", "creative", or any user-defined name).
    pub name: String,
    /// Whether this is the main (router) agent.
    pub is_main: bool,
    provider: Box<dyn crate::providers::Provider>,
    tools: Vec<Box<dyn Tool>>,
    history: Vec<ConversationMessage>,
    system_prompt: String,
    temperature: f64,
    max_history: usize,
}

impl Agent {
    /// Create an agent from config for a named agent.
    /// If `is_main` is true, the agent gets the routing prompt and transfer tool.
    pub fn from_config(
        config: &Config,
        agent_name: &str,
        is_main: bool,
        mut tools: Vec<Box<dyn Tool>>,
        available_agents: &[String],
    ) -> Result<Self> {
        let provider_config = config
            .provider_for(agent_name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No provider configured for agent '{}'. Add [providers.{}] to your config.",
                    agent_name,
                    agent_name
                )
            })?;

        let provider: Box<dyn crate::providers::Provider> = match provider_config.kind.as_str() {
            "anthropic" => {
                Box::new(crate::providers::anthropic::AnthropicProvider::new(
                    &provider_config.api_key,
                    &provider_config.model,
                    provider_config.base_url.as_deref(),
                    Some(agent_name),
                ))
            }
            "openai" => {
                // Native OpenAI API (api.openai.com)
                Box::new(crate::providers::openai::OpenAIProvider::new(
                    &provider_config.api_key,
                    &provider_config.model,
                    provider_config.base_url.as_deref(),
                    Some(agent_name),
                ))
            }
            "ollama" => {
                Box::new(crate::providers::ollama::OllamaProvider::new(
                    &provider_config.model,
                    provider_config.base_url.as_deref(),
                    Some(agent_name),
                ))
            }
            kind => {
                // "compatible" or any unknown kind → OpenAI-compatible endpoint
                // base_url MUST be set in config for this path.
                let base_url = provider_config.base_url.as_deref().unwrap_or_else(|| {
                    tracing::warn!(
                        "Provider kind '{}' requires base_url in config — defaulting to localhost:8080",
                        kind
                    );
                    "http://localhost:8080/v1"
                });
                Box::new(crate::providers::compatible::CompatibleProvider::new(
                    &provider_config.api_key,
                    &provider_config.model,
                    base_url,
                    Some(agent_name),
                ))
            }
        };

        let souls_dir = config.souls_dir();
        let agent_files = identity::load_identity(&souls_dir, agent_name)?;

        // Build system prompt
        let mut system_prompt = prompt::build_system_prompt(
            &agent_files,
            &tools,
            &config.workspace_dir(),
            &provider_config.model,
        );

        // Main agent gets the routing section
        if is_main && available_agents.len() > 1 {
            system_prompt.push_str(&identity::routing_prompt(available_agents, &souls_dir));

            // Add a transfer_to_agent tool to the tools list
            tools.push(Box::new(TransferTool {
                available_agents: available_agents
                    .iter()
                    .filter(|a| a.as_str() != "main")
                    .cloned()
                    .collect(),
            }));
        }

        Ok(Self {
            name: agent_name.to_string(),
            is_main,
            provider,
            tools,
            history: Vec::new(),
            system_prompt,
            temperature: provider_config.temperature,
            max_history: 50,
        })
    }

    /// Get conversation history.
    pub fn history(&self) -> &[ConversationMessage] {
        &self.history
    }

    /// Clear conversation history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Inject the result of a completed background sub-agent into this agent's
    /// conversation history. The main agent will be aware of it on the next turn.
    pub fn inject_result(&mut self, agent_name: &str, result: &str) {
        self.history.push(ConversationMessage {
            role: "user".into(),
            content: format!(
                "Tool results:\n\n<tool_result name=\"transfer_to_agent\">\nAgent '{agent_name}' completed:\n\n{result}\n</tool_result>"
            ),
        });
        self.trim_history();
    }

    /// Run a single turn: send a user message, handle tool calls, return final response.
    /// If the agent uses transfer_to_agent, returns TurnResult::Transfer so the caller
    /// can execute the sub-agent and feed the result back.
    pub async fn turn(&mut self, user_message: &str) -> Result<TurnResult> {
        self.history.push(ConversationMessage {
            role: "user".into(),
            content: user_message.to_string(),
        });

        self.trim_history();

        // Build messages with system prompt
        let mut messages = vec![ConversationMessage {
            role: "system".into(),
            content: self.system_prompt.clone(),
        }];
        messages.extend(self.history.clone());

        let mut iterations = 0;
        let max_iterations = 10;

        loop {
            iterations += 1;
            if iterations > max_iterations {
                tracing::warn!("Agent loop hit max iterations ({max_iterations})");
                break;
            }

            let response = self
                .provider
                .chat(&messages, Some(self.temperature))
                .await?;

            let content = &response.content;

            // Check for tool calls in the response
            let tool_calls = dispatcher::parse_tool_calls(content);

            if tool_calls.is_empty() {
                // No tool calls — this is the final response
                self.history.push(ConversationMessage {
                    role: "assistant".into(),
                    content: content.clone(),
                });
                return Ok(TurnResult::Response(content.clone()));
            }

            // Execute each tool call and collect results
            let mut tool_results = String::new();
            for call in &tool_calls {
                let result = dispatcher::execute_tool_call(call, &self.tools).await;

                // Check for transfer_to_agent marker
                if result.success && result.output.starts_with("[TRANSFER_PENDING:") {
                    if let Some(transfer) = parse_transfer_marker(&result.output) {
                        // Save the assistant message that requested the transfer
                        self.history.push(ConversationMessage {
                            role: "assistant".into(),
                            content: content.clone(),
                        });
                        return Ok(TurnResult::Transfer {
                            target_agent: transfer.0,
                            task: transfer.1,
                        });
                    }
                }

                tool_results.push_str(&format!(
                    "<tool_result name=\"{}\">\n{}\n</tool_result>\n\n",
                    call.name,
                    if result.success {
                        &result.output
                    } else {
                        result.error.as_deref().unwrap_or("Unknown error")
                    }
                ));
            }

            // Add to conversation
            messages.push(ConversationMessage {
                role: "assistant".into(),
                content: content.clone(),
            });
            messages.push(ConversationMessage {
                role: "user".into(),
                content: format!("Tool results:\n\n{tool_results}"),
            });

            self.history.push(ConversationMessage {
                role: "assistant".into(),
                content: content.clone(),
            });
            self.history.push(ConversationMessage {
                role: "user".into(),
                content: format!("Tool results:\n\n{tool_results}"),
            });
        }

        Ok(TurnResult::Response("[Agent reached maximum tool iterations]".into()))
    }

    /// Continue a turn after receiving a sub-agent result.
    /// Feeds the sub-agent's response back as a tool result and continues the conversation.
    pub async fn continue_with_result(&mut self, agent_name: &str, result: &str) -> Result<TurnResult> {
        let tool_result_msg = format!(
            "<tool_result name=\"transfer_to_agent\">\nAgent '{agent_name}' completed the task:\n\n{result}\n</tool_result>"
        );

        // Feed the sub-agent result back
        self.history.push(ConversationMessage {
            role: "user".into(),
            content: format!("Tool results:\n\n{tool_result_msg}"),
        });

        self.trim_history();

        let mut messages = vec![ConversationMessage {
            role: "system".into(),
            content: self.system_prompt.clone(),
        }];
        messages.extend(self.history.clone());

        let mut iterations = 0;
        let max_iterations = 10;

        loop {
            iterations += 1;
            if iterations > max_iterations {
                break;
            }

            let response = self.provider.chat(&messages, Some(self.temperature)).await?;
            let content = &response.content;
            let tool_calls = dispatcher::parse_tool_calls(content);

            if tool_calls.is_empty() {
                self.history.push(ConversationMessage {
                    role: "assistant".into(),
                    content: content.clone(),
                });
                return Ok(TurnResult::Response(content.clone()));
            }

            let mut tool_results = String::new();
            for call in &tool_calls {
                let result = dispatcher::execute_tool_call(call, &self.tools).await;

                if result.success && result.output.starts_with("[TRANSFER_PENDING:") {
                    if let Some(transfer) = parse_transfer_marker(&result.output) {
                        self.history.push(ConversationMessage {
                            role: "assistant".into(),
                            content: content.clone(),
                        });
                        return Ok(TurnResult::Transfer {
                            target_agent: transfer.0,
                            task: transfer.1,
                        });
                    }
                }

                tool_results.push_str(&format!(
                    "<tool_result name=\"{}\">\n{}\n</tool_result>\n\n",
                    call.name,
                    if result.success { &result.output } else { result.error.as_deref().unwrap_or("Unknown error") }
                ));
            }

            messages.push(ConversationMessage {
                role: "assistant".into(),
                content: content.clone(),
            });
            messages.push(ConversationMessage {
                role: "user".into(),
                content: format!("Tool results:\n\n{tool_results}"),
            });
            self.history.push(ConversationMessage {
                role: "assistant".into(),
                content: content.clone(),
            });
            self.history.push(ConversationMessage {
                role: "user".into(),
                content: format!("Tool results:\n\n{tool_results}"),
            });
        }

        Ok(TurnResult::Response("[Agent reached maximum tool iterations]".into()))
    }

    fn trim_history(&mut self) {
        if self.history.len() > self.max_history * 2 {
            let trim_to = self.history.len() - self.max_history * 2;
            self.history.drain(..trim_to);
        }
    }
}

// ─── Transfer Tool ──────────────────────────────────────────────────────────

/// A tool that allows the main agent to transfer work to a sub-agent.
/// The actual execution happens in main.rs where we have access to all agents.
/// Here, the tool just validates the request and returns a marker for main.rs to handle.
struct TransferTool {
    available_agents: Vec<String>,
}

#[async_trait::async_trait]
impl Tool for TransferTool {
    fn name(&self) -> &str {
        "transfer_to_agent"
    }

    fn description(&self) -> &str {
        "Transfer a task to another agent. The agent will work on the task in the shared workspace and return results. Use this when a task matches another agent's specialization."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": format!("Name of the agent to transfer to. Available: {}", self.available_agents.join(", "))
                },
                "task": {
                    "type": "string",
                    "description": "Description of the task to delegate. Be specific and include all necessary context."
                }
            },
            "required": ["agent", "task"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<crate::tools::ToolResult> {
        let agent_name = args["agent"].as_str().unwrap_or_default();
        let task = args["task"].as_str().unwrap_or_default();

        if !self.available_agents.iter().any(|a| a == agent_name) {
            return Ok(crate::tools::ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unknown agent '{}'. Available: {}",
                    agent_name,
                    self.available_agents.join(", ")
                )),
            });
        }

        if task.is_empty() {
            return Ok(crate::tools::ToolResult {
                success: false,
                output: String::new(),
                error: Some("Task description is required".into()),
            });
        }

        // Return a marker — the actual delegation is handled by the caller
        // who has access to all agent instances.
        Ok(crate::tools::ToolResult {
            success: true,
            output: format!("[TRANSFER_PENDING:{}] {}", agent_name, task),
            error: None,
        })
    }
}

/// Parse a `[TRANSFER_PENDING:agent_name] task description` marker.
/// Returns (agent_name, task) if the marker is valid.
fn parse_transfer_marker(output: &str) -> Option<(String, String)> {
    let rest = output.strip_prefix("[TRANSFER_PENDING:")?;
    let end_bracket = rest.find(']')?;
    let agent_name = rest[..end_bracket].trim().to_string();
    let task = rest[end_bracket + 1..].trim().to_string();
    if agent_name.is_empty() || task.is_empty() {
        return None;
    }
    Some((agent_name, task))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_transfer_marker_valid() {
        let output = "[TRANSFER_PENDING:developer] Write the homepage HTML";
        let result = parse_transfer_marker(output);
        assert_eq!(
            result,
            Some(("developer".to_string(), "Write the homepage HTML".to_string()))
        );
    }

    #[test]
    fn parse_transfer_marker_empty_task() {
        let output = "[TRANSFER_PENDING:developer] ";
        let result = parse_transfer_marker(output);
        assert_eq!(result, None);
    }

    #[test]
    fn parse_transfer_marker_invalid() {
        let output = "Some random tool output";
        let result = parse_transfer_marker(output);
        assert_eq!(result, None);
    }
}
