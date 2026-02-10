use crate::agent::claude::ClaudeClient;
use crate::agent::engine::{AgentEngine, AgentOutcome, RateLimitConfig};
use crate::agent::prompt;
use crate::agent::tools::ToolRegistry;
use crate::error::Result;
use crate::platform::Platform;
use crate::server::AppState;
use crate::workflow::types::WorkflowOutcome;
use crate::workspace::WorkspaceManager;

pub async fn respond_to_review(
    state: &AppState,
    installation_id: u64,
    repo_full_name: &str,
    clone_url: &str,
    pr_number: u64,
    pr_branch: &str,
    review_body: &str,
) -> Result<WorkflowOutcome> {
    let platform = &state.platform;
    let config = &state.config;

    // Fetch all reviews to get the latest context
    let reviews = platform
        .get_reviews(installation_id, repo_full_name, pr_number)
        .await?;

    // Format review comments for the prompt
    let review_comments_text = reviews
        .iter()
        .flat_map(|r| {
            r.comments.iter().map(|c| {
                let location = match (&c.path, c.line) {
                    (Some(path), Some(line)) => format!(" ({path}:{line})"),
                    (Some(path), None) => format!(" ({path})"),
                    _ => String::new(),
                };
                format!("**@{}**{}: {}", c.author, location, c.body)
            })
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    // Get access token for git operations
    let token = platform.get_access_token(installation_id).await?;

    // Set up workspace (checkout existing branch)
    let workspace_mgr = WorkspaceManager::new(&config.workspace);
    let workspace = workspace_mgr
        .setup_for_review(clone_url, &token, repo_full_name, pr_branch)
        .await?;

    // Run the agent
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
        initial_backoff: std::time::Duration::from_secs(config.claude.rate_limit_backoff_secs),
    };
    let engine = AgentEngine::new(claude, tools, config.claude.max_turns, rate_limit);

    let system = prompt::system_prompt_for_review(
        repo_full_name,
        pr_number,
        review_body,
        &review_comments_text,
    );

    let initial_message = format!(
        "Please address the code review feedback on PR #{pr_number}. Read the review comments and make the requested changes."
    );

    // Reviews don't have a cancellation mechanism (PRs stay open), so pass a no-op check
    let outcome = engine
        .run(&system, &workspace.path, &initial_message, || async { false })
        .await;

    let result = match outcome {
        AgentOutcome::Cancelled => {
            let _ = workspace_mgr.cleanup(&workspace).await;
            return Ok(WorkflowOutcome::Failed {
                error: "Cancelled".to_string(),
            });
        }
        AgentOutcome::Completed { summary } => {
            let commit_msg = format!("fix: address review feedback on PR #{pr_number}\n\n{summary}");

            let token = platform.get_access_token(installation_id).await?;
            let has_changes = workspace_mgr.finalize(&workspace, &commit_msg, &token).await?;

            if has_changes {
                // Post a comment on the PR
                let _ = platform
                    .post_comment(
                        installation_id,
                        repo_full_name,
                        pr_number,
                        &format!("I've addressed the review feedback and pushed the changes.\n\n## Changes Made\n\n{summary}\n\n---\n*Mycelium*"),
                    )
                    .await;

                WorkflowOutcome::ReviewAddressed
            } else {
                let _ = platform
                    .post_comment(
                        installation_id,
                        repo_full_name,
                        pr_number,
                        &format!("I reviewed the feedback but didn't find code changes to make.\n\n{summary}\n\n---\n*Mycelium*"),
                    )
                    .await;

                WorkflowOutcome::NoChanges
            }
        }
        AgentOutcome::ClarificationNeeded { question } => {
            let _ = platform
                .post_comment(
                    installation_id,
                    repo_full_name,
                    pr_number,
                    &format!("I need some clarification on the review feedback:\n\n{question}\n\n---\n*Mycelium*"),
                )
                .await;

            WorkflowOutcome::ClarificationRequested
        }
        AgentOutcome::TurnLimitReached { partial_summary } => {
            let _ = platform
                .post_comment(
                    installation_id,
                    repo_full_name,
                    pr_number,
                    &format!("I wasn't able to fully address the review feedback within the allowed number of turns.\n\n{partial_summary}\n\n---\n*Mycelium*"),
                )
                .await;

            WorkflowOutcome::Failed {
                error: "Turn limit reached".to_string(),
            }
        }
        AgentOutcome::RateLimited { message } => {
            tracing::warn!(pr = pr_number, "Agent hit rate limit");
            let _ = platform
                .post_comment(
                    installation_id,
                    repo_full_name,
                    pr_number,
                    "I hit the Claude API rate limit and had to stop. Please try again later.\n\n---\n*Mycelium*",
                )
                .await;

            WorkflowOutcome::Failed {
                error: format!("Rate limited: {message}"),
            }
        }
        AgentOutcome::Failed { error } => {
            let _ = platform
                .post_comment(
                    installation_id,
                    repo_full_name,
                    pr_number,
                    &format!("I encountered an error:\n\n```\n{error}\n```\n\n---\n*Mycelium*"),
                )
                .await;

            WorkflowOutcome::Failed { error }
        }
    };

    let _ = workspace_mgr.cleanup(&workspace).await;

    Ok(result)
}
