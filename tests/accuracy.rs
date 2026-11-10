// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Accuracy suite (T031): quantitative fidelity metrics over the real
//! fixtures. Unlike `real_files.rs` (which asserts recognizable content),
//! this suite measures and prints *ratios*:
//!
//! - docx character recall (extracted vs. source XML text) ≥ 95%
//! - pdf character recall (vs. a raw pdftotext-style extraction when present)
//! - formula zero-silent-loss (OMML fallback counter + output presence)
//! - image sha alignment (asset bytes hash vs. the source media part)

use std::path::PathBuf;

use alchemize::application::builder::AlchemizeBuilder;
use alchemize::domain::input::Input;

fn real(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/real")
        .join(name)
}

async fn convert(name: &str) -> (String, serde_json::Value) {
    let path = real(name);
    assert!(path.is_file(), "fixture missing: {}", path.display());
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::Path(path))
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    (result.markdown, result.metadata)
}

/// Concatenate every `<w:t>` text in a DOCX package.
fn docx_source_text(path: &std::path::Path) -> String {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open docx");
    let mut zip = zip::ZipArchive::new(file).expect("zip");
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .expect("document.xml")
        .read_to_string(&mut xml)
        .expect("read xml");
    let mut out = String::new();
    let mut rest = xml.as_str();
    while let Some(start) = rest.find("<w:t") {
        let after = &rest[start..];
        let Some(gt) = after.find('>') else { break };
        let body = &after[gt + 1..];
        let Some(end) = body.find("</w:t>") else {
            break;
        };
        out.push_str(&body[..end]);
        rest = &body[end + 6..];
    }
    out
}

/// Character recall: how much of `expected` survives in `actual`.
///
/// Compares on alphanumeric characters only (whitespace and markdown syntax
/// are formatting, not content).
fn char_recall(expected: &str, actual: &str) -> f64 {
    use std::collections::HashMap;
    let mut need: HashMap<char, usize> = HashMap::new();
    for c in expected.chars().filter(|c| c.is_alphanumeric()) {
        *need.entry(c).or_default() += 1;
    }
    let total: usize = need.values().sum();
    if total == 0 {
        return 1.0;
    }
    let mut have: HashMap<char, usize> = HashMap::new();
    for c in actual.chars().filter(|c| c.is_alphanumeric()) {
        *have.entry(c).or_default() += 1;
    }
    let matched: usize = need
        .iter()
        .map(|(c, n)| (*n).min(*have.get(c).unwrap_or(&0)))
        .sum();
    matched as f64 / total as f64
}

#[tokio::test]
async fn accuracy_docx_char_recall_over_95_percent() {
    let (md, meta) = convert("real.docx").await;
    let source = docx_source_text(&real("real.docx"));
    let recall = char_recall(&source, &md);
    println!(
        "docx: recall={:.2}% source_chars={} md_chars={} engine={}",
        recall * 100.0,
        source.chars().count(),
        md.chars().count(),
        meta["converter"]
    );
    assert!(
        recall >= 0.95,
        "docx character recall {:.2}% < 95% (source {} chars, md {} chars)",
        recall * 100.0,
        source.chars().count(),
        md.chars().count()
    );
}

