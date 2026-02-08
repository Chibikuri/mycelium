pub mod sequential;
pub mod startup;
pub mod task;

use std::collections::VecDeque;
use std::sync::Arc;

use crate::server::AppState;
use crate::workflow;

use task::Task;

/// Simple task queue backed by a VecDeque per repo.
pub struct TaskQueue {
    /// Pending tasks per repository (processed sequentially).
    queues: std::collections::HashMap<String, VecDeque<Task>>,
    /// Notification channel for the processor.
    notify: Option<tokio::sync::mpsc::UnboundedSender<()>>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self {
            queues: std::collections::HashMap::new(),
            notify: None,
        }
    }

    pub fn set_notifier(&mut self, tx: tokio::sync::mpsc::UnboundedSender<()>) {
        self.notify = Some(tx);
    }

    pub fn enqueue(&mut self, repo: &str, task: Task) {
        tracing::info!(repo = repo, task = %task.description(), "Enqueuing task");
        self.queues
            .entry(repo.to_string())
            .or_default()
            .push_back(task);

        if let Some(ref tx) = self.notify {
            let _ = tx.send(());
        }
    }

    /// Remove all pending tasks for a specific issue from the queue.
    pub fn cancel_issue(&mut self, repo_full_name: &str, issue_number: u64) {
        if let Some(queue) = self.queues.get_mut(repo_full_name) {
            let before = queue.len();
            queue.retain(|task| {
                !matches!(task, Task::ResolveIssue { issue_number: n, .. } if *n == issue_number)
            });
            let removed = before - queue.len();
            if removed > 0 {
                tracing::info!(
                    repo = repo_full_name,
                    issue = issue_number,
                    removed = removed,
                    "Cancelled queued tasks for closed issue"
                );
            }
        }
    }

    /// Take the next task from any repo that has pending work.
    pub fn take_next(&mut self) -> Option<Task> {
        // Round-robin: find first repo with a task
        let repo = self
            .queues
            .iter()
            .find(|(_, q)| !q.is_empty())
            .map(|(k, _)| k.clone());

        if let Some(repo) = repo {
            let task = self.queues.get_mut(&repo).and_then(|q| q.pop_front());
            // Clean up empty queues
            if self.queues.get(&repo).map_or(false, |q| q.is_empty()) {
                self.queues.remove(&repo);
            }
            task
        } else {
            None
        }
    }
}

/// Run the background queue processor.
pub async fn run_queue_processor(state: Arc<AppState>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    {
        let mut queue = state.task_queue.write().await;
        queue.set_notifier(tx);
    }

    tracing::info!("Queue processor started");

    loop {
        // Wait for notification
        let _ = rx.recv().await;

        // Process all available tasks
        loop {
            let task = {
                let mut queue = state.task_queue.write().await;
                queue.take_next()
            };

            let task = match task {
                Some(t) => t,
                None => break,
            };

            tracing::info!(task = %task.description(), "Processing task");

            match &task {
                Task::ResolveIssue {
                    installation_id,
                    repo_full_name,
                    clone_url,
                    default_branch,
                    issue_number,
                    issue_title,
                    issue_body,
                    mode,
                } => {
                    let result = workflow::issue::resolve_issue(
                        &state,
                        *installation_id,
                        repo_full_name,
                        clone_url,
                        default_branch,
                        *issue_number,
                        issue_title,
                        issue_body,
                        *mode,
                    )
                    .await;

                    match result {
                        Ok(outcome) => {
                            tracing::info!(
                                task = %task.description(),
                                outcome = ?outcome,
                                "Task completed"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                task = %task.description(),
                                error = %e,
                                "Task failed"
                            );
                        }
                    }
                }
                Task::RespondToReview {
                    installation_id,
                    repo_full_name,
                    clone_url,
                    pr_number,
                    pr_branch,
                    review_body,
                } => {
                    let result = workflow::review::respond_to_review(
                        &state,
                        *installation_id,
                        repo_full_name,
                        clone_url,
                        *pr_number,
                        pr_branch,
                        review_body,
                    )
                    .await;

                    match result {
                        Ok(outcome) => {
                            tracing::info!(
                                task = %task.description(),
                                outcome = ?outcome,
                                "Task completed"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                task = %task.description(),
                                error = %e,
                                "Task failed"
                            );
                        }
                    }
                }
            }
        }
    }
}
