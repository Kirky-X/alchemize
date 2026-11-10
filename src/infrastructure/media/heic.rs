// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! HEIC/HEIF converter (T028).
//!
//! Decoding needs a HEIF codec. Rather than binding libheif (no maintained
//! Rust crate with a vendored build), the converter probes for the OS
//! decoder — macOS `sips` — and converts to PNG with it. Without a decoder
//! the payload degrades to a metadata-only description (brand, box-derived
//! dimensions) and says so in the markdown; nothing silently disappears.

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter, which_program};
use crate::domain::asset::Asset;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// HEIC/HEIF converter backed by the OS image decoder.
pub struct HeicConverter;

impl HeicConverter {
    /// Create a new HEIC converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for HeicConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether an OS decoder is available (`sips` on macOS).
pub fn decoder_available() -> bool {
    which_program("sips")
}

#[async_trait]
impl Converter for HeicConverter {
    fn name(&self) -> &'static str {
        "heic"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read HEIC input: {e}")))?;
        let info = ifd::describe(&bytes);
        let filename = input.filename().unwrap_or("image.heic").to_string();

        if decoder_available() {
            match decode_with_sips(&bytes, &filename).await {
                Ok(png) => {
                    let mut markdown = String::new();
                    markdown.push_str("![");
                    markdown.push_str(&filename);
                    markdown.push_str("](images/0000-heic.png)\n\n");
                    if let Some((w, h)) = info.dimensions {
                        markdown.push_str(&format!("Dimensions: {w}×{h}\n\n"));
                    }
                    let metadata = serde_json::json!({
                        "source_format": "heic",
                        "engine": "sips",
                        "brand": info.brand,
                        "width": info.dimensions.map(|d| d.0),
                        "height": info.dimensions.map(|d| d.1),
                        "decoded_bytes": png.len(),
                    });
                    let asset = Asset::new("images/0000-heic.png", png);
                    return Ok(ConvertResult::with_metadata(
                        markdown,
                        ContentType::Document,
                        metadata,
                    )
                    .with_quality(ConversionQuality::LocalFull)
                    .with_asset(asset));
                }
                Err(e) => {
                    // Decoder present but failed: report it, do not pretend.
                    let markdown = format!(
                        "*HEIC decode failed ({e}).*\n\n{}",
                        ifd::metadata_markdown(&filename, &info)
                    );
                    let metadata = serde_json::json!({
                        "source_format": "heic",
                        "engine": "sips",
                        "decode_error": e,
                        "brand": info.brand,
                    });
                    return Ok(ConvertResult::with_metadata(
                        markdown,
                        ContentType::Document,
                        metadata,
                    )
                    .with_quality(ConversionQuality::LocalFallback));
                }
            }
        }

        // No decoder: keep the structural facts we can derive from the boxes.
        let markdown = ifd::metadata_markdown(&filename, &info);
        let metadata = serde_json::json!({
            "source_format": "heic",
            "engine": "metadata-only",
            "brand": info.brand,
            "width": info.dimensions.map(|d| d.0),
            "height": info.dimensions.map(|d| d.1),
            "decoder": "absent",
        });
        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFallback),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["heic", "heif"]
    }

    fn priority(&self) -> u8 {
        10
    }

    fn availability(&self) -> Availability {
        // Always produces output (metadata-only when no decoder exists).
        Availability::Available
    }
}

