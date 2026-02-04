use std::path::Path;
use std::process::Stdio;

use crate::error::{AppError, Result};

/// Run a git command in the specified directory and return stdout.
async fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Git(format!("Failed to run git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Git(format!(
            "git {} failed: {stderr}",
            args.join(" ")
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Clone a repository into the target directory.
pub async fn clone(url: &str, target: &Path, token: &str) -> Result<()> {
    // Insert token into the clone URL for authentication
    let authed_url = inject_token(url, token)?;

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth=1", &authed_url])
        .arg(target)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Git(format!("Failed to run git clone: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Redact the token from error messages
        let safe_stderr = stderr.replace(token, "***");
        return Err(AppError::Git(format!("git clone failed: {safe_stderr}")));
    }

    Ok(())
}

/// Fetch the full history for a shallow clone (needed for some operations).
pub async fn unshallow(dir: &Path) -> Result<()> {
    // This may fail if already unshallowed, which is fine
    let _ = run_git(dir, &["fetch", "--unshallow"]).await;
    Ok(())
}

/// Fetch a specific remote branch and check it out.
pub async fn fetch_and_checkout(dir: &Path, branch_name: &str) -> Result<()> {
    run_git(dir, &["fetch", "origin", branch_name]).await?;
    run_git(dir, &["checkout", "-b", branch_name, &format!("origin/{branch_name}")]).await?;
    Ok(())
}

/// Create and checkout a new branch.
pub async fn create_branch(dir: &Path, branch_name: &str) -> Result<()> {
    run_git(dir, &["checkout", "-b", branch_name]).await?;
    Ok(())
}

/// Checkout an existing branch.
pub async fn checkout(dir: &Path, branch_name: &str) -> Result<()> {
    run_git(dir, &["checkout", branch_name]).await?;
    Ok(())
}

/// Stage all changes.
pub async fn add_all(dir: &Path) -> Result<()> {
    run_git(dir, &["add", "-A"]).await?;
    Ok(())
}

/// Commit with a message.
pub async fn commit(dir: &Path, message: &str) -> Result<()> {
    // Configure git user for the commit
    run_git(dir, &["config", "user.name", "Mycelium Bot"]).await?;
    run_git(dir, &["config", "user.email", "mycelium[bot]@users.noreply.github.com"]).await?;

    run_git(dir, &["commit", "-m", message, "--allow-empty"]).await?;
    Ok(())
}

/// Push the current branch to origin.
pub async fn push(dir: &Path, branch_name: &str) -> Result<()> {
    run_git(dir, &["push", "origin", branch_name]).await?;
    Ok(())
}

/// Push with force (for review responses that amend).
pub async fn force_push(dir: &Path, branch_name: &str) -> Result<()> {
    run_git(dir, &["push", "--force-with-lease", "origin", branch_name]).await?;
    Ok(())
}

/// Check if there are any staged or unstaged changes.
pub async fn has_changes(dir: &Path) -> Result<bool> {
    let status = run_git(dir, &["status", "--porcelain"]).await?;
    Ok(!status.trim().is_empty())
}

/// Inject an access token into a GitHub HTTPS clone URL.
fn inject_token(url: &str, token: &str) -> Result<String> {
    if let Some(rest) = url.strip_prefix("https://") {
        Ok(format!("https://x-access-token:{token}@{rest}"))
    } else {
        Err(AppError::Git(format!(
            "Expected HTTPS clone URL, got: {url}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_token() {
        let url = "https://github.com/owner/repo.git";
        let token = "ghs_abc123";
        let result = inject_token(url, token).unwrap();
        assert_eq!(
            result,
            "https://x-access-token:ghs_abc123@github.com/owner/repo.git"
        );
    }

    #[test]
    fn test_inject_token_non_https() {
        let url = "git@github.com:owner/repo.git";
        let token = "ghs_abc123";
        assert!(inject_token(url, token).is_err());
    }
}
