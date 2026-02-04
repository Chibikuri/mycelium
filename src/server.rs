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
}

impl AppState {
    pub async fn new(config: AppConfig) -> crate::error::Result<Self> {
        let platform = GitHubPlatform::new(&config.github).await?;
        let task_queue = RwLock::new(TaskQueue::new());

        Ok(Self {
            config,
            platform,
            task_queue,
        })
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
