// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Polling backoff shared by every MinerU client.

use std::time::Duration;

use super::audit::audit_mineru_retry;
use super::types::{INITIAL_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS};

/// Polling backoff (`agent`, `precision`, `wrapper`).
///
/// Attempts are 1-based: `INITIAL_POLL_INTERVAL_SECS × 2^(attempt-1)` capped
/// at `MAX_POLL_INTERVAL_SECS` — i.e. `2, 4, 8, 16, 30, 30, …` seconds — via
/// [`limiteron::retry::delay_for_attempt`] with zero jitter, so the polling
/// rhythm stays deterministic across clients.
///
/// Every scheduled retry emits a structured audit event (R-audit-001); this is
/// the single shared retry point for all three clients.
pub(super) fn poll_delay(endpoint_host: &str, attempt: u32) -> Duration {
    let delay = limiteron::retry::delay_for_attempt(
        attempt,
        Duration::from_secs(INITIAL_POLL_INTERVAL_SECS),
        2.0,
        Duration::from_secs(MAX_POLL_INTERVAL_SECS),
        0.0,
    );
    audit_mineru_retry(endpoint_host, attempt, delay.as_millis() as u64);
    delay
}
