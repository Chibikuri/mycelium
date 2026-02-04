use std::collections::HashSet;
use std::sync::Arc;

use axum::{routing::post, Router};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;

use crate::config::AppConfig;
use crate::platform::github::GitHubPlatform;
use crate::queue::TaskQueue;

pub struct AppState {
    pub config: AppConfig,
    pub platform: GitHubPlatform,
    pub task_queue: RwLock<TaskQueue>,
    /// Set of cancelled issue keys ("owner/repo#123") for in-flight cancellation.
    pub cancelled: RwLock<HashSet<String>>,
}

impl AppState {
    pub async fn new(config: AppConfig) -> crate::error::Result<Self> {
        let platform = GitHubPlatform::new(&config.github).await?;
        let task_queue = RwLock::new(TaskQueue::new());

        Ok(Self {
            config,
            platform,
            task_queue,
            cancelled: RwLock::new(HashSet::new()),
        })
    }

    /// Mark an issue as cancelled so in-flight agents stop.
    pub async fn cancel_issue(&self, repo_full_name: &str, issue_number: u64) {
        let key = format!("{repo_full_name}#{issue_number}");
        tracing::info!(key = %key, "Cancelling issue");
        self.cancelled.write().await.insert(key);
    }

    /// Check if an issue has been cancelled.
    pub async fn is_cancelled(&self, repo_full_name: &str, issue_number: u64) -> bool {
        let key = format!("{repo_full_name}#{issue_number}");
        self.cancelled.read().await.contains(&key)
    }

    /// Clear cancellation (after the task has been stopped).
    pub async fn clear_cancellation(&self, repo_full_name: &str, issue_number: u64) {
        let key = format!("{repo_full_name}#{issue_number}");
        self.cancelled.write().await.remove(&key);
    }
}

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/webhooks/github", post(crate::webhook::handler::handle_webhook))
        .route("/health", axum::routing::get(health_check))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health_check() -> &'static str {
    "ok"
}
