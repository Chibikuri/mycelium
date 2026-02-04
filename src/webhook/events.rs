use serde::Deserialize;

/// Top-level webhook event parsed from the payload based on X-GitHub-Event header.
#[derive(Debug)]
pub enum WebhookEvent {
    Issues(IssuesEvent),
    IssueComment(IssueCommentEvent),
    PullRequestReview(PullRequestReviewEvent),
    PullRequestReviewComment(PullRequestReviewCommentEvent),
    Ping,
    Unsupported(String),
}

#[derive(Debug, Deserialize)]
pub struct IssuesEvent {
    pub action: String,
    pub issue: IssuePayload,
    pub repository: RepositoryPayload,
    pub installation: Option<InstallationPayload>,
    pub label: Option<LabelPayload>,
}

#[derive(Debug, Deserialize)]
pub struct IssueCommentEvent {
    pub action: String,
    pub issue: IssuePayload,
    pub comment: CommentPayload,
    pub repository: RepositoryPayload,
    pub installation: Option<InstallationPayload>,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestReviewEvent {
    pub action: String,
    pub review: ReviewPayload,
    pub pull_request: PullRequestPayload,
    pub repository: RepositoryPayload,
    pub installation: Option<InstallationPayload>,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestReviewCommentEvent {
    pub action: String,
    pub comment: ReviewCommentPayload,
    pub pull_request: PullRequestPayload,
    pub repository: RepositoryPayload,
    pub installation: Option<InstallationPayload>,
}

#[derive(Debug, Deserialize)]
pub struct IssuePayload {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<LabelPayload>,
    pub user: UserPayload,
    pub pull_request: Option<serde_json::Value>, // Present if issue is a PR
}

#[derive(Debug, Deserialize)]
pub struct LabelPayload {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CommentPayload {
    pub id: u64,
    pub body: Option<String>,
    pub user: UserPayload,
}

#[derive(Debug, Deserialize)]
pub struct ReviewPayload {
    pub id: u64,
    pub body: Option<String>,
    pub state: String, // "approved", "changes_requested", "commented"
    pub user: UserPayload,
}

#[derive(Debug, Deserialize)]
pub struct ReviewCommentPayload {
    pub id: u64,
    pub body: Option<String>,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub user: UserPayload,
    pub diff_hunk: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestPayload {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub head: PullRequestRef,
    pub base: PullRequestRef,
    pub user: UserPayload,
}

#[derive(Debug, Deserialize)]
pub struct PullRequestRef {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
}

#[derive(Debug, Deserialize)]
pub struct RepositoryPayload {
    pub id: u64,
    pub full_name: String,
    pub clone_url: String,
    pub default_branch: String,
}

#[derive(Debug, Deserialize)]
pub struct UserPayload {
    pub login: String,
    pub id: u64,
    #[serde(rename = "type", default)]
    pub user_type: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallationPayload {
    pub id: u64,
}

impl WebhookEvent {
    pub fn parse(event_type: &str, payload: &[u8]) -> Result<Self, serde_json::Error> {
        match event_type {
            "issues" => {
                let event: IssuesEvent = serde_json::from_slice(payload)?;
                Ok(WebhookEvent::Issues(event))
            }
            "issue_comment" => {
                let event: IssueCommentEvent = serde_json::from_slice(payload)?;
                Ok(WebhookEvent::IssueComment(event))
            }
            "pull_request_review" => {
                let event: PullRequestReviewEvent = serde_json::from_slice(payload)?;
                Ok(WebhookEvent::PullRequestReview(event))
            }
            "pull_request_review_comment" => {
                let event: PullRequestReviewCommentEvent = serde_json::from_slice(payload)?;
                Ok(WebhookEvent::PullRequestReviewComment(event))
            }
            "ping" => Ok(WebhookEvent::Ping),
            other => Ok(WebhookEvent::Unsupported(other.to_string())),
        }
    }
}
