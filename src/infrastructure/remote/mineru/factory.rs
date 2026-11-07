// Copyright © 2026 Kirky.X. All rights reserved.
//! Factory that selects the appropriate MinerU client based on file size .

use super::agent::MineruAgentClient;
use super::client::MineruClient;
use super::precision::MineruPrecisionClient;
use crate::domain::error::AlchemizeError;

/// Create a MinerU client based on file size .
///
/// - Files ≤10MB: Uses [`MineruAgentClient`] (no token required)
/// - Files >10MB: Uses [`MineruPrecisionClient`] (requires MINERU_API_TOKEN)
///
/// # Arguments
///
/// - `file_size_bytes`: Size of the file to be parsed
///
/// # Returns
///
/// A boxed [`MineruClient`] implementation.
pub fn create_mineru_client(file_size_bytes: u64) -> Result<Box<dyn MineruClient>, AlchemizeError> {
    const LIGHTWEIGHT_MAX_BYTES: u64 = 10 * 1024 * 1024; // 10MB

    if file_size_bytes <= LIGHTWEIGHT_MAX_BYTES {
        Ok(Box::new(MineruAgentClient::new()?))
    } else {
        Ok(Box::new(MineruPrecisionClient::from_env()?))
    }
}
