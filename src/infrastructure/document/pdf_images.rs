// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Embedded PDF image extraction — lopdf structure walk (feature `pdfium`).
//!
//! pdfium-render 0.5.9 seals page-object handles behind a `pub(crate)`
//! trait, so image bytes cannot come from the pdfium binding. This module
//! walks the PDF structure with lopdf (crates.io 0.47, call-site
//! decompression limits) and extracts embedded image XObjects:
//!
//! - `DCTDecode` / `JPXDecode` streams pass through as-is (`jpg` / `jp2`);
//! - plain or `FlateDecode` 8bpc gray/RGB samples are re-encoded to PNG;
//! - everything else (CMYK, Indexed, 16bpc) is skipped with an explicit
//!   reason recorded in `Extraction::skipped` — surfaced in conversion
//!   metadata, never silently dropped.

#![cfg(feature = "pdfium")]

use std::collections::HashMap;

use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};

/// Per-stream decompression ceiling for untrusted image streams — aligns
/// with the text tier's bound (`pdf::MAX_STREAM_DECOMPRESSED_BYTES`). 0.47's
/// plain `decompressed_content` (no limit arg) is unbounded, so the FlateDecode path must
/// go through the `_with_limit` variant explicitly.
const IMAGE_STREAM_LIMIT: usize = 64 * 1024 * 1024;

/// Maximum pages walked per document — runaway page trees fail loudly.
const MAX_PAGES: u32 = 4096;

/// Maximum parent hops when resolving inherited `/Resources`.
const MAX_PARENT_HOPS: u8 = 16;

/// Maximum image entries per document — the aggregate-memory budget the
/// per-stream 64 MiB cap cannot provide. Mirrors the container family's
/// total-bytes tier (`containers::MAX_TOTAL_BYTES`).
const MAX_IMAGES: usize = 256;

/// Maximum cumulative decompressed image bytes per document.
const MAX_TOTAL_IMAGE_BYTES: usize = 256 * 1024 * 1024;

/// Skip-ledger cap: keep the first 32 reasons, then count the overflow.
const MAX_SKIP_REASONS: usize = 32;

/// One distinct embedded image XObject.
pub struct ImageEntry {
    /// Encoded bytes (JPEG / JP2 passthrough, or PNG re-encode).
    pub bytes: Vec<u8>,
    /// Filename extension matching the encoding (`jpg` / `jp2` / `png`).
    pub ext: &'static str,
}

/// Where an image appears: 0-based page index, entry index.
pub type ImagePosition = (u32, usize);

/// Extraction outcome with the explicit skip ledger.
pub struct Extraction {
    /// Distinct image XObjects, in first-seen order.
    pub entries: Vec<ImageEntry>,
    /// Every (page, entry) reference, in page order.
    pub positions: Vec<ImagePosition>,
    /// Skipped images with reasons — surfaced in metadata, never silent.
    pub skipped: Vec<String>,
}

/// Walk every page's `/XObject` resources and extract image streams.
pub fn extract_page_images(bytes: &[u8]) -> Extraction {
    // 解析期 ObjStm/XRefStm 同样受 64 MiB 上限约束（裸 load_mem 无界）
    let doc = match Document::load_mem_with_options(
        bytes,
        LoadOptions::with_max_decompressed_size(IMAGE_STREAM_LIMIT),
    ) {
        Ok(doc) => doc,
        Err(e) => {
            // lopdf parse failure must not lose pdfium's text extraction:
            // report as skip reason; images are simply absent.
            return Extraction {
                entries: Vec::new(),
                positions: Vec::new(),
                skipped: vec![format!("lopdf parse failed: {e}")],
            };
        }
    };

    let mut walker = Walker {
        doc: &doc,
        entries: Vec::new(),
        positions: Vec::new(),
        skipped: Vec::new(),
        seen: HashMap::new(),
        total_bytes: 0,
        skip_overflow: 0,
    };
    for (page_no, id) in doc.get_pages() {
        if page_no > MAX_PAGES {
            walker.push_skip(format!("page tree truncated at {MAX_PAGES} pages"));
            break;
        }
        walker.walk_page(page_no, id);
    }
    if walker.skip_overflow > 0 {
        walker.skipped.push(format!(
            "... and {} more skipped (ledger capped at {MAX_SKIP_REASONS})",
            walker.skip_overflow
        ));
    }
    Extraction {
        entries: walker.entries,
        positions: walker.positions,
        skipped: walker.skipped,
    }
}

