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

use crate::application::builder::MineruConfig;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

use super::TaskStatus;
use super::audit::{audit_mineru_failure, endpoint_host_label};
use super::http_limit::{
    MAX_ERROR_RESPONSE_BYTES, MAX_POLL_RESPONSE_BYTES, MAX_SUBMIT_RESPONSE_BYTES,
    read_limited_json, read_limited_text, truncate_body,
};
use super::validated_url::validate_base_url;
use super::{redact_secrets, strip_presigned_params};
use crate::application::ports::{Converter, EngineProvenanceDescriptor};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::provenance::NetworkEgress;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

// Configuration

/// Configuration for the MinerU HTTP client.
#[derive(Clone)]
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

impl std::fmt::Debug for MinerUHttpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // api_token deliberately redacted: never leak secrets in logs
        // (mirrors MineruConfig's manual Debug).
        f.debug_struct("MinerUHttpConfig")
            .field("base_url", &self.base_url)
            .field("timeout_secs", &self.timeout_secs)
            .field(
                "api_token",
                &if self.api_token.is_some() {
                    "<redacted>"
                } else {
                    "None"
                },
            )
            .field("lightweight_max_bytes", &self.lightweight_max_bytes)
            .finish()
    }
}

impl Default for MinerUHttpConfig {
    fn default() -> Self {
        Self::from_config(&crate::application::builder::MineruConfig::default())
    }
}

