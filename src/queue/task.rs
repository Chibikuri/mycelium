/// Whether the agent should implement changes or just research.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMode {
    /// Explore the codebase, make changes, and open a PR.
    Implement,
    /// Explore the codebase, report findings as a comment. No PR.
    Research,
}

/// Tasks that can be enqueued for processing.
#[derive(Debug, Clone)]
pub enum Task {
    ResolveIssue {
        installation_id: u64,
        repo_full_name: String,
        clone_url: String,
        default_branch: String,
        issue_number: u64,
        issue_title: String,
        issue_body: String,
        mode: IssueMode,
    },
    RespondToReview {
        installation_id: u64,
        repo_full_name: String,
        clone_url: String,
        pr_number: u64,
        pr_branch: String,
        review_body: String,
    },
}

impl Task {
    pub fn repo_full_name(&self) -> &str {
        match self {
            Task::ResolveIssue { repo_full_name, .. } => repo_full_name,
            Task::RespondToReview { repo_full_name, .. } => repo_full_name,
        }
    }

    pub fn description(&self) -> String {
        match self {
            Task::ResolveIssue {
                repo_full_name,
                issue_number,
                ..
            } => format!("Resolve issue #{issue_number} on {repo_full_name}"),
            Task::RespondToReview {
                repo_full_name,
                pr_number,
                ..
            } => format!("Respond to review on PR #{pr_number} on {repo_full_name}"),
        }
    }
}
