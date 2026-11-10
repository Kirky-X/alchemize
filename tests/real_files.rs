// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Real-file parsing tests.
//!
//! Every fixture under `tests/fixtures/real/` is a genuine file harvested
//! from this machine or the sibling projects (macOS system documentation,
//! etyma wordlists, vecboost sources/assets) — not a hand-assembled minimal
//! archive. Assertions check *real content* (specific words/structures from
//! the source), not just non-emptiness.

use alchemize::application::builder::AlchemizeBuilder;
use alchemize::domain::input::Input;

fn real(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/real")
        .join(name)
}

async fn convert(name: &str) -> (String, serde_json::Value) {
    let path = real(name);
    assert!(path.is_file(), "real fixture missing: {}", path.display());
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::Path(path))
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    (result.markdown, result.metadata)
}

// -- PDF (real macOS License.pdf, 100KB, multiple pages) ----------------------

#[tokio::test]
async fn real_pdf_extracts_license_text() {
    let (md, meta) = convert("real.pdf").await;
    let chars = md.chars().count();
    assert!(
        chars > 2000,
        "extracted only {chars} chars from a 100KB PDF"
    );
    // Real license documents carry these tokens (case-insensitive check).
    let lower = md.to_lowercase();
    assert!(
        lower.contains("licen") || lower.contains("apple") || lower.contains("copyright"),
        "no recognizable license vocabulary; first 500 chars: {}",
        &md.chars().take(500).collect::<String>()
    );
    // Page structure from the local extractor.
    assert!(
        md.contains("## Page 1"),
        "missing page markers: {}",
        &md[..md.len().min(300)]
    );
    let pages = meta["page_count"].as_u64().unwrap_or(0);
    assert!(
        pages >= 3,
        "expected a multi-page document, got {pages} pages"
    );
}

// -- DOCX (real 英语专业四八级词汇表 Word document) ---------------------------

#[tokio::test]
async fn real_docx_extracts_wordlist() {
    let (md, _) = convert("real.docx").await;
    let chars = md.chars().count();
    assert!(
        chars > 5000,
        "extracted only {chars} chars from a 900KB DOCX"
    );
    // A vocabulary list: many alphabetic runs, some long lines.
    let words = md.split_whitespace().count();
    assert!(words > 500, "expected a dense wordlist, got {words} words");
    let has_latin = md.chars().filter(|c| c.is_ascii_alphabetic()).count();
    assert!(has_latin > 1000, "only {has_latin} latin chars");
}

// -- XLSX (real MorphoLEX_en.xlsx, 6.7MB morphology lexicon) -----------------

#[tokio::test]
async fn real_xlsx_is_disabled_by_default() {
    // Spreadsheet support was removed: no engine accepts xlsx/ods.
    let path = real("real.xlsx");
    assert!(path.is_file(), "real fixture missing: {}", path.display());
    let alch = AlchemizeBuilder::new().build();
    let err = alch
        .convert(&Input::Path(path))
        .await
        .expect_err("default engine set must decline spreadsheets");
    assert!(
        matches!(err, alchemize::AlchemizeError::UnsupportedFormat(_)),
        "{err:?}"
    );
}

// -- PNG (real Dock icon bitmap) ---------------------------------------------

#[tokio::test]
async fn real_png_degrades_with_metadata() {
    let (md, meta) = convert("real.png").await;
    assert!(
        md.contains("real.png") || md.contains("byte"),
        "metadata markdown expected: {md}"
    );
    assert_eq!(meta["byte_count"], 2825);
}

// -- HTML (real macOS License.html) -------------------------------------------

#[tokio::test]
async fn real_html_extracts_license_body_without_css() {
    let (md, _) = convert("real.html").await;
    assert!(
        md.chars().count() > 1000,
        "html body too short: {}",
        md.len()
    );
    let lower = md.to_lowercase();
    assert!(
        lower.contains("licen") || lower.contains("apple"),
        "license vocabulary missing"
    );
    // Real HTML carries a <style> block whose CSS must not leak into prose.
    for css_marker in ["--color-link", "overflow-wrap", "{ --"] {
        assert!(
            !md.contains(css_marker),
            "CSS leaked into markdown ({css_marker}): {}",
            &md.chars().take(300).collect::<String>()
        );
    }
}

/// Whether a `soffice` binary is on PATH (legacy RTF needs it).
fn soffice_available() -> bool {
    std::env::var("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|d| {
                let c = d.join("soffice");
                c.is_file()
            })
        })
        .unwrap_or(false)
}

// -- RTF (real macOS License.rtf; legacy-office needs soffice) ----------------

#[cfg(feature = "libreoffice")]
#[tokio::test]
async fn real_rtf_via_legacy_office() {
    let path = real("real.rtf");
    let alch = AlchemizeBuilder::new().build();
    let result = alch.convert(&Input::Path(path)).await;
    if !soffice_available() {
        // No LibreOffice: the engine reports Unavailable and the chain has no
        // other rtf handler — the failure must be explicit (the registry
        // reports every attempt, including the unavailable reason).
        match result {
            Err(alchemize::AlchemizeError::AllBackendsFailed { ext, attempts }) => {
                assert_eq!(ext, "rtf");
                assert!(
                    attempts
                        .iter()
                        .any(|(name, why)| name == "legacy-office" && why.contains("soffice")),
                    "attempts: {attempts:?}"
                );
            }
            other => panic!("without soffice rtf must fail; got {other:?}"),
        }
        return;
    }
    let result = result.unwrap();
    assert!(result.markdown.chars().count() > 500, "rtf body too short");
    assert_eq!(result.metadata["engine"], "libreoffice");
    let lower = result.markdown.to_lowercase();
    assert!(
        lower.contains("licen") || lower.contains("apple"),
        "rtf vocabulary missing"
    );
}

