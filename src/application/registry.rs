// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Converter registry — priority-sorted routing with fallback chain.

use tracing::{debug, warn};

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::ConvertResult;

/// Registry of converters, sorted by priority (highest value first).
///
/// When multiple converters support the same file extension, the one with
/// the highest [`priority`](Converter::priority) value is tried first. If it
/// fails, the next converter in the chain is attempted as a fallback.
/// Converters whose [`availability`](Converter::availability) is
/// `Unavailable` are skipped (recorded as an attempt with their reason).
pub struct ConverterRegistry {
    converters: Vec<Box<dyn Converter>>,
    /// When true, remote engines (`is_remote() == true`) are excluded.
    no_remote: bool,
}

impl ConverterRegistry {
    /// Create an empty registry (remote engines allowed).
    pub fn new() -> Self {
        Self {
            converters: Vec::new(),
            no_remote: false,
        }
    }

    /// Create an empty registry that excludes remote engines.
    pub fn new_no_remote() -> Self {
        Self {
            converters: Vec::new(),
            no_remote: true,
        }
    }

    /// Register a converter.
    ///
    /// The converter is inserted in priority-sorted order.
    pub fn register(&mut self, converter: Box<dyn Converter>) {
        let priority = converter.priority();
        let pos = self
            .converters
            .iter()
            .position(|c| c.priority() < priority)
            .unwrap_or(self.converters.len());
        debug!(name = converter.name(), priority, "registering converter");
        self.converters.insert(pos, converter);
    }

    /// Convert input using the best matching converter.
    ///
    /// Finds all converters that accept the input (via [`can_convert`](Converter::can_convert)),
    /// tries them in priority order, and returns the first successful result.
    /// If all converters fail, returns [`AlchemizeError::AllBackendsFailed`]
    /// with the ordered per-engine attempts.
    pub async fn convert(&self, input: &Input) -> Result<ConvertResult, AlchemizeError> {
        self.convert_pref(input, None).await
    }

    /// Convert honouring a preferred converter name.
    ///
    /// When `prefer` names a candidate that accepts the input, it is tried
    /// first; the remaining candidates keep their priority order as fallbacks.
    /// An unknown / non-matching name is ignored (falls back to plain
    /// priority routing) rather than erroring, so a stale preference never
    /// blocks conversion.
    pub async fn convert_pref(
        &self,
        input: &Input,
        prefer: Option<&str>,
    ) -> Result<ConvertResult, AlchemizeError> {
        let mut candidates: Vec<&dyn Converter> = self
            .converters
            .iter()
            .map(|c| c.as_ref())
            .filter(|c| c.can_convert(input))
            .filter(|c| !(self.no_remote && c.is_remote()))
            .collect();

        if let Some(pref) = prefer {
            let pref_lc = pref.to_ascii_lowercase();
            // Exact engine names win globally; only when no candidate matches
            // exactly does prefix matching apply (in priority order). A single
            // pass would let a higher-priority `pdfium` shadow an exact `pdf`
            // preference.
            let idx = candidates
                .iter()
                .position(|c| c.name().to_ascii_lowercase() == pref_lc)
                .or_else(|| {
                    candidates
                        .iter()
                        .position(|c| c.name().to_ascii_lowercase().starts_with(&pref_lc))
                });
            if let Some(idx) = idx {
                let preferred = candidates.remove(idx);
                candidates.insert(0, preferred);
            }
        }

        if candidates.is_empty() {
            let ext = input.extension().unwrap_or_else(|| "<none>".to_string());
            return Err(AlchemizeError::UnsupportedFormat(ext));
        }

        let mut attempts: Vec<(String, String)> = Vec::new();
        // Depth rejection is control-flow, not a "backend failed" detail:
        // callers (container nesting) must be able to distinguish it from a
        // missing converter. Keep the first one seen and return it instead of
        // the generic aggregate.
        let mut depth_limit: Option<u32> = None;
        for (i, converter) in candidates.iter().enumerate() {
            if let Availability::Unavailable { reason } = converter.availability() {
                debug!(
                    engine = converter.name(),
                    reason = %reason,
                    "skipping unavailable converter"
                );
                attempts.push((
                    converter.name().to_string(),
                    crate::tr_args("unavailable", &[("reason", reason.to_string())]),
                ));
                continue;
            }
            debug!(
                attempt = i + 1,
                total = candidates.len(),
                engine = converter.name(),
                "trying converter"
            );
            match converter.convert(input.clone()).await {
                Ok(mut result) => {
                    // Record which engine actually produced this result so
                    // `--engine` effects are observable downstream. A
                    // non-object metadata payload (string/array/number) is
                    // replaced — provenance must never be silently dropped.
                    // A present-but-null `converter` key counts as missing.
                    if result
                        .metadata
                        .get("converter")
                        .is_none_or(|value| value.is_null())
                    {
                        if !result.metadata.is_object() {
                            result.metadata = serde_json::json!({});
                        }
                        if let Some(obj) = result.metadata.as_object_mut() {
                            obj.insert(
                                "converter".to_string(),
                                serde_json::Value::String(converter.name().to_string()),
                            );
                        }
                    }
                    return Ok(result);
                }
                Err(e) => {
                    if let AlchemizeError::DepthLimitExceeded { limit } = &e {
                        depth_limit.get_or_insert(*limit);
                    }
                    // Log the variant label only: converter messages may embed
                    // remote response bodies or internal endpoints.
                    warn!(
                        error_kind = e.kind(),
                        attempt = i + 1,
                        engine = converter.name(),
                        "{}",
                        crate::tr("converter-failed-trying-next")
                    );
                    attempts.push((converter.name().to_string(), e.to_string()));
                }
            }
        }

        if let Some(limit) = depth_limit {
            return Err(AlchemizeError::DepthLimitExceeded { limit });
        }
        let ext = input.extension().unwrap_or_else(|| "<none>".to_string());
        Err(AlchemizeError::AllBackendsFailed { ext, attempts })
    }

