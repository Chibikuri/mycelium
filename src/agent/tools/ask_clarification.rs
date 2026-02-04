use std::path::Path;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::claude::ToolDefinition;
use crate::agent::tools::{Tool, ToolOutput};
use crate::error::Result;

pub struct AskClarificationTool;

#[async_trait]
impl Tool for AskClarificationTool {
    fn name(&self) -> &str {
        "ask_clarification"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ask_clarification".to_string(),
            description: "Ask the issue author or reviewer for clarification. Use this when the issue description is ambiguous, requirements are unclear, or you need more information before proceeding. This will post a comment on the issue and stop the current task.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The clarification question to ask"
                    }
                },
                "required": ["question"]
            }),
        }
    }

    async fn execute(
        &self,
        _workspace_root: &Path,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let question = match input["question"].as_str() {
            Some(q) => q,
            None => return Ok(ToolOutput::Error("Missing 'question' parameter".to_string())),
        };

        Ok(ToolOutput::ClarificationNeeded(question.to_string()))
    }
}