/// Per-page walker: extracts each page's image XObjects in order.
struct Walker<'a> {
    doc: &'a Document,
    entries: Vec<ImageEntry>,
    positions: Vec<ImagePosition>,
    skipped: Vec<String>,
    /// XObject id → entry index; repeated references share one entry.
    seen: HashMap<ObjectId, usize>,
    /// Cumulative decompressed bytes kept in `entries` (budget tracking).
    total_bytes: usize,
    /// Reasons beyond `MAX_SKIP_REASONS` (appended as one final entry).
    skip_overflow: usize,
}

impl Walker<'_> {
    fn walk_page(&mut self, page_no: u32, id: ObjectId) {
        let page_index = page_no - 1; // lopdf numbers pages 1-based
        let Some(resources) = self.page_resources(id) else {
            return; // no resources at all: page carries no images
        };
        let Ok(xobj_obj) = resources.get(b"XObject") else {
            return;
        };
        let xobjects: Dictionary = match self.deref(xobj_obj).as_dict() {
            Ok(d) => d.clone(),
            Err(e) => {
                self.push_skip(format!(
                    "page {}: /XObject not a dict ({e})",
                    page_index + 1
                ));
                return;
            }
        };

        for (name, value) in xobjects.iter() {
            if self.entries.len() >= MAX_IMAGES {
                self.push_skip(format!(
                    "image budget exhausted (>{} images), remaining skipped",
                    MAX_IMAGES
                ));
                return;
            }
            // Non-Image XObjects (e.g. Form) are not images at all: not
            // ledger material. Images nested inside Form XObjects are out
            // of scope for this walk (documented limitation).
            let is_image = self
                .doc
                .dereference(value)
                .ok()
                .and_then(|(_, o)| o.as_stream().ok())
                .and_then(|s| s.dict.get(b"Subtype").ok())
                .and_then(|o| self.doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_name().ok())
                .map(|n| n == b"Image")
                .unwrap_or(false);
            if !is_image {
                continue;
            }
            let (id, obj) = match self.doc.dereference(value) {
                Ok((Some(id), obj)) => (id, obj),
                Ok((None, _)) => {
                    self.push_skip(format!(
                        "page {}: image {name:?} has no object id",
                        page_index + 1
                    ));
                    continue;
                }
                Err(e) => {
                    self.push_skip(format!(
                        "page {}: image {name:?} unresolved ({e})",
                        page_index + 1
                    ));
                    continue;
                }
            };
            let stream = match obj.as_stream() {
                Ok(s) => s,
                Err(e) => {
                    self.push_skip(format!(
                        "page {}: image {name:?} not a stream ({e})",
                        page_index + 1
                    ));
                    continue;
                }
            };
            // Same XObject referenced on several pages: one entry, both
            // positions recorded.
            if let Some(&idx) = self.seen.get(&id) {
                self.positions.push((page_index, idx));
                continue;
            }
            match decode_image(self.doc, stream) {
                Ok(entry) => {
                    if self.total_bytes + entry.bytes.len() > MAX_TOTAL_IMAGE_BYTES {
                        self.push_skip(format!(
                            "page {}: image {name:?}: total image budget exhausted",
                            page_index + 1
                        ));
                        continue;
                    }
                    self.total_bytes += entry.bytes.len();
                    let idx = self.entries.len();
                    self.entries.push(entry);
                    self.seen.insert(id, idx);
                    self.positions.push((page_index, idx));
                }
                Err(reason) => {
                    self.push_skip(format!("page {}: image {name:?}: {reason}", page_index + 1));
                }
            }
        }
    }

    /// Ledger push with the 32-reason cap; overflow is counted, then
    /// summarized as one final entry by [`extract_page_images`].
    fn push_skip(&mut self, reason: String) {
        if self.skipped.len() < MAX_SKIP_REASONS {
            self.skipped.push(reason);
        } else {
            self.skip_overflow += 1;
        }
    }

    /// Resolve `/Resources`, walking up the page tree for inherited ones.
    fn page_resources(&self, id: ObjectId) -> Option<Dictionary> {
        let mut current = id;
        for _ in 0..MAX_PARENT_HOPS {
            let dict = self.doc.get_object(current).ok()?.as_dict().ok()?;
            if let Ok(res) = dict.get(b"Resources") {
                return self.deref(res).as_dict().ok().cloned();
            }
            current = dict.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    fn deref<'a>(&'a self, obj: &'a Object) -> &'a Object {
        match self.doc.dereference(obj) {
            Ok((_, resolved)) => resolved,
            Err(_) => obj,
        }
    }
}

/// Decode one image XObject stream, or explain why not.
///
/// Free function so the `&mut self` walker never fights the `&Document`
/// borrow that produced `stream`.
fn decode_image(doc: &Document, stream: &lopdf::Stream) -> Result<ImageEntry, String> {
    let dict = &stream.dict;

    let width = int_of(dict.get(b"Width").ok(), doc);
    let height = int_of(dict.get(b"Height").ok(), doc);
    let (width, height) = match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 && w <= 1 << 20 && h <= 1 << 20 => (w, h),
        _ => return Err("invalid dimensions".into()),
    };
    let bpc = int_of(dict.get(b"BitsPerComponent").ok(), doc).unwrap_or(8);
    if bpc != 8 {
        return Err(format!("{bpc} bpc unsupported (8 only)"));
    }

    let filters = filter_names(dict, doc);
    let samples: Vec<u8> = match filters.as_slice() {
        [] => stream.content.clone(),
        [f] if f == b"DCTDecode" => stream.content.clone(),
        [f] if f == b"JPXDecode" => stream.content.clone(),
        [f] if f == b"FlateDecode" => stream
            .decompressed_content_with_limit(IMAGE_STREAM_LIMIT)
            .map_err(|e| format!("flate decode failed: {e}"))?,
        other => {
            return Err(format!(
                "filter chain {:?} unsupported",
                other
                    .iter()
                    .map(|f| String::from_utf8_lossy(f).into_owned())
                    .collect::<Vec<_>>()
            ));
        }
    };

    if matches!(filters.as_slice(), [f] if f == b"DCTDecode") {
        // Passthrough must actually be a JPEG: reject mislabeled streams
        // (arbitrary bytes must not land on disk as .jpg).
        if !samples.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Err("DCTDecode stream is not a JPEG (magic bytes missing)".into());
        }
        return Ok(ImageEntry {
            ext: "jpg",
            bytes: samples,
        });
    }
    if matches!(filters.as_slice(), [f] if f == b"JPXDecode") {
        // JP2 signature box: 00 00 00 0C 6A 50 20 20.
        if !samples.starts_with(&[0x00, 0x00, 0x00, 0x0C, b'j', b'P', b' ', b' ']) {
            return Err("JPXDecode stream is not JPEG 2000 (signature box missing)".into());
        }
        return Ok(ImageEntry {
            ext: "jp2",
            bytes: samples,
        });
    }

    // Sample re-encoding needs a colorspace to know the channel count.
    let channels = match colorspace_channels(dict, doc) {
        Some(c) => c,
        None => return Err("colorspace unsupported".into()),
    };
    let expected = (width as usize) * (height as usize) * channels;
    if samples.len() != expected {
        return Err(format!(
            "sample count {} != expected {expected}",
            samples.len()
        ));
    }
    let png = encode_png(samples, width as u32, height as u32, channels)
        .ok_or_else(|| "PNG encode failed".to_string())?;
    Ok(ImageEntry {
        ext: "png",
        bytes: png,
    })
}

