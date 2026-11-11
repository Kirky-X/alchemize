// SPDX-License-Identifier: Apache-2.0
//! pdfium local PDF engine (feature `pdf-local`) — Chrome's PDF library via
//! pdfium-render, loaded dynamically at runtime.
//!
//! Binary compatibility: bindings are generated from the `pdfium_6295`
//! feature of pdfium-render 0.9, so the dynamic library must be the
//! matching pdfium release (chromium/6295-class). Newer or older builds can
//! bind but expose an unsupported ABI combination — the runtime e2e matrix
//! for 0.9 is pending a matching binary (see `dependency-feature-gating`).
//!
//! Extraction: per-page characters with coordinates, regrouped into
//! paragraphs by vertical gaps; embedded images come from the lopdf
//! structure walk (`pdf_images`), referenced per page as `images/…` assets.
//! Library probe order: `ALCHEMIZE_PDFIUM_PATH` → `~/.cache/alchemize/lib`
//! → system library; missing → Unavailable with a fetch hint (no silent
//! degradation, no network).

#![cfg(feature = "pdf-local")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter};
use crate::domain::asset::Asset;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Environment variable naming the pdfium dynamic library file.
pub const ENV_PDFIUM_PATH: &str = "ALCHEMIZE_PDFIUM_PATH";

fn default_lib_dir() -> &'static PathBuf {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".cache/alchemize/lib")
    })
}

/// Candidate library paths in probe order.
fn library_candidates() -> Vec<PathBuf> {
    let mut c = Vec::new();
    if let Ok(explicit) = std::env::var(ENV_PDFIUM_PATH) {
        c.push(PathBuf::from(explicit));
    }
    let lib = if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else {
        "libpdfium.so"
    };
    c.push(default_lib_dir().join(lib));
    c
}

/// Resolve the pdfium library path, or explain why not.
pub fn resolve_library() -> Result<PathBuf, String> {
    for candidate in library_candidates() {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "pdfium library not found: set {ENV_PDFIUM_PATH} or place the pdfium build under {} \
         (download from https://github.com/bblanchon/pdfium-binaries)",
        default_lib_dir().display()
    ))
}

/// PDF converter backed by pdfium.
pub struct PdfiumConverter {
    library: Option<PathBuf>,
}

impl PdfiumConverter {
    /// Probe the environment and construct.
    pub fn new() -> Self {
        Self {
            library: resolve_library().ok(),
        }
    }
}

impl Default for PdfiumConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for PdfiumConverter {
    fn name(&self) -> &'static str {
        "pdfium"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read PDF input: {e}")))?;
        let library = self
            .library
            .clone()
            .ok_or_else(|| AlchemizeError::Pdf("pdfium library unavailable".into()))?;

        let result = tokio::task::spawn_blocking(move || extract_all(&library, &bytes))
            .await
            .map_err(|e| AlchemizeError::other(format!("pdfium task panicked: {e}")))?;

        let (markdown, assets, page_count, skip_reasons) = result?;

        let metadata = serde_json::json!({
            "source_format": "pdf",
            "page_count": page_count,
            "asset_count": assets.len(),
            "images_skipped": skip_reasons.len(),
            "images_skip_reasons": skip_reasons.iter().take(8).collect::<Vec<_>>(),
            "engine": "pdfium",
        });
        let mut out = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        for asset in assets {
            out = out.with_asset(asset);
        }
        Ok(out)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pdf"]
    }

    fn priority(&self) -> u8 {
        20
    }

    fn availability(&self) -> Availability {
        match &self.library {
            Some(path) => {
                // Path known: still let bind errors surface at convert time.
                let _ = path;
                Availability::Available
            }
            None => match resolve_library() {
                Err(reason) => Availability::Unavailable { reason },
                Ok(_) => Availability::Available,
            },
        }
    }
}

/// One blocking task: lopdf image extraction (structure walk) + pdfium
/// text extraction, assets named `images/{seq:04}-{sha8}.{ext}` with
/// content dedup — same convention as the docx-deep converter.
fn extract_all(
    library: &std::path::Path,
    bytes: &[u8],
) -> Result<(String, Vec<Asset>, usize, Vec<String>), AlchemizeError> {
    // Image extraction first: pure Rust, no pdfium involvement, and its
    // lopdf failure is a skip reason rather than a conversion failure.
    let mut extraction = crate::infrastructure::document::pdf_images::extract_page_images(bytes);
    // One sha per entry (not per position): a large image referenced on
    // many pages hashes exactly once. Naming goes through the shared
    // AssetNamer — the hash and the `images/{seq:04}-{sha8}.{ext}`
    // contract come from the same source and cannot drift.
    let mut namer = crate::domain::asset::AssetNamer::new();
    let mut entry_ref: Vec<String> = vec![String::new(); extraction.entries.len()];
    for (idx, image) in extraction.entries.iter_mut().enumerate() {
        let bytes = std::mem::take(&mut image.bytes);
        entry_ref[idx] = namer.reference_owned(bytes, image.ext);
    }
    let assets = namer.into_assets();
    let mut page_refs: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (page, entry) in &extraction.positions {
        if let Some(r) = entry_ref.get(*entry)
            && !r.is_empty()
        {
            page_refs.entry(*page).or_default().push(r.clone());
        }
    }

    let (markdown, page_count) = extract(library, bytes, &page_refs)?;
    Ok((markdown, assets, page_count, extraction.skipped))
}

