// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! MinerU Precision Extract API client .
//!
//! Used for larger files (>10MB or >20 pages).
//! Requires API token from environment variable `MINERU_API_TOKEN`.
//!
//! Endpoint: `/api/v4/extract/task`
//!
//! Limits: max 200MB, max 200 pages.

use crate::application::builder::MineruConfig;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::http_limit::{
    MAX_ERROR_RESPONSE_BYTES, MAX_POLL_RESPONSE_BYTES, MAX_RESULT_RESPONSE_BYTES,
    MAX_SUBMIT_RESPONSE_BYTES, read_limited_json, read_limited_text, truncate_body,
};
use super::types::DEFAULT_TIMEOUT_SECS;
use super::validated_url::{DEFAULT_MINERU_ALLOWLIST, validate_base_url, validate_file_url};
use super::{MineruClient, ParseOptions, TaskId, TaskStatus, ValidatedFileUrl};
use crate::domain::error::AlchemizeError;

/// Environment variable name for the API token.
pub(crate) const MINERU_API_TOKEN_ENV: &str = "MINERU_API_TOKEN";

/// Default base URL for MinerU Precision API.
///
/// Config override: set `precision_base_url` under `[parser.mineru]` and
/// construct via [`MineruPrecisionClient::from_config`].
pub(crate) const DEFAULT_PRECISION_BASE_URL: &str = "https://mineru.net";

/// Maximum file size supported by Precision API (200MB).
pub(crate) const PRECISION_MAX_FILE_SIZE: u64 = 200 * 1024 * 1024;

// API Types

/// Task submission request for Precision API.
#[derive(Debug, Clone, Serialize)]
struct PrecisionTaskSubmitRequest {
    url: String,
    enable_formula: bool,
    enable_table: bool,
    model_version: String,
}

/// Task submission response from Precision API.
#[derive(Debug, Clone, Deserialize)]
struct PrecisionTaskSubmitResponse {
    task_id: String,
}

/// Task status response from Precision API polling.
#[derive(Debug, Clone, Deserialize)]
struct PrecisionTaskStatusResponse {
    status: String,
    /// URL to download the result zip file.
    result_url: Option<String>,
    /// Direct markdown content (if available).
    markdown: Option<String>,
    error: Option<String>,
}

// MineruPrecisionClient

/// MinerU Precision Extract API client .
///
/// Suitable for larger files (up to 200MB, 200 pages).
/// Requires authentication via `MINERU_API_TOKEN` environment variable.
pub struct MineruPrecisionClient {
    http: Client,
    base_url: String,
    token: String,
    timeout_secs: u64,
}

impl MineruPrecisionClient {
    /// Create a client from the environment variable `MINERU_API_TOKEN`.
    ///
    /// Returns an error if the token is not set.
    pub fn from_env() -> Result<Self, AlchemizeError> {
        // External service token — follows third-party naming convention (no MNEMIS_ prefix).
        // Corresponding config path: [parser.mineru] api_token (not yet in config schema).
        let token = std::env::var(MINERU_API_TOKEN_ENV).map_err(|_| {
            AlchemizeError::Other(
                "MINERU_API_TOKEN environment variable not set. Required for Precision API."
                    .to_string(),
            )
        })?;
        Self::with_token(token)
    }

    /// Create a client using `precision_base_url` from `[parser.mineru]` config.
    ///
    /// The API token is still read from the `MINERU_API_TOKEN` environment
    /// variable. Returns an error if the token is not set.
    pub fn from_config(config: &MineruConfig) -> Result<Self, AlchemizeError> {
        // External service token — follows third-party naming convention (no MNEMIS_ prefix).
        // Corresponding config path: [parser.mineru] api_token (not yet in config schema).
        let token = std::env::var(MINERU_API_TOKEN_ENV).map_err(|_| {
            AlchemizeError::Other(
                "MINERU_API_TOKEN environment variable not set. Required for Precision API."
                    .to_string(),
            )
        })?;
        Self::with_base_url_and_token(&config.precision_base_url, token)
    }

