// Agent module — core agent struct, prompt builder, and conversation loop.
// All agents have the same structure. The main agent additionally gets a
// routing prompt and a transfer_to_agent tool.

pub mod dispatcher;
pub mod prompt;

use crate::config::Config;
use crate::identity;
use crate::providers::ConversationMessage;
use crate::tools::Tool;
use anyhow::Result;

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

        let provider = {
            let base_url = match provider_config.kind.as_str() {
                "openai" => provider_config.base_url.as_deref()
                    .unwrap_or("https://api.openai.com/v1"),
                "anthropic" => provider_config.base_url.as_deref()
                    .unwrap_or("https://api.anthropic.com/v1"),
                "openrouter" | _ => provider_config.base_url.as_deref()
                    .unwrap_or("https://openrouter.ai/api/v1"),
            };
            crate::providers::openrouter::OpenRouterProvider::new(
                &provider_config.api_key,
                &provider_config.model,
                Some(base_url),
                Some(agent_name),
            )
        };

        let souls_dir = config.souls_dir();
        let (agent_identity, soul) = identity::load_identity(&souls_dir, agent_name)?;

        // Build system prompt
        let mut system_prompt = prompt::build_system_prompt(
            &agent_identity,
            &soul,
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
            provider: Box::new(provider),
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

    /// Run a single turn: send a user message, handle tool calls, return final response.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
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
                return Ok(content.clone());
            }

            // Execute each tool call and collect results
            let mut tool_results = String::new();
            for call in &tool_calls {
                let result = dispatcher::execute_tool_call(call, &self.tools).await;
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

        Ok("[Agent reached maximum tool iterations]".into())
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
        // For now, return a pending marker that the main loop recognizes.
        Ok(crate::tools::ToolResult {
            success: true,
            output: format!("[TRANSFER_PENDING:{}] {}", agent_name, task),
            error: None,
        })
    }
}
