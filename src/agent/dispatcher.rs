// Tool call parser and executor.
//
// Parses XML-style tool calls from agent responses and dispatches them.

use crate::tools::{Tool, ToolResult};
use serde_json::Value;

/// A parsed tool call from the agent's response.
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: Value,
}

/// Parse tool calls from an agent's response text.
///
/// Looks for patterns like:
/// ```
/// <tool_call name="tool_name">
/// {"param": "value"}
/// </tool_call>
/// ```
pub fn parse_tool_calls(text: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    let mut remaining = text;

    while let Some(start_idx) = remaining.find("<tool_call") {
        let after_tag = &remaining[start_idx..];

        // Find the closing >
        let Some(tag_end) = after_tag.find('>') else {
            remaining = &remaining[start_idx + 10..];
            continue;
        };

        let tag = &after_tag[..tag_end + 1];

        // Extract name attribute
        let name = extract_attribute(tag, "name").unwrap_or_default();
        if name.is_empty() {
            remaining = &remaining[start_idx + tag_end + 1..];
            continue;
        }

        // Find closing </tool_call>
        let body_start = start_idx + tag_end + 1;
        let Some(end_idx) = remaining[body_start..].find("</tool_call>") else {
            remaining = &remaining[body_start..];
            continue;
        };

        let body = remaining[body_start..body_start + end_idx].trim();

        // Parse JSON arguments
        let arguments = if body.is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(body).unwrap_or_else(|_| {
                // Try to be lenient — if it's not valid JSON, wrap as a string
                Value::Object({
                    let mut map = serde_json::Map::new();
                    map.insert("raw".into(), Value::String(body.to_string()));
                    map
                })
            })
        };

        calls.push(ParsedToolCall {
            name: name.to_string(),
            arguments,
        });

        remaining = &remaining[body_start + end_idx + 12..]; // 12 = len("</tool_call>")
    }

    calls
}

/// Execute a parsed tool call against the available tools.
pub async fn execute_tool_call(call: &ParsedToolCall, tools: &[Box<dyn Tool>]) -> ToolResult {
    // Find matching tool
    let tool = tools.iter().find(|t| t.name() == call.name);

    match tool {
        Some(t) => match t.execute(call.arguments.clone()).await {
            Ok(result) => result,
            Err(e) => ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Tool execution error: {e}")),
            },
        },
        None => ToolResult {
            success: false,
            output: String::new(),
            error: Some(format!("Unknown tool: {}", call.name)),
        },
    }
}

fn extract_attribute<'a>(tag: &'a str, attr_name: &str) -> Option<&'a str> {
    let pattern = format!("{attr_name}=\"");
    let start = tag.find(&pattern)? + pattern.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_tool_call() {
        let text = r#"Let me read that file.

<tool_call name="file_read">
{"path": "README.md"}
</tool_call>

Here are the results."#;

        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_read");
        assert_eq!(calls[0].arguments["path"], "README.md");
    }

    #[test]
    fn parse_multiple_tool_calls() {
        let text = r#"<tool_call name="file_read">
{"path": "a.txt"}
</tool_call>

<tool_call name="file_write">
{"path": "b.txt", "content": "hello"}
</tool_call>"#;

        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "file_read");
        assert_eq!(calls[1].name, "file_write");
    }

    #[test]
    fn parse_no_tool_calls() {
        let text = "Just a normal response with no tool calls.";
        let calls = parse_tool_calls(text);
        assert!(calls.is_empty());
    }
}
