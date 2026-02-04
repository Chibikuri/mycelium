use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};

use crate::queue::task::{IssueMode, Task};
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
        WebhookEvent::PullRequestReviewComment(comment_event) => {
            handle_pr_review_comment_event(&state, comment_event).await
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
    let research_label = format!("{trigger_label}:research");

    // Handle issue closed or unlabeled — cancel any in-flight work
    if event.action == "closed" || event.action == "unlabeled" {
        if event.action == "closed" || event.label.as_ref().map_or(false, |l| {
            l.name == *trigger_label || l.name == research_label
        }) {
            tracing::info!(
                repo = %event.repository.full_name,
                issue = %event.issue.number,
                action = %event.action,
                "Issue closed or unlabeled, cancelling tasks"
            );
            let mut queue = state.task_queue.write().await;
            queue.cancel_issue(&event.repository.full_name, event.issue.number);
            state.cancel_issue(&event.repository.full_name, event.issue.number).await;
        }
        return StatusCode::OK;
    }

    // We care about "labeled" action where a trigger label was added
    if event.action != "labeled" {
        return StatusCode::OK;
    }

    let added_label = match event.label.as_ref() {
        Some(l) => &l.name,
        None => return StatusCode::OK,
    };

    // Determine mode from which label was added
    let mode = if added_label == &research_label {
        IssueMode::Research
    } else if added_label == trigger_label {
        IssueMode::Implement
    } else {
        return StatusCode::OK;
    };

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
        mode = ?mode,
        "Issue labeled with trigger label, enqueuing task"
    );

    let task = Task::ResolveIssue {
        installation_id,
        repo_full_name: event.repository.full_name.clone(),
        clone_url: event.repository.clone_url.clone(),
        default_branch: event.repository.default_branch.clone(),
        issue_number: event.issue.number,
        issue_title: event.issue.title.clone(),
        issue_body: event.issue.body.clone().unwrap_or_default(),
        mode,
    };

    let mut queue = state.task_queue.write().await;
    queue.enqueue(&event.repository.full_name, task);

    StatusCode::ACCEPTED
}

async fn handle_issue_comment_event(
    state: &AppState,
    event: crate::webhook::events::IssueCommentEvent,
) -> StatusCode {
    // Only handle new comments on issues (not PRs) that have a trigger label
    if event.action != "created" {
        return StatusCode::OK;
    }

    // Ignore comments from bots (including our own) to prevent feedback loops
    if event.comment.user.user_type == "Bot" || event.comment.user.login.ends_with("[bot]") {
        tracing::debug!(
            user = %event.comment.user.login,
            "Ignoring comment from bot"
        );
        return StatusCode::OK;
    }

    let installation_id = match event.installation.as_ref() {
        Some(inst) => inst.id,
        None => return StatusCode::BAD_REQUEST,
    };

    // PR comment — route to review workflow if the PR branch is a mycelium branch
    if event.issue.pull_request.is_some() {
        use crate::platform::Platform;

        let pr = match state
            .platform
            .get_pull_request(
                installation_id,
                &event.repository.full_name,
                event.issue.number,
            )
            .await
        {
            Ok(pr) => pr,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to fetch PR details for comment");
                return StatusCode::OK;
            }
        };

        // Only respond to comments on PRs created by mycelium
        if !pr.head_branch.starts_with("mycelium/") {
            return StatusCode::OK;
        }

        tracing::info!(
            repo = %event.repository.full_name,
            pr = %event.issue.number,
            "New comment on mycelium PR, enqueuing review response task"
        );

        let comment_body = event.comment.body.clone().unwrap_or_default();
        let task = Task::RespondToReview {
            installation_id,
            repo_full_name: event.repository.full_name.clone(),
            clone_url: event.repository.clone_url.clone(),
            pr_number: event.issue.number,
            pr_branch: pr.head_branch,
            review_body: comment_body,
        };

        let mut queue = state.task_queue.write().await;
        queue.enqueue(&event.repository.full_name, task);

        return StatusCode::ACCEPTED;
    }

    let trigger_label = &state.config.github.trigger_label;
    let research_label = format!("{trigger_label}:research");

    // Determine mode from labels on the issue
    let mode = if event.issue.labels.iter().any(|l| l.name == research_label) {
        IssueMode::Research
    } else if event.issue.labels.iter().any(|l| l.name == *trigger_label) {
        IssueMode::Implement
    } else {
        return StatusCode::OK;
    };

    tracing::info!(
        repo = %event.repository.full_name,
        issue = %event.issue.number,
        mode = ?mode,
        "New comment on tracked issue, enqueuing task"
    );

    let task = Task::ResolveIssue {
        installation_id,
        repo_full_name: event.repository.full_name.clone(),
        clone_url: event.repository.clone_url.clone(),
        default_branch: event.repository.default_branch.clone(),
        issue_number: event.issue.number,
        issue_title: event.issue.title.clone(),
        issue_body: event.issue.body.clone().unwrap_or_default(),
        mode,
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

async fn handle_pr_review_comment_event(
    state: &AppState,
    event: crate::webhook::events::PullRequestReviewCommentEvent,
) -> StatusCode {
    // Only handle new line comments
    if event.action != "created" {
        return StatusCode::OK;
    }

    // Ignore bot comments
    if event.comment.user.user_type == "Bot" || event.comment.user.login.ends_with("[bot]") {
        return StatusCode::OK;
    }

    // Only respond on mycelium branches
    if !event.pull_request.head.ref_name.starts_with("mycelium/") {
        return StatusCode::OK;
    }

    let installation_id = match event.installation.as_ref() {
        Some(inst) => inst.id,
        None => return StatusCode::BAD_REQUEST,
    };

    // Build context with file path and line info
    let comment_body = event.comment.body.clone().unwrap_or_default();
    let location = match (&event.comment.path, event.comment.line) {
        (Some(path), Some(line)) => format!("Line comment on `{path}` line {line}"),
        (Some(path), None) => format!("Line comment on `{path}`"),
        _ => "Line comment".to_string(),
    };
    let diff_context = event
        .comment
        .diff_hunk
        .as_deref()
        .map(|h| format!("\n\nDiff context:\n```\n{h}\n```"))
        .unwrap_or_default();

    let review_body = format!("{location}:\n\n{comment_body}{diff_context}");

    tracing::info!(
        repo = %event.repository.full_name,
        pr = %event.pull_request.number,
        path = ?event.comment.path,
        line = ?event.comment.line,
        "Line comment on mycelium PR, enqueuing review response task"
    );

    let task = Task::RespondToReview {
        installation_id,
        repo_full_name: event.repository.full_name.clone(),
        clone_url: event.repository.clone_url.clone(),
        pr_number: event.pull_request.number,
        pr_branch: event.pull_request.head.ref_name.clone(),
        review_body,
    };

    let mut queue = state.task_queue.write().await;
    queue.enqueue(&event.repository.full_name, task);

    StatusCode::ACCEPTED
}
