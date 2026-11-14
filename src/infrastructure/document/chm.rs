// SPDX-License-Identifier: Apache-2.0
//! CHM (Compiled HTML Help) converter — extract HTML topics, render each
//! through the existing HTML chain, and merge into one Markdown document.
//!
//! Ordering follows the `.hhc` table of contents when one is present
//! (unlisted topics are appended in lexicographic path order); entry
//! budgets are explicit and truncation is reported in metadata rather
//! than silently dropped.

#![cfg(feature = "chm")]

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Maximum number of topics rendered into the output document.
const MAX_TOPICS: usize = 512;
/// Maximum size of a single topic's decompressed bytes (skipped when larger).
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;
/// Maximum cumulative topic bytes rendered before truncation.
const MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;

/// CHM converter: HTML topics in, one merged Markdown document out.
#[derive(Default)]
pub struct ChmConverter;

impl ChmConverter {
    /// Create a new CHM converter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Converter for ChmConverter {
    fn name(&self) -> &'static str {
        "chm"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        // 同步枚举/LZX 解压/html 渲染是 CPU 密集段，与 pdf/pdfium 一致包
        // spawn_blocking，避免钉死 async worker；panic 经 JoinError 显性化。
        tokio::task::spawn_blocking(move || convert_sync(input))
            .await
            .map_err(|e| AlchemizeError::other(format!("CHM extraction task failed: {e}")))?
    }

    fn supported_extensions(&self) -> &[&str] {
        &["chm"]
    }

    fn priority(&self) -> u8 {
        10
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }
}

/// Whether a CHM entry is a renderable HTML topic: `.htm`/`.html` only,
/// excluding the `#`/`$` internal streams and the Help workshop files.
fn is_topic_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let last = lower.rsplit('/').next().unwrap_or("");
    if last.starts_with('#') || last.starts_with('$') {
        return false;
    }
    if lower.ends_with(".hhp") || lower.ends_with(".hhc") || lower.ends_with(".hhk") {
        return false;
    }
    lower.ends_with(".htm") || lower.ends_with(".html")
}

/// The whole (synchronous) CHM conversion: enumerate → order → budget →
/// render. Runs on the blocking pool.
fn convert_sync(input: Input) -> Result<ConvertResult, AlchemizeError> {
    // libchm only opens real paths: pass through `Path`, spool `Bytes`
    // through a temp file (heic does the same; pdfium reads in memory).
    let keep_alive;
    let path: PathBuf = match &input {
        Input::Path(p) => p.clone(),
        Input::Bytes { data, .. } => {
            let mut tmp = tempfile::NamedTempFile::new().map_err(|e| {
                AlchemizeError::other(format!("failed to create temp file for CHM: {e}"))
            })?;
            std::io::Write::write_all(&mut tmp, data)
                .map_err(|e| AlchemizeError::other(format!("failed to spool CHM input: {e}")))?;
            keep_alive = tmp;
            keep_alive.path().to_path_buf()
        }
    };

    let html = crate::infrastructure::document::HtmlConverter::new();
    let mut chm = libchm::ChmFile::open(&path)
        .map_err(|e| AlchemizeError::other(format!("CHM open failed: {e}")))?;

    let entries = chm
        .entries(libchm::EntrySel::ALL)
        .map_err(|e| AlchemizeError::other(format!("CHM enumeration failed: {e}")))?;
    let entry_count = entries.len();

    let topic_paths: Vec<String> = entries
        .iter()
        .filter(|e| !e.is_dir() && is_topic_path(&e.path))
        .map(|e| e.path.clone())
        .collect();
    // Decompressed length is known up front: drop oversized topics here
    // instead of reading and discarding them. Reads go through the
    // enumerated entry itself (never a fresh directory lookup), so a
    // duplicate-path entry cannot slip a different length past the budget.
    let idx_by_lower: HashMap<String, usize> = entries
        .iter()
        .enumerate()
        .map(|(idx, e)| (e.path.trim_start_matches('/').to_ascii_lowercase(), idx))
        .collect();
    let len_by_lower: HashMap<String, u64> = idx_by_lower
        .iter()
        .map(|(key, idx)| (key.clone(), entries[*idx].length))
        .collect();
    let toc = read_toc_order(&mut chm, &entries);
    let ordered = order_topics(&topic_paths, &toc);

    if ordered.is_empty() {
        // No HTML topics: an entry inventory is the honest fallback.
        let mut out = String::from("# CHM entry inventory\n\n");
        for entry in &entries {
            out.push_str(&format!(
                "- {}\n",
                crate::infrastructure::document::shared::escape_markdown_label(&entry.path)
            ));
        }
        let metadata = serde_json::json!({
            "source_format": "chm",
            "topic_count": 0,
            "entry_count": entry_count,
            "truncated": false,
        });
        return Ok(
            ConvertResult::with_metadata(out, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFallback),
        );
    }

    let mut out = String::new();
    let mut rendered = 0usize;
    let mut truncated = false;
    let (selected, dropped) = select_within_budget(
        &ordered,
        &len_by_lower,
        MAX_TOPICS,
        MAX_ENTRY_BYTES,
        MAX_TOTAL_BYTES,
    );
    truncated |= dropped;
    for topic in selected {
        let bytes = match idx_by_lower
            .get(topic.trim_start_matches('/').to_ascii_lowercase().as_str())
            .and_then(|&idx| chm.read(&entries[idx]).ok())
        {
            Some(bytes) => bytes,
            None => {
                // A single unreadable topic must not sink the document;
                // skip it and say so. (A render failure still fails the
                // whole conversion: skip = we never got the bytes, fail =
                // we had them but they are unusable.)
                truncated = true;
                let safe: String = topic.chars().filter(|c| !c.is_control()).collect();
                tracing::warn!("chm: skipping unreadable topic {safe}");
                continue;
            }
        };

        let html_body = String::from_utf8_lossy(&bytes);
        let converted = html
            .convert_str(&html_body)
            .map_err(|e| AlchemizeError::other(format!("CHM topic {topic} failed: {e}")))?;
        out.push_str(&format!(
            "## {}\n\n",
            crate::infrastructure::document::shared::escape_markdown_label(topic)
        ));
        out.push_str(converted.markdown.trim());
        out.push_str("\n\n");
        rendered += 1;
    }

    let metadata = serde_json::json!({
        "source_format": "chm",
        "topic_count": rendered,
        "entry_count": entry_count,
        "truncated": truncated,
    });
    Ok(
        ConvertResult::with_metadata(out, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull),
    )
}

