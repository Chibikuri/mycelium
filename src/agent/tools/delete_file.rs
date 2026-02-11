use std::path::Path;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::claude::ToolDefinition;
use crate::agent::tools::{require_param, verified_path, Tool, ToolOutput};
use crate::error::Result;

pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_file".to_string(),
            description: "Delete a file from the repository.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the file to delete from the repository root"
                    }
                },
                "required": ["path"]
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

        let full_path = match verified_path(workspace_root, path_str) {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };

        if !full_path.exists() {
            return Ok(ToolOutput::Error(format!("File not found: {path_str}")));
        }

        if !full_path.is_file() {
            return Ok(ToolOutput::Error(format!("{path_str} is not a file")));
        }

        match tokio::fs::remove_file(&full_path).await {
            Ok(()) => Ok(ToolOutput::Success(format!("Successfully deleted {path_str}"))),
            Err(e) => Ok(ToolOutput::Error(format!("Failed to delete file: {e}"))),
        }
    }
}
