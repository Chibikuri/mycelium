/// Outcome of a workflow execution.
#[derive(Debug)]
pub enum WorkflowOutcome {
    /// Successfully resolved: PR was created.
    PullRequestCreated { pr_number: u64 },
    /// Successfully pushed fixes in response to review.
    ReviewAddressed,
    /// Agent needs clarification; comment posted on issue.
    ClarificationRequested,
    /// No changes were needed or produced.
    NoChanges,
    /// Workflow failed with an error.
    Failed { error: String },
}
