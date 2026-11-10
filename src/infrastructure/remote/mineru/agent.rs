// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! MinerU Agent Lightweight API client .
//!
//! Used for files ≤10MB and ≤20 pages.
//! No API token required.
//!
//! Endpoint: `/api/v1/agent/parse/url`

use crate::application::builder::MineruConfig;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::http_limit::{
    MAX_ERROR_RESPONSE_BYTES, MAX_POLL_RESPONSE_BYTES, MAX_SUBMIT_RESPONSE_BYTES,
    read_limited_json, read_limited_text, truncate_body,
};
use super::types::DEFAULT_TIMEOUT_SECS;
use super::validated_url::validate_base_url;
use super::{MineruClient, ParseOptions, TaskId, TaskStatus, ValidatedFileUrl};
use crate::domain::error::AlchemizeError;

/// Default base URL for MinerU Agent API.
///
/// Config override: set `agent_base_url` under `[parser.mineru]` and construct
/// via [`MineruAgentClient::from_config`].
pub(crate) const DEFAULT_AGENT_BASE_URL: &str = "https://mineru.net";

// API Types

/// Task submission response from Agent API.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentTaskSubmitResponse {
    task_id: String,
}

/// Task status response from Agent API polling.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentTaskStatusResponse {
    status: String,
    markdown: Option<String>,
    error: Option<String>,
}

// MineruAgentClient

/// MinerU Agent Lightweight API client .
///
/// Suitable for small files (≤10MB, ≤20 pages).
/// No authentication required.
pub struct MineruAgentClient {
    http: Client,
    base_url: String,
    timeout_secs: u64,
}

impl MineruAgentClient {
    /// Create a new Agent client with default configuration.
    pub fn new() -> Result<Self, AlchemizeError> {
        Self::with_base_url(DEFAULT_AGENT_BASE_URL)
    }

    /// Create a new Agent client using `agent_base_url` from `[parser.mineru]` config.
    pub fn from_config(config: &MineruConfig) -> Result<Self, AlchemizeError> {
        Self::with_base_url(&config.agent_base_url)
    }

    /// Create a new Agent client with a custom base URL.
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, AlchemizeError> {
        let base_url = base_url.into();
        validate_base_url(&base_url)?;

        let http = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            // SSRF defense: never follow redirects. A compromised MinerU could
            // return 3xx to an internal address (e.g. cloud metadata).
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AlchemizeError::Other(format!("failed to build HTTP client: {e}")))?;

        Ok(Self {
            http,
            base_url,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        })
    }

    /// Set custom timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Submit a parsing task to the Agent API.
    async fn submit(&self, file_url: &ValidatedFileUrl) -> Result<TaskId, AlchemizeError> {
        let url = format!("{}/api/v1/agent/parse/url", self.base_url);

        let body = serde_json::json!({
            "url": file_url.as_str(),
        });

        debug!(url = %url, file_url = %file_url.as_str(), "submitting task to MinerU Agent API");

        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| {
            AlchemizeError::MineruUnavailable(format!(
                "cannot reach MinerU at {}: {e}",
                self.base_url
            ))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = truncate_body(
                read_limited_text(resp, MAX_ERROR_RESPONSE_BYTES)
                    .await
                    .unwrap_or_else(|_| "<unreadable>".to_string()),
            );
            return Err(AlchemizeError::MineruConversion(format!(
                "MinerU Agent API returned {status}: {body}"
            )));
        }

        let result: AgentTaskSubmitResponse = read_limited_json(resp, MAX_SUBMIT_RESPONSE_BYTES)
            .await
            .map_err(|e| {
                AlchemizeError::MineruConversion(format!("failed to parse MinerU response: {e}"))
            })?;

        info!(task_id = %result.task_id, "MinerU Agent task submitted");
        TaskId::new(result.task_id)
    }

    /// Poll task status.
    async fn poll(
        &self,
        task_id: &TaskId,
    ) -> Result<(TaskStatus, Option<String>, Option<String>), AlchemizeError> {
        let url = format!("{}/api/v1/agent/task/{}", self.base_url, task_id.as_str());

        debug!(task_id = %task_id.as_str(), "polling MinerU Agent task status");

        let resp = self.http.get(&url).send().await.map_err(|e| {
            AlchemizeError::MineruUnavailable(format!("polling MinerU task {}: {e}", task_id))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = truncate_body(
                read_limited_text(resp, MAX_ERROR_RESPONSE_BYTES)
                    .await
                    .unwrap_or_else(|_| "<unreadable>".to_string()),
            );
            return Err(AlchemizeError::MineruConversion(format!(
                "MinerU poll returned {status}: {body}"
            )));
        }

        let result: AgentTaskStatusResponse = read_limited_json(resp, MAX_POLL_RESPONSE_BYTES)
            .await
            .map_err(|e| {
                AlchemizeError::MineruConversion(format!(
                    "failed to parse MinerU poll response: {e}"
                ))
            })?;

        let status = match result.status.as_str() {
            "pending" => TaskStatus::Pending,
            "processing" => TaskStatus::Processing,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            other => {
                // Remote-controlled: bound length and strip control chars
                // before it reaches the log.
                let status_label = crate::infrastructure::document::shared::sanitize_label(
                    &truncate_body(other.to_string()),
                );
                warn!(status = %status_label, "unknown task status");
                TaskStatus::Failed
            }
        };

        Ok((status, result.markdown, result.error))
    }
}

