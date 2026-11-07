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
            if let Some(idx) = candidates.iter().position(|c| {
                c.name().to_ascii_lowercase() == pref_lc
                    || c.name().to_ascii_lowercase().starts_with(&pref_lc)
            }) {
                let preferred = candidates.remove(idx);
                candidates.insert(0, preferred);
            }
        }

        if candidates.is_empty() {
            let ext = input.extension().unwrap_or_else(|| "<none>".to_string());
            return Err(AlchemizeError::UnsupportedFormat(ext));
        }

        let mut attempts: Vec<(String, String)> = Vec::new();
        for (i, converter) in candidates.iter().enumerate() {
            if let Availability::Unavailable { reason } = converter.availability() {
                debug!(
                    engine = converter.name(),
                    reason = %reason,
                    "skipping unavailable converter"
                );
                attempts.push((converter.name().to_string(), format!("unavailable: {reason}")));
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
                    // `--engine` effects are observable downstream.
                    if result.metadata.get("converter").is_none() {
                        if result.metadata.is_null() {
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
                    warn!(error = %e, attempt = i + 1, engine = converter.name(), "converter failed, trying next");
                    attempts.push((converter.name().to_string(), e.to_string()));
                }
            }
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
        registry.register(stub(&["pdf"], 5, "fallback", ConversionQuality::LocalFallback));
        let input = Input::bytes(vec![], "t.pdf");

        let default = registry.convert(&input).await.unwrap();
        assert_eq!(default.markdown, "primary");
        assert_eq!(default.metadata["converter"], "primary");

        let preferred = registry.convert_pref(&input, Some("fallback")).await.unwrap();
        assert_eq!(preferred.markdown, "fallback");
        assert_eq!(preferred.metadata["converter"], "fallback");
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
    async fn convert_uses_highest_priority() {
        let mut registry = ConverterRegistry::new();
        registry.register(stub(&["pdf"], 5, "fallback", ConversionQuality::LocalFallback));
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
        registry.register(stub(&["pdf"], 10, "fallback", ConversionQuality::LocalFallback));
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
        registry.register(stub(&["pdf"], 10, "fallback", ConversionQuality::LocalFallback));
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