    /// Create a client with an explicit token.
    pub fn with_token(token: impl Into<String>) -> Result<Self, AlchemizeError> {
        Self::with_base_url_and_token(DEFAULT_PRECISION_BASE_URL, token)
    }

    /// Create a client with custom base URL and token.
    pub fn with_base_url_and_token(
        base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Result<Self, AlchemizeError> {
        let base_url = base_url.into();
        let token = token.into();
        validate_base_url(&base_url)?;

        if token.is_empty() {
            return Err(AlchemizeError::Other(
                "MinerU API token cannot be empty".to_string(),
            ));
        }

        let http = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            // SSRF defense: never follow redirects. A compromised MinerU could
            // return 3xx to an internal address (e.g. cloud metadata), bypassing
            // the allowlist check on the original URL.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AlchemizeError::Other(format!("failed to build HTTP client: {e}")))?;

        Ok(Self {
            http,
            base_url,
            token,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        })
    }

    /// Set custom timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Submit a parsing task to the Precision API.
    async fn submit(
        &self,
        file_url: &ValidatedFileUrl,
        options: &ParseOptions,
    ) -> Result<TaskId, AlchemizeError> {
        let url = format!("{}/api/v4/extract/task", self.base_url);

        let body = PrecisionTaskSubmitRequest {
            url: file_url.as_str().to_string(),
            enable_formula: options.enable_formula,
            enable_table: options.enable_table,
            model_version: options.model_version.clone(),
        };

        debug!(
            url = %url,
            file_url = %file_url.as_str(),
            enable_formula = options.enable_formula,
            enable_table = options.enable_table,
            model_version = %options.model_version,
            "submitting task to MinerU Precision API"
        );

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
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
                "MinerU Precision API returned {status}: {body}"
            )));
        }

        let result: PrecisionTaskSubmitResponse =
            read_limited_json(resp, MAX_SUBMIT_RESPONSE_BYTES)
                .await
                .map_err(|e| {
                    AlchemizeError::MineruConversion(format!(
                        "failed to parse MinerU response: {e}"
                    ))
                })?;

        info!(task_id = %result.task_id, "MinerU Precision task submitted");
        TaskId::new(result.task_id)
    }

    /// Poll task status.
    async fn poll(
        &self,
        task_id: &TaskId,
    ) -> Result<(TaskStatus, Option<String>, Option<String>, Option<String>), AlchemizeError> {
        let url = format!("{}/api/v4/extract/task/{}", self.base_url, task_id.as_str());

        debug!(task_id = %task_id.as_str(), "polling MinerU Precision task status");

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| {
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

        let result: PrecisionTaskStatusResponse = read_limited_json(resp, MAX_POLL_RESPONSE_BYTES)
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

        Ok((status, result.result_url, result.markdown, result.error))
    }

    /// Fetch the result from a URL (if result_url is provided).
    ///
    /// # SSRF Defense
    ///
    /// `result_url` comes from the MinerU API response and is therefore
    /// second-order user input — it must be re-validated before fetching to
    /// prevent SSRF via a malicious or compromised MinerU service returning an
    /// internal URL. We validate against `DEFAULT_MINERU_ALLOWLIST`, which
    /// restricts the host to MinerU's official CDN domains. The bearer token
    /// is only attached after validation succeeds, so it cannot leak to
    /// arbitrary hosts.
    async fn fetch_result(&self, result_url: &str) -> Result<String, AlchemizeError> {
        debug!(result_url = %result_url, "fetching MinerU result");

        // Second-order SSRF defense: re-validate the URL returned by MinerU
        // before fetching. Validation against the MinerU allowlist guarantees
        // the host is a MinerU-controlled CDN domain, making bearer_auth safe.
        let validated = validate_file_url(result_url, DEFAULT_MINERU_ALLOWLIST)?;

        let resp = self
            .http
            .get(validated.as_str())
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| {
                AlchemizeError::MineruConversion(format!("failed to fetch result: {e}"))
            })?;

        if !resp.status().is_success() {
            return Err(AlchemizeError::MineruConversion(format!(
                "result fetch returned {}",
                resp.status()
            )));
        }

        // Try to parse as JSON with markdown field, otherwise return raw text
        let text = read_limited_text(resp, MAX_RESULT_RESPONSE_BYTES)
            .await
            .map_err(|e| AlchemizeError::MineruConversion(format!("failed to read result: {e}")))?;

        // If it looks like JSON, try to extract markdown field
        if text.trim_start().starts_with('{')
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(md) = json.get("markdown").and_then(|v| v.as_str())
        {
            return Ok(md.to_string());
        }

        Ok(text)
    }
}

