// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! [`ContainerBase`] implementation — `mod.rs` stays declaration-only.

use std::io::Read;
use std::sync::Arc;

use crate::application::registry::ConverterRegistry;
use crate::domain::asset::Asset;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};
use crate::infrastructure::document::shared::{code_span_label, escape_markdown_label, fence_for};

use super::{
    ContainerBase, ContainerEntry, FALLBACK_TEXT_BYTES, MAX_CHAIN_BYTES, MAX_DEPTH,
    MAX_ENTRY_BYTES, MAX_TOTAL_BYTES, RegistrySlot,
};

tokio::task_local! {
    /// Nesting depth of the current container conversion chain.
    ///
    /// Task-local (not a shared atomic): concurrent top-level conversions
    /// each get their own counter, so one conversion's nesting can never
    /// exhaust another's budget.
    static CHAIN_DEPTH: std::cell::Cell<u32>;
}

tokio::task_local! {
    /// Cumulative decompressed bytes materialized by the current chain.
    ///
    /// Per-layer limits do not bound the sum: three nested layers can each
    /// hold `MAX_TOTAL_BYTES` while the parent entries stay alive. This
    /// chain-level budget caps cumulative materialized bytes; a layer is
    /// charged after extraction, so the instantaneous peak can exceed the
    /// budget by at most one layer's `MAX_TOTAL_BYTES`.
    static CHAIN_BYTES: std::cell::Cell<u64>;
}

/// Run `fut` inside a container-depth scope.
///
/// A top-level container conversion opens a fresh counter; a nested container
/// conversion (already inside an enclosing scope) inherits it, so `MAX_DEPTH`
/// bounds the whole nesting chain. Concurrent futures in the same task each
/// enter their own scope, because the task-local is only active during the
/// poll of the scoped future.
pub(crate) async fn with_depth_scope<F: std::future::Future>(fut: F) -> F::Output {
    if CHAIN_DEPTH.try_with(|_| ()).is_ok() {
        fut.await
    } else {
        CHAIN_DEPTH
            .scope(
                std::cell::Cell::new(0),
                CHAIN_BYTES.scope(std::cell::Cell::new(0), fut),
            )
            .await
    }
}

/// Charge decompressed bytes against the current chain budget.
///
/// Returns an error (fail closed) once the chain exceeds
/// [`MAX_CHAIN_BYTES`]; outside a scope the charge is a no-op.
pub(crate) fn charge_chain_bytes(bytes: u64) -> Result<(), AlchemizeError> {
    CHAIN_BYTES
        .try_with(|total| {
            let next = total.get().saturating_add(bytes);
            if next > MAX_CHAIN_BYTES {
                return Err(AlchemizeError::other(format!(
                    "container chain decompressed size exceeds the {MAX_CHAIN_BYTES}-byte safety limit"
                )));
            }
            total.set(next);
            Ok(())
        })
        .unwrap_or(Ok(()))
}

/// RAII guard: releases one nesting level when the conversion scope ends.
pub(crate) struct DepthGuard;

impl Drop for DepthGuard {
    fn drop(&mut self) {
        let _ = CHAIN_DEPTH.try_with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Acquire one level of nesting depth for the current chain.
///
/// Fails once the chain reaches [`MAX_DEPTH`]; the error is surfaced by the
/// registry to the enclosing entry, which falls back to an attachment
/// instead of recursing forever (zip-bomb style nesting protection).
pub(crate) fn acquire_depth() -> Result<DepthGuard, AlchemizeError> {
    CHAIN_DEPTH
        .try_with(|depth| {
            let current = depth.get();
            if current >= MAX_DEPTH {
                return Err(AlchemizeError::DepthLimitExceeded { limit: MAX_DEPTH });
            }
            depth.set(current + 1);
            Ok(DepthGuard)
        })
        .unwrap_or_else(|_| {
            Err(AlchemizeError::other(
                "container depth scope not entered: container converters must run inside with_depth_scope",
            ))
        })
}

/// Enforce the total decompressed size limit.
pub(crate) fn check_total_bytes(total: u64) -> Result<(), AlchemizeError> {
    if total > MAX_TOTAL_BYTES {
        return Err(AlchemizeError::other(format!(
            "decompressed container size {total} bytes exceeds the {MAX_TOTAL_BYTES}-byte safety limit"
        )));
    }
    Ok(())
}

/// Enforce the per-entry size limit.
// Only the entry-enumerating containers (tar/7z) call this; a build with only
// single-stream format features legitimately leaves it unused.
#[cfg_attr(not(any(feature = "tar", feature = "sevenz")), allow(dead_code))]
pub(crate) fn check_entry_size(name: &str, size: u64) -> Result<(), AlchemizeError> {
    if size > MAX_ENTRY_BYTES {
        return Err(AlchemizeError::other(format!(
            "entry '{name}' is {size} bytes, exceeds the {MAX_ENTRY_BYTES}-byte per-entry limit"
        )));
    }
    Ok(())
}

/// Read `reader` to end, failing when the output exceeds `limit`
/// (decompression-bomb protection: returns an error instead of exhausting
/// memory).
// Only the single-stream decoders (gzip/xz/zstd) call this.
#[cfg_attr(
    not(any(feature = "gzip", feature = "xz", feature = "zstd")),
    allow(dead_code)
)]
pub(crate) fn read_limited(
    reader: impl Read,
    limit: u64,
    context: &str,
) -> Result<Vec<u8>, AlchemizeError> {
    let mut buf = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| AlchemizeError::other(format!("{context}: {e}")))?;
    if buf.len() as u64 > limit {
        return Err(AlchemizeError::other(format!(
            "{context}: decompressed size exceeds the {limit}-byte safety limit"
        )));
    }
    Ok(buf)
}

