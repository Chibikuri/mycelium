use crate::platform::types;

/// Map octocrab Issue to our platform Issue type.
pub fn map_issue(
    issue: &octocrab::models::issues::Issue,
    comments: Vec<octocrab::models::issues::Comment>,
) -> types::Issue {
    types::Issue {
        number: issue.number,
        title: issue.title.clone(),
        body: issue.body.clone().unwrap_or_default(),
        labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
        comments: comments.into_iter().map(map_comment).collect(),
    }
}

fn map_comment(comment: octocrab::models::issues::Comment) -> types::Comment {
    types::Comment {
        id: comment.id.into_inner(),
        author: comment.user.login,
        body: comment.body.unwrap_or_default(),
    }
}

pub fn map_pull_request(pr: octocrab::models::pulls::PullRequest) -> types::PullRequest {
    types::PullRequest {
        number: pr.number,
        title: pr.title.clone().unwrap_or_default(),
        body: pr.body.clone().unwrap_or_default(),
        head_branch: pr
            .head
            .label
            .clone()
            .unwrap_or_default(),
        base_branch: pr
            .base
            .label
            .clone()
            .unwrap_or_default(),
    }
}
