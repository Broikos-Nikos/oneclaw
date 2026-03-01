use super::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct FileWriteTool {
    workspace_dir: PathBuf,
}

impl FileWriteTool {
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
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file and parent directories if they don't exist. Provide path relative to workspace or absolute."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to the file instead of overwriting"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let path_str = args["path"].as_str().unwrap_or_default();
        let content = args["content"].as_str().unwrap_or_default();
        let append = args["append"].as_bool().unwrap_or(false);

        let resolved = self.resolve_path(path_str);

        // Create parent directories
        if let Some(parent) = resolved.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to create directory: {e}")),
                });
            }
        }

        let result = if append {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&resolved)
                .await;
            match file {
                Ok(ref mut f) => f.write_all(content.as_bytes()).await.map_err(|e| e.into()),
                Err(e) => Err(anyhow::anyhow!(e)),
            }
        } else {
            tokio::fs::write(&resolved, content)
                .await
                .map_err(|e| e.into())
        };

        match result {
            Ok(()) => Ok(ToolResult {
                success: true,
                output: format!("Written {} bytes to {}", content.len(), resolved.display()),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to write {}: {e}", resolved.display())),
            }),
        }
    }
}