/// Parse the `.hhc` table of contents and return the `Local` targets in
/// document order. A malformed, missing, or over-budget `.hhc` yields an
/// empty list — ordering then falls back to lexicographic paths.
fn read_toc_order(chm: &mut libchm::ChmFile, entries: &[libchm::Entry]) -> Vec<String> {
    let Some(hhc) = entries
        .iter()
        .find(|e| e.path.to_ascii_lowercase().ends_with(".hhc"))
    else {
        return Vec::new();
    };
    // The directory-declared length is attacker-controlled input; the
    // per-entry budget applies to `.hhc` too, before any allocation.
    if hhc.length as usize > MAX_ENTRY_BYTES {
        tracing::warn!("chm: skipping oversized table of contents");
        return Vec::new();
    }
    let Ok(bytes) = chm.read(hhc) else {
        return Vec::new();
    };
    extract_local_targets(&String::from_utf8_lossy(&bytes))
}

/// Extract `Local` parameter values from `.hhc` markup: `<param name="Local"
/// value="topic.htm">` lines, scanned case-insensitively without regex.
fn extract_local_targets(hhc: &str) -> Vec<String> {
    let lower = hhc.to_ascii_lowercase();
    let mut targets = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find("name=\"local\"") {
        let value_start = cursor + rel;
        // The `value="…"` attribute follows on the same <param> tag.
        let Some(rest_rel) = lower[value_start..].find("value=\"") else {
            break;
        };
        let rest = value_start + rest_rel + "value=\"".len();
        let Some(end_rel) = lower[rest..].find('"') else {
            break;
        };
        let target = hhc[rest..rest + end_rel].trim();
        if !target.is_empty() {
            targets.push(target.to_string());
        }
        cursor = rest + end_rel;
    }
    targets
}

/// Order topic paths: `.hhc` order first (targets resolved case-insensitively
/// against actual entry paths), remaining topics appended lexicographically.
/// Always returns the canonical entry path form, never the raw TOC target.
fn order_topics<'a>(topic_paths: &'a [String], toc_targets: &[String]) -> Vec<&'a str> {
    let by_lower: HashMap<String, &'a str> = topic_paths
        .iter()
        .map(|p| (p.trim_start_matches('/').to_ascii_lowercase(), p.as_str()))
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut ordered: Vec<&'a str> = Vec::new();
    for target in toc_targets {
        let key = target.trim_start_matches('/').to_ascii_lowercase();
        if let Some(path) = by_lower.get(&key)
            && seen.insert(key)
        {
            ordered.push(path);
        }
    }
    let mut rest: Vec<&'a str> = topic_paths
        .iter()
        .map(String::as_str)
        .filter(|p| !seen.contains(&p.trim_start_matches('/').to_ascii_lowercase()))
        .collect();
    rest.sort();
    for path in rest {
        if seen.insert(path.trim_start_matches('/').to_ascii_lowercase()) {
            ordered.push(path);
        }
    }
    ordered
}

