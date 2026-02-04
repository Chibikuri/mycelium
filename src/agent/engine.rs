use std::path::Path;

use crate::agent::claude::{
    ClaudeClient, ContentBlock, Message, MessageContent, MessagesRequest,
};
use crate::agent::tools::{ToolOutput, ToolRegistry};
use crate::error::{AppError, Result};

/// Outcome of an agent run.
#[derive(Debug)]
pub enum AgentOutcome {
    /// Agent completed successfully with a summary of changes.
    Completed { summary: String },
    /// Agent needs clarification from a human.
    ClarificationNeeded { question: String },
    /// Agent hit the turn limit without finishing.
    TurnLimitReached { partial_summary: String },
    /// Agent hit Claude API rate limits.
    RateLimited { message: String },
    /// Agent was cancelled (e.g., issue closed).
    Cancelled,
    /// Agent encountered an error.
    Failed { error: String },
}

pub struct AgentEngine {
    client: ClaudeClient,
    tools: ToolRegistry,
    max_turns: u32,
}

impl AgentEngine {
    pub fn new(client: ClaudeClient, tools: ToolRegistry, max_turns: u32) -> Self {
        Self {
            client,
            tools,
            max_turns,
        }
    }

    /// Run the agentic loop.
    ///
    /// - `system_prompt`: The system prompt with context about the task.
    /// - `workspace_root`: The root directory of the cloned repo.
    /// - `initial_message`: The initial user message to start the conversation.
    /// - `is_cancelled`: Async callback checked each turn; returns true if work should stop.
    pub async fn run<F, Fut>(
        &self,
        system_prompt: &str,
        workspace_root: &Path,
        initial_message: &str,
        is_cancelled: F,
    ) -> AgentOutcome
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let tool_definitions = self.tools.definitions();

        let mut messages = vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text(initial_message.to_string()),
        }];

        let mut total_input_tokens = 0u32;
        let mut total_output_tokens = 0u32;

        for turn in 0..self.max_turns {
            // Check for cancellation before each turn
            if is_cancelled().await {
                tracing::info!("Agent cancelled");
                return AgentOutcome::Cancelled;
            }

            tracing::info!(turn = turn, "Agent turn");

            let request = MessagesRequest {
                model: self.client.model().to_string(),
                max_tokens: self.client.max_tokens(),
                system: system_prompt.to_string(),
                messages: messages.clone(),
                tools: tool_definitions.clone(),
            };

            let response = match self.client.send_message(&request).await {
                Ok(r) => r,
                Err(AppError::ClaudeRateLimited(msg)) => {
                    tracing::warn!("Claude API rate limited, stopping agent");
                    return AgentOutcome::RateLimited { message: msg };
                }
                Err(e) => {
                    return AgentOutcome::Failed {
                        error: format!("Claude API error: {e}"),
                    };
                }
            };

            total_input_tokens += response.usage.input_tokens;
            total_output_tokens += response.usage.output_tokens;

            tracing::info!(
                input_tokens = response.usage.input_tokens,
                output_tokens = response.usage.output_tokens,
                stop_reason = ?response.stop_reason,
                "Claude response"
            );

            // Check stop reason
            let stop_reason = response.stop_reason.as_deref().unwrap_or("unknown");

            match stop_reason {
                "end_turn" => {
                    // Agent is done -- extract the summary from the text blocks
                    let summary = extract_text(&response.content);
                    tracing::info!(
                        total_input_tokens,
                        total_output_tokens,
                        turns = turn + 1,
                        "Agent completed"
                    );
                    return AgentOutcome::Completed { summary };
                }
                "tool_use" => {
                    // Agent wants to use tools -- process each tool call
                    // First, add the assistant's message to the conversation
                    messages.push(Message {
                        role: "assistant".to_string(),
                        content: MessageContent::Blocks(response.content.clone()),
                    });

                    // Execute tool calls and collect results
                    let mut tool_results = Vec::new();

                    for block in &response.content {
                        if let ContentBlock::ToolUse { id, name, input } = block {
                            tracing::info!(tool = %name, "Executing tool");

                            let result = self.execute_tool(workspace_root, name, input).await;

                            match result {
                                Ok(ToolOutput::Success(content)) => {
                                    tracing::debug!(tool = %name, "Tool succeeded");
                                    tool_results.push(ContentBlock::ToolResult {
                                        tool_use_id: id.clone(),
                                        content,
                                        is_error: None,
                                    });
                                }
                                Ok(ToolOutput::Error(error)) => {
                                    tracing::warn!(tool = %name, error = %error, "Tool error");
                                    tool_results.push(ContentBlock::ToolResult {
                                        tool_use_id: id.clone(),
                                        content: error,
                                        is_error: Some(true),
                                    });
                                }
                                Ok(ToolOutput::ClarificationNeeded(question)) => {
                                    tracing::info!(
                                        "Agent requesting clarification: {}",
                                        question
                                    );
                                    return AgentOutcome::ClarificationNeeded { question };
                                }
                                Err(e) => {
                                    tracing::error!(tool = %name, error = %e, "Tool execution error");
                                    tool_results.push(ContentBlock::ToolResult {
                                        tool_use_id: id.clone(),
                                        content: format!("Internal error: {e}"),
                                        is_error: Some(true),
                                    });
                                }
                            }
                        }
                    }

                    // Add tool results as a user message
                    messages.push(Message {
                        role: "user".to_string(),
                        content: MessageContent::Blocks(tool_results),
                    });
                }
                "max_tokens" => {
                    // Ran out of tokens in this turn
                    tracing::warn!("Agent response hit max_tokens limit");
                    // Add partial response and continue
                    messages.push(Message {
                        role: "assistant".to_string(),
                        content: MessageContent::Blocks(response.content),
                    });
                    messages.push(Message {
                        role: "user".to_string(),
                        content: MessageContent::Text("Please continue.".to_string()),
                    });
                }
                other => {
                    tracing::warn!(stop_reason = other, "Unexpected stop reason");
                    return AgentOutcome::Failed {
                        error: format!("Unexpected stop reason: {other}"),
                    };
                }
            }
        }

        tracing::warn!(max_turns = self.max_turns, "Agent hit turn limit");
        AgentOutcome::TurnLimitReached {
            partial_summary: "Agent reached maximum number of turns without completing the task."
                .to_string(),
        }
    }

    async fn execute_tool(
        &self,
        workspace_root: &Path,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<ToolOutput> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| AppError::Agent(format!("Unknown tool: {name}")))?;

        tool.execute(workspace_root, input.clone()).await
    }
}

fn extract_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text { text } = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