    /// Synchronous twin of [`convert_pref`](Self::convert_pref) for the sync
    /// facade ([`Alchemize::convert_sync`]).
    ///
    /// Deliberately self-contained rather than sharing candidate assembly
    /// with the async path: touching `convert_pref` would forfeit the
    /// "async path is untouched" verification anchor. The ~30-line overlap
    /// (prefer reordering, availability skip, attempts) is a documented
    /// trade-off, not an oversight.
    ///
    /// Semantics mirrored from the async dispatch, with one filter change:
    /// candidates must additionally be [`is_sync_capable`](Converter::is_sync_capable).
    ///
    /// The `DepthLimitExceeded` branch is mirrored defensively for structural
    /// parity with the async twin: no current sync-capable engine wraps
    /// container dispatch (containers are never sync-capable), so it cannot
    /// fire today — the mirror turns that into an enforceable invariant
    /// instead of an honor-system assumption.
    ///
    /// Empty candidate set distinguishes two refusals: the format has
    /// engines but none sync-capable → `NotSyncCapable` (engines listed);
    /// the format has no engine at all → `UnsupportedFormat`.
    ///
    /// Troubleshooting note: an engine that declares
    /// [`is_sync_capable`](Converter::is_sync_capable) `= true` without
    /// overriding `convert_sync` fails closed here as an `AllBackendsFailed`
    /// attempt carrying the default `NotSyncCapable` message — not as the
    /// empty-candidate `NotSyncCapable` refusal.
    pub fn convert_pref_sync(
        &self,
        input: &Input,
        prefer: Option<&str>,
    ) -> Result<ConvertResult, AlchemizeError> {
        let candidates: Vec<&dyn Converter> = self
            .converters
            .iter()
            .map(|c| c.as_ref())
            .filter(|c| c.can_convert(input))
            .filter(|c| c.is_sync_capable())
            .filter(|c| !(self.no_remote && c.is_remote()))
            .collect();

        if candidates.is_empty() {
            let ext = input.extension().unwrap_or_else(|| "<none>".to_string());
            let async_capable: Vec<&str> = self
                .converters
                .iter()
                .map(|c| c.as_ref())
                .filter(|c| c.can_convert(input))
                .filter(|c| !(self.no_remote && c.is_remote()))
                .map(|c| c.name())
                .collect();
            if async_capable.is_empty() {
                return Err(AlchemizeError::UnsupportedFormat(ext));
            }
            return Err(AlchemizeError::NotSyncCapable(format!(
                "no sync-capable engine for '{ext}' (async-only engines: {})",
                async_capable.join(", ")
            )));
        }

        let mut candidates = candidates;
        if let Some(pref) = prefer {
            let pref_lc = pref.to_ascii_lowercase();
            let idx = candidates
                .iter()
                .position(|c| c.name().to_ascii_lowercase() == pref_lc)
                .or_else(|| {
                    candidates
                        .iter()
                        .position(|c| c.name().to_ascii_lowercase().starts_with(&pref_lc))
                });
            if let Some(idx) = idx {
                let preferred = candidates.remove(idx);
                candidates.insert(0, preferred);
            }
        }

        let mut attempts: Vec<(String, String)> = Vec::new();
        // Depth rejection is control-flow, not a "backend failed" detail —
        // same contract as the async twin (see its comment). Unreachable
        // today: no sync-capable engine wraps container dispatch, but the
        // mirror keeps that an enforceable invariant, not an assumption.
        let mut depth_limit: Option<u32> = None;
        for (i, converter) in candidates.iter().enumerate() {
            if let Availability::Unavailable { reason } = converter.availability() {
                debug!(
                    engine = converter.name(),
                    reason = %reason,
                    "skipping unavailable converter (sync)"
                );
                attempts.push((
                    converter.name().to_string(),
                    crate::tr_args("unavailable", &[("reason", reason.to_string())]),
                ));
                continue;
            }
            debug!(
                attempt = i + 1,
                total = candidates.len(),
                engine = converter.name(),
                "trying converter (sync)"
            );
            match converter.convert_sync(input) {
                Ok(mut result) => {
                    // Same provenance contract as the async dispatch: the
                    // engine that actually produced the result must be
                    // observable; a null/missing `converter` key counts as
                    // missing, and non-object metadata is replaced.
                    if result
                        .metadata
                        .get("converter")
                        .is_none_or(|value| value.is_null())
                    {
                        if !result.metadata.is_object() {
                            result.metadata = serde_json::json!({});
                        }
                        if let Some(obj) = result.metadata.as_object_mut() {
                            obj.insert(
                                "converter".to_string(),
                                serde_json::Value::String(converter.name().to_string()),
                            );
                        }
                    }
                    return Ok(result);
                }
                Err(e) => {
                    if let AlchemizeError::DepthLimitExceeded { limit } = &e {
                        depth_limit.get_or_insert(*limit);
                    }
                    // Log the variant label only: converter messages may embed
                    // remote response bodies or internal endpoints (mirrors
                    // the async twin's failure logging).
                    warn!(
                        error_kind = e.kind(),
                        attempt = i + 1,
                        engine = converter.name(),
                        "{}",
                        crate::tr("converter-failed-trying-next-sync")
                    );
                    attempts.push((converter.name().to_string(), e.to_string()));
                }
            }
        }

        if let Some(limit) = depth_limit {
            return Err(AlchemizeError::DepthLimitExceeded { limit });
        }
        let ext = input.extension().unwrap_or_else(|| "<none>".to_string());
        Err(AlchemizeError::AllBackendsFailed { ext, attempts })
    }

