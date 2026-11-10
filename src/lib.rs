// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Alchemize — transmute documents into Markdown.

pub mod application;
pub mod domain;
pub mod infrastructure;

pub use application::builder::{Alchemize, AlchemizeBuilder, HtmlDeepEngineConfig, MineruConfig};
pub use application::registry::ConverterRegistry;
pub use domain::error::AlchemizeError;
pub use domain::input::{Input, InputFormat, detect_format};

/// Crate version (mirrors Cargo.toml via workspace inheritance).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Test-only helpers shared by unit tests that touch process-global state.
#[cfg(test)]
pub(crate) mod test_support {
    use tokio::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::const_new(());

    /// Serialize tests that read or write the process-global environment
    /// (`PATH`): `set_var` affects every thread, so unsynchronized swaps make
    /// unrelated tests observe a foreign `PATH`.
    ///
    /// Async-aware (`tokio::sync::Mutex`): some callers intentionally hold
    /// the guard across `await` points to serialize whole conversions, and a
    /// std guard held across `await` both blocks the executor and fails
    /// `clippy::await_holding_lock`.
    pub(crate) async fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }
}
