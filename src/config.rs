use serde::Deserialize;
use std::path::PathBuf;

use crate::error::{AppError, Result};

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub github: GitHubConfig,
    pub claude: ClaudeConfig,
    pub workspace: WorkspaceConfig,
    pub agent: AgentConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Deserialize, Clone)]
pub struct GitHubConfig {
    pub app_id: u64,
    pub private_key_path: PathBuf,
    pub webhook_secret: String,
    #[serde(default = "default_trigger_label")]
    pub trigger_label: String,
}

// Manual Debug impl to avoid leaking the webhook secret
impl std::fmt::Debug for GitHubConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubConfig")
            .field("app_id", &self.app_id)
            .field("private_key_path", &self.private_key_path)
            .field("webhook_secret", &"[REDACTED]")
            .field("trigger_label", &self.trigger_label)
            .finish()
    }
}

#[derive(Deserialize, Clone)]
pub struct ClaudeConfig {
    pub api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
}

// Manual Debug impl to avoid leaking the API key
impl std::fmt::Debug for ClaudeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeConfig")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("max_turns", &self.max_turns)
            .finish()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkspaceConfig {
    #[serde(default = "default_workspace_dir")]
    pub base_dir: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentConfig {
    #[serde(default = "default_max_file_size")]
    pub max_file_size_bytes: usize,
    #[serde(default = "default_max_search_results")]
    pub max_search_results: usize,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    3000
}

fn default_trigger_label() -> String {
    "mycelium".to_string()
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

fn default_max_tokens() -> u32 {
    16384
}

fn default_max_turns() -> u32 {
    50
}

fn default_workspace_dir() -> PathBuf {
    PathBuf::from("/tmp/mycelium-workspaces")
}

fn default_max_file_size() -> usize {
    512 * 1024 // 512 KB
}

fn default_max_search_results() -> usize {
    50
}

impl AppConfig {
    pub fn load(config_path: Option<&str>) -> Result<Self> {
        let mut builder = config::Config::builder();

        // Load from file if specified
        if let Some(path) = config_path {
            builder = builder.add_source(config::File::with_name(path));
        } else {
            // Try default paths
            builder = builder.add_source(
                config::File::with_name("mycelium")
                    .required(false),
            );
        }

        // Environment variable overrides with MYCELIUM_ prefix
        builder = builder.add_source(
            config::Environment::with_prefix("MYCELIUM")
                .separator("__")
                .try_parsing(true),
        );

        let config = builder
            .build()
            .map_err(|e| AppError::Config(e.to_string()))?;

        config
            .try_deserialize()
            .map_err(|e| AppError::Config(e.to_string()))
    }

    pub fn webhook_secret(&self) -> &str {
        &self.github.webhook_secret
    }

    pub fn claude_api_key(&self) -> &str {
        &self.claude.api_key
    }
}
