use std::path::Path;
use std::time::Duration;

use crate::agent::claude::{
    CacheControl, ClaudeClient, ContentBlock, Message, MessageContent, MessagesRequest,
    SystemContent,
};
use crate::agent::tools::{ToolOutput, ToolRegistry};
use crate::config::AppConfig;
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

/// Rate limit retry configuration.
pub struct RateLimitConfig {
    /// Whether to retry on rate limit. If false, fail immediately on 429.
    pub enabled: bool,
    /// Maximum number of retries before giving up.
    pub max_retries: u32,
    /// Initial backoff duration (doubles each retry).
    pub initial_backoff: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 5,
            initial_backoff: Duration::from_secs(15),
        }
    }
}

pub struct AgentEngine {
    client: ClaudeClient,
    tools: ToolRegistry,
    max_turns: u32,
    rate_limit: RateLimitConfig,
}

impl AgentEngine {
    pub fn new(
        client: ClaudeClient,
        tools: ToolRegistry,
        max_turns: u32,
        rate_limit: RateLimitConfig,
    ) -> Self {
        Self {
            client,
            tools,
            max_turns,
            rate_limit,
        }
    }

    pub fn from_config(config: &AppConfig) -> Self {
        let claude = ClaudeClient::new(
            config.claude_api_key(),
            &config.claude.model,
            config.claude.max_tokens,
        );
        let tools = ToolRegistry::new(
            config.agent.max_file_size_bytes,
            config.agent.max_search_results,
        );
        let rate_limit = RateLimitConfig {
            enabled: config.claude.rate_limit_retry,
            max_retries: config.claude.rate_limit_max_retries,
            initial_backoff: Duration::from_secs(config.claude.rate_limit_backoff_secs),
        };
        Self::new(claude, tools, config.claude.max_turns, rate_limit)
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
        // Build cached tool definitions — mark the last tool for caching
        // so the entire system prompt + tools prefix is cached across turns
        let mut tool_definitions = self.tools.definitions();
        if let Some(last) = tool_definitions.last_mut() {
            last.cache_control = Some(CacheControl::ephemeral());
        }

        // System prompt with cache_control so it's cached across turns
        let system = vec![SystemContent::cached_text(system_prompt)];

        let mut messages = vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text(initial_message.to_string()),
        }];

        let mut total_input_tokens = 0u32;
        let mut total_output_tokens = 0u32;
        let mut total_cache_read_tokens = 0u32;
        let mut total_cache_creation_tokens = 0u32;

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
                system: system.clone(),
                messages: messages.clone(),
                tools: tool_definitions.clone(),
            };

            // Send with optional retry-on-rate-limit
            let response = {
                let mut retries = 0u32;
                loop {
                    match self.client.send_message(&request).await {
                        Ok(r) => break r,
                        Err(AppError::ClaudeRateLimited(ref msg)) => {
                            if !self.rate_limit.enabled
                                || retries >= self.rate_limit.max_retries
                            {
                                if retries > 0 {
                                    tracing::warn!(
                                        retries,
                                        "Rate limited too many times, stopping agent"
                                    );
                                } else {
                                    tracing::warn!("Rate limited, retry disabled");
                                }
                                return AgentOutcome::RateLimited {
                                    message: msg.clone(),
                                };
                            }
                            retries += 1;
                            let backoff = self.rate_limit.initial_backoff
                                * 2u32.saturating_pow(retries - 1);
                            tracing::info!(
                                retry = retries,
                                backoff_secs = backoff.as_secs(),
                                "Rate limited, waiting before retry"
                            );
                            tokio::time::sleep(backoff).await;

                            // Check cancellation during backoff
                            if is_cancelled().await {
                                tracing::info!("Agent cancelled during rate limit backoff");
                                return AgentOutcome::Cancelled;
                            }
                        }
                        Err(e) => {
                            return AgentOutcome::Failed {
                                error: format!("Claude API error: {e}"),
                            };
                        }
                    }
                }
            };

            total_input_tokens += response.usage.input_tokens;
            total_output_tokens += response.usage.output_tokens;
            total_cache_read_tokens += response.usage.cache_read_input_tokens.unwrap_or(0);
            total_cache_creation_tokens += response.usage.cache_creation_input_tokens.unwrap_or(0);

            tracing::info!(
                input_tokens = response.usage.input_tokens,
                output_tokens = response.usage.output_tokens,
                cache_read = response.usage.cache_read_input_tokens.unwrap_or(0),
                cache_creation = response.usage.cache_creation_input_tokens.unwrap_or(0),
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
                        total_cache_read_tokens,
                        total_cache_creation_tokens,
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
