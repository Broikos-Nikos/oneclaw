// Agent module — core agent struct, prompt builder, and conversation loop.
// All agents have the same structure. The main agent additionally gets
// per-agent delegate tools — one named tool per sub-agent, following the
// "AgentTool" philosophy from Google ADK / Antigravity:
//   • Each sub-agent appears as its own named tool (e.g. delegate_to_developer)
//   • The tool description is loaded from the sub-agent's soul (role + strengths)
//   • The main LLM semantically matches the task to the right tool by description
//   • Delegation is synchronous — result flows back inline as a tool result

pub mod dispatcher;
pub mod prompt;
pub mod task_manager;

use crate::config::Config;
use crate::identity;
use crate::providers::ConversationMessage;
use crate::tools::Tool;
use anyhow::Result;
use std::sync::Arc;

/// Run a message through any agent and return a text result.
///
/// Creates a fresh agent instance, runs the task through it, and returns
/// the final response. With the AgentTool pattern, any sub-agent delegation
/// is handled inline by `delegate_to_X` tool calls — this function just
/// runs a single agent to completion.
///
/// `depth` is kept for call-site compatibility but no longer enforces recursion
/// since delegation is now synchronous inside AgentDelegateTool.
/// Use `is_main = true` only for the root orchestrator agent.
pub async fn run_agent_task(
    config: &Config,
    agent_name: &str,
    task: &str,
    is_main: bool,
    _depth: u32,
) -> anyhow::Result<String> {
    let workspace_dir = config.workspace_dir();
    let souls_dir = config.souls_dir();
    let available = crate::identity::discover_agents(&souls_dir);
    let tools = crate::tools::core_tools(&workspace_dir);

    let mut agent = Agent::from_config(config, agent_name, is_main, tools, &available)?;
    match agent.turn(task).await? {
        TurnResult::Response(text) => Ok(text),
        TurnResult::Transfer { target_agent, .. } => {
            // Should not happen — AgentDelegateTool handles delegation inline.
            tracing::warn!(
                "Unexpected Transfer to '{}' from agent '{}' — delegation is now inline",
                target_agent, agent_name
            );
            Ok(format!("[Unexpected delegation to '{target_agent}' — result unavailable]"))
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

        // Main agent gets the routing section + one delegate tool per sub-agent.
        // This follows the AgentTool pattern: each sub-agent is its own named
        // tool with a description from its soul, so the LLM picks the right
        // one by semantic match — no explicit agent name required.
        if is_main && available_agents.len() > 1 {
            system_prompt.push_str(&identity::routing_prompt(available_agents, &souls_dir));

            let config_arc = Arc::new(config.clone());
            for sub_name in available_agents.iter().filter(|a| a.as_str() != "main") {
                let description = identity::load_identity(&souls_dir, sub_name)
                    .map(|f| build_delegate_description(sub_name, &f.identity))
                    .unwrap_or_else(|_| format!("Delegate to the {} specialist agent.", sub_name));
                let tool_name = format!("delegate_to_{}", sanitize_agent_name(sub_name));
                tools.push(Box::new(AgentDelegateTool {
                    agent_name: sub_name.clone(),
                    tool_name,
                    description,
                    config: Arc::clone(&config_arc),
                }));
            }
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

// ─── Agent Delegate Tools (AgentTool philosophy) ────────────────────────────
//
// Instead of one generic `transfer_to_agent(agent, task)` tool, each sub-agent
// is exposed as its own named tool: `delegate_to_developer`, `delegate_to_creative`,
// etc. The description comes from the agent's soul (role + strengths), so the
// LLM semantically picks the right agent without needing to know its name.
//
// This follows the Google ADK `AgentTool` / Antigravity "Skills" pattern:
// sub-agents are treated as first-class tools, run synchronously, and return
// results inline — the main agent continues reasoning with the result.

struct AgentDelegateTool {
    agent_name: String,
    tool_name: String,
    description: String,
    config: Arc<Config>,
}

#[async_trait::async_trait]
impl Tool for AgentDelegateTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The specific task to give this agent. Be concrete — include all relevant context, file names, goals, and constraints."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<crate::tools::ToolResult> {
        let task = args["task"].as_str().unwrap_or_default();
        if task.is_empty() {
            return Ok(crate::tools::ToolResult {
                success: false,
                output: String::new(),
                error: Some("task is required".into()),
            });
        }

        tracing::info!("[main] → delegating to [{}]: {}", self.agent_name, task);

        // Run the sub-agent synchronously — result flows back as a tool result.
        // This is the AgentTool pattern: the main agent gets the answer inline
        // and can reason over it before responding to the user.
        let result = Box::pin(run_agent_task(
            &self.config,
            &self.agent_name,
            task,
            false,
            0,
        ))
        .await
        .unwrap_or_else(|e| format!("[{} error: {}]", self.agent_name, e));

        Ok(crate::tools::ToolResult {
            success: true,
            output: result,
            error: None,
        })
    }
}

/// Build the tool description for a sub-agent delegate tool.
/// Derived from the agent's identity — role, strengths, and personality.
fn build_delegate_description(agent_name: &str, identity: &crate::identity::AgentIdentity) -> String {
    // Use the agent's configured display name, or capitalise the folder name.
    let display_name = if !identity.name.is_empty() {
        identity.name.clone()
    } else {
        let mut c = agent_name.chars();
        match c.next() {
            None => agent_name.to_string(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    };

    let mut desc = if !identity.role.is_empty() {
        format!("{} ({}). ", display_name, identity.role)
    } else {
        format!("{} agent. ", display_name)
    };
    if !identity.strengths.is_empty() {
        desc.push_str(&format!("Strengths: {}. ", identity.strengths.join(", ")));
    }
    desc.push_str(&format!(
        "Use this tool when the task matches {}'s specialization — the result \
         is returned directly to you so you can synthesize and respond.",
        agent_name
    ));
    desc
}

/// Sanitize an agent name for use in a tool name (only alphanumeric and underscores).
fn sanitize_agent_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