/// True for OS metadata entries that must not become document content:
/// macOS AppleDouble files (`._name` basenames) and resource-fork folders
/// (`__MACOSX` path segments). Ordinary dotfiles are payload here (a `.env`
/// inside a tar may be the document); the zip converter additionally drops
/// hidden files, which is a deliberate difference, not an inconsistency.
// Only the entry-enumerating containers (tar/7z) call this.
#[cfg_attr(not(any(feature = "tar", feature = "sevenz")), allow(dead_code))]
pub(crate) fn is_metadata_entry(name: &str) -> bool {
    let basename = name.rsplit(['/', '\\']).next().unwrap_or(name);
    basename.starts_with("._") || name.split(['/', '\\']).any(|seg| seg == "__MACOSX")
}

/// Fallback rendering for an entry no converter supports.
///
/// Text (valid UTF-8, non-empty) is inlined into a fenced code block tagged
/// with the entry extension; binary content is listed as an attachment and
/// preserved as an [`Asset`]. Both kinds are recorded by the caller in
/// `metadata.attachments`. Entry names are attacker-controlled: the fence
/// info string keeps ASCII alphanumerics only, labels are sanitized, and the
/// fence is lengthened past any backtick run in the body so payload text
/// cannot close the block.
///
/// `depth_limited` distinguishes a nesting-limit rejection from "no converter
/// matched" in the rendered reason.
pub(crate) fn fallback_entry(
    name: &str,
    bytes: &[u8],
    depth_limited: bool,
) -> (String, Option<Asset>) {
    let ext: String = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("text")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(16)
        .collect();
    let ext = if ext.is_empty() {
        "text".to_string()
    } else {
        ext
    };

    // A depth-limited entry is a nested container, not a text document:
    // always take the attachment path so the rendered reason is visible.
    // Text containing control characters (ANSI escapes etc.) is treated as
    // binary — matching the zip converter's inline branch.
    if !depth_limited
        && let Ok(text) = std::str::from_utf8(bytes)
        && !text.trim().is_empty()
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
    {
        let body = if text.len() > FALLBACK_TEXT_BYTES {
            let mut cut = FALLBACK_TEXT_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            format!(
                "{}\n… (truncated, {} bytes total)",
                &text[..cut],
                text.len()
            )
        } else {
            text.to_string()
        };
        let fence = fence_for(&body);
        return (format!("{fence}{ext}\n{body}\n{fence}\n"), None);
    }

    let reason = if depth_limited {
        "nested container exceeds the depth limit"
    } else {
        "unsupported inner format"
    };
    let markdown = format!(
        "> Attachment: `{}` ({size} bytes) — {reason}; content preserved as attachment.\n",
        code_span_label(name),
        size = bytes.len(),
    );
    (markdown, Some(Asset::new(name, bytes.to_vec())))
}

impl ContainerBase {
    /// Create a base whose registry is filled after the enclosing registry
    /// is assembled (builder wiring; see [`RegistrySlot`]).
    pub fn with_slot(slot: RegistrySlot) -> Self {
        Self {
            registry: slot,
            #[cfg(test)]
            owned: None,
        }
    }

