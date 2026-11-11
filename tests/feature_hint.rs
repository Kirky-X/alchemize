// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Feature-hint diagnostics: a format this build did not compile in must
//! say which feature turns it on, instead of leaving the user guessing.
//!
//! Gated on `cli + text` with `pdf` absent: that is the only build state in
//! which the hint path is reachable *and* the `alch` binary exists. CI runs
//! this file explicitly (`cargo test --test feature_hint --no-default-features
//! --features "cli,text"`) and asserts it is non-empty — a silent
//! `0 tests` pass would disable the regression.

#![cfg(all(feature = "cli", feature = "text", not(feature = "pdf")))]

use alchemize::application::builder::AlchemizeBuilder;
use alchemize::domain::error::AlchemizeError;
use alchemize::domain::feature_map::{enabled_features, features_for_extension, missing_features};
use alchemize::domain::input::Input;

/// Library surface: the table names the capability feature that would
/// provide the extension.
#[test]
fn hint_names_the_missing_capability_feature() {
    assert_eq!(features_for_extension("pdf"), Some(&["pdf"][..]));
    let missing = missing_features("pdf").expect("pdf feature is off in this build");
    assert_eq!(missing, vec!["pdf"]);
    // Formats without a capability feature get no hint at all.
    assert!(missing_features("xyz").is_none());
    assert!(features_for_extension("xlsx").is_none());
}

/// The build reports its compiled-in capabilities honestly.
#[test]
fn enabled_features_reflect_this_build() {
    let enabled = enabled_features();
    assert_eq!(enabled.contains(&"text"), cfg!(feature = "text"));
    assert_eq!(enabled.contains(&"pdf"), cfg!(feature = "pdf"));
}

/// CLI: converting a pdf in a build without the pdf feature reports the
/// missing feature on stderr and keeps exit code 2 (unsupported format).
///
/// Locale-independent: the expected strings are rendered through the same
/// catalog the binary uses, under the locale the child process will detect
/// (the parent's environment), instead of pinning English.
#[test]
fn cli_unsupported_format_names_the_missing_feature() {
    let lang = alchemize::i18n::locale_of_current_process();
    let tr = |key: &str, args: &[(&str, String)]| alchemize::i18n::translate_for(&lang, key, args);
    let stderr = run_alch_on("doc.pdf", b"%PDF-1.7\n");
    let expected = tr("unsupported-format", &[("ext", "pdf".into())]);
    assert!(
        stderr.contains(&expected),
        "stderr must name the format: {stderr}"
    );
    let expected = tr(
        "unsupported-format-requires-feature",
        &[
            ("ext", "pdf".into()),
            ("names", "'pdf'".into()),
            ("plural", String::new()),
            ("them", "it".into()),
            ("flags", "pdf".into()),
        ],
    );
    assert!(
        stderr.contains(&expected),
        "stderr must name the missing feature: {stderr}"
    );
    assert!(
        stderr.contains("--features pdf"),
        "stderr must name the rebuild flag: {stderr}"
    );
}

/// Extensions come from user-supplied filenames: control characters and
/// Unicode bidi controls must not reach the terminal (CWE-150).
#[test]
fn cli_sanitizes_control_and_bidi_characters_in_the_extension() {
    for (name, forbidden) in [
        ("evil.\u{1b}[31mRED\nFAKE", '\u{1b}'),
        ("evil.\u{202e}pdf", '\u{202e}'),
        ("evil.\u{200e}pdf", '\u{200e}'),
        ("evil.\u{061c}pdf", '\u{061c}'),
    ] {
        let stderr = run_alch_on(name, b"irrelevant");
        assert!(
            !stderr.contains(forbidden),
            "raw {forbidden:?} reached stderr for {name:?}: {stderr:?}"
        );
        // Locale-independent prefix of the diagnostic line.
        let lang = alchemize::i18n::locale_of_current_process();
        let expected = alchemize::i18n::translate_for(&lang, "unsupported-format", &[])
            .split(['：', ':'])
            .next()
            .unwrap_or("unsupported format")
            .to_string();
        assert!(
            stderr.contains(&expected),
            "diagnostic line missing for {name:?}: {stderr}"
        );
    }
}

/// Library: the error stays `UnsupportedFormat` (public API shape unchanged);
/// only the CLI diagnostic carries the hint.
#[tokio::test]
async fn library_error_shape_unchanged() {
    let alch = AlchemizeBuilder::new().build();
    let err = alch
        .convert(&Input::bytes(b"%PDF-1.7\n".to_vec(), "doc.pdf"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, AlchemizeError::UnsupportedFormat(ref ext) if ext == "pdf"),
        "unexpected error: {err:?}"
    );
}

/// Run `alch convert <file>` on a temporary file and return its stderr.
fn run_alch_on(filename: &str, bytes: &[u8]) -> String {
    let exe = env!("CARGO_BIN_EXE_alch");
    let dir = std::env::temp_dir().join(format!(
        "alchemize-hint-{}-{}",
        std::process::id(),
        filename.len()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let fixture = dir.join(filename);
    std::fs::write(&fixture, bytes).expect("write fixture");

    let out = std::process::Command::new(exe)
        .arg("convert")
        .arg(&fixture)
        .output()
        .expect("run alch");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    assert_eq!(
        out.status.code(),
        Some(2),
        "unsupported formats exit 2; stderr: {stderr}"
    );
    assert!(
        out.stdout.is_empty(),
        "diagnostics must not touch stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    let _ = std::fs::remove_dir_all(&dir);
    stderr
}