#[cfg(not(feature = "libreoffice"))]
#[tokio::test]
async fn real_rtf_without_engine_reports_unsupported() {
    let path = real("real.rtf");
    let alch = AlchemizeBuilder::new().build();
    let result = alch.convert(&Input::Path(path)).await;
    assert!(
        matches!(result, Err(alchemize::AlchemizeError::UnsupportedFormat(_))),
        "default build has no rtf engine, expected UnsupportedFormat"
    );
}

// -- Rust source (real vecboost lib.rs, 792 lines) ----------------------------

#[tokio::test]
async fn real_rust_source_fenced_verbatim() {
    let (md, meta) = convert("real.rs").await;
    assert!(md.starts_with("````rust\n"), "missing rust fence");
    assert!(md.trim_end().ends_with("````"));
    assert!(md.contains("pub mod"), "source body missing");
    // Verbatim preservation: the fence body equals the file bytes.
    let original = std::fs::read_to_string(real("real.rs")).unwrap();
    assert_eq!(md, format!("````rust\n{original}\n````\n"));
    assert_eq!(meta["source_format"], "code");
}

// -- Markdown (real spec proposal, passthrough) -------------------------------

#[tokio::test]
async fn real_markdown_passthrough_identical() {
    let (md, _) = convert("real.md").await;
    let original = std::fs::read_to_string(real("real.md")).unwrap();
    assert_eq!(md, original, "markdown input must pass through unchanged");
    assert!(md.contains("## Motivation"));
}

// -- WAV (real 10s speech, vecboost asset) ------------------------------------

#[tokio::test]
async fn real_wav_metadata_mode_without_asr() {
    let (md, meta) = convert("real.wav").await;
    // No transcriber configured in the default build: metadata degradation.
    assert!(
        md.contains("audio") || md.contains("wav") || md.contains("SR"),
        "audio metadata markdown expected: {md}"
    );
    assert_eq!(meta["byte_count"], 320078);
}

// -- Duration/size sanity: WAV header parses to ~10 seconds ------------------

#[test]
fn real_wav_header_duration_sanity() {
    let bytes = std::fs::read(real("real.wav")).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    // Parse fmt chunk: mono, 16kHz, 16-bit → data length / 32000 = seconds.
    let data_pos = bytes
        .windows(4)
        .position(|w| w == b"data")
        .expect("data chunk");
    let data_len = u32::from_le_bytes([
        bytes[data_pos + 4],
        bytes[data_pos + 5],
        bytes[data_pos + 6],
        bytes[data_pos + 7],
    ]) as f64;
    let seconds = data_len / 32_000.0;
    assert!(
        (9.0..=11.0).contains(&seconds),
        "expected ~10s of speech, header says {seconds:.1}s"
    );
}

// -- MP4 (real AI-generated video, direct path from etyma) -------------------

#[tokio::test]
async fn real_mp4_degraded_or_extracted() {
    let path = std::path::Path::new(
        "/Volumes/ZHITAI-1T/projects/etyma/video-output/agnes_video_t2vid_1152x768_1784828436_067a6d97.mp4",
    );
    assert!(path.is_file(), "real mp4 missing at {}", path.display());
    let alch = AlchemizeBuilder::new().build();
    let result = alch.convert(&Input::Path(path.to_path_buf())).await;
    let result = result.expect("mp4 conversion must not error");
    // ffmpeg absent on this host → degraded metadata; ffmpeg present →
    // extraction attempted. Both paths must produce markdown.
    assert!(!result.markdown.is_empty());
    assert_eq!(result.content_type, alchemize::domain::ContentType::Video);
}

// -- detect_format on real archives (ZIP header-name scanner) -----------------

#[test]
fn detect_format_reads_real_archives_from_header_names() {
    use alchemize::domain::input::{InputFormat, detect_format};

    let cases = [
        ("real.docx", InputFormat::Docx),
        ("real.pptx", InputFormat::Pptx),
        // ODF family collapses to Odt (the format carries odt/ods/odp).
        ("real.odt", InputFormat::Odt),
        ("real.odp", InputFormat::Odt),
    ];
    for (name, expected) in cases {
        let bytes = std::fs::read(real(name)).unwrap();
        let input = Input::bytes(bytes, name.to_string());
        assert_eq!(detect_format(&input), Some(expected), "{name}");
    }

    for (dir, name, expected) in [
        ("epub", "sample.epub", InputFormat::Epub),
        ("xlsx", "sample.xlsx", InputFormat::Xlsx),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(dir)
            .join(name);
        let bytes = std::fs::read(&path).unwrap();
        let input = Input::bytes(bytes, name.to_string());
        assert_eq!(detect_format(&input), Some(expected), "{dir}/{name}");
    }
}

// -- containers feature: nested members dispatch through the assembled registry

#[cfg(feature = "containers")]
#[tokio::test]
async fn real_tar_nested_members_convert_through_engines() {
    // `real.tar` holds real.png + real.md + main.rs. With the registry slot
    // wired, every member must dispatch to an engine (image → metadata
    // markdown, text/code → content) instead of degrading to attachments.
    let path = real("real.tar");
    assert!(path.is_file(), "real fixture missing: {}", path.display());
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::Path(path))
        .await
        .expect("tar conversion must not error");
    let converted = result.metadata["converted_count"].as_u64().unwrap_or(0);
    let file_count = result.metadata["file_count"].as_u64().unwrap_or(0);
    assert_eq!(file_count, 3, "expected three members: {}", result.markdown);
    assert_eq!(
        converted, file_count,
        "every member must dispatch to an engine (no attachment fallback): {}",
        result.markdown
    );
    assert!(result.markdown.contains("real.png"), "{}", result.markdown);
}
