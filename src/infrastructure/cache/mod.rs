// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Conversion-result dedup cache (feature `cache`).
//!
//! Capability feature (not in `default`/`full`). The implementation lives in
//! `store`; this module only wires the public surface for the crate.

mod store;

pub use store::{CacheStats, ConversionCache};
