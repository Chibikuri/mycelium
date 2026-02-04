pub mod ask_clarification;
pub mod create_file;
pub mod delete_file;
pub mod list_directory;
pub mod read_file;
pub mod search_code;
pub mod write_file;

use std::path::Path;

use async_trait::async_trait;

use crate::agent::claude::ToolDefinition;
use crate::error::Result;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    async fn execute(
        &self,
        workspace_root: &Path,
        input: serde_json::Value,
    ) -> Result<ToolOutput>;
}

pub enum ToolOutput {
    /// Normal text result returned to Claude.
    Success(String),
    /// Error result returned to Claude (the agent can recover).
    Error(String),
    /// Special signal: agent needs human input.
    ClarificationNeeded(String),
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new(max_file_size: usize, max_search_results: usize) -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(read_file::ReadFileTool::new(max_file_size)),
            Box::new(list_directory::ListDirectoryTool),
            Box::new(search_code::SearchCodeTool::new(max_search_results)),
            Box::new(write_file::WriteFileTool),
            Box::new(create_file::CreateFileTool),
            Box::new(delete_file::DeleteFileTool),
            Box::new(ask_clarification::AskClarificationTool),
        ];

        Self { tools }
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| t.definition()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }
}
