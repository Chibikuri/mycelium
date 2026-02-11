use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::claude::ToolDefinition;
use crate::agent::tools::{require_param, Tool, ToolOutput};
use crate::error::Result;

pub struct SearchCodeTool {
    max_results: usize,
}

impl SearchCodeTool {
    pub fn new(max_results: usize) -> Self {
        Self { max_results }
    }
}

#[async_trait]
impl Tool for SearchCodeTool {
    fn name(&self) -> &str {
        "search_code"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_code".to_string(),
            description: "Search for a pattern in the codebase using grep. Returns matching lines with file paths and line numbers. Use this to find relevant code, function definitions, usages, etc.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Search pattern (grep-compatible regex)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional: restrict search to this subdirectory (relative to repo root)"
                    },
                    "include": {
                        "type": "string",
                        "description": "Optional: file glob pattern to include (e.g., '*.rs', '*.py')"
                    }
                },
                "required": ["pattern"]
            }),
            cache_control: None,
        }
    }

    async fn execute(
        &self,
        workspace_root: &Path,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let pattern = require_param!(input, "pattern");

        let search_dir = if let Some(path) = input["path"].as_str() {
            workspace_root.join(path)
        } else {
            workspace_root.to_path_buf()
        };

        if !search_dir.exists() {
            return Ok(ToolOutput::Error(format!(
                "Search directory does not exist: {}",
                input["path"].as_str().unwrap_or(".")
            )));
        }

        let mut args = vec![
            "-rn".to_string(),
            "--max-count=5".to_string(), // Max matches per file
            format!("--max-count={}", self.max_results),
        ];

        if let Some(include) = input["include"].as_str() {
            args.push(format!("--include={include}"));
        }

        // Exclude common non-code directories
        args.extend_from_slice(&[
            "--exclude-dir=.git".to_string(),
            "--exclude-dir=node_modules".to_string(),
            "--exclude-dir=target".to_string(),
            "--exclude-dir=.venv".to_string(),
            "--exclude-dir=vendor".to_string(),
        ]);

        args.push(pattern.to_string());
        args.push(".".to_string());

        let output = tokio::process::Command::new("grep")
            .args(&args)
            .current_dir(&search_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);

                if stdout.is_empty() {
                    return Ok(ToolOutput::Success(
                        "No matches found".to_string(),
                    ));
                }

                // Truncate to max results
                let lines: Vec<&str> = stdout.lines().take(self.max_results).collect();
                let result = lines.join("\n");

                let total_lines = stdout.lines().count();
                if total_lines > self.max_results {
                    Ok(ToolOutput::Success(format!(
                        "{result}\n\n... ({} more matches truncated)",
                        total_lines - self.max_results
                    )))
                } else {
                    Ok(ToolOutput::Success(result))
                }
            }
            Err(e) => Ok(ToolOutput::Error(format!("Search failed: {e}"))),
        }
    }
}
