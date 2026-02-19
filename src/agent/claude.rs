use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

const CLAUDE_API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

pub struct ClaudeClient {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl ClaudeClient {
    pub fn new(api_key: &str, model: &str, max_tokens: u32) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // 5 min timeout for long Sonnet responses
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            api_key: api_key.to_string(),
            model: model.to_string(),
            max_tokens,
        }
    }

    pub async fn send_message(&self, request: &MessagesRequest) -> Result<MessagesResponse> {
        let response = self
            .client
            .post(CLAUDE_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await;

        let response = match response {
            Ok(r) => r,
            Err(e) => {
                // All reqwest send errors are transient (timeout, connection, DNS, etc.)
                return Err(AppError::ClaudeTransient(format!(
                    "HTTP request error: {e:#}"
                )));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(AppError::ClaudeRateLimited(format!(
                    "Rate limited (429): {body}"
                )));
            }
            // 5xx and 529 (overloaded) are transient
            if status.is_server_error() || status.as_u16() == 529 {
                return Err(AppError::ClaudeTransient(format!(
                    "Server error ({status}): {body}"
                )));
            }
            return Err(AppError::ClaudeApi(format!(
                "API returned {status}: {body}"
            )));
        }

        let body = response.json::<MessagesResponse>().await?;
        Ok(body)
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }
}

// --- Cache control ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

// --- Request types ---

#[derive(Debug, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: Vec<SystemContent>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
}

/// A block in the system prompt array (supports cache_control).
#[derive(Debug, Clone, Serialize)]
pub struct SystemContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemContent {
    /// Create a text block with cache_control: ephemeral.
    pub fn cached_text(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: text.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

// --- Response types ---

#[derive(Debug, Deserialize)]
pub struct MessagesResponse {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}
