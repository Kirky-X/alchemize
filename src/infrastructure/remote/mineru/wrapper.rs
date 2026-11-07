// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! MinerU Converter wrapper implementing the [`Converter`] trait.
//!
//! This module provides [`MinerUConverter`] that implements the [`Converter`] trait
//! for integration with the parser registry. It handles local file uploads by
//! first uploading the file to a temporary URL, then using the MinerU API.
//!
//! For direct URL-based parsing (files already hosted on CDN), use
//! [`MineruAgentClient`] or [`MineruPrecisionClient`] directly.

use async_trait::async_trait;
use crate::application::builder::MineruConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

use super::validated_url::validate_base_url;
use super::TaskStatus;
use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

// Configuration

/// Configuration for the MinerU HTTP client.
#[derive(Debug, Clone)]
pub struct MinerUHttpConfig {
    /// Base URL of the MinerU service (e.g. `http://localhost:8888`).
    pub base_url: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Optional bearer token for authenticated endpoints.
    pub api_token: Option<String>,
    /// Maximum file size in bytes for the lightweight API (default 10MB).
    pub lightweight_max_bytes: u64,
}

impl Default for MinerUHttpConfig {
    fn default() -> Self {
        Self::from_config(&crate::application::builder::MineruConfig::default())
    }
}

impl MinerUHttpConfig {
    /// Build a [`MinerUConfig`] from the core [`MineruConfig`].
    ///
    /// `base_url` comes from config; `timeout_secs`, `api_token`, and
    /// `lightweight_max_bytes` keep their defaults. This is the config-driven
    /// entry point — the hardcoded `base_url` lives only in
    /// [`MineruConfig::default`].
    pub fn from_config(config: &MineruConfig) -> Self {
        Self {
            base_url: config.base_url.clone(),
            timeout_secs: 300,
            api_token: None,
            lightweight_max_bytes: 10 * 1024 * 1024, // 10MB
        }
    }

    /// Create a new config pointing at the given base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            ..Self::default()
        }
    }

    /// Set the API token for authenticated endpoints.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.api_token = Some(token.into());
        self
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

// API Response Types

/// Task submission response from MinerU API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSubmitResponse {
    /// Unique task identifier assigned by MinerU.
    pub task_id: String,
}

/// Task status response from MinerU polling endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusResponse {
    /// Current status of the task.
    pub status: TaskStatus,
    /// Result markdown content (present when status is Completed).
    pub markdown: Option<String>,
    /// Error message (present when status is Failed).
    pub error: Option<String>,
}

// MinerUConverter

/// PDF converter backed by the MinerU external service.
///
/// Sends PDF files to a MinerU HTTP API endpoint for high-fidelity conversion
/// to Markdown, including tables, formulas, and chart descriptions.
///
/// # Errors
///
/// Returns [`AlchemizeError::MineruUnavailable`] when the MinerU service cannot
/// be reached, and [`AlchemizeError::MineruConversion`] on API-level failures.
pub struct MinerUConverter {
    config: MinerUHttpConfig,
    http: reqwest::Client,
}

impl MinerUConverter {
    /// Create a new MinerU converter with the given configuration.
    ///
    /// Returns an error if the HTTP client cannot be constructed
    /// (e.g. invalid TLS backend configuration).
    pub fn new(config: MinerUHttpConfig) -> Result<Self, AlchemizeError> {
        validate_base_url(&config.base_url)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            // SSRF defense: never follow redirects. A compromised MinerU could
            // return 3xx to an internal address (e.g. cloud metadata).
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AlchemizeError::Other(format!("failed to build HTTP client: {e}")))?;