    /// Test-only: create a base around an already assembled registry. The
    /// caller's `Arc` must outlive the base (the slot holds a weak handle).
    #[cfg(test)]
    pub(crate) fn with_arc(registry: Arc<ConverterRegistry>) -> Self {
        let slot: RegistrySlot = Arc::new(std::sync::OnceLock::new());
        let _ = slot.set(Arc::downgrade(&registry));
        Self {
            registry: slot,
            owned: Some(registry),
        }
    }

    /// The registry used for nested entry conversion, once wired.
    pub fn registry(&self) -> Option<Arc<ConverterRegistry>> {
        #[cfg(test)]
        if let Some(owned) = &self.owned {
            return Some(Arc::clone(owned));
        }
        self.registry.get().and_then(std::sync::Weak::upgrade)
    }

    /// Convert extracted entries into markdown, delegating each one to the
    /// registry. Entries without a matching converter degrade to a fenced
    /// code block (text) or an attachment (binary), both recorded in
    /// `metadata.attachments`.
    ///
    /// An unwired registry slot degrades every entry the same way and sets
    /// `metadata.registry_wired = false`; it is not an error, so callers that
    /// assemble their own registry (direct `default_converters` use) keep
    /// working.
    pub async fn convert_container(
        &self,
        source_format: &'static str,
        entries: Result<Vec<ContainerEntry>, AlchemizeError>,
    ) -> Result<ConvertResult, AlchemizeError> {
        let entries = entries?;
        let registry = self.registry();

        let total: u64 = entries.iter().map(|e| e.bytes.len() as u64).sum();
        check_total_bytes(total)?;
        charge_chain_bytes(total)?;

        let mut parts: Vec<String> = Vec::new();
        let mut assets: Vec<Asset> = Vec::new();
        let mut attachments: Vec<String> = Vec::new();
        let file_count = entries.len();
        let mut converted_count = 0usize;
        let mut depth_limited = 0usize;
        let mut nested_attachments = 0usize;

        for entry in entries {
            let nested = Input::bytes(entry.bytes.clone(), entry.name.clone());
            let converted = match &registry {
                Some(registry) => registry.convert(&nested).await,
                None => Err(AlchemizeError::other(
                    "container registry slot is not wired",
                )),
            };
            match converted {
                Ok(mut result) => {
                    // Propagate the child's depth rejections: a real nesting
                    // attack is rejected several layers below, and the
                    // top-level result must still expose it.
                    depth_limited = depth_limited.saturating_add(
                        result
                            .metadata
                            .get("depth_limited")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize,
                    );
                    nested_attachments = nested_attachments
                        .saturating_add(
                            result
                                .metadata
                                .get("attachments")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0),
                        )
                        .saturating_add(
                            result
                                .metadata
                                .get("nested_attachments")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as usize,
                        );
                    parts.push(format!("## {}\n", escape_markdown_label(&entry.name)));
                    parts.push(std::mem::take(&mut result.markdown));
                    parts.push(String::new());
                    assets.append(&mut result.assets);
                    converted_count += 1;
                }
                Err(err) => {
                    // Depth rejection and "no converter matched" both fall
                    // back to an attachment; keep them distinguishable in
                    // both the rendered reason and the metadata counter.
                    let depth_hit = matches!(err, AlchemizeError::DepthLimitExceeded { .. });
                    if depth_hit {
                        depth_limited += 1;
                    }
                    let (markdown, asset) = fallback_entry(&entry.name, &entry.bytes, depth_hit);
                    parts.push(markdown);
                    parts.push(String::new());
                    if let Some(asset) = asset {
                        assets.push(asset);
                    }
                    attachments.push(entry.name);
                }
            }
        }

        let markdown = if parts.is_empty() {
            format!("*{source_format} archive contains no convertible files.*\n")
        } else {
            parts.join("\n")
        };

        let metadata = serde_json::json!({
            "source_format": source_format,
            "file_count": file_count,
            "converted_count": converted_count,
            "attachments": attachments,
            "nested_attachments": nested_attachments,
            "registry_wired": registry.is_some(),
            "depth_limited": depth_limited,
        });

        let mut result = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        // Attachments extracted by the fallback path must reach the caller;
        // dropping them here would silently lose the payloads.
        for asset in assets {
            result = result.with_asset(asset);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::document::text::TextConverter;

    fn registry_with_text() -> Arc<ConverterRegistry> {
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(TextConverter::new()));
        Arc::new(registry)
    }

