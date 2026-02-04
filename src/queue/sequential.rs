use std::collections::HashMap;

use tokio::sync::mpsc;

use super::task::Task;

/// Per-repo sequential task queue.
///
/// Each repository gets its own channel so tasks for the same repo
/// are processed sequentially, avoiding git conflicts.
pub struct PerRepoQueue {
    senders: HashMap<String, mpsc::UnboundedSender<Task>>,
}

impl PerRepoQueue {
    pub fn new() -> Self {
        Self {
            senders: HashMap::new(),
        }
    }

    /// Enqueue a task for a repository.
    /// Creates a new channel + processor if this is the first task for the repo.
    pub fn enqueue<F>(&mut self, repo: &str, task: Task, processor: F)
    where
        F: Fn(mpsc::UnboundedReceiver<Task>) + Send + 'static,
    {
        if let Some(sender) = self.senders.get(repo) {
            if sender.send(task.clone()).is_err() {
                // Channel closed, recreate
                self.senders.remove(repo);
                self.create_and_send(repo, task, processor);
            }
        } else {
            self.create_and_send(repo, task, processor);
        }
    }

    fn create_and_send<F>(&mut self, repo: &str, task: Task, processor: F)
    where
        F: Fn(mpsc::UnboundedReceiver<Task>) + Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(task).expect("freshly created channel");
        self.senders.insert(repo.to_string(), tx);
        tokio::spawn(async move {
            processor(rx);
        });
    }
}