/// Decode HEIC bytes to PNG with `sips`.
async fn decode_with_sips(bytes: &[u8], filename: &str) -> Result<Vec<u8>, String> {
    let work = tempfile::Builder::new()
        .prefix("alch-heic-")
        .tempdir()
        .map_err(|e| format!("tempdir failed: {e}"))?;
    let input = work.path().join(filename);
    let output = work.path().join("out.png");
    std::fs::write(&input, bytes).map_err(|e| format!("staging failed: {e}"))?;

    let status = tokio::process::Command::new("sips")
        .arg("-s")
        .arg("format")
        .arg("png")
        .arg(&input)
        .arg("--out")
        .arg(&output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("sips spawn failed: {e}"))?;
    if !status.status.success() {
        return Err(format!(
            "sips exited {}: {}",
            status.status,
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    std::fs::read(&output).map_err(|e| format!("decoded PNG unreadable: {e}"))
}

/// ISO-BMFF box facts: brand and image dimensions.
pub mod ifd {
    /// Facts extracted without decoding pixels.
    #[derive(Default, Debug, Clone)]
    pub struct HeicInfo {
        /// `ftyp` major brand (`heic`, `mif1`, …).
        pub brand: Option<String>,
        /// `ispe` width/height pair.
        pub dimensions: Option<(u32, u32)>,
    }

    /// Walk the box tree far enough to read `ftyp` and the first `ispe`.
    pub fn describe(bytes: &[u8]) -> HeicInfo {
        let mut info = HeicInfo::default();
        if bytes.len() < 12 {
            return info;
        }
        // ftyp: size(4) type(4) major_brand(4) …
        if &bytes[4..8] == b"ftyp" {
            let brand = &bytes[8..12];
            info.brand = Some(String::from_utf8_lossy(brand).trim().to_string());
        }
        info.dimensions = find_ispe(bytes);
        info
    }

    /// Search for an `ispe` box (`size, 'ispe', version/flags, w, h`).
    ///
    /// A linear scan is enough: `ispe` is 20 bytes and appears only in the
    /// item-property container, always after the 4-byte size + type.
    fn find_ispe(bytes: &[u8]) -> Option<(u32, u32)> {
        let mut i = 0usize;
        while i + 20 <= bytes.len() {
            if &bytes[i + 4..i + 8] == b"ispe" {
                let w = u32::from_be_bytes(bytes[i + 12..i + 16].try_into().ok()?);
                let h = u32::from_be_bytes(bytes[i + 16..i + 20].try_into().ok()?);
                if w > 0 && h > 0 && w < 100_000 && h < 100_000 {
                    return Some((w, h));
                }
            }
            i += 1;
        }
        None
    }

    /// Metadata-only markdown describing the container.
    pub fn metadata_markdown(filename: &str, info: &HeicInfo) -> String {
        let mut out = String::new();
        out.push_str(&format!("# {filename}\n\n"));
        out.push_str("HEIC/HEIF image. ");
        if let Some((w, h)) = info.dimensions {
            out.push_str(&format!("Dimensions: {w}×{h}. "));
        }
        if let Some(brand) = &info.brand {
            out.push_str(&format!("Brand: `{brand}`. "));
        }
        out.push_str(
            "\n\n*Pixels not decoded: no HEIF decoder on this host \
             (macOS `sips` provides one; set up a decoder to extract the image).*\n",
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ISOBMFF header with an `ispe` box (1920×1080).
    fn fake_heic() -> Vec<u8> {
        let mut v = Vec::new();
        // ftyp box: size 20, 'ftyp', 'heic', minor 0, compatible 'mif1'
        v.extend_from_slice(&20u32.to_be_bytes());
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(b"heic");
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b"mif1");
        // ispe box: size 20, 'ispe', version/flags 0, w, h
        v.extend_from_slice(&20u32.to_be_bytes());
        v.extend_from_slice(b"ispe");
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&1920u32.to_be_bytes());
        v.extend_from_slice(&1080u32.to_be_bytes());
        v
    }

    #[test]
    fn box_walk_reads_brand_and_dimensions() {
        let info = ifd::describe(&fake_heic());
        assert_eq!(info.brand.as_deref(), Some("heic"));
        assert_eq!(info.dimensions, Some((1920, 1080)));
    }

    #[test]
    fn truncated_input_yields_no_facts() {
        let info = ifd::describe(&[1, 2, 3]);
        assert!(info.brand.is_none());
        assert!(info.dimensions.is_none());
    }

    #[tokio::test]
    async fn converter_reports_metadata_or_decodes() {
        let result = HeicConverter::new()
            .convert(Input::bytes(fake_heic(), "photo.heic"))
            .await
            .unwrap();
        // With a decoder the fake container fails to decode → fallback branch
        // still carries the facts; without one the metadata branch runs.
        assert!(
            result.markdown.contains("1920×1080"),
            "md: {}",
            result.markdown
        );
    }

    #[tokio::test]
    async fn decode_failure_is_reported_not_hidden() {
        if !decoder_available() {
            return; // branch needs a decoder to fail
        }
        // Structurally valid boxes, but no actual image data: sips rejects it.
        let mut bad = fake_heic();
        bad.extend_from_slice(&[0u8; 64]);
        let result = HeicConverter::new()
            .convert(Input::bytes(bad, "broken.heic"))
            .await
            .unwrap();
        assert!(
            result.metadata["decode_error"].as_str().is_some(),
            "decode failure must be surfaced: {}",
            result.metadata
        );
        assert!(
            result.markdown.contains("decode failed"),
            "md: {}",
            result.markdown
        );
        assert!(result.assets.is_empty(), "no asset on failure");
    }

    #[test]
    fn contract() {
        let c = HeicConverter::new();
        assert_eq!(c.name(), "heic");
        assert_eq!(c.supported_extensions(), &["heic", "heif"]);
    }

    #[tokio::test]
    async fn real_heic_decodes_to_png_asset() {
        // Self-skip when the fixture or the OS decoder is missing, so the
        // coverage run exercises the real decode path wherever possible.
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/real/real.heic");
        if !path.is_file() {
            eprintln!("skipping: real.heic fixture missing");
            return;
        }
        if matches!(
            HeicConverter::new().availability(),
            crate::application::ports::Availability::Unavailable { .. }
        ) {
            eprintln!("skipping: no OS HEIC decoder");
            return;
        }
        let result = HeicConverter::new()
            .convert(Input::Path(path))
            .await
            .unwrap();
        assert_eq!(result.assets.len(), 1, "decoded PNG asset expected");
        assert!(
            result.assets[0].bytes.starts_with(b"\x89PNG"),
            "asset must be a PNG"
        );
    }
}