#[async_trait]
impl MineruClient for MineruPrecisionClient {
    async fn parse(
        &self,
        file_url: &ValidatedFileUrl,
        options: ParseOptions,
    ) -> Result<String, AlchemizeError> {
        let task_id = self.submit(file_url, &options).await?;

        let mut attempt: u32 = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(self.timeout_secs);

        loop {
            if std::time::Instant::now() > deadline {
                return Err(AlchemizeError::MineruConversion(format!(
                    "MinerU task {} timed out after {}s",
                    task_id, self.timeout_secs
                )));
            }

            let (status, result_url, markdown, error) = self.poll(&task_id).await?;

            match status {
                TaskStatus::Completed => {
                    // Prefer direct markdown, otherwise fetch from result_url
                    let content = if let Some(md) = markdown {
                        md
                    } else if let Some(url) = result_url {
                        self.fetch_result(&url).await?
                    } else {
                        String::new()
                    };

                    info!(task_id = %task_id, len = content.len(), "MinerU Precision task completed");
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
        PRECISION_MAX_FILE_SIZE // 200MB
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation_requires_token() {
        // Without env var, should fail (env mutation is unsafe since 2024 edition)
        unsafe { std::env::remove_var(MINERU_API_TOKEN_ENV) };
        let result = MineruPrecisionClient::from_env();
        assert!(result.is_err());
    }

    #[test]
    fn client_creation_with_token() {
        let client = MineruPrecisionClient::with_token("test-token").unwrap();
        assert_eq!(client.token, "test-token");
        assert_eq!(client.base_url, DEFAULT_PRECISION_BASE_URL);
    }

    #[test]
    fn client_creation_empty_token_fails() {
        let result = MineruPrecisionClient::with_token("");
        assert!(result.is_err());
    }

    #[test]
    fn client_creation_invalid_url() {
        let result = MineruPrecisionClient::with_base_url_and_token("ftp://invalid", "token");
        assert!(result.is_err());
    }

    #[test]
    fn client_timeout_custom() {
        let client = MineruPrecisionClient::with_token("token")
            .unwrap()
            .with_timeout(600);
        assert_eq!(client.timeout_secs, 600);
    }

    #[test]
    fn max_file_size_is_200mb() {
        let client = MineruPrecisionClient::with_token("token").unwrap();
        assert_eq!(client.max_file_size(), 200 * 1024 * 1024);
    }

    #[test]
    fn precision_task_submit_request_serde() {
        let req = PrecisionTaskSubmitRequest {
            url: "https://example.com/file.pdf".to_string(),
            enable_formula: true,
            enable_table: false,
            model_version: "vlm".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("enable_formula"));
        assert!(json.contains("vlm"));
    }

    #[test]
    fn precision_task_submit_response_serde() {
        let json = r#"{"task_id":"xyz-789"}"#;
        let resp: PrecisionTaskSubmitResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.task_id, "xyz-789");
    }

    #[test]
    fn precision_task_status_response_serde() {
        let json = r##"{"status":"completed","result_url":"https://cdn.mineru.net/result.zip","markdown":null,"error":null}"##;
        let resp: PrecisionTaskStatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "completed");
        assert_eq!(
            resp.result_url.as_deref(),
            Some("https://cdn.mineru.net/result.zip")
        );
    }
}
