// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Conversion-result dedup cache implementation (feature `cache`).
//!
//! High-cost, idempotent conversion paths (MinerU cloud conversion, local ASR
//! transcription) can reuse a prior result within a TTL window, so the same
//! untrusted input is handed to the external service only once. Storage,
//! bloom admission, and TTL eviction come from `oxcache`; single-flight is
//! implemented here over `Mutex<HashSet> + Notify` so it does not depend on
//! oxcache's async API surface.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::VERSION;
use crate::domain::error::AlchemizeError;
use crate::domain::feature_map::enabled_features;
use crate::domain::result::ConvertResult;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

/// Default capacity of the in-memory cache (entries), before TTL eviction.
const DEFAULT_CAPACITY: u64 = 1024;

/// Cache key: SHA-256 of `content ‖ VERSION ‖ enabled_features ‖ engine`,
/// rendered as lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(String);

impl CacheKey {
    /// Borrow the backing `String` for oxcache's `&K` API.
    fn inner(&self) -> &String {
        &self.0
    }
}

/// Cache counters: hits, misses, and coalesced single-flight waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub singleflight_waits: u64,
}

/// TTL dedup cache for expensive conversion paths.
pub struct ConversionCache {
    inner: oxcache::Cache<String, ConvertResult>,
    in_flight: Mutex<HashSet<CacheKey>>,
    notify: Notify,
    hits: AtomicU64,
    misses: AtomicU64,
    singleflight_waits: AtomicU64,
}

impl ConversionCache {
    /// Build a cache that evicts entries after `ttl`.
    pub fn new(ttl: Duration) -> Self {
        let inner = oxcache::Cache::builder()
            .capacity(DEFAULT_CAPACITY)
            .ttl(ttl)
            .build_sync()
            .expect("in-memory cache backend must build");
        Self {
            inner,
            in_flight: Mutex::new(HashSet::new()),
            notify: Notify::new(),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            singleflight_waits: AtomicU64::new(0),
        }
    }

    /// Derive a stable key from content, build version, feature fingerprint,
    /// and engine name. Any change to those inputs yields a different key.
    pub fn key_for(content: &[u8], engine_name: &str) -> CacheKey {
        let mut hasher = Sha256::new();
        hasher.update(content);
        hasher.update(VERSION.as_bytes());
        hasher.update(enabled_features().join(",").as_bytes());
        hasher.update(engine_name.as_bytes());
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        CacheKey(hex)
    }

    /// Look up a result. Bloom-negative and expired entries return `None`.
    pub async fn get(&self, key: &CacheKey) -> Option<ConvertResult> {
        let hit = self.inner.get(key.inner()).await.ok().flatten();
        if hit.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        hit
    }

    /// Execute `factory` once per key, coalescing concurrent callers.
    ///
    /// The first caller registers the key and runs the factory; concurrent
    /// callers wait on the broadcast and then read the stored result. A failed
    /// factory writes nothing; a waiter that sees no entry runs the factory
    /// itself and observes the same error semantics (no negative caching).
    pub async fn get_or_insert_with<F, Fut>(
        &self,
        key: CacheKey,
        factory: F,
    ) -> Result<ConvertResult, AlchemizeError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<ConvertResult, AlchemizeError>>,
    {
        if let Some(hit) = self.get(&key).await {
            return Ok(hit);
        }

        let is_leader = {
            let mut in_flight = self.in_flight.lock().unwrap();
            in_flight.insert(key.clone())
        };

        if !is_leader {
            self.singleflight_waits.fetch_add(1, Ordering::Relaxed);
            loop {
                self.notify.notified().await;
                if !self.in_flight.lock().unwrap().contains(&key) {
                    break;
                }
            }
            if let Some(hit) = self.get(&key).await {
                return Ok(hit);
            }
            return factory().await;
        }

        let result = factory().await;
        if let Ok(ref value) = result {
            let _ = self.inner.set_with_ttl(key.inner(), value, None).await;
        }
        {
            let mut in_flight = self.in_flight.lock().unwrap();
            in_flight.remove(&key);
        }
        self.notify.notify_waiters();
        result
    }

