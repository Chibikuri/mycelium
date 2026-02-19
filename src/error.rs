use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Webhook verification failed: {0}")]
    WebhookVerification(String),

    #[error("GitHub API error: {0}")]
    GitHubApi(String),

    #[error("Git operation failed: {0}")]
    Git(String),

    #[error("Workspace error: {0}")]
    Workspace(String),

    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Claude API error: {0}")]
    ClaudeApi(String),

    #[error("Claude API rate limited: {0}")]
    ClaudeRateLimited(String),

    #[error("Claude API transient error: {0}")]
    ClaudeTransient(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<octocrab::Error> for AppError {
    fn from(e: octocrab::Error) -> Self {
        AppError::GitHubApi(e.to_string())
    }
}

impl From<git2::Error> for AppError {
    fn from(e: git2::Error) -> Self {
        AppError::Git(e.message().to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
