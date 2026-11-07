// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Shared types and constants for MinerU clients.

/// Default timeout for polling (5 minutes).
pub(super) const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Initial polling interval (2 seconds).
pub(super) const INITIAL_POLL_INTERVAL_SECS: u64 = 2;

/// Maximum polling interval (30 seconds).
pub(super) const MAX_POLL_INTERVAL_SECS: u64 = 30;

/// Options for MinerU parsing.
#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// Enable formula recognition (LaTeX extraction).
    pub enable_formula: bool,
    /// Enable table recognition.
    pub enable_table: bool,
    /// Model version: "vlm" (recommended), "pipeline", "MinerU-HTML".
    pub model_version: String,
}

impl ParseOptions {
    /// Create default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable formula recognition.
    pub fn with_formula(mut self, enable: bool) -> Self {
        self.enable_formula = enable;
        self
    }

    /// Enable table recognition.
    pub fn with_table(mut self, enable: bool) -> Self {
        self.enable_table = enable;
        self
    }

    /// Set model version.
    pub fn with_model(mut self, version: impl Into<String>) -> Self {
        self.model_version = version.into();
        self
    }
}

/// Task identifier returned by MinerU API.
///
/// The inner string is validated to contain only URL-safe characters
/// (alphanumerics, `-`, `_`) to prevent path traversal or URL injection
/// when the ID is interpolated into poll/status endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskId(String);

impl TaskId {
    /// Construct a new `TaskId` from a raw API response string.
    ///
    /// Only available within the `mineru` module — external callers cannot
    /// forge task IDs. The string is validated against an allowlist of
    /// characters (`A-Za-z0-9_-`) to prevent path traversal (`../`) or
    /// query-string injection (`?`, `&`, `=`) when the ID is interpolated
    /// into the poll URL.
    ///
    /// Returns `Err` if the string is empty or contains disallowed characters.
    pub(super) fn new(s: String) -> Result<Self, crate::domain::error::AlchemizeError> {
        if s.is_empty() {
            return Err(crate::domain::error::AlchemizeError::MineruUnavailable(
                "MinerU returned an empty task_id".to_string(),
            ));
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(crate::domain::error::AlchemizeError::MineruUnavailable(format!(
                "MinerU returned a task_id with disallowed characters: {s}"
            )));
        }
        Ok(Self(s))
    }

    /// Get the task ID as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Task status during polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task is queued waiting for processing.
    Pending,
    /// Task is currently being processed.
    Processing,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
}
