// SPDX-License-Identifier: Apache-2.0
//! MinerU API integration (feature `mineru`).
//!
//! - [`MineruClient`] trait — unified abstraction for both APIs
//! - [`MineruAgentClient`] — lightweight API for small files
//! - [`MineruPrecisionClient`] — precision API for larger files (requires token)
//! - [`ValidatedFileUrl`] — SSRF-safe URL wrapper
//! - [`create_mineru_client`] — factory that selects the appropriate client

mod agent;
mod backoff;
mod client;
mod factory;
mod http_limit;
mod precision;
#[cfg(test)]
mod tests;
mod types;
mod validated_url;
mod wrapper;

pub use agent::MineruAgentClient;
pub use client::MineruClient;
pub use factory::create_mineru_client;
pub use precision::MineruPrecisionClient;
pub use types::{ParseOptions, TaskId, TaskStatus};
pub use validated_url::{DEFAULT_MINERU_ALLOWLIST, ValidatedFileUrl, validate_file_url};
pub use wrapper::{MinerUConverter, MinerUHttpConfig, TaskStatusResponse, TaskSubmitResponse};
