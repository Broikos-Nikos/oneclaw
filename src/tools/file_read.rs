use super::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct FileReadTool {
    workspace_dir: PathBuf,
}

impl FileReadTool {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace_dir.join(p)
        }
    }
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Provide the file path relative to the workspace or as an absolute path."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to read"
                },
                "start_line": {
                    "type": "integer",
                    "description": "Optional start line (1-indexed)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "Optional end line (1-indexed, inclusive)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let path_str = args["path"]
            .as_str()
            .unwrap_or_default();
        let resolved = self.resolve_path(path_str);

        match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => {
                let start = args["start_line"].as_u64().map(|n| n as usize);
                let end = args["end_line"].as_u64().map(|n| n as usize);

                let output = if start.is_some() || end.is_some() {
                    let lines: Vec<&str> = content.lines().collect();
                    let s = start.unwrap_or(1).saturating_sub(1);
                    let e = end.unwrap_or(lines.len()).min(lines.len());
                    lines[s..e].join("\n")
                } else {
                    content
                };

                Ok(ToolResult {
                    success: true,
                    output,
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to read {}: {e}", resolved.display())),
            }),
        }
    }
}
