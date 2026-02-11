use std::collections::HashMap;
use std::sync::Arc;

use axum::{routing::post, Router};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;

use crate::config::AppConfig;
use crate::platform::github::GitHubPlatform;
use crate::queue::TaskQueue;

/// Reason why an issue was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationReason {
    /// The issue was closed.
    IssueClosed,
    /// The trigger label was removed (but issue is still open).
    LabelRemoved,
}

/// Info about an in-flight issue (for cleanup on shutdown).
#[derive(Debug, Clone)]
pub struct InFlightIssue {
    pub installation_id: u64,
    pub repo_full_name: String,
    pub issue_number: u64,
}

pub struct AppState {
    pub config: AppConfig,
    pub platform: GitHubPlatform,
    pub task_queue: RwLock<TaskQueue>,
    /// Map of cancelled issue keys ("owner/repo#123") to cancellation reason.
    pub cancelled: RwLock<HashMap<String, CancellationReason>>,
    /// Set of in-flight issues (those with :working label).
    pub in_flight: RwLock<HashMap<String, InFlightIssue>>,
}

fn issue_key(repo_full_name: &str, issue_number: u64) -> String {
    format!("{repo_full_name}#{issue_number}")
}

impl AppState {
    pub async fn new(config: AppConfig) -> crate::error::Result<Self> {
        let platform = GitHubPlatform::new(&config.github).await?;
        let task_queue = RwLock::new(TaskQueue::new());

        Ok(Self {
            config,
            platform,
            task_queue,
            cancelled: RwLock::new(HashMap::new()),
            in_flight: RwLock::new(HashMap::new()),
        })
    }

    /// Mark an issue as cancelled so in-flight agents stop.
    pub async fn cancel_issue(
        &self,
        repo_full_name: &str,
        issue_number: u64,
        reason: CancellationReason,
    ) {
        let key = issue_key(repo_full_name, issue_number);
        tracing::info!(key = %key, reason = ?reason, "Cancelling issue");
        self.cancelled.write().await.insert(key, reason);
    }

    /// Check if an issue has been cancelled and return the reason.
    pub async fn get_cancellation_reason(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Option<CancellationReason> {
        let key = issue_key(repo_full_name, issue_number);
        self.cancelled.read().await.get(&key).copied()
    }

    /// Check if an issue has been cancelled.
    pub async fn is_cancelled(&self, repo_full_name: &str, issue_number: u64) -> bool {
        let key = issue_key(repo_full_name, issue_number);
        self.cancelled.read().await.contains_key(&key)
    }

    /// Clear cancellation (after the task has been stopped).
    pub async fn clear_cancellation(&self, repo_full_name: &str, issue_number: u64) {
        let key = issue_key(repo_full_name, issue_number);
        self.cancelled.write().await.remove(&key);
    }

    /// Register an issue as in-flight (has :working label).
    pub async fn register_in_flight(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
    ) {
        let key = issue_key(repo_full_name, issue_number);
        self.in_flight.write().await.insert(
            key,
            InFlightIssue {
                installation_id,
                repo_full_name: repo_full_name.to_string(),
                issue_number,
            },
        );
    }

    /// Unregister an issue from in-flight tracking.
    pub async fn unregister_in_flight(&self, repo_full_name: &str, issue_number: u64) {
        let key = issue_key(repo_full_name, issue_number);
        self.in_flight.write().await.remove(&key);
    }

    /// Get all in-flight issues (for shutdown cleanup).
    pub async fn get_in_flight_issues(&self) -> Vec<InFlightIssue> {
        self.in_flight.read().await.values().cloned().collect()
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
