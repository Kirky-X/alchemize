// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Unified MinerU client trait .

use async_trait::async_trait;

use super::types::ParseOptions;
use super::validated_url::ValidatedFileUrl;
use crate::domain::error::AlchemizeError;

/// Unified abstraction for MinerU API clients .
///
/// Both Agent Lightweight and Precision Extract APIs share the same
/// high-level flow: submit → poll → get result.
///
/// This trait abstracts the differences, allowing consumers to use
/// either client interchangeably.
#[async_trait]
pub trait MineruClient: Send + Sync {
    /// Submit a parsing task and poll until completion.
    ///
    /// Returns the Markdown content on success.
    async fn parse(
        &self,
        file_url: &ValidatedFileUrl,
        options: ParseOptions,
    ) -> Result<String, AlchemizeError>;

    /// Get the maximum file size this client supports.
    fn max_file_size(&self) -> u64;
}