/// Line-extraction internals.
struct Line {
    text: String,
    top: f32,
    bottom: f32,
    left: f32,
    font_size: f32,
}

fn extract(
    library: &std::path::Path,
    bytes: &[u8],
    page_images: &BTreeMap<u32, Vec<String>>,
) -> Result<(String, usize), AlchemizeError> {
    use pdfium_render::prelude::*;

    let bindings = Pdfium::bind_to_library(library.display().to_string())
        .map_err(|e| AlchemizeError::Pdf(format!("pdfium bind failed: {e:?}")))?;
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| AlchemizeError::Pdf(format!("pdfium open failed: {e:?}")))?;

    let page_count = doc.pages().len();
    let mut markdown = String::new();

    // pdfium-render 0.9 removed PdfPage::index(); the iterator order is the
    // page order, so enumerate instead.
    for (page_index, page) in doc.pages().iter().enumerate() {
        let mut lines: Vec<Line> = Vec::new();

        for object in page.objects().iter() {
            if let PdfPageObject::Text(text) = &object {
                let run = text.text().clone();
                if run.trim().is_empty() {
                    continue;
                }
                let Ok(bounds) = text.bounds() else {
                    lines.push(Line {
                        text: run,
                        top: 0.0,
                        bottom: 0.0,
                        left: 0.0,
                        font_size: text.scaled_font_size().value,
                    });
                    continue;
                };
                lines.push(Line {
                    text: run,
                    top: bounds.top().value,
                    bottom: bounds.bottom().value,
                    left: bounds.left().value,
                    font_size: text.scaled_font_size().value,
                });
            }
        }

        // Reading order: top edge, then left edge.
        lines.sort_by(|a, b| {
            a.top
                .partial_cmp(&b.top)
                .map(|o| {
                    o.then(
                        a.left
                            .partial_cmp(&b.left)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
                })
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if !markdown.is_empty() {
            markdown.push('\n');
        }
        markdown.push_str(&format!("## Page {}\n\n", page_index + 1));

        // Median font size drives both paragraph gaps and heading detection.
        let mut sizes: Vec<f32> = lines.iter().map(|l| l.font_size).collect();
        sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_size = sizes.get(sizes.len() / 2).copied().unwrap_or(12.0).max(1.0);

        let mut para = String::new();
        let mut prev_bottom: Option<f32> = None;
        for line in &lines {
            if let Some(pb) = prev_bottom {
                let gap = line.top - pb;
                if gap > median_size * 1.6 && !para.is_empty() {
                    markdown.push_str(para.trim_end());
                    markdown.push_str("\n\n");
                    para.clear();
                }
            }
            let is_heading =
                line.font_size > median_size * 1.35 && line.text.trim().chars().count() < 120;
            if is_heading {
                if !para.is_empty() {
                    markdown.push_str(para.trim_end());
                    markdown.push_str("\n\n");
                    para.clear();
                }
                markdown.push_str(&format!("## {}\n\n", line.text.trim()));
            } else {
                if !para.is_empty() {
                    para.push(' ');
                }
                para.push_str(line.text.trim());
            }
            prev_bottom = Some(line.bottom);
        }
        if !para.is_empty() {
            markdown.push_str(para.trim_end());
            markdown.push('\n');
        }

        if lines.is_empty() {
            markdown.push_str("*(scanned page: no text layer — raster export pending)*\n");
        }

        // Image references extracted from this page's XObjects, after the
        // text block. `page_images` keys match pdfium's 0-based page index.
        if let Some(refs) = page_images.get(&(page_index as u32)) {
            for reference in refs {
                markdown.push_str(&format!("![image]({reference})\n\n"));
            }
        }
    }

    if markdown.trim().is_empty() {
        markdown = "*PDF contains no extractable text.*\n".to_string();
    }
    Ok((markdown, page_count as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes env-var mutation: parallel tests share the process
    /// environment, so the missing-library tests flipping
    /// ALCHEMIZE_PDFIUM_PATH would race with the real-library probe.
    ///
    /// Async-aware (`tokio::sync::Mutex`): some tests intentionally hold the
    /// guard across `await` points (the pdfium engine reads the env var
    /// lazily during conversion), and a std guard held across `await` both
    /// blocks the executor and fails `clippy::await_holding_lock`.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    #[tokio::test]
    async fn missing_library_reports_unavailable_with_hint() {
        let _env = env_guard().await;
        let saved = std::env::var(ENV_PDFIUM_PATH).ok();
        unsafe { std::env::set_var(ENV_PDFIUM_PATH, "/nonexistent/libpdfium.dylib") };
        let conv = PdfiumConverter::new();
        match conv.availability() {
            Availability::Unavailable { reason } => {
                assert!(reason.contains("ALCHEMIZE_PDFIUM_PATH"), "{reason}");
                assert!(reason.contains("pdfium-binaries"));
            }
            Availability::Available => panic!("must be unavailable"),
        }
        restore_env(saved);
    }

    /// Restore ALCHEMIZE_PDFIUM_PATH to its pre-test value: sibling tests
    /// (real-library probe) rely on the harness-provided environment.
    fn restore_env(saved: Option<String>) {
        match saved {
            Some(v) => unsafe { std::env::set_var(ENV_PDFIUM_PATH, v) },
            None => unsafe { std::env::remove_var(ENV_PDFIUM_PATH) },
        }
    }

    #[test]
    fn priority_top_of_pdf_chain() {
        assert_eq!(PdfiumConverter::new().priority(), 20);
    }

    #[tokio::test]
    async fn convert_without_library_errors_cleanly() {
        let _env = env_guard().await;
        let saved = std::env::var(ENV_PDFIUM_PATH).ok();
        unsafe { std::env::set_var(ENV_PDFIUM_PATH, "/nonexistent/libpdfium.dylib") };
        let conv = PdfiumConverter::new();
        let input = Input::bytes(b"%PDF-1.4 fake".to_vec(), "x.pdf");
        let result = conv.convert(input).await;
        assert!(result.is_err());
        restore_env(saved);
    }

    #[tokio::test]
    #[ignore = "requires pdfium library (ALCHEMIZE_PDFIUM_PATH or ~/.cache/alchemize/lib)"]
    async fn real_pdf_extracts_text() {
        let _env = env_guard().await;
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/real/real.pdf");
        let conv = PdfiumConverter::new();
        assert_eq!(conv.availability(), Availability::Available);
        let result = conv.convert(Input::Path(path)).await.unwrap();
        assert!(result.markdown.chars().count() > 2000);
        assert!(result.markdown.contains("## Page 1"));
        // Image pipeline wired: skip ledger always present (empty or not).
        assert!(
            result.metadata.get("images_skipped").is_some(),
            "images_skipped metadata missing: {:?}",
            result.metadata
        );
    }

    #[tokio::test]
    #[ignore = "requires pdfium library (ALCHEMIZE_PDFIUM_PATH or ~/.cache/alchemize/lib)"]
    async fn image_pdf_produces_asset_and_reference() {
        use lopdf::{Stream, dictionary};
        let _env = env_guard().await;
        // 4x4 JPEG via the image crate (png feature set includes jpeg).
        let img = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(8, 8, |x, y| {
            image::Rgb([(x as u8) * 30, (y as u8) * 30, 255u8])
        }));
        let mut jpg = std::io::Cursor::new(Vec::new());
        img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpg, 90,
        ))
        .unwrap();
        let jpg = jpg.into_inner();

        let mut doc = lopdf::Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let image_id = doc.add_object(lopdf::Object::Stream(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 8,
                "Height" => 8,
                "BitsPerComponent" => 8,
                "ColorSpace" => "DeviceRGB",
                "Filter" => lopdf::Object::Name(b"DCTDecode".to_vec()),
            },
            jpg.clone(),
        )));
        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Im0" => image_id },
        });
        let content_id = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
        });
        doc.objects.insert(
            pages_id,
            lopdf::Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut pdf = Vec::new();
        doc.save_to(&mut pdf).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("image.pdf");
        std::fs::write(&path, &pdf).unwrap();

        let conv = PdfiumConverter::new();
        assert_eq!(conv.availability(), Availability::Available);
        let result = conv.convert(Input::Path(path)).await.unwrap();
        // Asset present, named per the shared images/{seq:04}-{sha8}.jpg
        // convention, and referenced from the markdown body.
        assert_eq!(result.assets.len(), 1, "one embedded image");
        let asset = &result.assets[0];
        assert!(
            asset.filename.starts_with("images/0000-"),
            "{}",
            asset.filename
        );
        assert!(asset.filename.ends_with(".jpg"), "{}", asset.filename);
        let sha8 = asset.sha8();
        assert!(
            asset.filename.contains(&sha8),
            "filename {name} must embed sha8 {sha8}",
            name = asset.filename
        );
        assert_eq!(asset.bytes[..2], [0xFF, 0xD8], "JPEG passthrough intact");
        assert!(
            result
                .markdown
                .contains(&format!("![image]({})", asset.filename)),
            "markdown must reference the asset: {}",
            result.markdown
        );
        assert_eq!(result.metadata["images_skipped"], 0);
    }
}
