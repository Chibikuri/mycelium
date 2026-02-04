use async_trait::async_trait;
use octocrab::Octocrab;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::GitHubConfig;
use crate::error::{AppError, Result};
use crate::platform::types::*;
use crate::platform::Platform;

use super::auth::generate_app_jwt;
use super::mapper;

pub struct GitHubPlatform {
    config: GitHubConfig,
    /// Cache of installation tokens: installation_id -> (token, expiry)
    token_cache: Arc<RwLock<std::collections::HashMap<u64, (String, chrono::DateTime<chrono::Utc>)>>>,
}

impl GitHubPlatform {
    pub async fn new(config: &GitHubConfig) -> Result<Self> {
        // Validate the private key exists
        if !config.private_key_path.exists() {
            return Err(AppError::Config(format!(
                "GitHub App private key not found at: {}",
                config.private_key_path.display()
            )));
        }

        Ok(Self {
            config: config.clone(),
            token_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Get an octocrab instance authenticated as an installation.
    async fn installation_client(&self, installation_id: u64) -> Result<Octocrab> {
        let token = self.get_access_token(installation_id).await?;
        Octocrab::builder()
            .personal_token(token)
            .build()
            .map_err(|e| AppError::GitHubApi(format!("Failed to build octocrab client: {e}")))
    }

    fn parse_repo(repo_full_name: &str) -> Result<(&str, &str)> {
        let parts: Vec<&str> = repo_full_name.splitn(2, '/').collect();
        if parts.len() != 2 {
            return Err(AppError::GitHubApi(format!(
                "Invalid repo name: {repo_full_name}"
            )));
        }
        Ok((parts[0], parts[1]))
    }
}

#[async_trait]
impl Platform for GitHubPlatform {
    async fn get_access_token(&self, installation_id: u64) -> Result<String> {
        // Check cache
        {
            let cache = self.token_cache.read().await;
            if let Some((token, expiry)) = cache.get(&installation_id) {
                if *expiry > chrono::Utc::now() + chrono::Duration::minutes(5) {
                    return Ok(token.clone());
                }
            }
        }

        // Generate new token
        let jwt = generate_app_jwt(self.config.app_id, &self.config.private_key_path)?;

        let client = Octocrab::builder()
            .personal_token(jwt)
            .build()
            .map_err(|e| AppError::GitHubApi(format!("Failed to build JWT client: {e}")))?;

        let url = format!("/app/installations/{installation_id}/access_tokens");
        let response: serde_json::Value = client
            .post(&url, None::<&()>)
            .await
            .map_err(|e| AppError::GitHubApi(format!("Failed to create installation token: {e}")))?;

        let token = response["token"]
            .as_str()
            .ok_or_else(|| AppError::GitHubApi("No token in response".to_string()))?
            .to_string();

        let expires_at = response["expires_at"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(1));

        // Cache the token
        let mut cache = self.token_cache.write().await;
        cache.insert(installation_id, (token.clone(), expires_at));

        Ok(token)
    }

    async fn get_issue(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<Issue> {
        let client = self.installation_client(installation_id).await?;
        let (owner, repo) = Self::parse_repo(repo_full_name)?;

        let issue = client
            .issues(owner, repo)
            .get(issue_number)
            .await?;

        let comments_page = client
            .issues(owner, repo)
            .list_comments(issue_number)
            .per_page(100)
            .send()
            .await?;

        Ok(mapper::map_issue(&issue, comments_page.items))
    }

    async fn post_comment(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()> {
        let client = self.installation_client(installation_id).await?;
        let (owner, repo) = Self::parse_repo(repo_full_name)?;

        client
            .issues(owner, repo)
            .create_comment(issue_number, body)
            .await?;

        Ok(())
    }

    async fn create_pull_request(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        pr: &CreatePullRequest,
    ) -> Result<PullRequest> {
        let client = self.installation_client(installation_id).await?;
        let (owner, repo) = Self::parse_repo(repo_full_name)?;

        let created = client
            .pulls(owner, repo)
            .create(&pr.title, &pr.head_branch, &pr.base_branch)
            .body(&pr.body)
            .send()
            .await?;

        Ok(mapper::map_pull_request(created))
    }

    async fn add_label(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        let client = self.installation_client(installation_id).await?;
        let (owner, repo) = Self::parse_repo(repo_full_name)?;

        client
            .issues(owner, repo)
            .add_labels(issue_number, &[label.to_string()])
            .await?;

        Ok(())
    }

    async fn remove_label(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        let client = self.installation_client(installation_id).await?;
        let (owner, repo) = Self::parse_repo(repo_full_name)?;

        // octocrab doesn't have a direct remove_label, use the API directly
        let url = format!("/repos/{owner}/{repo}/issues/{issue_number}/labels/{label}");
        let _: serde_json::Value = client
            .delete(&url, None::<&()>)
            .await
            .map_err(|e| AppError::GitHubApi(format!("Failed to remove label: {e}")))?;

        Ok(())
    }

    async fn get_reviews(
        &self,
        installation_id: u64,
        repo_full_name: &str,
        pr_number: u64,
    ) -> Result<Vec<Review>> {
        let client = self.installation_client(installation_id).await?;
        let (owner, repo) = Self::parse_repo(repo_full_name)?;

        let url = format!("/repos/{owner}/{repo}/pulls/{pr_number}/reviews");
        let reviews: Vec<serde_json::Value> = client
            .get(&url, None::<&()>)
            .await
            .map_err(|e| AppError::GitHubApi(format!("Failed to fetch reviews: {e}")))?;

        let mut result = Vec::new();
        for review in reviews {
            let review_id = review["id"].as_u64().unwrap_or(0);

            // Fetch review comments
            let comments_url = format!(
                "/repos/{owner}/{repo}/pulls/{pr_number}/reviews/{review_id}/comments"
            );
            let comments: Vec<serde_json::Value> = client
                .get(&comments_url, None::<&()>)
                .await
                .unwrap_or_default();

            let review_comments: Vec<ReviewComment> = comments
                .into_iter()
                .map(|c| ReviewComment {
                    id: c["id"].as_u64().unwrap_or(0),
                    author: c["user"]["login"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                    body: c["body"].as_str().unwrap_or("").to_string(),
                    path: c["path"].as_str().map(|s| s.to_string()),
                    line: c["line"].as_u64(),
                    diff_hunk: c["diff_hunk"].as_str().map(|s| s.to_string()),
                })
                .collect();

            let state = match review["state"].as_str().unwrap_or("") {
                "APPROVED" => ReviewState::Approved,
                "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
                _ => ReviewState::Commented,
            };

            result.push(Review {
                id: review_id,
                author: review["user"]["login"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string(),
                body: review["body"].as_str().unwrap_or("").to_string(),
                state,
                comments: review_comments,
            });
        }

        Ok(result)
    }
}