impl MinerUHttpConfig {
    /// Build a [`MinerUConfig`] from the core [`MineruConfig`].
    ///
    /// `base_url`, `timeout_secs` and `api_token` are forwarded from the
    /// config; `lightweight_max_bytes` keeps its default. Dropping the token
    /// here would send unauthenticated requests to cloud endpoints.
    pub fn from_config(config: &MineruConfig) -> Self {
        Self {
            base_url: config.base_url.clone(),
            timeout_secs: config.timeout_secs,
            api_token: config.api_token.clone(),
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

/// Cap remote response bodies embedded in error messages: they can be large
/// and may carry internal detail, so never propagate them unbounded.
async fn read_error_body(resp: reqwest::Response) -> String {
    truncate_body(
        read_limited_text(resp, MAX_ERROR_RESPONSE_BYTES)
            .await
            .unwrap_or_else(|_| "<unreadable>".to_string()),
    )
}

/// Error-path credential stripping (R-redact-002). When a token is configured,
/// scrub its occurrences; always scrub presigned URL parameters (reqwest error
/// Display and upstream bodies can echo both).
fn scrub_error_text(text: &str, token: Option<&str>) -> String {
    let redacted = match token {
        Some(tok) => redact_secrets(text, &[tok]),
        None => text.to_string(),
    };
    strip_presigned_params(&redacted)
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
            // SSRF defense (connect-time): every resolved address is
            // revalidated against the same policy as parse-time validation,
            // closing the DNS-rebinding window (deny-by-default).
            .dns_resolver(std::sync::Arc::new(super::SsrfSafeResolver::default()))
            .build()
            .map_err(|e| {
                AlchemizeError::Other(crate::tr_args(
                    "failed-to-build-http-client",
                    &[("source", e.to_string())],
                ))
            })?;

        Ok(Self { http, config })
    }

    /// Submit a PDF file to the MinerU lightweight API.
    async fn submit_lightweight(
        &self,
        data: &[u8],
        filename: &str,
    ) -> Result<String, AlchemizeError> {
        let url = format!("{}/api/v1/agent/parse/url", self.config.base_url);

        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/pdf")
            .map_err(|e| {
                AlchemizeError::MineruConversion(crate::tr_args(
                    "mime-error",
                    &[("source", e.to_string())],
                ))
            })?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let mut builder = self.http.post(&url).multipart(form);
        if let Some(ref token) = self.config.api_token {
            builder = builder.bearer_auth(token);
        }

        debug!(url = %url, filename = %filename, "submitting PDF to MinerU lightweight API");

        let resp = builder.send().await.map_err(|e| {
            // reqwest errors can embed the full request URL (R-redact-002).
            let source = scrub_error_text(&e.to_string(), self.config.api_token.as_deref());
            AlchemizeError::MineruUnavailable(crate::tr_args(
                "cannot-reach-mineru",
                &[
                    ("base", self.config.base_url.to_string()),
                    ("source", source),
                ],
            ))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let kind = if status.is_server_error() {
                "http_5xx"
            } else {
                "http_4xx"
            };
            audit_mineru_failure(
                &endpoint_host_label(&self.config.base_url),
                status.as_u16(),
                0,
                kind,
            );
            let body = read_error_body(resp).await;
            // Upstream body may echo the token or a presigned URL (R-redact-002).
            let body = scrub_error_text(&body, self.config.api_token.as_deref());
            return Err(AlchemizeError::MineruConversion(crate::tr_args(
                "mineru-api-returned",
                &[("status", status.to_string()), ("body", body)],
            )));
        }

        let result: TaskSubmitResponse = read_limited_json(resp, MAX_SUBMIT_RESPONSE_BYTES)
            .await
            .map_err(|e| {
            AlchemizeError::MineruConversion(crate::tr_args(
                "failed-to-parse-mineru-response",
                &[("source", e.to_string())],
            ))
        })?;

        info!(task_id = %result.task_id, "{}", crate::tr("mineru-task-submitted"));
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
            // reqwest errors can embed the full request URL (R-redact-002).
            let source = scrub_error_text(&e.to_string(), self.config.api_token.as_deref());
            AlchemizeError::MineruUnavailable(crate::tr_args(
                "polling-mineru-task",
                &[("task", task_id.to_string()), ("source", source)],
            ))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let kind = if status.is_server_error() {
                "http_5xx"
            } else {
                "http_4xx"
            };
            audit_mineru_failure(
                &endpoint_host_label(&self.config.base_url),
                status.as_u16(),
                0,
                kind,
            );
            let body = read_error_body(resp).await;
            // Upstream body may echo the token or a presigned URL (R-redact-002).
            let body = scrub_error_text(&body, self.config.api_token.as_deref());
            return Err(AlchemizeError::MineruConversion(crate::tr_args(
                "mineru-poll-returned",
                &[("status", status.to_string()), ("body", body)],
            )));
        }

        read_limited_json(resp, MAX_POLL_RESPONSE_BYTES)
            .await
            .map_err(|e| {
                AlchemizeError::MineruConversion(crate::tr_args(
                    "failed-to-parse-mineru-poll-response",
                    &[("source", e.to_string())],
                ))
            })
    }

    /// Submit and poll until completion or timeout with exponential backoff.
    async fn parse_pdf(&self, data: &[u8], filename: &str) -> Result<String, AlchemizeError> {
        let task_id = self.submit_lightweight(data, filename).await?;

        let mut attempt: u32 = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(self.config.timeout_secs);

        loop {
            if std::time::Instant::now() > deadline {
                return Err(AlchemizeError::MineruConversion(crate::tr_args(
                    "mineru-task-timed-out",
                    &[
                        ("task", task_id.to_string()),
                        ("secs", self.config.timeout_secs.to_string()),
                    ],
                )));
            }

            let status_resp = self.poll_task(&task_id).await?;

            match status_resp.status {
                TaskStatus::Completed => {
                    let markdown = status_resp.markdown.unwrap_or_default();
                    info!(task_id = %task_id, len = markdown.len(), "{}", crate::tr("mineru-task-completed"));
                    return Ok(markdown);
                }
                TaskStatus::Failed => {
                    let error = scrub_error_text(
                        &truncate_body(
                            status_resp
                                .error
                                .unwrap_or_else(|| crate::tr("mineru-unknown-error")),
                        ),
                        self.config.api_token.as_deref(),
                    );
                    return Err(AlchemizeError::MineruConversion(crate::tr_args(
                        "mineru-task-failed",
                        &[("task", task_id.to_string()), ("error", error)],
                    )));
                }
                TaskStatus::Pending | TaskStatus::Processing => {
                    attempt += 1;
                    let delay = super::backoff::poll_delay(
                        &endpoint_host_label(&self.config.base_url),
                        attempt,
                    );
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
        let bytes = input.to_bytes().await.map_err(|e| {
            AlchemizeError::Other(crate::tr_args(
                "failed-to-read-mineru-input",
                &[("source", e.to_string())],
            ))
        })?;

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

    /// Data leaves this process: uploads go to the configured MinerU endpoint
    /// and results come back via presigned download URLs.
    fn provenance_descriptor(&self) -> EngineProvenanceDescriptor {
        EngineProvenanceDescriptor {
            network_egress: Some(NetworkEgress {
                endpoint_host: endpoint_host_of(&self.config.base_url),
                presigned: true,
            }),
            ..EngineProvenanceDescriptor::default()
        }
    }
}

/// Extract the host (no scheme/port/path/query/credentials) from a base URL.
fn endpoint_host_of(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| base_url.to_string())
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

    /// The descriptor declares the configured endpoint host only — never the
    /// full URL (path/query could carry credentials).
    #[test]
    fn provenance_descriptor_declares_endpoint_host_only() {
        let converter =
            MinerUConverter::new(MinerUHttpConfig::new("http://mineru.example:8888")).unwrap();
        let d = converter.provenance_descriptor();
        let egress = d.network_egress.expect("mineru must declare egress");
        assert_eq!(egress.endpoint_host, "mineru.example");
        assert!(!egress.endpoint_host.contains('/'));
        assert!(!egress.endpoint_host.contains('?'));
        assert!(!egress.endpoint_host.contains(':'));
        assert!(egress.presigned);
        assert!(d.external_tools.is_empty());
        assert!(d.model_bundles.is_empty());
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

    #[test]
    fn debug_redacts_api_token() {
        let config = MinerUHttpConfig::new("http://mineru:9999").with_token("secret-token");
        let dbg = format!("{config:?}");
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("secret-token"));
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    /// Sequential HTTP/1.1 responder with full-body drain（multipart 上传
    /// 场景需读完 Content-Length 再应答，防客户端写侧断裂）。
    async fn spawn_server(responses: Vec<(u16, &'static str)>) -> u16 {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut total = Vec::new();
                let mut buf = [0u8; 8192];
                let mut content_len: Option<usize> = None;
                let _ = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                total.extend_from_slice(&buf[..n]);
                                if content_len.is_none()
                                    && let Some(pos) =
                                        total.windows(4).position(|w| w == b"\r\n\r\n")
                                {
                                    let headers = String::from_utf8_lossy(&total[..pos]);
                                    for line in headers.lines() {
                                        if let Some(v) = line
                                            .to_ascii_lowercase()
                                            .strip_prefix("content-length:")
                                        {
                                            content_len = v.trim().parse().ok();
                                        }
                                    }
                                    let head_end = pos + 4;
                                    if let Some(len) = content_len
                                        && total.len() >= head_end + len
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                })
                .await;
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        port
    }

    fn pdf_bytes() -> Vec<u8> {
        b"%PDF-1.4 minimal fixture".to_vec()
    }

    fn converter(base: String) -> MinerUConverter {
        MinerUConverter::new(MinerUHttpConfig {
            base_url: base,
            api_token: Some("tok".to_string()),
            ..MinerUHttpConfig::default()
        })
        .unwrap()
    }

    fn pdf_input() -> Input {
        Input::Bytes {
            data: pdf_bytes(),
            filename: "doc.pdf".into(),
            mimetype: None,
            extension: Some("pdf".into()),
        }
    }

    /// 全链：multipart submit → poll completed → markdown。
    #[tokio::test]
    async fn wrapper_parse_full_chain() {
        let port = spawn_server(vec![
            (200, r##"{"task_id":"w-1"}"##),
            (
                200,
                r##"{"status":"completed","markdown":"# wrapped out","error":null}"##,
            ),
        ])
        .await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let result = c.convert(pdf_input()).await.unwrap();
        assert_eq!(result.markdown, "# wrapped out");
    }

    /// 任务失败：error 字段透传。
    #[tokio::test]
    async fn wrapper_failed_task() {
        let port = spawn_server(vec![
            (200, r##"{"task_id":"w-2"}"##),
            (
                200,
                r##"{"status":"failed","error":"oom","markdown":null}"##,
            ),
        ])
        .await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let err = c.convert(pdf_input()).await.unwrap_err();
        assert!(err.to_string().contains("oom"), "{}", err);
    }

    /// submit 非 2xx（multipart 路径）。
    #[tokio::test]
    async fn wrapper_submit_http_error() {
        let port = spawn_server(vec![(403, "forbidden")]).await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let err = c.convert(pdf_input()).await.unwrap_err();
        assert!(err.to_string().contains("403"), "{}", err);
    }

    /// poll 非 2xx。
    #[tokio::test]
    async fn wrapper_poll_http_error() {
        let port = spawn_server(vec![(200, r##"{"task_id":"w-3"}"##), (502, "bad gateway")]).await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let err = c.convert(pdf_input()).await.unwrap_err();
        assert!(err.to_string().contains("502"), "{}", err);
    }

    /// read_error_body：非 JSON 错误体截断进消息（不限 content-type）。
    #[tokio::test]
    async fn wrapper_error_body_truncated_into_message() {
        let port = spawn_server(vec![(500, "plain text boom")]).await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let err = c.convert(pdf_input()).await.unwrap_err();
        assert!(err.to_string().contains("plain text boom"), "{}", err);
    }

    /// MinerUHttpConfig Debug：api_token 缺失 → "None" 占位（红action 臂）。
    #[test]
    fn wrapper_config_debug_renders_none_token() {
        let rendered = format!("{:?}", MinerUHttpConfig::default());
        assert!(rendered.contains("None"), "{rendered}");
    }

    /// submit 200 但响应体非 JSON → 解析错误臂。
    #[tokio::test]
    async fn wrapper_submit_bad_json_errors() {
        let port = spawn_server(vec![(200, "this is not json")]).await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let err = c.convert(pdf_input()).await.unwrap_err();
        assert!(
            err.to_string().contains("failed to parse MinerU response"),
            "{err}"
        );
    }

    /// submit 成功后服务器消失：poll 连接拒收 → MineruUnavailable 臂。
    #[tokio::test]
    async fn wrapper_poll_refused_after_submit() {
        let port = spawn_server(vec![(200, r##"{"task_id":"w-r1"}"##)]).await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let err = c.convert(pdf_input()).await.unwrap_err();
        let anchor = crate::tr("polling-mineru-task")
            .split('{')
            .next()
            .unwrap()
            .trim()
            .to_string();
        assert!(err.to_string().contains(&anchor), "{err}");
    }

    /// poll 200 但响应体非 JSON → poll 解析错误臂。
    #[tokio::test]
    async fn wrapper_poll_bad_json_errors() {
        let port = spawn_server(vec![
            (200, r##"{"task_id":"w-r2"}"##),
            (200, "definitely not json"),
        ])
        .await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let err = c.convert(pdf_input()).await.unwrap_err();
        assert!(err.to_string().contains("failed to parse"), "{err}");
    }

    /// pending → 退避重试 → completed：轮询循环走 Pending/Processing 臂。
    #[tokio::test]
    async fn wrapper_pending_then_completed_retries() {
        let port = spawn_server(vec![
            (200, r##"{"task_id":"w-r3"}"##),
            (
                200,
                r##"{"status":"pending","markdown":null,"error":null}"##,
            ),
            (
                200,
                r##"{"status":"completed","markdown":"# after retry","error":null}"##,
            ),
        ])
        .await;
        let c = converter(format!("http://127.0.0.1:{port}"));
        let result = c.convert(pdf_input()).await.unwrap();
        assert_eq!(result.markdown, "# after retry");
    }

    /// Path 输入文件缺失 → 读取错误臂（to_bytes 失败）。
    #[tokio::test]
    async fn wrapper_missing_path_errors_on_read() {
        let c = converter("http://127.0.0.1:1".into());
        let err = c
            .convert(Input::Path(std::path::PathBuf::from(
                "/nonexistent/alchemize-missing.pdf",
            )))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("failed to read input"), "{err}");
    }
}
