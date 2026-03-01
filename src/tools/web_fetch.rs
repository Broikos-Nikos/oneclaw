use super::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page and return its content as plain text. Useful for reading documentation, articles, etc."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let url = args["url"].as_str().unwrap_or_default();

        let client = reqwest::Client::builder()
            .user_agent("OneClaw/0.1")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        match client.get(url).send().await {
            Ok(response) => {
                let body = response.text().await.unwrap_or_default();
                let text = nanohtml2text::html2text(&body);

                // Truncate to reasonable size
                let truncated = if text.len() > 8000 {
                    format!("{}...\n[truncated at 8000 chars]", &text[..8000])
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
                error: Some(format!("Failed to fetch {url}: {e}")),
            }),
        }
    }
}