        Ok(Self { http, config })
    }

    /// Submit a PDF file to the MinerU lightweight API.
    async fn submit_lightweight(&self, data: &[u8], filename: &str) -> Result<String, AlchemizeError> {
        let url = format!("{}/api/v1/agent/parse/url", self.config.base_url);

        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/pdf")
            .map_err(|e| AlchemizeError::MineruConversion(format!("mime error: {e}")))?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let mut builder = self.http.post(&url).multipart(form);
        if let Some(ref token) = self.config.api_token {
            builder = builder.bearer_auth(token);
        }

        debug!(url = %url, filename = %filename, "submitting PDF to MinerU lightweight API");

        let resp = builder.send().await.map_err(|e| {
            AlchemizeError::MineruUnavailable(format!(
                "cannot reach MinerU at {}: {e}",
                self.config.base_url
            ))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AlchemizeError::MineruConversion(format!(
                "MinerU API returned {status}: {body}"
            )));
        }

        let result: TaskSubmitResponse = resp.json().await.map_err(|e| {
            AlchemizeError::MineruConversion(format!("failed to parse MinerU response: {e}"))
        })?;

        info!(task_id = %result.task_id, "MinerU task submitted");
        Ok(result.task_id)
    }

    /// Poll MinerU for task completion.
    async fn poll_task(&self, task_id: &str) -> Result<TaskStatusResponse, AlchemizeError> {
        let url = format!("{}/api/v1/agent/task/{}", self.config.base_url, task_id);

        debug!(task_id = %task_id, "polling MinerU task status");

        let mut builder = self.http.get(&url);
        if let Some(ref token) = self.config.api_token {
            builder = builder.bearer_auth(token);
        }

        let resp = builder.send().await.map_err(|e| {
            AlchemizeError::MineruUnavailable(format!("polling MinerU task {task_id}: {e}"))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AlchemizeError::MineruConversion(format!(
                "MinerU poll returned {status}: {body}"
            )));
        }

        resp.json().await.map_err(|e| {
            AlchemizeError::MineruConversion(format!("failed to parse MinerU poll response: {e}"))
        })
    }

    /// Submit and poll until completion or timeout with exponential backoff.
    async fn parse_pdf(&self, data: &[u8], filename: &str) -> Result<String, AlchemizeError> {
        let task_id = self.submit_lightweight(data, filename).await?;

        let mut attempt: u32 = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(self.config.timeout_secs);

        loop {
            if std::time::Instant::now() > deadline {
                return Err(AlchemizeError::MineruConversion(format!(
                    "MinerU task {task_id} timed out after {}s",
                    self.config.timeout_secs
                )));
            }

            let status_resp = self.poll_task(&task_id).await?;

            match status_resp.status {
                TaskStatus::Completed => {
                    let markdown = status_resp.markdown.unwrap_or_default();
                    info!(task_id = %task_id, len = markdown.len(), "MinerU task completed");
                    return Ok(markdown);
                }
                TaskStatus::Failed => {
                    let error = status_resp
                        .error
                        .unwrap_or_else(|| "unknown error".to_string());
                    return Err(AlchemizeError::MineruConversion(format!(
                        "MinerU task {task_id} failed: {error}"
                    )));
                }
                TaskStatus::Pending | TaskStatus::Processing => {
                    attempt += 1;
                    let delay = super::backoff::poll_delay(attempt);
                    debug!(
                        task_id = %task_id,
                        status = ?status_resp.status,
                        next_interval_secs = delay.as_secs(),
                        "task in progress, waiting"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

#[async_trait]
impl Converter for MinerUConverter {
    fn name(&self) -> &'static str {
        "mineru"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read input: {e}")))?;

        let filename = input.filename().unwrap_or("document.pdf");

        let markdown = self.parse_pdf(&bytes, filename).await?;

        let metadata = serde_json::json!({
            "source_format": "pdf",
            "converter": "mineru",
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::Api),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pdf"]
    }

    /// Lower priority value than the local PDF converter (10), so MinerU is
    /// only tried when the local chain fails (higher values are tried first).
    fn priority(&self) -> u8 {
        5
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_values() {
        let config = MinerUHttpConfig::default();
        assert_eq!(config.base_url, "http://localhost:8888");
        assert_eq!(config.timeout_secs, 300);
        assert!(config.api_token.is_none());
        assert_eq!(config.lightweight_max_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn config_builder_pattern() {
        let config = MinerUHttpConfig::new("http://mineru:9999")
            .with_token("secret-token")
            .with_timeout(600);
        assert_eq!(config.base_url, "http://mineru:9999");
        assert_eq!(config.timeout_secs, 600);
        assert_eq!(config.api_token.as_deref(), Some("secret-token"));
    }

    #[test]
    fn supported_extensions_is_pdf() {
        let converter = MinerUConverter::new(MinerUHttpConfig::default()).unwrap();
        assert_eq!(converter.supported_extensions(), &["pdf"]);
    }

    #[test]
    fn can_convert_pdf_input() {
        let converter = MinerUConverter::new(MinerUHttpConfig::default()).unwrap();
        let input = Input::Bytes {
            data: vec![],
            filename: "test.pdf".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(converter.can_convert(&input));
    }

    #[test]
    fn cannot_convert_non_pdf() {
        let converter = MinerUConverter::new(MinerUHttpConfig::default()).unwrap();
        let input = Input::Bytes {
            data: vec![],
            filename: "test.docx".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(!converter.can_convert(&input));
    }

    #[test]
    fn task_status_serde() {
        let json = r##"{"status":"completed","markdown":"# Hello","error":null}"##;
        let resp: TaskStatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, TaskStatus::Completed);
        assert_eq!(resp.markdown.as_deref(), Some("# Hello"));
        assert!(resp.error.is_none());
    }

    #[test]
    fn task_submit_response_serde() {
        let json = r#"{"task_id":"abc-123"}"#;
        let resp: TaskSubmitResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.task_id, "abc-123");
    }

    #[tokio::test]
    async fn convert_fails_when_service_unreachable() {
        let converter = MinerUConverter::new(MinerUHttpConfig::new("http://127.0.0.1:1")).unwrap();
        let input = Input::Bytes {
            data: b"%PDF-fake".to_vec(),
            filename: "test.pdf".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AlchemizeError::MineruUnavailable(_)));
    }
}
