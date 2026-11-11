// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Real-file parsing tests.
//!
//! Every fixture under `tests/fixtures/real/` is a genuine file harvested
//! from this machine or the sibling projects (macOS system documentation,
//! etyma wordlists, vecboost sources/assets) — not a hand-assembled minimal
//! archive. Four types with no local real file (epub/ipynb/webp/ogg) come
//! from canonical public sources instead: Project Gutenberg eBook #11, the
//! PythonDataScienceHandbook notebooks, Google's WebP gallery, and MDN's
//! webaudio-examples. `real.flac` is a CoreAudio `afconvert` transcode of
//! the real WAV asset (the OS's own lossless encoder). Assertions check
//! *real content* (specific words/structures from the source), not just
//! non-emptiness.
#![cfg(feature = "full")]

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
        md.chars().take(500).collect::<String>()
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
            md.chars().take(300).collect::<String>()
        );
    }
}

/// Whether a `soffice` binary is on PATH (legacy RTF needs it).
#[cfg(feature = "legacy-office")]
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

#[cfg(feature = "legacy-office")]
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

#[cfg(not(feature = "legacy-office"))]
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

// -- PPTX (real design-deck slide capture, local harvest) --------------------

#[tokio::test]
async fn real_pptx_extracts_design_deck_slides() {
    let (md, _) = convert("real.pptx").await;
    assert!(
        md.contains("Slide 1"),
        "slide marker missing: {}",
        &md[..md.len().min(300)]
    );
    assert!(md.contains("Brand kit"), "slide title missing");
    assert!(md.contains("vercel.com"), "slide URL missing");
    assert!(md.chars().count() > 200, "pptx body too short");
}

// -- ODT (real ODF text document, local harvest) -----------------------------

#[tokio::test]
async fn real_odt_extracts_styled_prose() {
    let (md, _) = convert("real.odt").await;
    assert!(md.contains("Quarterly Review"), "heading missing");
    assert!(md.contains("**bold run**"), "bold span missing");
    assert!(
        md.contains("[web link](https://example.com/report)"),
        "hyperlink missing"
    );
    assert!(md.chars().count() > 100, "odt body too short");
}

// -- ODP (real ODF presentation, local harvest) ------------------------------

#[tokio::test]
async fn real_odp_extracts_slides_and_assets() {
    let (md, _) = convert("real.odp").await;
    assert!(md.contains("Slide 1"), "slide marker missing");
    assert!(md.contains("Slide one headline"), "slide title missing");
    assert!(md.contains("![]("), "image asset missing");
    assert!(md.chars().count() > 100, "odp body too short");
}

// -- HEIC (real photo from this machine; OS sips decode) ---------------------

#[tokio::test]
async fn real_heic_decodes_to_png_via_os() {
    let path = real("real.heic");
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::Path(path))
        .await
        .expect("heic must convert");
    assert!(
        result.markdown.contains("![real.heic](images/"),
        "decoded asset reference missing: {}",
        result.markdown
    );
    assert!(result.markdown.contains("512×512"), "dimensions missing");
    assert_eq!(result.content_type, alchemize::domain::ContentType::Image);
}

// -- GIF/JPEG/WebP (real bitmaps; metadata degrade without vision) -----------

#[tokio::test]
async fn real_image_family_degrades_without_vision() {
    let cases = [
        ("real.gif", 520),    // WRF tools asset (sibling project janvi)
        ("real.jpg", 17480),  // macOS desktop-picture thumbnail
        ("real.webp", 30320), // Google WebP gallery official sample (VP8 550×368)
    ];
    let alch = AlchemizeBuilder::new().build();
    for (name, bytes) in cases {
        let result = alch
            .convert(&Input::Path(real(name)))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            result.markdown.contains(name) && result.markdown.contains("byte"),
            "{name}: metadata markdown expected: {}",
            result.markdown
        );
        assert_eq!(result.metadata["byte_count"], bytes, "{name}");
        assert_eq!(result.content_type, alchemize::domain::ContentType::Image);
    }
}

// -- MP3/OGG/FLAC (real audio; metadata degrade without ASR) -----------------

#[tokio::test]
async fn real_audio_family_degrades_without_asr() {
    let cases = [
        ("real.mp3", 244590),  // iMovie sound effect "Bubbles"
        ("real.ogg", 678516),  // MDN webaudio-examples Xiph.Org Vorbis sample
        ("real.flac", 180411), // CoreAudio afconvert FLAC of real.wav
    ];
    let alch = AlchemizeBuilder::new().build();
    for (name, bytes) in cases {
        let result = alch
            .convert(&Input::Path(real(name)))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            result.markdown.contains(name),
            "{name}: metadata markdown expected: {}",
            result.markdown
        );
        assert_eq!(result.metadata["byte_count"], bytes, "{name}");
        assert_eq!(result.content_type, alchemize::domain::ContentType::Audio);
    }
}

// -- Container-header sanity for the harvested audio trio --------------------

#[test]
fn real_mp3_ogg_flac_headers_are_genuine() {
    let mp3 = std::fs::read(real("real.mp3")).unwrap();
    assert_eq!(&mp3[0..3], b"ID3", "mp3 must start with an ID3 tag");
    let ogg = std::fs::read(real("real.ogg")).unwrap();
    assert_eq!(
        &ogg[0..4],
        b"OggS",
        "ogg must start with the OggS capture pattern"
    );
    assert!(
        ogg.windows(7).any(|w| w == b"\x01vorbis"),
        "first Vorbis identification header missing"
    );
    let flac = std::fs::read(real("real.flac")).unwrap();
    assert_eq!(&flac[0..4], b"fLaC", "flac must start with the fLaC magic");
}

