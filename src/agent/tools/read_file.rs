use std::path::Path;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::claude::ToolDefinition;
use crate::agent::tools::{require_param, verified_path, Tool, ToolOutput};
use crate::error::Result;

pub struct ReadFileTool {
    max_file_size: usize,
}

impl ReadFileTool {
    pub fn new(max_file_size: usize) -> Self {
        Self { max_file_size }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of a file. Returns the file content as text. Use this to understand existing code before making changes.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the file from the repository root"
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

        // Check file size
        let metadata = tokio::fs::metadata(&full_path).await.map_err(|e| {
            crate::error::AppError::Workspace(format!("Failed to read file metadata: {e}"))
        })?;

        if metadata.len() as usize > self.max_file_size {
            return Ok(ToolOutput::Error(format!(
                "File is too large ({} bytes, max {} bytes)",
                metadata.len(),
                self.max_file_size
            )));
        }

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => Ok(ToolOutput::Success(content)),
            Err(e) => Ok(ToolOutput::Error(format!("Failed to read file: {e}"))),
        }
    }
}
