use std::sync::Arc;

use crate::platform::Platform;
use crate::queue::task::{IssueMode, Task};
use crate::server::AppState;

/// Scan for issues with trigger labels and enqueue them on startup.
///
/// This allows the service to resume work after a restart.
pub async fn scan_pending_issues(state: &Arc<AppState>) {
    let trigger_label = &state.config.github.trigger_label;
    let research_label = format!("{trigger_label}:research");
    let working_label = format!("{trigger_label}:working");

    tracing::info!("Scanning for pending issues with trigger labels...");

    // List all installations
    let installations = match state.platform.list_installations().await {
        Ok(installations) => installations,
        Err(e) => {
            tracing::error!(error = %e, "Failed to list installations on startup");
            return;
        }
    };

    tracing::info!(count = installations.len(), "Found installations");

    for installation in installations {
        // List repositories for this installation
        let repos = match state.platform.list_installation_repos(installation.id).await {
            Ok(repos) => repos,
            Err(e) => {
                tracing::warn!(
                    installation_id = installation.id,
                    error = %e,
                    "Failed to list repos for installation"
                );
                continue;
            }
        };

        for repo in repos {
            // Check for issues with the trigger label (implementation mode)
            if let Ok(issues) = state
                .platform
                .list_open_issues_with_label(installation.id, &repo.full_name, trigger_label)
                .await
            {
                for issue in issues {
                    // Skip issues that are already being worked on
                    if issue.labels.iter().any(|l| l == &working_label) {
                        tracing::debug!(
                            repo = %repo.full_name,
                            issue = issue.number,
                            "Skipping issue already being worked on"
                        );
                        continue;
                    }

                    // Skip if it also has the research label (handled below)
                    if issue.labels.iter().any(|l| l == &research_label) {
                        continue;
                    }

                    tracing::info!(
                        repo = %repo.full_name,
                        issue = issue.number,
                        title = %issue.title,
                        "Enqueuing pending issue (implement mode)"
                    );

                    let task = Task::ResolveIssue {
                        installation_id: installation.id,
                        repo_full_name: repo.full_name.clone(),
                        clone_url: repo.clone_url.clone(),
                        default_branch: repo.default_branch.clone(),
                        issue_number: issue.number,
                        issue_title: issue.title,
                        issue_body: issue.body,
                        mode: IssueMode::Implement,
                    };

                    let mut queue = state.task_queue.write().await;
                    queue.enqueue(&repo.full_name, task);
                }
            }

            // Check for issues with the research label
            if let Ok(issues) = state
                .platform
                .list_open_issues_with_label(installation.id, &repo.full_name, &research_label)
                .await
            {
                for issue in issues {
                    // Skip issues that are already being worked on
                    if issue.labels.iter().any(|l| l == &working_label) {
                        tracing::debug!(
                            repo = %repo.full_name,
                            issue = issue.number,
                            "Skipping issue already being worked on"
                        );
                        continue;
                    }

                    tracing::info!(
                        repo = %repo.full_name,
                        issue = issue.number,
                        title = %issue.title,
                        "Enqueuing pending issue (research mode)"
                    );

                    let task = Task::ResolveIssue {
                        installation_id: installation.id,
                        repo_full_name: repo.full_name.clone(),
                        clone_url: repo.clone_url.clone(),
                        default_branch: repo.default_branch.clone(),
                        issue_number: issue.number,
                        issue_title: issue.title,
                        issue_body: issue.body,
                        mode: IssueMode::Research,
                    };

                    let mut queue = state.task_queue.write().await;
                    queue.enqueue(&repo.full_name, task);
                }
            }
        }
    }

    tracing::info!("Startup scan complete");
}