    #[tokio::test]
    async fn convert_container_renders_entries_and_fallback() {
        let base = ContainerBase::with_arc(registry_with_text());
        let list = vec![
            ContainerEntry {
                name: "notes.txt".to_string(),
                bytes: b"hello from tar".to_vec(),
            },
            ContainerEntry {
                // 0xFF is invalid UTF-8: a genuine binary payload.
                name: "blob.bin".to_string(),
                bytes: vec![0xFF, 0xFE, 0x00],
            },
        ];
        let result = base.convert_container("tar", Ok(list)).await.unwrap();
        assert!(result.markdown.contains("## notes.txt"));
        assert!(result.markdown.contains("hello from tar"));
        // notes.txt is converted by TextConverter (raw text, no fence).
        // Binary entry: no registry match -> attachment, preserved as asset.
        assert!(result.markdown.contains("blob.bin"));
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets[0].filename, "blob.bin");
        assert_eq!(result.metadata["file_count"], 2);
        assert_eq!(result.metadata["converted_count"], 1);
        assert_eq!(result.metadata["registry_wired"], true);
        let attachments = result.metadata["attachments"].as_array().unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0], "blob.bin");
    }

    #[tokio::test]
    async fn convert_container_empty() {
        let base = ContainerBase::with_arc(registry_with_text());
        let result = base.convert_container("tar", Ok(Vec::new())).await.unwrap();
        assert!(result.markdown.contains("no convertible files"));
        assert_eq!(result.metadata["file_count"], 0);
    }

    #[tokio::test]
    async fn convert_container_propagates_entries_error() {
        let base = ContainerBase::with_arc(registry_with_text());
        let err = base
            .convert_container("tar", Err(AlchemizeError::other("boom")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[tokio::test]
    async fn convert_container_without_wired_slot_degrades_to_attachments() {
        // Direct `default_converters` callers may never fill the slot. That
        // must degrade (attachments, metadata flag) — not fail the archive.
        let base = ContainerBase::with_slot(RegistrySlot::default());
        let list = vec![ContainerEntry {
            name: "notes.txt".to_string(),
            bytes: b"plain text".to_vec(),
        }];
        let result = base.convert_container("tar", Ok(list)).await.unwrap();
        assert_eq!(result.metadata["registry_wired"], false);
        assert_eq!(result.metadata["converted_count"], 0);
        assert_eq!(result.metadata["attachments"][0], "notes.txt");
        assert!(result.markdown.contains("plain text"));
    }

    struct MetadataConverter;

    #[async_trait::async_trait]
    impl crate::application::ports::Converter for MetadataConverter {
        fn name(&self) -> &'static str {
            "meta"
        }

        async fn convert(&self, _input: Input) -> Result<ConvertResult, AlchemizeError> {
            Ok(ConvertResult::with_metadata(
                "child body".to_string(),
                ContentType::Document,
                serde_json::json!({"depth_limited": 2, "attachments": ["a", "b"]}),
            ))
        }

        fn supported_extensions(&self) -> &[&str] {
            &["meta"]
        }
    }

    #[tokio::test]
    async fn nested_depth_and_attachment_counters_propagate() {
        // A real depth attack is rejected several layers down; its counters
        // must reach the top-level result instead of dying with child
        // metadata.
        let mut registry = ConverterRegistry::new();
        registry.register(Box::new(MetadataConverter));
        let base = ContainerBase::with_arc(Arc::new(registry));
        let list = vec![ContainerEntry {
            name: "a.meta".to_string(),
            bytes: b"x".to_vec(),
        }];
        let result = base.convert_container("tar", Ok(list)).await.unwrap();
        assert_eq!(result.metadata["depth_limited"], 2);
        assert_eq!(result.metadata["nested_attachments"], 2);
        assert_eq!(result.metadata["converted_count"], 1);
    }

    #[tokio::test]
    async fn chain_byte_budget_rejects_cumulative_overrun() {
        with_depth_scope(async {
            assert!(charge_chain_bytes(MAX_CHAIN_BYTES).is_ok());
            let err = charge_chain_bytes(1).unwrap_err();
            assert!(err.to_string().contains("chain decompressed size"), "{err}");
        })
        .await;
        // A fresh chain gets a fresh budget.
        with_depth_scope(async {
            assert!(charge_chain_bytes(MAX_CHAIN_BYTES).is_ok());
        })
        .await;
    }

    #[test]
    fn metadata_entry_filter_drops_appledouble_only() {
        assert!(is_metadata_entry("._notes.txt"));
        assert!(is_metadata_entry("dir/._notes.txt"));
        assert!(is_metadata_entry("__MACOSX/notes.txt"));
        assert!(is_metadata_entry("deep/__MACOSX/notes.txt"));
        // Ordinary dotfiles are payload, not OS metadata.
        assert!(!is_metadata_entry(".env"));
        assert!(!is_metadata_entry("config/.hidden"));
        // Substring lookalikes are payload: only an exact path segment is
        // AppleDouble metadata.
        assert!(!is_metadata_entry("notes__MACOSX.md"));
        assert!(!is_metadata_entry("__MACOSX_legacy/notes.txt"));
    }

    #[test]
    fn entry_size_limit_rejects_oversize() {
        assert!(check_entry_size("big.bin", MAX_ENTRY_BYTES).is_ok());
        let err = check_entry_size("big.bin", MAX_ENTRY_BYTES + 1).unwrap_err();
        assert!(err.to_string().contains("big.bin"));
        assert!(err.to_string().contains("per-entry limit"));
    }

    #[test]
    fn total_size_limit_rejects_oversize() {
        assert!(check_total_bytes(MAX_TOTAL_BYTES).is_ok());
        let err = check_total_bytes(MAX_TOTAL_BYTES + 1).unwrap_err();
        assert!(err.to_string().contains("safety limit"));
    }

    #[test]
    fn read_limited_allows_exact_limit_and_rejects_over() {
        let data = [7u8; 10];
        assert_eq!(read_limited(&data[..], 10, "t").unwrap().len(), 10);
        let err = read_limited(&data[..], 9, "t").unwrap_err();
        assert!(err.to_string().contains("safety limit"));
    }

    #[test]
    fn fallback_entry_inlines_text_and_truncates() {
        let (md, asset) = fallback_entry("notes.log", b"plain text body", false);
        assert!(md.contains("```log"));
        assert!(md.contains("plain text body"));
        assert!(asset.is_none());

        let long = vec![b'a'; FALLBACK_TEXT_BYTES + 100];
        let (md, _) = fallback_entry("big.txt", &long, false);
        assert!(md.contains("truncated"));
    }

    #[test]
    fn fallback_entry_binary_becomes_asset() {
        let (md, asset) = fallback_entry("img.dat", &[0xFF, 0xD8, 0xFF, 0x00], false);
        assert!(md.contains("Attachment"));
        assert!(md.contains("img.dat"));
        let asset = asset.unwrap();
        assert_eq!(asset.filename, "img.dat");
        assert_eq!(asset.bytes, vec![0xFF, 0xD8, 0xFF, 0x00]);
    }

    #[test]
    fn fallback_entry_text_with_control_chars_becomes_asset() {
        // ANSI escape sequences must not ride into the fenced block.
        let payload = b"ok\x1b[31mred\x1b[0m";
        let (md, asset) = fallback_entry("term.log", payload, false);
        assert!(md.contains("Attachment"), "{md}");
        assert!(asset.is_some());
    }

    #[tokio::test]
    async fn depth_is_scoped_per_conversion_chain() {
        // Two concurrent chains must each get a fresh budget: the old shared
        // atomic let one conversion's nesting exhaust another's.
        let a = with_depth_scope(async {
            let g1 = acquire_depth().unwrap();
            tokio::task::yield_now().await;
            let g2 = acquire_depth().unwrap();
            tokio::task::yield_now().await;
            let g3 = acquire_depth().unwrap();
            assert!(acquire_depth().is_err(), "4th level must be rejected");
            drop(g1);
            drop(g2);
            drop(g3);
            assert!(acquire_depth().is_ok(), "depth must release on drop");
        });
        let b = with_depth_scope(async {
            let _g1 = acquire_depth().unwrap();
            tokio::task::yield_now().await;
            let _g2 = acquire_depth().unwrap();
            tokio::task::yield_now().await;
            let _g3 = acquire_depth().unwrap();
            assert!(
                acquire_depth().is_err(),
                "independent chain still limited at MAX_DEPTH"
            );
        });
        tokio::join!(a, b);
    }

    #[tokio::test]
    async fn nested_scope_inherits_enclosing_chain() {
        let outer = with_depth_scope(async {
            let _g1 = acquire_depth().unwrap();
            // A nested converter enters another scope call; it must reuse the
            // enclosing counter rather than reset it.
            with_depth_scope(async {
                let _g2 = acquire_depth().unwrap();
                let _g3 = acquire_depth().unwrap();
                assert!(acquire_depth().is_err());
            })
            .await;
        });
        outer.await;
    }
}
