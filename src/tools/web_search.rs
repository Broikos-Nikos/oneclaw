use super::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using DuckDuckGo. Returns search results with titles, URLs, and snippets."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let query = args["query"].as_str().unwrap_or_default();
        let _max_results = args["max_results"].as_u64().unwrap_or(5);

        let encoded = urlencoding::encode(query);
        let url = format!("https://html.duckduckgo.com/html/?q={encoded}");

        let client = reqwest::Client::builder()
            .user_agent("OneClaw/0.1")
            .build()?;

        match client.get(&url).send().await {
            Ok(response) => {
                let body = response.text().await.unwrap_or_default();
                let text = nanohtml2text::html2text(&body);

                // Truncate to avoid giant outputs
                let truncated = if text.len() > 4000 {
                    format!("{}...\n[truncated]", &text[..4000])
                } else {
                    text
                };

                Ok(ToolResult {
                    success: true,
                    output: truncated,
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Search failed: {e}")),
            }),
        }
    }
}
