use crate::agent::claude::ClaudeClient;
use crate::agent::engine::{AgentEngine, AgentOutcome, RateLimitConfig};
use crate::agent::prompt;
use crate::agent::tools::ToolRegistry;
use crate::error::Result;
use crate::platform::types::CreatePullRequest;
use crate::platform::Platform;
use crate::queue::task::IssueMode;
use crate::server::{AppState, CancellationReason};
use crate::workflow::types::WorkflowOutcome;
use crate::workspace::WorkspaceManager;

pub async fn resolve_issue(
    state: &AppState,
    installation_id: u64,
    repo_full_name: &str,
    clone_url: &str,
    default_branch: &str,
    issue_number: u64,
    issue_title: &str,
    issue_body: &str,
    mode: IssueMode,
) -> Result<WorkflowOutcome> {
    let platform = &state.platform;
    let config = &state.config;
    let research_only = mode == IssueMode::Research;

    // Add "working" label and register as in-flight
    let _ = platform
        .add_label(
            installation_id,
            repo_full_name,
            issue_number,
            &format!("{}:working", config.github.trigger_label),
        )
        .await;
    state
        .register_in_flight(installation_id, repo_full_name, issue_number)
        .await;

    // Fetch full issue with comments
    let issue = platform
        .get_issue(installation_id, repo_full_name, issue_number)
        .await?;

    // Format comments for the prompt
    let comments_text = issue
        .comments
        .iter()
        .map(|c| format!("**@{}:** {}", c.author, c.body))
        .collect::<Vec<_>>()
        .join("\n\n");

    // Get access token for git operations
    let token = platform.get_access_token(installation_id).await?;

    // Set up workspace
    let workspace_mgr = WorkspaceManager::new(&config.workspace);
    let workspace = workspace_mgr
        .setup_for_issue(clone_url, &token, repo_full_name, issue_number)
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

    let system = prompt::system_prompt_for_issue(
        repo_full_name,
        issue_number,
        issue_title,
        issue_body,
        &comments_text,
        research_only,
    );

    let initial_message = if research_only {
        format!(
            "Please research issue #{issue_number}: {issue_title}\n\nExplore the repository and report your findings. Do not modify any files."
        )
    } else {
        format!(
            "Please resolve issue #{issue_number}: {issue_title}\n\nStart by exploring the repository structure to understand the codebase, then implement the necessary changes."
        )
    };

    let repo_name = repo_full_name.to_string();
    let outcome = engine
        .run(&system, &workspace.path, &initial_message, || {
            let state_ref = &state;
            let repo_ref = &repo_name;
            async move { state_ref.is_cancelled(repo_ref, issue_number).await }
        })
        .await;

    // Clear cancellation flag now that we're done
    state.clear_cancellation(repo_full_name, issue_number).await;

    let result = match outcome {
        AgentOutcome::Cancelled => {
            // Check the cancellation reason to provide appropriate feedback
            let reason = state
                .get_cancellation_reason(repo_full_name, issue_number)
                .await;

            let _ = platform
                .remove_label(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    &format!("{}:working", config.github.trigger_label),
                )
                .await;

            match reason {
                Some(CancellationReason::IssueClosed) => {
                    tracing::info!(issue = issue_number, "Task cancelled (issue closed)");
                    // No comment needed - issue is closed
                }
                Some(CancellationReason::LabelRemoved) => {
                    tracing::info!(issue = issue_number, "Task cancelled (label removed)");
                    let _ = platform
                        .post_comment(
                            installation_id,
                            repo_full_name,
                            issue_number,
                            "Task stopped because the trigger label was removed. Re-add the label to restart.\n\n---\n*Mycelium*",
                        )
                        .await;
                }
                None => {
                    tracing::info!(issue = issue_number, "Task cancelled (unknown reason)");
                }
            }

            // Unregister from in-flight tracking and cleanup workspace
            state
                .unregister_in_flight(repo_full_name, issue_number)
                .await;
            let _ = workspace_mgr.cleanup(&workspace).await;

            let error_msg = match reason {
                Some(CancellationReason::IssueClosed) => "Cancelled (issue closed)",
                Some(CancellationReason::LabelRemoved) => "Cancelled (label removed)",
                None => "Cancelled",
            };
            return Ok(WorkflowOutcome::Failed {
                error: error_msg.to_string(),
            });
        }
        AgentOutcome::Completed { summary } => {
            if research_only {
                // Research mode: post findings as a comment, no PR
                let _ = platform
                    .post_comment(
                        installation_id,
                        repo_full_name,
                        issue_number,
                        &format!("## Research Findings\n\n{summary}\n\n---\n*Mycelium*"),
                    )
                    .await;

                let _ = platform
                    .remove_label(
                        installation_id,
                        repo_full_name,
                        issue_number,
                        &format!("{}:working", config.github.trigger_label),
                    )
                    .await;
                let _ = platform
                    .add_label(
                        installation_id,
                        repo_full_name,
                        issue_number,
                        &format!("{}:done", config.github.trigger_label),
                    )
                    .await;

                WorkflowOutcome::ResearchPosted
            } else {
                // Implementation mode: commit, push, create PR
                let commit_msg = format!(
                    "fix: resolve #{issue_number} - {issue_title}\n\n{summary}"
                );

                let has_changes = workspace_mgr.finalize(&workspace, &commit_msg).await?;

                if has_changes {
                    let pr = platform
                        .create_pull_request(
                            installation_id,
                            repo_full_name,
                            &CreatePullRequest {
                                title: format!("Fix #{issue_number}: {issue_title}"),
                                body: format!(
                                    "Resolves #{issue_number}\n\n## Summary\n\n{summary}\n\n---\n*Automated by Mycelium*"
                                ),
                                head_branch: workspace.branch.clone(),
                                base_branch: default_branch.to_string(),
                            },
                        )
                        .await?;

                    let _ = platform
                        .remove_label(
                            installation_id,
                            repo_full_name,
                            issue_number,
                            &format!("{}:working", config.github.trigger_label),
                        )
                        .await;
                    let _ = platform
                        .add_label(
                            installation_id,
                            repo_full_name,
                            issue_number,
                            &format!("{}:done", config.github.trigger_label),
                        )
                        .await;

                    WorkflowOutcome::PullRequestCreated {
                        pr_number: pr.number,
                    }
                } else {
                    let _ = platform
                        .post_comment(
                            installation_id,
                            repo_full_name,
                            issue_number,
                            &format!("I analyzed the issue but didn't find any code changes needed.\n\n{summary}\n\n---\n*Mycelium*"),
                        )
                        .await;

                    WorkflowOutcome::NoChanges
                }
            }
        }
        AgentOutcome::ClarificationNeeded { question } => {
            let _ = platform
                .post_comment(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    &format!("I need some clarification before I can proceed:\n\n{question}\n\n---\n*Mycelium*"),
                )
                .await;

            let _ = platform
                .remove_label(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    &format!("{}:working", config.github.trigger_label),
                )
                .await;

            WorkflowOutcome::ClarificationRequested
        }
        AgentOutcome::TurnLimitReached { partial_summary } => {
            let _ = platform
                .post_comment(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    &format!("I wasn't able to fully resolve this issue within the allowed number of turns.\n\n{partial_summary}\n\n---\n*Mycelium*"),
                )
                .await;

            let _ = platform
                .remove_label(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    &format!("{}:working", config.github.trigger_label),
                )
                .await;

            WorkflowOutcome::Failed {
                error: "Turn limit reached".to_string(),
            }
        }
        AgentOutcome::RateLimited { message } => {
            tracing::warn!(issue = issue_number, "Agent hit rate limit");
            let _ = platform
                .post_comment(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    "I hit the Claude API rate limit and had to stop. Please try again later by re-adding the label.\n\n---\n*Mycelium*",
                )
                .await;

            let _ = platform
                .remove_label(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    &format!("{}:working", config.github.trigger_label),
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
                    issue_number,
                    &format!("I encountered an error while trying to resolve this issue:\n\n```\n{error}\n```\n\n---\n*Mycelium*"),
                )
                .await;

            let _ = platform
                .remove_label(
                    installation_id,
                    repo_full_name,
                    issue_number,
                    &format!("{}:working", config.github.trigger_label),
                )
                .await;

            WorkflowOutcome::Failed { error }
        }
    };

    // Unregister from in-flight tracking and cleanup workspace
    state
        .unregister_in_flight(repo_full_name, issue_number)
        .await;
    let _ = workspace_mgr.cleanup(&workspace).await;

    Ok(result)
}