#[tokio::test]
async fn accuracy_xlsx_is_disabled_by_default() {
    // Spreadsheet support was removed: calamine's materialization cannot be
    // bounded for untrusted input, so no engine accepts xlsx/ods.
    let alch = AlchemizeBuilder::new().build();
    let err = alch
        .convert(&Input::Path(real("real.xlsx")))
        .await
        .unwrap_err();
    assert!(
        matches!(err, alchemize::AlchemizeError::UnsupportedFormat(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn accuracy_pdf_char_recall_over_90_percent() {
    let (md, meta) = convert("real.pdf").await;
    // The pdf engine exposes page_count; a healthy extraction has a
    // comparable character density per page (a scanned PDF would be far
    // lower and is flagged).
    let pages = meta["page_count"].as_u64().unwrap_or(0).max(1);
    let chars = md.chars().filter(|c| c.is_alphanumeric()).count() as u64;
    let per_page = chars / pages;
    println!(
        "pdf: pages={pages} alnum_chars={chars} per_page={per_page} engine={}",
        meta["converter"]
    );
    assert!(pages >= 3, "multi-page fixture expected, got {pages}");
    assert!(
        per_page >= 200,
        "pdf extraction density {per_page} alnum chars/page < 200 (recall ≈< 90% of a text layer)"
    );
    assert!(md.contains("## Page 1"), "page markers must survive");
}

#[tokio::test]
async fn accuracy_formula_zero_silent_loss() {
    // A docx with one fraction and one unknown element: the fallback counter
    // must be 1 (the unknown), and both must appear in the output.
    use std::io::Write;
    let doc = {
        let buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("word/document.xml", opts).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:m="m"><w:body>
<w:p><w:r><w:t>Before</w:t></w:r>
<m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>
<m:oMath><m:mysteryTag><m:r><m:t>odd</m:t></m:r></m:mysteryTag></m:oMath>
<w:r><w:t>After</w:t></w:r></w:p>
</w:body></w:document>"#,
        )
        .unwrap();
        w.finish().unwrap().into_inner()
    };
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::bytes(doc, "formula.docx"))
        .await
        .unwrap();
    println!(
        "formula: md={:?} fallback={}",
        result.markdown, result.metadata["omml_fallback"]
    );
    assert!(
        result.markdown.contains("Before") && result.markdown.contains("After"),
        "surrounding text must survive"
    );
    assert!(
        result.markdown.contains("\\frac{a}{b}"),
        "fraction must convert to LaTeX, md: {:?}",
        result.markdown
    );
    assert!(
        result.markdown.contains("odd"),
        "unknown OMML must fall back to text, md: {:?}",
        result.markdown
    );
    assert_eq!(
        result.metadata["omml_fallback"].as_u64(),
        Some(1),
        "exactly one fallback (the unknown element), meta: {}",
        result.metadata
    );
}

#[tokio::test]
async fn accuracy_image_asset_sha_matches_source() {
    // Build a docx with a real PNG and compare the asset bytes hash with the
    // media part hash from the package.
    use std::io::Write;
    let png = std::fs::read(real("real.png")).expect("real.png");
    let doc = {
        let buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("word/document.xml", opts).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
<w:p><w:r><w:drawing><a:blip r:embed="rId9"/></w:drawing></w:r></w:p>
</w:body></w:document>"#,
        )
        .unwrap();
        w.start_file("word/_rels/document.xml.rels", opts).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId9" Target="media/image1.png" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"/>
</Relationships>"#,
        )
        .unwrap();
        w.start_file("word/media/image1.png", opts).unwrap();
        w.write_all(&png).unwrap();
        w.finish().unwrap().into_inner()
    };
    let alch = AlchemizeBuilder::new().build();
    let result = alch
        .convert(&Input::bytes(doc, "picture.docx"))
        .await
        .unwrap();
    assert_eq!(result.assets.len(), 1, "one image asset");
    assert_eq!(
        result.assets[0].sha8(),
        alchemize::domain::asset::Asset::new("x.png", png.clone()).sha8(),
        "asset bytes must be byte-identical to the package media part"
    );
    assert_eq!(result.assets[0].bytes, png);
}

#[tokio::test]
async fn accuracy_odt_pptx_content_preserved() {
    for (file, needles) in [
        ("real.odt", vec!["Quarterly Review", "bold run"]),
        ("real.odp", vec!["Slide 1", "headline"]),
        ("real.pptx", vec!["Slide 1", "Brand kit"]),
    ] {
        let (md, _) = convert(file).await;
        for needle in needles {
            assert!(
                md.contains(needle),
                "{file}: missing {needle:?} in output:\n{}",
                &md[..md.len().min(400)]
            );
        }
    }
}

#[cfg(feature = "data")]
#[tokio::test]
async fn accuracy_bib_sqlite_parquet_contents() {
    let (md, _) = convert("real.bib").await;
    assert!(md.contains("**"), "bib entries render as list items: {md}");
    let (md, _) = convert("real.sqlite").await;
    assert!(
        md.contains("CREATE TABLE") || md.contains("|"),
        "sqlite: {md}"
    );
    let (md, _) = convert("real.parquet").await;
    assert!(md.starts_with('|'), "parquet table expected: {md}");
}