/// Apply the three render budgets to the ordered topic list: drop topics
/// whose decompressed length exceeds `max_entry`, keep at most `max_topics`,
/// and stop before the cumulative `max_total` would overflow. Returns the
/// renderable topics in order plus whether anything was dropped — every
/// budget drop must surface as `truncated` in the metadata.
fn select_within_budget<'a>(
    ordered: &[&'a str],
    len_by_lower: &HashMap<String, u64>,
    max_topics: usize,
    max_entry: usize,
    max_total: usize,
) -> (Vec<&'a str>, bool) {
    let mut dropped = ordered.len() > max_topics;
    let mut selected: Vec<&'a str> = Vec::new();
    let mut total = 0usize;
    for topic in ordered.iter().take(max_topics) {
        let key = topic.trim_start_matches('/').to_ascii_lowercase();
        let len = len_by_lower.get(&key).copied().unwrap_or(0) as usize;
        if len > max_entry {
            dropped = true;
            continue;
        }
        if total + len > max_total {
            dropped = true;
            break;
        }
        total += len;
        selected.push(topic);
    }
    (selected, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract() {
        let c = ChmConverter::new();
        assert_eq!(c.name(), "chm");
        assert_eq!(c.supported_extensions(), &["chm"]);
        assert_eq!(c.availability(), Availability::Available);
    }

    #[test]
    fn topic_filter_excludes_internal_streams_and_workshop_files() {
        assert!(is_topic_path("/index.html"));
        assert!(is_topic_path("/guide/intro.htm"));
        assert!(!is_topic_path("#WINDOWS"));
        assert!(!is_topic_path("/$FIADfontTable"));
        assert!(!is_topic_path("/project.hhp"));
        assert!(!is_topic_path("/contents.hhc"));
        assert!(!is_topic_path("/index.hhk"));
        assert!(!is_topic_path("/image.png"));
    }

    #[test]
    fn local_targets_are_extracted_in_document_order() {
        let hhc = "<html><ul><li><object type=\"text/sitemap\">\
                   <param name=\"Name\" value=\"Intro\">\
                   <param name=\"Local\" value=\"intro.htm\">\
                   </object></li><li><object type=\"text/sitemap\">\
                   <param name=\"Name\" value=\"Usage\">\
                   <param name=\"Local\" value=\"GUIDE/usage.htm\">\
                   </object></li></ul></html>";
        assert_eq!(
            extract_local_targets(hhc),
            vec!["intro.htm".to_string(), "GUIDE/usage.htm".to_string()]
        );
    }

    #[test]
    fn ordering_resolves_case_insensitively_and_appends_unlisted() {
        let topics = vec![
            "/zzz.htm".to_string(),
            "/intro.htm".to_string(),
            "/guide/usage.htm".to_string(),
        ];
        let ordered = order_topics(&topics, &["intro.htm".to_string()]);
        assert_eq!(ordered[0], "/intro.htm");
        // Unlisted topics keep lexicographic order after the listed ones.
        assert_eq!(ordered[1], "/guide/usage.htm");
        assert_eq!(ordered[2], "/zzz.htm");
    }

    #[test]
    fn budget_drops_oversized_topics() {
        let topics = ["/big.htm".to_string(), "/ok.htm".to_string()];
        let lens = HashMap::from([
            ("big.htm".to_string(), 17u64 * 1024 * 1024), // > 16 MiB
            ("ok.htm".to_string(), 100u64),
        ]);
        let (selected, dropped) = select_within_budget(
            &topics.iter().map(String::as_str).collect::<Vec<_>>(),
            &lens,
            512,
            16 * 1024 * 1024,
            256 * 1024 * 1024,
        );
        assert_eq!(selected, vec!["/ok.htm"]);
        // The oversized drop must surface so metadata can report it.
        assert!(dropped);
    }

    #[test]
    fn budget_caps_topic_count() {
        let topics: Vec<String> = (0..600).map(|i| format!("/t{i:03}.htm")).collect();
        let refs: Vec<&str> = topics.iter().map(String::as_str).collect();
        let lens: HashMap<String, u64> = topics
            .iter()
            .map(|t| (t.trim_matches('/').to_string(), 10u64))
            .collect();
        let (selected, dropped) = select_within_budget(&refs, &lens, 512, 1024, usize::MAX / 2);
        assert_eq!(selected.len(), 512);
        assert!(dropped);
    }

    #[test]
    fn budget_stops_before_cumulative_overflow() {
        // 20 topics × 15 MiB = 300 MiB > the 256 MiB cumulative budget,
        // while each stays under the 16 MiB per-entry budget: rendering
        // stops at 17 topics (255 MiB; the 18th would overflow).
        let topics: Vec<String> = (0..20).map(|i| format!("/c{i:02}.htm")).collect();
        let refs: Vec<&str> = topics.iter().map(String::as_str).collect();
        let lens: HashMap<String, u64> = topics
            .iter()
            .map(|t| (t.trim_matches('/').to_string(), 15u64 * 1024 * 1024))
            .collect();
        let (selected, dropped) =
            select_within_budget(&refs, &lens, 512, 16 * 1024 * 1024, 256 * 1024 * 1024);
        assert_eq!(selected.len(), 17);
        assert!(dropped);
    }

    #[tokio::test]
    async fn garbage_input_errors_without_panicking() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut tmp, b"not a chm file at all").unwrap();
        let err = ChmConverter::new()
            .convert(Input::Path(tmp.path().to_path_buf()))
            .await;
        assert!(err.is_err(), "garbage must not silently convert");
    }

    // --- minimal uncompressed CHM fixture ------------------------------------
    //
    // libchm (and upstream) ships no writer, so the fixture is assembled
    // here byte-by-byte against the container layout the parser validates:
    // ITSF v3 header, ITSP v1 directory header, one PMGL leaf block, and a
    // raw (uncompressed, space 0) content section.

    /// Big-endian base-128 cword, as parsed by `parse_cword`: groups of 7
    /// bits, highest group first, with the continuation bit (0x80) set on
    /// every byte except the last.
    fn cword(value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut v = value;
        loop {
            out.push((v & 0x7f) as u8);
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        out.reverse();
        let n = out.len();
        for byte in &mut out[..n - 1] {
            *byte |= 0x80;
        }
        out
    }

    /// One PMGL entry: cword-prefixed path + space/start/length cwords.
    fn pmgl_entry(path: &str, start: u64, length: u64) -> Vec<u8> {
        let mut entry = cword(path.len() as u64);
        entry.extend_from_slice(path.as_bytes());
        entry.extend(cword(0)); // space 0 = uncompressed section
        entry.extend(cword(start));
        entry.extend(cword(length));
        entry
    }

    /// ITSF v3 + ITSP v1 + one PMGL block + raw content, for `files`.
    fn assemble_chm(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut content = Vec::new();
        let mut entries = Vec::new();
        for (path, bytes) in files {
            entries.push(pmgl_entry(path, content.len() as u64, bytes.len() as u64));
            content.extend_from_slice(bytes);
        }
        const BLOCK_LEN: usize = 0x1000;
        const ITSF_LEN: usize = 0x60;
        const ITSP_LEN: usize = 0x54;
        let dir_offset = ITSF_LEN as u64;
        let dir_len = (ITSP_LEN + BLOCK_LEN) as u64;
        let data_offset = dir_offset + dir_len;

        let mut pmgl = vec![0u8; BLOCK_LEN];
        pmgl[0..4].copy_from_slice(b"PMGL");
        let entry_bytes: usize = entries.iter().map(Vec::len).sum();
        let free_space = BLOCK_LEN - 0x14 - entry_bytes;
        pmgl[4..8].copy_from_slice(&(free_space as u32).to_le_bytes());
        pmgl[0x10..0x14].copy_from_slice(&(-1i32).to_le_bytes());
        pmgl[0x14..0x14 + entry_bytes].copy_from_slice(&entries.concat());

        let mut itsp = vec![0u8; ITSP_LEN];
        itsp[0..4].copy_from_slice(b"ITSP");
        itsp[4..8].copy_from_slice(&1u32.to_le_bytes());
        itsp[8..12].copy_from_slice(&(ITSP_LEN as u32).to_le_bytes());
        itsp[0x10..0x14].copy_from_slice(&(BLOCK_LEN as u32).to_le_bytes());
        itsp[0x1c..0x20].copy_from_slice(&(-1i32).to_le_bytes()); // no PMGI root
        itsp[0x20..0x24].copy_from_slice(&0i32.to_le_bytes()); // head = block 0

        let mut itsf = vec![0u8; ITSF_LEN];
        itsf[0..4].copy_from_slice(b"ITSF");
        itsf[4..8].copy_from_slice(&3u32.to_le_bytes());
        itsf[8..12].copy_from_slice(&(ITSF_LEN as u32).to_le_bytes());
        itsf[0x48..0x50].copy_from_slice(&dir_offset.to_le_bytes());
        itsf[0x50..0x58].copy_from_slice(&dir_len.to_le_bytes());
        itsf[0x58..0x60].copy_from_slice(&data_offset.to_le_bytes());

        let mut out = itsf;
        out.extend_from_slice(&itsp);
        out.extend_from_slice(&pmgl);
        out.extend_from_slice(&content);
        out
    }

    /// Two listed topics (`.hhc` order, one case-mismatched), one unlisted
    /// topic, an internal stream, a workshop file, and a folder entry.
    fn sample_chm() -> Vec<u8> {
        const HHC: &str = "<html><body><ul>\
<li><object type=\"text/sitemap\"><param name=\"Name\" value=\"Intro\">\
<param name=\"Local\" value=\"intro.htm\"></object></li>\
<li><object type=\"text/sitemap\"><param name=\"Name\" value=\"Usage\">\
<param name=\"Local\" value=\"GUIDE/Usage.htm\"></object></li>\
</ul></body></html>";
        const INTRO: &str = "<html><head><title>Intro</title></head>\
<body><h1>Getting Started</h1><p>Welcome to the intro page.</p></body></html>";
        const USAGE: &str = "<html><head><title>Usage</title></head>\
<body><h1>Using the tool</h1><p>Run <code>alch</code> to convert.</p></body></html>";
        const ZZZ: &str = "<html><body><h1>Appendix</h1><p>Unlisted topic.</p></body></html>";
        let files: [(&str, &[u8]); 6] = [
            ("/contents.hhc", HHC.as_bytes()),
            ("/intro.htm", INTRO.as_bytes()),
            ("/guide/usage.htm", USAGE.as_bytes()),
            ("/zzz.htm", ZZZ.as_bytes()),
            ("/project.hhp", b"[OPTIONS]\nCOMPRESS=No\n"),
            ("/#WINDOWS", b"\x00\x01"),
        ];
        assemble_chm(&files)
    }

    /// An uncompressed CHM with no `.htm`/`.html` entries at all.
    fn topicless_chm() -> Vec<u8> {
        assemble_chm(&[("/project.hhp", b"[OPTIONS]\n"), ("/data.txt", b"hi")])
    }

    fn chm_file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut tmp, bytes).unwrap();
        tmp
    }

    #[tokio::test]
    async fn renders_topics_in_toc_order_with_merging() {
        let tmp = chm_file(&sample_chm());
        let result = ChmConverter::new()
            .convert(Input::Path(tmp.path().to_path_buf()))
            .await
            .unwrap();
        let md = &result.markdown;
        // Every listed and unlisted topic is rendered through the HTML chain.
        assert!(md.contains("Getting Started"), "{md}");
        assert!(md.contains("Using the tool"), "{md}");
        assert!(md.contains("Appendix"), "{md}");
        // TOC order wins: intro → usage → zzz (zzz unlisted, appended).
        let (i, u, z) = (
            md.find("intro.htm").unwrap(),
            md.find("usage.htm").unwrap(),
            md.find("zzz.htm").unwrap(),
        );
        assert!(i < u && u < z, "topic order: {md}");
        // Internal streams and workshop files are not rendered as topics.
        assert!(!md.contains("#WINDOWS"), "{md}");
        assert!(!md.contains("project.hhp"), "{md}");
        // Case-insensitive TOC resolution ("GUIDE/Usage.htm" → actual path).
        assert_eq!(result.metadata["topic_count"].as_u64(), Some(3));
        assert_eq!(result.metadata["entry_count"].as_u64(), Some(6));
        assert_eq!(result.metadata["truncated"].as_bool(), Some(false));
    }

    #[tokio::test]
    async fn inventory_fallback_when_no_topics() {
        let tmp = chm_file(&topicless_chm());
        let result = ChmConverter::new()
            .convert(Input::Path(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("entry inventory"),
            "{}",
            result.markdown
        );
        assert!(result.markdown.contains("data.txt"), "{}", result.markdown);
        assert_eq!(result.metadata["topic_count"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn bytes_input_matches_path_input() {
        let bytes_result = ChmConverter::new()
            .convert(Input::bytes(sample_chm(), "sample.chm"))
            .await
            .unwrap();
        assert_eq!(bytes_result.metadata["topic_count"].as_u64(), Some(3));
        assert!(bytes_result.markdown.contains("Getting Started"));
    }
}
