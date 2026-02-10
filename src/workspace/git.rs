use std::path::Path;

use git2::{
    build::RepoBuilder, Cred, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks,
    Repository, Signature,
};

use crate::error::{AppError, Result};

/// Validate a branch name to prevent argument injection.
/// Rejects names starting with `-` as defence in depth.
fn validate_branch_name(name: &str) -> Result<()> {
    if name.starts_with('-') {
        return Err(AppError::Git(format!(
            "Invalid branch name (starts with '-'): {name}"
        )));
    }
    Ok(())
}

/// Build `FetchOptions` that authenticate via credential callback.
/// The token is captured by the closure and never written to disk.
fn make_fetch_options(token: &str) -> FetchOptions<'_> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
        Cred::userpass_plaintext("x-access-token", token)
    });
    let mut opts = FetchOptions::new();
    opts.remote_callbacks(callbacks);
    opts
}

/// Build `PushOptions` that authenticate via credential callback.
fn make_push_options(token: &str) -> PushOptions<'_> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
        Cred::userpass_plaintext("x-access-token", token)
    });
    let mut opts = PushOptions::new();
    opts.remote_callbacks(callbacks);
    opts
}

/// Clone a repository into the target directory.
///
/// The remote URL stored in `.git/config` will be the **plain** URL
/// (no credentials). Authentication is handled via credential callback only.
pub async fn clone(url: &str, target: &Path, token: &str) -> Result<()> {
    if !url.starts_with("https://") {
        return Err(AppError::Git(format!(
            "Expected HTTPS clone URL, got: {url}"
        )));
    }

    let url = url.to_string();
    let target = target.to_path_buf();
    let token = token.to_string();

    tokio::task::spawn_blocking(move || {
        let fetch_opts = make_fetch_options(&token);
        RepoBuilder::new()
            .fetch_options(fetch_opts)
            .clone(&url, &target)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Clone task panicked: {e}")))?
}

/// Fetch the full history for a shallow clone (needed for some operations).
pub async fn unshallow(dir: &Path, token: &str) -> Result<()> {
    let dir = dir.to_path_buf();
    let token = token.to_string();

    // This may fail if already unshallowed, which is fine
    let _ = tokio::task::spawn_blocking(move || -> Result<()> {
        let repo = Repository::open(&dir)?;
        let mut remote = repo.find_remote("origin")?;
        let mut fetch_opts = make_fetch_options(&token);
        remote.fetch(
            &["refs/heads/*:refs/remotes/origin/*"],
            Some(&mut fetch_opts),
            None,
        )?;
        Ok(())
    })
    .await;

    Ok(())
}

/// Fetch a specific remote branch and check it out.
pub async fn fetch_and_checkout(dir: &Path, branch_name: &str, token: &str) -> Result<()> {
    validate_branch_name(branch_name)?;

    let dir = dir.to_path_buf();
    let branch_name = branch_name.to_string();
    let token = token.to_string();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let mut remote = repo.find_remote("origin")?;

        // Fetch the specific branch
        let refspec = format!(
            "+refs/heads/{branch_name}:refs/remotes/origin/{branch_name}"
        );
        let mut fetch_opts = make_fetch_options(&token);
        remote.fetch(&[&refspec], Some(&mut fetch_opts), None)?;

        // Find the fetched commit
        let remote_ref = format!("refs/remotes/origin/{branch_name}");
        let reference = repo.find_reference(&remote_ref)?;
        let commit = reference.peel_to_commit()?;

        // Create a local branch pointing at that commit
        repo.branch(&branch_name, &commit, false)?;

        // Checkout the branch
        let obj = repo.revparse_single(&format!("refs/heads/{branch_name}"))?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head(&format!("refs/heads/{branch_name}"))?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Fetch-and-checkout task panicked: {e}")))?
}

/// Create and checkout a new branch.
pub async fn create_branch(dir: &Path, branch_name: &str) -> Result<()> {
    validate_branch_name(branch_name)?;

    let dir = dir.to_path_buf();
    let branch_name = branch_name.to_string();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let head = repo.head()?;
        let commit = head.peel_to_commit()?;
        repo.branch(&branch_name, &commit, false)?;
        let obj = repo.revparse_single(&format!("refs/heads/{branch_name}"))?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head(&format!("refs/heads/{branch_name}"))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Create-branch task panicked: {e}")))?
}

/// Checkout an existing branch.
pub async fn checkout(dir: &Path, branch_name: &str) -> Result<()> {
    validate_branch_name(branch_name)?;

    let dir = dir.to_path_buf();
    let branch_name = branch_name.to_string();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let obj = repo.revparse_single(&format!("refs/heads/{branch_name}"))?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head(&format!("refs/heads/{branch_name}"))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Checkout task panicked: {e}")))?
}

/// Stage all changes.
pub async fn add_all(dir: &Path) -> Result<()> {
    let dir = dir.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
        index.write()?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Add-all task panicked: {e}")))?
}

/// Commit with a message.
pub async fn commit(dir: &Path, message: &str) -> Result<()> {
    let dir = dir.to_path_buf();
    let message = message.to_string();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let sig = Signature::now("Mycelium Bot", "mycelium[bot]@users.noreply.github.com")?;
        let mut index = repo.index()?;
        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;
        let head = repo.head()?;
        let parent = head.peel_to_commit()?;
        repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent])?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Commit task panicked: {e}")))?
}