#[async_trait]
impl MineruClient for MineruAgentClient {
    async fn parse(
        &self,
        file_url: &ValidatedFileUrl,
        _options: ParseOptions,
    ) -> Result<String, AlchemizeError> {
        let task_id = self.submit(file_url).await?;

        let mut attempt: u32 = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(self.timeout_secs);

        loop {
            if std::time::Instant::now() > deadline {
                return Err(AlchemizeError::MineruConversion(format!(
                    "MinerU task {} timed out after {}s",
                    task_id, self.timeout_secs
                )));
            }

            let (status, markdown, error) = self.poll(&task_id).await?;

            match status {
                TaskStatus::Completed => {
                    let content = markdown.unwrap_or_default();
                    info!(task_id = %task_id, len = content.len(), "MinerU Agent task completed");
                    return Ok(content);
                }
                TaskStatus::Failed => {
                    let err_msg =
                        truncate_body(error.unwrap_or_else(|| "unknown error".to_string()));
                    return Err(AlchemizeError::MineruConversion(format!(
                        "MinerU task {} failed: {}",
                        task_id, err_msg
                    )));
                }
                TaskStatus::Pending | TaskStatus::Processing => {
                    attempt += 1;
                    let delay = super::backoff::poll_delay(attempt);
                    debug!(
                        task_id = %task_id,
                        status = ?status,
                        next_interval_secs = delay.as_secs(),
                        "task in progress, waiting"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    fn max_file_size(&self) -> u64 {
        10 * 1024 * 1024 // 10MB
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation_default() {
        let client = MineruAgentClient::new().unwrap();
        assert_eq!(client.base_url, DEFAULT_AGENT_BASE_URL);
        assert_eq!(client.timeout_secs, DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn client_creation_custom_url() {
        let client = MineruAgentClient::with_base_url("http://localhost:8888").unwrap();
        assert_eq!(client.base_url, "http://localhost:8888");
    }

    #[test]
    fn client_creation_invalid_url() {
        let result = MineruAgentClient::with_base_url("ftp://invalid");
        assert!(result.is_err());
    }

    #[test]
    fn client_timeout_custom() {
        let client = MineruAgentClient::new().unwrap().with_timeout(600);
        assert_eq!(client.timeout_secs, 600);
    }

    #[test]
    fn max_file_size_is_10mb() {
        let client = MineruAgentClient::new().unwrap();
        assert_eq!(client.max_file_size(), 10 * 1024 * 1024);
    }

    #[test]
    fn agent_task_submit_response_serde() {
        let json = r#"{"task_id":"abc-123"}"#;
        let resp: AgentTaskSubmitResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.task_id, "abc-123");
    }

    #[test]
    fn agent_task_status_response_serde() {
        let json = r##"{"status":"completed","markdown":"# Hello","error":null}"##;
        let resp: AgentTaskStatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "completed");
        assert_eq!(resp.markdown.as_deref(), Some("# Hello"));
        assert!(resp.error.is_none());
    }
}
