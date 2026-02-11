use std::path::Path;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::claude::ToolDefinition;
use crate::agent::tools::{require_param, verified_path, Tool, ToolOutput};
use crate::error::Result;

pub struct CreateFileTool;

#[async_trait]
impl Tool for CreateFileTool {
    fn name(&self) -> &str {
        "create_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_file".to_string(),
            description: "Create a new file with the given content. The file must not already exist. Parent directories will be created automatically.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path for the new file from the repository root"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content for the new file"
                    }
                },
                "required": ["path", "content"]
            }),
            cache_control: None,
        }
    }

    async fn execute(
        &self,
        workspace_root: &Path,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let path_str = require_param!(input, "path");
        let content = require_param!(input, "content");

        let full_path = match verified_path(workspace_root, path_str) {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        if full_path.exists() {
            return Ok(ToolOutput::Error(format!(
                "File already exists: {path_str}. Use write_file to modify existing files."
            )));
        }

        match tokio::fs::write(&full_path, content).await {
            Ok(()) => Ok(ToolOutput::Success(format!("Successfully created {path_str}"))),
            Err(e) => Ok(ToolOutput::Error(format!("Failed to create file: {e}"))),
        }
    }
}
