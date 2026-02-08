pub mod github;
pub mod types;

use async_trait::async_trait;

use crate::error::Result;
use types::*;

#[async_trait]
pub trait Platform: Send + Sync {
    /// Get an installation-scoped access token.
    async fn get_access_token(&self, installation_id: u64) -> Result<String>;

    /// List all installations of this GitHub App.
    async fn list_installations(&self) -> Result<Vec<Installation>>;

    /// List all repositories accessible to an installation.
    async fn list_installation_repos(&self, installation_id: u64) -> Result<Vec<InstallationRepo>>;

    /// List open issues with a specific label.
    async fn list_open_issues_with_label(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        label: &str,
    ) -> Result<Vec<OpenIssue>>;

    /// Fetch a full issue with comments.
    async fn get_issue(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<Issue>;

    /// Post a comment on an issue or PR.
    async fn post_comment(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()>;

    /// Create a pull request.
    async fn create_pull_request(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        pr: &CreatePullRequest,
    ) -> Result<PullRequest>;

    /// Add a label to an issue or PR.
    async fn add_label(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()>;

    /// Remove a label from an issue or PR.
    async fn remove_label(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()>;

    /// Fetch a pull request.
    async fn get_pull_request(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        pr_number: u64,
    ) -> Result<PullRequest>;

    /// Fetch reviews on a PR.
    async fn get_reviews(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        pr_number: u64,
    ) -> Result<Vec<Review>>;
}
