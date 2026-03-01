// Tool trait and core tool implementations.

pub mod file_read;
pub mod file_write;
pub mod shell;
pub mod web_fetch;
pub mod web_search;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result returned by a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

/// Specification of a tool for the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Tool trait — implemented by each tool.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;

    async fn execute(&self, args: Value) -> Result<ToolResult>;
}

/// Get the list of all available core tools.
pub fn core_tools(workspace_dir: &std::path::Path) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(file_read::FileReadTool::new(workspace_dir)),
        Box::new(file_write::FileWriteTool::new(workspace_dir)),
        Box::new(shell::ShellTool::new(workspace_dir)),
        Box::new(web_search::WebSearchTool),
        Box::new(web_fetch::WebFetchTool),
    ]
}