    /// Snapshot of the current counters.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            singleflight_waits: self.singleflight_waits.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::result::ContentType;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration as StdDuration;
    use tokio::task::JoinSet;

    fn sample_result(markdown: &str) -> ConvertResult {
        ConvertResult::from_markdown(markdown, ContentType::Document)
    }

    #[test]
    fn key_is_stable_and_inputs_change_it() {
        let c = b"same bytes";
        let k1 = ConversionCache::key_for(c, "mineru");
        let k2 = ConversionCache::key_for(c, "mineru");
        assert_eq!(k1, k2, "same inputs must yield the same key");
        assert_ne!(k1, ConversionCache::key_for(b"other bytes", "mineru"));
        assert_ne!(k1, ConversionCache::key_for(c, "audio"));
        let empty = ConversionCache::key_for(b"", "mineru");
        assert_eq!(empty, ConversionCache::key_for(b"", "mineru"));
    }

    #[tokio::test]
    async fn get_hit_miss_and_expiry() {
        let cache = ConversionCache::new(StdDuration::from_secs(60));
        let key = ConversionCache::key_for(b"doc", "mineru");
        assert!(cache.get(&key).await.is_none(), "fresh cache must miss");
        cache
            .inner
            .set_with_ttl(key.inner(), &sample_result("markdown"), None)
            .await
            .unwrap();
        assert_eq!(
            cache.get(&key).await.unwrap().markdown,
            "markdown",
            "inserted entry must hit"
        );
        let stats = cache.stats();
        assert!(stats.hits >= 1 && stats.misses >= 1, "{stats:?}");
    }

    #[tokio::test]
    async fn expired_entry_returns_none() {
        let cache = ConversionCache::new(StdDuration::from_millis(1));
        let key = ConversionCache::key_for(b"doc", "mineru");
        cache
            .inner
            .set_with_ttl(key.inner(), &sample_result("m"), None)
            .await
            .unwrap();
        tokio::time::sleep(StdDuration::from_millis(20)).await;
        assert!(
            cache.get(&key).await.is_none(),
            "expired entry must not hit"
        );
    }

    #[tokio::test]
    async fn factory_runs_once_for_concurrent_callers() {
        let cache = Arc::new(ConversionCache::new(StdDuration::from_secs(60)));
        let key = ConversionCache::key_for(b"doc", "mineru");
        let calls = Arc::new(AtomicUsize::new(0));

        let mut set = JoinSet::new();
        for _ in 0..2 {
            let cache = cache.clone();
            let key = key.clone();
            let calls = calls.clone();
            set.spawn(async move {
                cache
                    .get_or_insert_with(key, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async {
                            // Yield so the second caller registers as a waiter
                            // while this leader is still in flight.
                            tokio::task::yield_now().await;
                            Ok(sample_result("shared"))
                        }
                    })
                    .await
                    .unwrap()
            });
        }
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].markdown, results[1].markdown);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "factory must run once");
        assert!(cache.stats().singleflight_waits >= 1);
    }

    #[tokio::test]
    async fn failed_factory_leaves_no_cache_entry() {
        let cache = Arc::new(ConversionCache::new(StdDuration::from_secs(60)));
        let key = ConversionCache::key_for(b"doc", "mineru");
        let calls = Arc::new(AtomicUsize::new(0));

        let mut set = JoinSet::new();
        for _ in 0..2 {
            let cache = cache.clone();
            let key = key.clone();
            let calls = calls.clone();
            set.spawn(async move {
                cache
                    .get_or_insert_with(key, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async { Err(AlchemizeError::Other("boom".to_string())) }
                    })
                    .await
            });
        }
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        for r in &results {
            assert!(r.is_err(), "both callers see the error");
        }
        assert!(
            cache.get(&key).await.is_none(),
            "failure must not be cached"
        );
    }
}