    /// Return the number of registered converters.
    pub fn len(&self) -> usize {
        self.converters.len()
    }

    /// Return true if no converters are registered.
    pub fn is_empty(&self) -> bool {
        self.converters.is_empty()
    }

    /// Aggregate availability of every registered converter.
    ///
    /// Returns `(name, extensions, availability)` for each — used by the CLI
    /// `doctor` command to report which engines are live vs degraded.
    pub fn availability(&self) -> Vec<(String, Vec<String>, Availability)> {
        self.converters
            .iter()
            .map(|c| {
                (
                    c.name().to_string(),
                    c.supported_extensions()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    c.availability(),
                )
            })
            .collect()
    }

    /// Return an iterator over registered converters (by reference).
    pub fn converters(&self) -> impl Iterator<Item = &dyn Converter> {
        self.converters.iter().map(|c| c.as_ref())
    }
}

impl Default for ConverterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::result::{ContentType, ConversionQuality};

    /// A stub converter that returns a fixed markdown string.
    struct StubConverter {
        extensions: &'static [&'static str],
        priority_val: u8,
        markdown: &'static str,
        quality: ConversionQuality,
        remote: bool,
    }

    #[async_trait::async_trait]
    impl Converter for StubConverter {
        async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
            Ok(
                ConvertResult::from_markdown(self.markdown, ContentType::Document)
                    .with_quality(self.quality),
            )
        }

        fn supported_extensions(&self) -> &[&str] {
            self.extensions
        }

        fn priority(&self) -> u8 {
            self.priority_val
        }

        fn name(&self) -> &'static str {
            self.markdown
        }

        fn is_remote(&self) -> bool {
            self.remote
        }
    }

    fn stub(
        extensions: &'static [&'static str],
        priority_val: u8,
        markdown: &'static str,
        quality: ConversionQuality,
    ) -> Box<StubConverter> {
        Box::new(StubConverter {
            extensions,
            priority_val,
            markdown,
            quality,
            remote: quality == ConversionQuality::Api,
        })
    }

    #[tokio::test]
    async fn convert_pref_tries_named_converter_first() {
        let mut registry = ConverterRegistry::new();
        registry.register(stub(&["pdf"], 20, "primary", ConversionQuality::Api));
        registry.register(stub(
            &["pdf"],
            5,
            "fallback",
            ConversionQuality::LocalFallback,
        ));
        let input = Input::bytes(vec![], "t.pdf");

        let default = registry.convert(&input).await.unwrap();
        assert_eq!(default.markdown, "primary");
        assert_eq!(default.metadata["converter"], "primary");

        let preferred = registry
            .convert_pref(&input, Some("fallback"))
            .await
            .unwrap();
        assert_eq!(preferred.markdown, "fallback");
        assert_eq!(preferred.metadata["converter"], "fallback");
    }

    #[tokio::test]
    async fn convert_pref_exact_name_beats_prefix_collision() {
        // `pdfium` sorts first by priority and is a prefix match for `pdf`;
        // the exact `pdf` engine must still win the preference.
        let mut registry = ConverterRegistry::new();
        registry.register(stub(&["pdf"], 20, "pdfium", ConversionQuality::Api));
        registry.register(stub(&["pdf"], 10, "pdf", ConversionQuality::LocalFallback));
        let input = Input::bytes(vec![], "t.pdf");

        let result = registry.convert_pref(&input, Some("pdf")).await.unwrap();
        assert_eq!(result.markdown, "pdf");
        assert_eq!(result.metadata["converter"], "pdf");

        // No exact hit: unique prefix still resolves (fallback semantics).
        let result = registry.convert_pref(&input, Some("pdfi")).await.unwrap();
        assert_eq!(result.markdown, "pdfium");
    }

    #[tokio::test]
    async fn convert_pref_ignores_unknown_name() {
        let mut registry = ConverterRegistry::new();
        registry.register(stub(&["pdf"], 10, "only", ConversionQuality::LocalFallback));
        let input = Input::bytes(vec![], "t.pdf");
        let result = registry
            .convert_pref(&input, Some("nonexistent"))
            .await
            .unwrap();
        assert_eq!(result.markdown, "only");
    }

    #[test]
    fn register_sorts_by_priority() {
        let mut registry = ConverterRegistry::new();
        registry.register(stub(&["pdf"], 20, "a", ConversionQuality::LocalFallback));
        registry.register(stub(&["pdf"], 5, "b", ConversionQuality::Api));

        let converters: Vec<_> = registry.converters().collect();
        assert_eq!(converters.len(), 2);
        assert_eq!(converters[0].priority(), 20);
        assert_eq!(converters[1].priority(), 5);
    }

    #[tokio::test]
    async fn convert_records_converter_over_non_object_metadata() {
        // A converter returning scalar metadata must not lose provenance.
        struct ScalarMetadata;
        #[async_trait::async_trait]
        impl Converter for ScalarMetadata {
            async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
                let mut result = ConvertResult::from_markdown("body", ContentType::Document);
                result.metadata = serde_json::json!("scalar");
                Ok(result)
            }
            fn supported_extensions(&self) -> &[&str] {
                &["pdf"]
            }
            fn priority(&self) -> u8 {
                10
            }
            fn name(&self) -> &'static str {
                "scalar"
            }
        }

        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(ScalarMetadata));
        let result = registry
            .convert(&Input::bytes(vec![], "t.pdf"))
            .await
            .unwrap();
        assert_eq!(result.metadata["converter"], "scalar");
    }

    #[tokio::test]
    async fn convert_uses_highest_priority() {
        let mut registry = ConverterRegistry::new();
        registry.register(stub(
            &["pdf"],
            5,
            "fallback",
            ConversionQuality::LocalFallback,
        ));
        registry.register(stub(&["pdf"], 20, "primary", ConversionQuality::Api));

        let input = Input::bytes(vec![], "test.pdf");
        let result = registry.convert(&input).await.unwrap();
        assert_eq!(result.markdown, "primary");
        assert_eq!(result.quality, ConversionQuality::Api);
    }

    #[tokio::test]
    async fn convert_falls_back_on_failure() {
        struct FailConverter;
        #[async_trait::async_trait]
        impl Converter for FailConverter {
            async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
                Err(AlchemizeError::Other("intentional failure".to_string()))
            }
            fn supported_extensions(&self) -> &[&str] {
                &["pdf"]
            }
            fn priority(&self) -> u8 {
                20
            }
        }

        let mut registry = ConverterRegistry::new();
        registry.register(stub(
            &["pdf"],
            10,
            "fallback",
            ConversionQuality::LocalFallback,
        ));
        registry.register(Box::new(FailConverter));

        let input = Input::bytes(vec![], "test.pdf");
        let result = registry.convert(&input).await.unwrap();
        assert_eq!(result.markdown, "fallback");
    }

    #[tokio::test]
    async fn unavailable_converter_is_skipped_not_failed() {
        struct UnavailableConverter;
        #[async_trait::async_trait]
        impl Converter for UnavailableConverter {
            async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
                Err(AlchemizeError::Other("must not be called".into()))
            }
            fn supported_extensions(&self) -> &[&str] {
                &["pdf"]
            }
            fn priority(&self) -> u8 {
                20
            }
            fn name(&self) -> &'static str {
                "engine-unavailable"
            }
            fn availability(&self) -> Availability {
                Availability::Unavailable {
                    reason: "no dependency".into(),
                }
            }
        }

        let mut registry = ConverterRegistry::new();
        registry.register(stub(
            &["pdf"],
            10,
            "fallback",
            ConversionQuality::LocalFallback,
        ));
        registry.register(Box::new(UnavailableConverter));

        let input = Input::bytes(vec![], "test.pdf");
        let result = registry.convert(&input).await.unwrap();
        assert_eq!(result.markdown, "fallback");
    }

    #[tokio::test]
    async fn convert_returns_error_when_no_match() {
        let registry = ConverterRegistry::new();
        let input = Input::bytes(vec![], "test.xyz");
        let result = registry.convert(&input).await;
        assert!(matches!(
            result.unwrap_err(),
            AlchemizeError::UnsupportedFormat(_)
        ));
    }

    #[tokio::test]
    async fn convert_returns_attempts_when_all_fail() {
        struct FailConverter(&'static str);
        #[async_trait::async_trait]
        impl Converter for FailConverter {
            async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
                Err(AlchemizeError::Other(format!("fail-{}", self.0)))
            }
            fn supported_extensions(&self) -> &[&str] {
                &["pdf"]
            }
            fn priority(&self) -> u8 {
                20
            }
            fn name(&self) -> &'static str {
                self.0
            }
        }

        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(FailConverter("engine-a")));
        registry.register(Box::new(FailConverter("engine-b")));

        let input = Input::bytes(vec![], "test.pdf");
        let err = registry.convert(&input).await.unwrap_err();
        match err {
            AlchemizeError::AllBackendsFailed { ext, attempts } => {
                assert_eq!(ext, "pdf");
                assert_eq!(attempts.len(), 2);
                assert_eq!(attempts[0].0, "engine-a");
                assert!(attempts[1].1.contains("fail-engine-b"));
            }
            other => panic!("expected AllBackendsFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_remote_filters_remote_engines() {
        let mut registry = ConverterRegistry::new_no_remote();
        registry.register(stub(&["pdf"], 20, "cloud", ConversionQuality::Api));
        registry.register(stub(&["pdf"], 10, "local", ConversionQuality::LocalFull));

        let input = Input::bytes(vec![], "t.pdf");
        let result = registry.convert(&input).await.unwrap();
        assert_eq!(result.markdown, "local");
    }

    /// 同步分发测试的可能力 stub：`sync` 开关模拟「声明 / 未声明同步能力」，
    /// `available` 开关模拟 availability 探针结果。
    struct SyncStub {
        extensions: &'static [&'static str],
        markdown: &'static str,
        name: &'static str,
        sync: bool,
        available: bool,
    }

    #[async_trait::async_trait]
    impl Converter for SyncStub {
        async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
            unreachable!("async path not exercised in sync dispatch tests")
        }
        fn supported_extensions(&self) -> &[&str] {
            self.extensions
        }
        fn name(&self) -> &'static str {
            self.name
        }
        fn is_sync_capable(&self) -> bool {
            self.sync
        }
        fn convert_sync(&self, _input: &Input) -> Result<ConvertResult, AlchemizeError> {
            Ok(ConvertResult::from_markdown(
                self.markdown,
                ContentType::Document,
            ))
        }
        fn availability(&self) -> Availability {
            if self.available {
                Availability::Available
            } else {
                Availability::Unavailable {
                    reason: "probe failed".into(),
                }
            }
        }
    }

    #[test]
    fn convert_pref_sync_converts_and_records_provenance() {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(SyncStub {
            extensions: &["html"],
            markdown: "synced",
            name: "sync-engine",
            sync: true,
            available: true,
        }));
        let input = Input::bytes(vec![], "t.html");
        let result = registry.convert_pref_sync(&input, None).unwrap();
        assert_eq!(result.markdown, "synced");
        assert_eq!(result.metadata["converter"], "sync-engine");
    }

    /// 「格式有引擎但全部纯异步」必须显式拒绝并点名引擎——不得静默换成
    /// 其他引擎，也不得冒充 UnsupportedFormat。
    #[test]
    fn convert_pref_sync_rejects_async_only_format_explicitly() {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(SyncStub {
            extensions: &["pdf"],
            markdown: "",
            name: "pdf-async",
            sync: false,
            available: true,
        }));
        let input = Input::bytes(vec![], "t.pdf");
        let err = registry.convert_pref_sync(&input, None).unwrap_err();
        assert!(
            matches!(err, AlchemizeError::NotSyncCapable(_)),
            "expected NotSyncCapable, got {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("no sync-capable engine"), "{msg}");
        assert!(
            msg.contains("pdf-async"),
            "engine names must surface: {msg}"
        );
    }

    #[test]
    fn convert_pref_sync_unknown_extension_is_unsupported_format() {
        let registry = ConverterRegistry::new();
        let input = Input::bytes(vec![], "t.xyz");
        let err = registry.convert_pref_sync(&input, None).unwrap_err();
        assert!(matches!(err, AlchemizeError::UnsupportedFormat(_)));
    }

    #[test]
    fn convert_pref_sync_skips_unavailable_and_reports_attempts() {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(SyncStub {
            extensions: &["html"],
            markdown: "never",
            name: "down-engine",
            sync: true,
            available: false,
        }));
        let input = Input::bytes(vec![], "t.html");
        let err = registry.convert_pref_sync(&input, None).unwrap_err();
        match err {
            AlchemizeError::AllBackendsFailed { ext, attempts } => {
                assert_eq!(ext, "html");
                assert_eq!(attempts.len(), 1);
                assert_eq!(attempts[0].0, "down-engine");
                // The attempt reason is the catalogized `unavailable` message;
                // assert the localized prefix without pinning a language.
                let expected = crate::tr_args("unavailable", &[("reason", String::new())]);
                let prefix = expected
                    .split(['：', ':'])
                    .next()
                    .unwrap_or("unavailable")
                    .to_string();
                assert!(attempts[0].1.contains(&prefix));
            }
            other => panic!("expected AllBackendsFailed, got {other:?}"),
        }
    }

    #[test]
    fn convert_pref_sync_prefers_named_engine() {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(SyncStub {
            extensions: &["html"],
            markdown: "aaa-out",
            name: "aaa",
            sync: true,
            available: true,
        }));
        registry.register(Box::new(SyncStub {
            extensions: &["html"],
            markdown: "zzz-out",
            name: "zzz",
            sync: true,
            available: true,
        }));
        let input = Input::bytes(vec![], "t.html");
        let default = registry.convert_pref_sync(&input, None).unwrap();
        assert_eq!(default.markdown, "aaa-out");
        let preferred = registry.convert_pref_sync(&input, Some("zzz")).unwrap();
        assert_eq!(preferred.markdown, "zzz-out");
    }

    #[test]
    fn len_and_is_empty_and_default() {
        let mut registry = ConverterRegistry::new();
        assert!(registry.is_empty());
        registry.register(stub(&["pdf"], 10, "x", ConversionQuality::LocalFull));
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(ConverterRegistry::default().is_empty());
    }
}