/// Resolve `/Filter` (Name or Array) into a list of filter names.
fn filter_names(dict: &Dictionary, doc: &Document) -> Vec<Vec<u8>> {
    let resolved = match dict.get(b"Filter") {
        Ok(o) => doc.dereference(o),
        Err(_) => return Vec::new(),
    };
    let (_, f) = match resolved {
        Ok(pair) => pair,
        Err(_) => return Vec::new(),
    };
    match f {
        Object::Name(n) => vec![n.to_vec()],
        Object::Array(items) => items
            .iter()
            .filter_map(|o| o.as_name().ok().map(|n| n.to_vec()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Channel count for sample re-encoding: 1 (gray) or 3 (RGB).
fn colorspace_channels(dict: &Dictionary, doc: &Document) -> Option<usize> {
    let cs = doc.dereference(dict.get(b"ColorSpace").ok()?).ok()?.1;
    match cs {
        Object::Name(n) => match n.as_slice() {
            b"DeviceRGB" => Some(3),
            b"DeviceGray" => Some(1),
            _ => None,
        },
        Object::Array(items) => {
            let head = doc.dereference(items.first()?).ok()?.1;
            if let Object::Name(n) = head {
                match n.as_slice() {
                    b"ICCBased" => {
                        let stream = items.get(1)?;
                        let (id, _) = doc.dereference(stream).ok()?;
                        let icc = doc.get_object(id?).ok()?.as_stream().ok()?;
                        match int_of(icc.dict.get(b"N").ok(), doc)? {
                            1 => Some(1),
                            3 => Some(3),
                            _ => None,
                        }
                    }
                    b"CalRGB" => Some(3),
                    b"CalGray" => Some(1),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Re-encode 8bpc samples as PNG (RGB 3ch or Gray 1ch).
fn encode_png(samples: Vec<u8>, width: u32, height: u32, channels: usize) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    let result = match channels {
        3 => image::DynamicImage::ImageRgb8(image::ImageBuffer::from_raw(width, height, samples)?)
            .write_to(&mut out, image::ImageOutputFormat::Png),
        1 => image::DynamicImage::ImageLuma8(image::ImageBuffer::from_raw(width, height, samples)?)
            .write_to(&mut out, image::ImageOutputFormat::Png),
        _ => return None,
    };
    result.ok()?;
    Some(out.into_inner())
}

/// Resolve an optional dictionary entry into an integer.
fn int_of(obj: Option<&Object>, doc: &Document) -> Option<i64> {
    let obj = doc.dereference(obj?).ok()?.1;
    obj.as_i64().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use lopdf::{Stream, dictionary};

    /// Build a minimal valid PDF around one image XObject dict + payload.
    fn fixture_pdf(image_dict: Dictionary, payload: Vec<u8>) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let image_id = doc.add_object(Object::Stream(Stream::new(image_dict, payload)));
        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Im0" => image_id },
        });
        let content_id = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 64.into(), 64.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    fn image_dict(filter: Option<&str>, colorspace: &str) -> Dictionary {
        let mut d = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 4,
            "Height" => 4,
            "BitsPerComponent" => 8,
            "ColorSpace" => colorspace,
        };
        if let Some(f) = filter {
            d.set("Filter", Object::Name(f.as_bytes().to_vec()));
        }
        d
    }

    fn encoded_jpeg() -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(4, 4, |x, y| {
            image::Rgb([x as u8 * 40, y as u8 * 40, 128])
        }));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageOutputFormat::Jpeg(80))
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn dct_image_extracts_as_jpeg_passthrough() {
        let pdf = fixture_pdf(image_dict(Some("DCTDecode"), "DeviceRGB"), encoded_jpeg());
        let ex = extract_page_images(&pdf);
        assert_eq!(ex.entries.len(), 1, "skipped: {:?}", ex.skipped);
        assert_eq!(ex.entries[0].ext, "jpg");
        assert_eq!(&ex.entries[0].bytes[..2], &[0xFF, 0xD8], "JPEG SOI marker");
        assert_eq!(ex.positions, vec![(0, 0)]);
        assert!(ex.skipped.is_empty(), "{:?}", ex.skipped);
    }

    #[test]
    fn plain_samples_reencode_to_png() {
        let samples: Vec<u8> = (0..48u8).collect(); // 4x4 RGB
        let pdf = fixture_pdf(image_dict(None, "DeviceRGB"), samples);
        let ex = extract_page_images(&pdf);
        assert_eq!(ex.entries.len(), 1, "skipped: {:?}", ex.skipped);
        assert_eq!(ex.entries[0].ext, "png");
        assert_eq!(&ex.entries[0].bytes[..4], &[0x89, b'P', b'N', b'G']);
        let decoded = image::load_from_memory(&ex.entries[0].bytes).unwrap();
        assert_eq!((decoded.dimensions()), (4, 4));
    }

    #[test]
    fn flate_samples_reencode_to_png() {
        // Hand-built zlib stream (stored block): lopdf's own compress() only
        // sets Filter when it actually shrinks the payload, so a 48-byte
        // fixture would silently skip compression and never reach the
        // FlateDecode match arm. A stored-block zlib wrapper is deterministic
        // and dependency-free, and forces the limit-bounded decode path.
        let samples: Vec<u8> = (0..48u8).collect(); // 4x4 RGB
        let mut zlib = vec![0x78, 0x01]; // CMF=deflate/32K, FLG check bits
        zlib.push(0x00); // non-final stored block
        zlib.extend_from_slice(&(samples.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(samples.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(&samples);
        let (mut a1, mut a2) = (1u32, 0u32); // adler32
        for b in &samples {
            a1 = (a1 + *b as u32) % 65521;
            a2 = (a2 + a1) % 65521;
        }
        zlib.extend_from_slice(&((a2 << 16) | a1).to_be_bytes());
        let mut s = Stream::new(image_dict(Some("FlateDecode"), "DeviceRGB"), zlib);
        s.dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let pdf = fixture_pdf(s.dict, s.content);
        let ex = extract_page_images(&pdf);
        assert_eq!(ex.entries.len(), 1, "skipped: {:?}", ex.skipped);
        assert_eq!(ex.entries[0].ext, "png");
        let decoded = image::load_from_memory(&ex.entries[0].bytes).unwrap();
        assert_eq!((decoded.dimensions()), (4, 4));
    }

    #[test]
    fn unsupported_colorspace_skips_explicitly() {
        // DeviceCMYK samples: 4 channels, outside the supported subset.
        let pdf = fixture_pdf(image_dict(None, "DeviceCMYK"), vec![0u8; 4 * 4 * 4]);
        let ex = extract_page_images(&pdf);
        assert!(ex.entries.is_empty());
        assert_eq!(ex.skipped.len(), 1, "{:?}", ex.skipped);
        assert!(ex.skipped[0].contains("colorspace"), "{:?}", ex.skipped);
    }

    #[test]
    fn shared_xobject_referenced_twice_keeps_both_positions() {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let image_id = doc.add_object(Object::Stream(Stream::new(
            image_dict(Some("DCTDecode"), "DeviceRGB"),
            encoded_jpeg(),
        )));
        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Im0" => image_id },
        });
        let content_id = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let mut page_ids = Vec::new();
        for _ in 0..2 {
            page_ids.push(doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
            }));
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<Object>>(),
                "Count" => 2,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();

        let ex = extract_page_images(&buf);
        assert_eq!(ex.entries.len(), 1, "one XObject = one entry");
        assert_eq!(
            ex.positions,
            vec![(0, 0), (1, 0)],
            "both references recorded"
        );
    }

    #[test]
    fn malformed_bytes_report_parse_skip() {
        let ex = extract_page_images(b"not a pdf at all");
        assert!(ex.entries.is_empty());
        assert!(
            ex.skipped[0].contains("lopdf parse failed"),
            "{:?}",
            ex.skipped
        );
    }

    #[test]
    fn image_budget_exhaustion_stops_with_reason() {
        // 300 distinct 4x4 image XObjects on one page: the 256-entry
        // aggregate budget must stop the walk with an explicit reason.
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let payload = encoded_jpeg();
        let mut xobj = lopdf::Dictionary::new();
        for i in 0..300u32 {
            let mut d = image_dict(Some("DCTDecode"), "DeviceRGB");
            d.set("Name", format!("img{i}"));
            xobj.set(
                format!("Im{i}"),
                Object::Reference(doc.add_object(Object::Stream(Stream::new(d, payload.clone())))),
            );
        }
        let resources_id = doc.add_object(dictionary! { "XObject" => xobj });
        let content_id = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();

        let ex = extract_page_images(&buf);
        assert_eq!(ex.entries.len(), 256, "budget caps entries");
        assert!(
            ex.skipped.iter().any(|r| r.contains("budget exhausted")),
            "{:?}",
            ex.skipped
        );
    }

    #[test]
    fn mislabeled_dct_stream_rejected_by_magic_bytes() {
        // Arbitrary payload labeled DCTDecode must not land on disk as .jpg.
        let pdf = fixture_pdf(
            image_dict(Some("DCTDecode"), "DeviceRGB"),
            b"<html>not a jpeg</html>".to_vec(),
        );
        let ex = extract_page_images(&pdf);
        assert!(ex.entries.is_empty(), "no asset for mislabeled stream");
        assert!(
            ex.skipped.iter().any(|r| r.contains("magic bytes missing")),
            "{:?}",
            ex.skipped
        );
    }

    #[test]
    fn gray_samples_reencode_to_png() {
        let samples: Vec<u8> = (0..16u8).collect(); // 4x4 gray
        let pdf = fixture_pdf(image_dict(None, "DeviceGray"), samples);
        let ex = extract_page_images(&pdf);
        assert_eq!(ex.entries.len(), 1, "skipped: {:?}", ex.skipped);
        assert_eq!(ex.entries[0].ext, "png");
        let decoded = image::load_from_memory(&ex.entries[0].bytes).unwrap();
        assert_eq!((decoded.dimensions()), (4, 4));
    }
}