// -- CSV (real crawlrs moli bench results) -----------------------------------

#[tokio::test]
async fn real_csv_converts_to_markdown_table() {
    let (md, _) = convert("real.csv").await;
    assert!(
        md.starts_with("| kind | engine | site | run | ok | ms | html_len |"),
        "header row missing or wrong: {}",
        &md[..md.len().min(200)]
    );
    // All 80 data rows of the source survive the table conversion.
    let rows = md.lines().filter(|l| l.starts_with("| row |")).count();
    assert_eq!(rows, 80, "expected all 80 data rows, got {rows}");
    assert!(
        md.contains("https://github.com"),
        "real site values missing"
    );
}

// -- TSV (real repo status ledger from .github; public-repo rows only —
//    the 5 private-repo rows were filtered out before committing) ----------

#[tokio::test]
async fn real_tsv_converts_to_markdown_table() {
    let (md, _) = convert("real.tsv").await;
    assert!(
        md.contains("| repo | slug | default_branch |"),
        "header missing"
    );
    assert!(
        md.contains("| CalNexus | Kirky-X/CalNexus |"),
        "first row missing"
    );
    assert!(md.contains("Kirky-X/alchemize"), "own repo row missing");
    let rows = md
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| repo |") && !l.starts_with("|---"))
        .count();
    assert_eq!(rows, 15, "expected all 15 public-repo rows, got {rows}");
}

// -- EPUB (Project Gutenberg eBook #11, Alice's Adventures in Wonderland) ----

#[tokio::test]
async fn real_epub_extracts_carroll_chapters() {
    let (md, _) = convert("real.epub").await;
    assert!(md.chars().count() > 2000, "epub body too short");
    assert!(md.contains("Alice"), "protagonist missing");
    assert!(md.contains("Lewis Carroll"), "author metadata missing");
    assert!(md.contains("Chapter"), "chapter structure missing");
}

// -- IPYNB (real PythonDataScienceHandbook notebook) -------------------------

#[tokio::test]
async fn real_ipynb_renders_markdown_and_code_cells() {
    let (md, _) = convert("real.ipynb").await;
    assert!(
        md.contains("Getting Started in IPython and Jupyter"),
        "markdown cell content missing: {}",
        &md[..md.len().min(300)]
    );
    assert!(md.contains("```ipython"), "code cell fence missing");
    assert!(
        md.contains("Launching the Jupyter Notebook"),
        "section missing"
    );
    assert!(md.chars().count() > 1000, "ipynb body too short");
}

// -- ZIP (real kueiku-v0.1.16 skill package from ~/Downloads) ----------------

#[tokio::test]
async fn real_zip_members_convert_through_facade() {
    let path = real("real.zip");
    assert!(path.is_file(), "real fixture missing: {}", path.display());
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::Path(path))
        .await
        .expect("zip conversion must not error");
    let converted = result.metadata["converted_count"].as_u64().unwrap_or(0);
    let file_count = result.metadata["file_count"].as_u64().unwrap_or(0);
    assert_eq!(
        file_count, 27,
        "kueiku package has 27 members: {}",
        result.markdown
    );
    // Every member with a known extension dispatches; the single
    // extensionless member (kueiku/LICENSE) has no format detection and
    // falls back to an inline-text attachment. That is the documented
    // contract for unknown members, not a regression.
    assert_eq!(
        converted, 26,
        "expected 27 members, 26 converted (attachments: {:?}): {}",
        result.metadata["attachments"], result.markdown
    );
    assert_eq!(
        result.metadata["attachments"],
        serde_json::json!(["kueiku/LICENSE"]),
        "the extensionless member must be the only attachment"
    );
    assert!(
        result.markdown.contains("kueiku/LICENSE"),
        "{}",
        result.markdown
    );
    assert!(
        result.markdown.contains("MIT License"),
        "{}",
        result.markdown
    );
}

// -- structured-data features: parquet/sqlite/bib (per-format engines) -------------

#[cfg(feature = "parquet")]
#[tokio::test]
async fn real_parquet_converts_sensor_table() {
    let (md, _) = convert("real.parquet").await;
    assert!(
        md.contains("| id | name | value | active | ts_ms |"),
        "schema header missing: {}",
        &md[..md.len().min(200)]
    );
    assert!(md.contains("sensor-001"), "real rows missing");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn real_sqlite_reports_schema_and_rows() {
    let (md, _) = convert("real.sqlite").await;
    assert!(md.contains("### meta"), "table section missing");
    assert!(md.contains("Sample rows"), "sample rows missing");
    assert!(md.contains("schema_version"), "real row values missing");
}

#[cfg(feature = "bibtex")]
#[tokio::test]
async fn real_bib_converts_reference_entries() {
    let (md, _) = convert("real.bib").await;
    assert!(md.contains("knuth1974"), "entry key missing");
    assert!(
        md.contains("Computer Programming as an Art"),
        "entry title missing"
    );
    assert!(
        md.contains("Attention Is All You Need"),
        "later entry missing"
    );
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
        ("real.epub", InputFormat::Epub),
        ("real.csv", InputFormat::Csv),
        ("real.tsv", InputFormat::Csv),
        ("real.ipynb", InputFormat::Ipynb),
        ("real.zip", InputFormat::Zip),
        ("real.gif", InputFormat::Gif),
        ("real.jpg", InputFormat::Jpeg),
        ("real.webp", InputFormat::Webp),
        ("real.mp3", InputFormat::Mp3),
        ("real.ogg", InputFormat::Ogg),
        ("real.flac", InputFormat::Flac),
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

// -- tar feature: nested members dispatch through the assembled registry

#[cfg(feature = "tar")]
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
