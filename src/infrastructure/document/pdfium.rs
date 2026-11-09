// SPDX-License-Identifier: Apache-2.0
//! pdfium local PDF engine (feature `pdfium`) — Chrome's PDF library via
//! pdfium-render, loaded dynamically at runtime.
//!
//! Binary compatibility: pdfium-render 0.5.x binds `FPDFFont_GetFontName`,
//! which pdfium builds after ~chromium/7000 no longer export — use a
//! 2023-era binaries release (e.g. chromium/6309) or the bind step fails
//! with `LoadLibraryError(DlSym)`.
//!
//! Extraction: per-page characters with coordinates, regrouped into
//! paragraphs by vertical gaps; embedded images come from the lopdf
//! structure walk (`pdf_images`), referenced per page as `images/…` assets.
//! Library probe order: `ALCHEMIZE_PDFIUM_PATH` → `~/.cache/alchemize/lib`
//! → system library; missing → Unavailable with a fetch hint (no silent
//! degradation, no network).

#![cfg(feature = "pdfium")]

use std::collections::{BTreeMap, HashMap};
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
    let extraction = crate::infrastructure::document::pdf_images::extract_page_images(bytes);
    let mut assets: Vec<Asset> = Vec::new();
    let mut by_sha: HashMap<String, String> = HashMap::new();
    let mut page_refs: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut seq: usize = 0;
    for (page, entry) in &extraction.positions {
        let image = &extraction.entries[*entry];
        let sha8 = sha8_hex(&image.bytes);
        let reference = if let Some(existing) = by_sha.get(&sha8) {
            existing.clone()
        } else {
            let r = format!("images/{seq:04}-{sha8}.{}", image.ext);
            seq += 1;
            by_sha.insert(sha8, r.clone());
            assets.push(Asset::new(r.clone(), image.bytes.clone()));
            r
        };
        page_refs.entry(*page).or_default().push(reference);
    }

    let (markdown, page_count) = extract(library, bytes, &page_refs)?;
    Ok((markdown, assets, page_count, extraction.skipped))
}

/// Leading 8 hex chars of the SHA-256 digest (mirrors `Asset::sha8`).
fn sha8_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
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
        .load_pdf_from_bytes(bytes, None)
        .map_err(|e| AlchemizeError::Pdf(format!("pdfium open failed: {e:?}")))?;

    let page_count = doc.pages().len();
    let mut markdown = String::new();

    for page in doc.pages().iter() {
        let page_index = page.index() as usize;
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
                        font_size: text.font_size().value,
                    });
                    continue;
                };
                lines.push(Line {
                    text: run,
                    top: bounds.top.value,
                    bottom: bounds.bottom.value,
                    left: bounds.left.value,
                    font_size: text.font_size().value,
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
                if gap > median_size * 1.6 {
                    if !para.is_empty() {
                        markdown.push_str(para.trim_end());
                        markdown.push_str("\n\n");
                        para.clear();
                    }
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
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn missing_library_reports_unavailable_with_hint() {
        let _env = env_guard();
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
        let _env = env_guard();
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
        let _env = env_guard();
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
        let _env = env_guard();
        // 4x4 JPEG via the image crate (png feature set includes jpeg).
        let img = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(8, 8, |x, y| {
            image::Rgb([(x as u8) * 30, (y as u8) * 30, 255u8])
        }));
        let mut jpg = std::io::Cursor::new(Vec::new());
        img.write_to(&mut jpg, image::ImageOutputFormat::Jpeg(90))
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
