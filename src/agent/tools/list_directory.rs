use std::path::Path;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::claude::ToolDefinition;
use crate::agent::tools::{require_param, verified_path, Tool, ToolOutput};
use crate::error::Result;

pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_directory".to_string(),
            description: "List the contents of a directory. Returns file and directory names with type indicators (file/dir). Use this to explore the project structure.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the directory from the repository root. Use '.' for the root."
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
            return Ok(ToolOutput::Error(format!("Directory not found: {path_str}")));
        }

        if !full_path.is_dir() {
            return Ok(ToolOutput::Error(format!("{path_str} is not a directory")));
        }

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&full_path).await.map_err(|e| {
            crate::error::AppError::Workspace(format!("Failed to read directory: {e}"))
        })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|e| {
            crate::error::AppError::Workspace(format!("Failed to read directory entry: {e}"))
        })? {
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip hidden files like .git
            if name.starts_with('.') {
                continue;
            }
            let file_type = entry.file_type().await.map_err(|e| {
                crate::error::AppError::Workspace(format!("Failed to get file type: {e}"))
            })?;
            let kind = if file_type.is_dir() { "dir" } else { "file" };
            entries.push(format!("{name} ({kind})"));
        }

        entries.sort();

        if entries.is_empty() {
            Ok(ToolOutput::Success("Directory is empty".to_string()))
        } else {
            Ok(ToolOutput::Success(entries.join("\n")))
        }
    }
}
