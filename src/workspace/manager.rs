use std::path::{Path, PathBuf};

use crate::config::WorkspaceConfig;
use crate::error::{AppError, Result};
use crate::workspace::git;

/// Manages workspace directories for agent operations.
pub struct WorkspaceManager {
    base_dir: PathBuf,
}

/// A checked-out workspace ready for the agent to work in.
pub struct Workspace {
    pub path: PathBuf,
    pub branch: String,
}

impl WorkspaceManager {
    pub fn new(config: &WorkspaceConfig) -> Self {
        Self {
            base_dir: config.base_dir.clone(),
        }
    }

    /// Clean up an existing workspace directory and ensure its parent exists.
    async fn prepare_workspace_dir(path: &Path) -> Result<()> {
        if path.exists() {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|e| AppError::Workspace(format!("Failed to clean workspace: {e}")))?;
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Workspace(format!("Failed to create workspace dir: {e}")))?;
        }
        Ok(())
    }

    /// Set up a workspace for a new issue: clone the repo and create a branch.
    pub async fn setup_for_issue(
        &self,
        clone_url: &str,
        token: &str,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<Workspace> {
        let branch = format!("mycelium/issue-{issue_number}");
        let workspace_path = self.workspace_path(repo_full_name, &branch);

        Self::prepare_workspace_dir(&workspace_path).await?;

        // Clone
        git::clone(clone_url, &workspace_path, token).await?;

        // Create branch
        git::create_branch(&workspace_path, &branch).await?;

        Ok(Workspace {
            path: workspace_path,
            branch,
        })
    }

    /// Set up a workspace for responding to a PR review: clone and checkout existing branch.
    pub async fn setup_for_review(
        &self,
        clone_url: &str,
        token: &str,
        repo_full_name: &str,
        branch: &str,
    ) -> Result<Workspace> {
        let workspace_path = self.workspace_path(repo_full_name, branch);

        Self::prepare_workspace_dir(&workspace_path).await?;

        // Clone (shallow, default branch only)
        git::clone(clone_url, &workspace_path, token).await?;

        // Fetch and checkout the specific branch
        git::fetch_and_checkout(&workspace_path, branch, token).await?;

        Ok(Workspace {
            path: workspace_path,
            branch: branch.to_string(),
        })
    }

    /// Commit and push changes from the workspace.
    ///
    /// When `force` is true the push uses `+refs/…` so it overwrites the remote
    /// branch even if histories have diverged (needed when re-processing an issue
    /// whose branch already exists from a previous attempt).
    pub async fn finalize(
        &self,
        workspace: &Workspace,
        commit_message: &str,
        token: &str,
        force: bool,
    ) -> Result<bool> {
        if !git::has_changes(&workspace.path).await? {
            tracing::info!("No changes to commit");
            return Ok(false);
        }

        git::add_all(&workspace.path).await?;
        git::commit(&workspace.path, commit_message).await?;
        if force {
            git::force_push(&workspace.path, &workspace.branch, token).await?;
        } else {
            git::push(&workspace.path, &workspace.branch, token).await?;
        }

        Ok(true)
    }

    /// Clean up a workspace directory.
    pub async fn cleanup(&self, workspace: &Workspace) -> Result<()> {
        if workspace.path.exists() {
            tokio::fs::remove_dir_all(&workspace.path)
                .await
                .map_err(|e| AppError::Workspace(format!("Failed to cleanup workspace: {e}")))?;
        }
        Ok(())
    }

    fn workspace_path(&self, repo_full_name: &str, branch: &str) -> PathBuf {
        let safe_name = repo_full_name.replace('/', "__");
        let safe_branch = branch.replace('/', "__");
        self.base_dir.join(format!("{safe_name}__{safe_branch}"))
    }

    /// Verify a path is within the workspace (path traversal protection).
    pub fn verify_path(workspace_root: &Path, requested_path: &Path) -> Result<PathBuf> {
        let full_path = workspace_root.join(requested_path);

        // Canonicalize to resolve .. and symlinks
        // If the file doesn't exist yet (create_file), canonicalize the parent
        let canonical = if full_path.exists() {
            full_path.canonicalize()
        } else {
            // For new files, verify the parent directory exists and is within workspace
            let parent = full_path
                .parent()
                .ok_or_else(|| AppError::Workspace("Invalid file path".to_string()))?;

            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| AppError::Workspace(format!("Failed to create directory: {e}")))?;
            }

            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| AppError::Workspace(format!("Failed to resolve path: {e}")))?;

            let file_name = full_path
                .file_name()
                .ok_or_else(|| AppError::Workspace("Invalid file name".to_string()))?;

            Ok(canonical_parent.join(file_name))
        }
        .map_err(|e| AppError::Workspace(format!("Failed to resolve path: {e}")))?;

        let canonical_root = workspace_root
            .canonicalize()
            .map_err(|e| AppError::Workspace(format!("Failed to resolve workspace root: {e}")))?;

        if !canonical.starts_with(&canonical_root) {
            return Err(AppError::Workspace(format!(
                "Path traversal detected: {} is outside workspace",
                requested_path.display()
            )));
        }

        Ok(canonical)
    }
}