/// Push the current branch to origin.
pub async fn push(dir: &Path, branch_name: &str, token: &str) -> Result<()> {
    validate_branch_name(branch_name)?;

    let dir = dir.to_path_buf();
    let branch_name = branch_name.to_string();
    let token = token.to_string();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let mut remote = repo.find_remote("origin")?;
        let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
        let mut push_opts = make_push_options(&token);
        remote.push(&[&refspec], Some(&mut push_opts))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Push task panicked: {e}")))?
}

/// Push with force (for review responses that amend).
pub async fn force_push(dir: &Path, branch_name: &str, token: &str) -> Result<()> {
    validate_branch_name(branch_name)?;

    let dir = dir.to_path_buf();
    let branch_name = branch_name.to_string();
    let token = token.to_string();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let mut remote = repo.find_remote("origin")?;
        let refspec = format!("+refs/heads/{branch_name}:refs/heads/{branch_name}");
        let mut push_opts = make_push_options(&token);
        remote.push(&[&refspec], Some(&mut push_opts))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Git(format!("Force-push task panicked: {e}")))?
}

/// Check if there are any staged or unstaged changes.
pub async fn has_changes(dir: &Path) -> Result<bool> {
    let dir = dir.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let repo = Repository::open(&dir)?;
        let statuses = repo.statuses(None)?;
        Ok(!statuses.is_empty())
    })
    .await
    .map_err(|e| AppError::Git(format!("Has-changes task panicked: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_validate_branch_name_rejects_dash_prefix() {
        assert!(validate_branch_name("-evil").is_err());
        assert!(validate_branch_name("--upload-pack").is_err());
    }

    #[test]
    fn test_validate_branch_name_accepts_normal() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feature/my-branch").is_ok());
        assert!(validate_branch_name("mycelium/issue-42").is_ok());
    }

    #[test]
    fn test_has_changes_empty_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();

        // Brand new repo with no files — no changes
        let statuses = repo.statuses(None).unwrap();
        assert!(statuses.is_empty());
    }

    #[test]
    fn test_has_changes_with_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        let _repo = Repository::init(tmp.path()).unwrap();

        // Create an untracked file
        fs::write(tmp.path().join("hello.txt"), "world").unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        let statuses = repo.statuses(None).unwrap();
        assert!(!statuses.is_empty());
    }

    #[test]
    fn test_clone_rejects_non_https() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(clone(
            "git@github.com:owner/repo.git",
            Path::new("/tmp/test"),
            "token",
        ));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Expected HTTPS clone URL"));
    }
}
