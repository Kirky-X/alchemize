// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Golden fixture tests: each format converts to its expected markdown.
#![cfg(feature = "full")]

use alchemize::application::builder::{Alchemize, AlchemizeBuilder};
use alchemize::domain::input::Input;

fn alch() -> Alchemize {
    AlchemizeBuilder::new().build()
}

fn fixture_dir(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Convert `sample.<ext>` and compare against `expected.md`.
///
/// `strip_metadata_tails`: the wav/mp4/png fixtures have no real decodable
/// payload, so those families assert metadata-mode output instead.
async fn golden(name: &str, ext: &str) -> String {
    let input = Input::Path(fixture_dir(name).join(format!("sample.{ext}")));
    alch()
        .convert(&input)
        .await
        .unwrap_or_else(|e| panic!("{name}: conversion failed: {e}"))
        .markdown
}

#[tokio::test]
async fn golden_txt() {
    assert_eq!(
        golden("txt", "txt").await,
        include_str!("fixtures/txt/expected.md")
    );
}

#[tokio::test]
async fn golden_md_passthrough() {
    assert_eq!(
        golden("md", "md").await,
        include_str!("fixtures/md/expected.md")
    );
}

#[tokio::test]
async fn golden_py_fenced() {
    assert_eq!(
        golden("py", "py").await,
        include_str!("fixtures/py/expected.md")
    );
}

#[tokio::test]
async fn golden_yaml_fenced() {
    assert_eq!(
        golden("yaml", "yaml").await,
        include_str!("fixtures/yaml/expected.md")
    );
}

#[tokio::test]
async fn golden_csv_table() {
    assert_eq!(
        golden("csv", "csv").await,
        include_str!("fixtures/csv/expected.md")
    );
}

#[tokio::test]
async fn golden_tsv_table() {
    assert_eq!(
        golden("tsv", "tsv").await,
        include_str!("fixtures/tsv/expected.md")
    );
}

#[tokio::test]
async fn golden_srt() {
    assert_eq!(
        golden("srt", "srt").await,
        include_str!("fixtures/srt/expected.md")
    );
}

#[tokio::test]
async fn golden_vtt() {
    assert_eq!(
        golden("vtt", "vtt").await,
        include_str!("fixtures/vtt/expected.md")
    );
}

#[tokio::test]
async fn golden_ass() {
    let md = golden("ass", "ass").await;
    assert!(md.contains("[00:01.000 - 00:02.000] Hello ASS"), "md: {md}");
}

#[tokio::test]
async fn golden_html_math_preserved() {
    let md = golden("html", "html").await;
    assert!(md.contains("$\\alpha+\\beta$"), "md: {md}");
    assert!(md.contains("$$c^2 = a^2 + b^2$$"), "md: {md}");
}

#[tokio::test]
async fn golden_ipynb() {
    let md = golden("ipynb", "ipynb").await;
    assert!(md.contains("# Title"));
    assert!(md.contains("```python\nx = 1\n```"), "md: {md}");
}

#[tokio::test]
async fn golden_zip_recursive() {
    let md = golden("zip", "zip").await;
    assert!(md.contains("### readme.txt"), "md: {md}");
    assert!(md.contains("Hello from ZIP"));
}

#[tokio::test]
async fn golden_epub_chapters() {
    let md = golden("epub", "epub").await;
    assert!(md.contains("Fixture Book"), "md: {md}");
    assert!(md.contains("Chapter One"), "md: {md}");
    assert!(md.contains("Epub body text."), "md: {md}");
}

#[tokio::test]
async fn golden_docx_text() {
    let md = golden("docx", "docx").await;
    assert!(md.contains("Hello DOCX"), "md: {md}");
}

#[tokio::test]
async fn golden_xlsx_disabled_by_default() {
    // Spreadsheet support was removed: no engine accepts xlsx/ods.
    let input = Input::Path(fixture_dir("xlsx").join("sample.xlsx"));
    let err = alch().convert(&input).await.unwrap_err();
    assert!(
        matches!(err, alchemize::AlchemizeError::UnsupportedFormat(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn golden_tex_env_aware() {
    let md = golden("tex", "tex").await;
    if which_pandoc() {
        assert!(md.contains("Hello"), "pandoc path md: {md}");
    } else {
        assert!(md.starts_with("```latex"), "fallback path md: {md}");
    }
}

#[tokio::test]
async fn golden_wav_metadata_degradation() {
    // Zero-sample wav: no transcriber configured → metadata markdown, not error.
    let md = golden("wav", "wav").await;
    assert!(!md.is_empty());
}

#[tokio::test]
async fn golden_mp4_metadata_degradation() {
    // Header-only mp4: no ffmpeg path taken → metadata markdown, not error.
    let md = golden("mp4", "mp4").await;
    assert!(!md.is_empty());
}

#[tokio::test]
async fn golden_png_metadata_degradation() {
    // Truncated png without vision provider: metadata markdown, not error.
    let md = golden("png", "png").await;
    assert!(!md.is_empty());
}

fn which_pandoc() -> bool {
    std::env::var("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("pandoc").is_file()))
        .unwrap_or(false)
}

/// proptest: arbitrary bytes never panic the magic detector.
#[cfg(test)]
mod proptests {
    use alchemize::domain::input::{Input, detect_format};
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn detect_never_panics(data in proptest::collection::vec(any::<u8>(), 0..64)) {
            let input = Input::Bytes {
                data,
                filename: "fuzz.bin".to_string(),
                mimetype: None,
                extension: None,
            };
            let _ = detect_format(&input);
        }
    }
}
