use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};

use crate::queue::task::Task;
use crate::server::AppState;
use crate::webhook::events::WebhookEvent;
use crate::webhook::signature::verify_signature;

pub async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // Extract required headers
    let signature = match headers.get("x-hub-signature-256").and_then(|v| v.to_str().ok()) {
        Some(sig) => sig.to_string(),
        None => {
            tracing::warn!("Missing X-Hub-Signature-256 header");
            return StatusCode::UNAUTHORIZED;
        }
    };

    let event_type = match headers.get("x-github-event").and_then(|v| v.to_str().ok()) {
        Some(et) => et.to_string(),
        None => {
            tracing::warn!("Missing X-GitHub-Event header");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Verify signature
    if let Err(e) = verify_signature(state.config.webhook_secret(), &body, &signature) {
        tracing::warn!(error = %e, "Webhook signature verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    // Parse event
    let event = match WebhookEvent::parse(&event_type, &body) {
        Ok(event) => event,
        Err(e) => {
            tracing::error!(error = %e, event_type = %event_type, "Failed to parse webhook event");
            return StatusCode::BAD_REQUEST;
        }
    };

    tracing::info!(event_type = %event_type, "Received webhook event");

    match event {
        WebhookEvent::Issues(issues_event) => {
            handle_issues_event(&state, issues_event).await
        }
        WebhookEvent::IssueComment(comment_event) => {
            handle_issue_comment_event(&state, comment_event).await
        }
        WebhookEvent::PullRequestReview(review_event) => {
            handle_pr_review_event(&state, review_event).await
        }
        WebhookEvent::PullRequestReviewComment(_) => {
            // Handled as part of the review event
            StatusCode::OK
        }
        WebhookEvent::Ping => {
            tracing::info!("Received ping event");
            StatusCode::OK
        }
        WebhookEvent::Unsupported(event_type) => {
            tracing::debug!(event_type = %event_type, "Ignoring unsupported event");
            StatusCode::OK
        }
    }
}

async fn handle_issues_event(
    state: &AppState,
    event: crate::webhook::events::IssuesEvent,
) -> StatusCode {
    let trigger_label = &state.config.github.trigger_label;

    // We care about "labeled" action where the trigger label was added
    if event.action != "labeled" {
        return StatusCode::OK;
    }

    let label_match = event
        .label
        .as_ref()
        .map(|l| l.name == *trigger_label)
        .unwrap_or(false);

    if !label_match {
        return StatusCode::OK;
    }

    // Don't process pull requests via the issues event
    if event.issue.pull_request.is_some() {
        return StatusCode::OK;
    }

    let installation_id = match event.installation.as_ref() {
        Some(inst) => inst.id,
        None => {
            tracing::warn!("No installation ID in issues event");
            return StatusCode::BAD_REQUEST;
        }
    };

    tracing::info!(
        repo = %event.repository.full_name,
        issue = %event.issue.number,
        "Issue labeled with trigger label, enqueuing resolution task"
    );

    let task = Task::ResolveIssue {
        installation_id,
        repo_full_name: event.repository.full_name.clone(),
        clone_url: event.repository.clone_url.clone(),
        default_branch: event.repository.default_branch.clone(),
        issue_number: event.issue.number,
        issue_title: event.issue.title.clone(),
        issue_body: event.issue.body.clone().unwrap_or_default(),
    };

    let mut queue = state.task_queue.write().await;
    queue.enqueue(&event.repository.full_name, task);

    StatusCode::ACCEPTED
}

async fn handle_issue_comment_event(
    state: &AppState,
    event: crate::webhook::events::IssueCommentEvent,
) -> StatusCode {
    // Only handle new comments on issues (not PRs) that have the trigger label
    if event.action != "created" {
        return StatusCode::OK;
    }

    // Skip if it's a PR comment (handled differently)
    if event.issue.pull_request.is_some() {
        return StatusCode::OK;
    }

    let has_trigger = event
        .issue
        .labels
        .iter()
        .any(|l| l.name == state.config.github.trigger_label);

    if !has_trigger {
        return StatusCode::OK;
    }

    let installation_id = match event.installation.as_ref() {
        Some(inst) => inst.id,
        None => return StatusCode::BAD_REQUEST,
    };

    tracing::info!(
        repo = %event.repository.full_name,
        issue = %event.issue.number,
        "New comment on tracked issue, enqueuing resolution task"
    );

    let task = Task::ResolveIssue {
        installation_id,
        repo_full_name: event.repository.full_name.clone(),
        clone_url: event.repository.clone_url.clone(),
        default_branch: event.repository.default_branch.clone(),
        issue_number: event.issue.number,
        issue_title: event.issue.title.clone(),
        issue_body: event.issue.body.clone().unwrap_or_default(),
    };

    let mut queue = state.task_queue.write().await;
    queue.enqueue(&event.repository.full_name, task);

    StatusCode::ACCEPTED
}

async fn handle_pr_review_event(
    state: &AppState,
    event: crate::webhook::events::PullRequestReviewEvent,
) -> StatusCode {
    // Only respond to reviews requesting changes
    if event.action != "submitted" {
        return StatusCode::OK;
    }

    if event.review.state != "changes_requested" {
        return StatusCode::OK;
    }

    let installation_id = match event.installation.as_ref() {
        Some(inst) => inst.id,
        None => return StatusCode::BAD_REQUEST,
    };

    tracing::info!(
        repo = %event.repository.full_name,
        pr = %event.pull_request.number,
        "Changes requested on PR, enqueuing review response task"
    );

    let task = Task::RespondToReview {
        installation_id,
        repo_full_name: event.repository.full_name.clone(),
        clone_url: event.repository.clone_url.clone(),
        pr_number: event.pull_request.number,
        pr_branch: event.pull_request.head.ref_name.clone(),
        review_body: event.review.body.clone().unwrap_or_default(),
    };

    let mut queue = state.task_queue.write().await;
    queue.enqueue(&event.repository.full_name, task);

    StatusCode::ACCEPTED
}
