// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Extension → capability-feature map (data only).
//!
//! This module holds *facts*, not user-facing text: which capability
//! features a conversion of a given extension requires. Presentation (the
//! CLI hint, language, flags) belongs to the caller.
//!
//! # Single source of truth
//! [`EXTENSION_FEATURES`] is the only table. It is kept honest by three
//! machine checks in `engines.rs`:
//! 1. **binding** (every build): for each entry whose feature is enabled in
//!    this build, a registered converter must accept the extension — so a
//!    wrong feature attribution fails the test in that feature's own cell;
//! 2. **coverage** (`full`): every extension some converter declares must
//!    appear in the table (plus the facade-level `zip`/`cbz` check);
//! 3. **feature-set** (`full`): the table's feature names and
//!    [`enabled_features`] must equal the `full` array parsed from
//!    `Cargo.toml`, so a forgotten entry or a typo cannot ship silently.

/// `(extensions, required capability features)` table.
///
/// Almost every format needs exactly one feature; combined formats need
/// several — `.tgz`/`.tar.gz` need both `tar` and `gzip`. Only extensions a
/// converter (or the facade's zip converter) really declares appear here:
/// alias spellings the engines do not accept (`docm`, `msg`, `xhtml`, …)
/// are deliberately absent, because a hint that does not actually fix the
/// conversion is worse than no hint.
pub const EXTENSION_FEATURES: &[(&str, &[&str])] = &[
    // Text family (text feature; default build)
    ("txt", &["text"]),
    ("text", &["text"]),
    ("log", &["text"]),
    ("md", &["text"]),
    ("markdown", &["text"]),
    ("rmd", &["text"]),
    ("qmd", &["text"]),
    ("org", &["text"]),
    ("rst", &["text"]),
    // Code family (same text converter)
    ("py", &["text"]),
    ("pyi", &["text"]),
    ("rs", &["text"]),
    ("js", &["text"]),
    ("mjs", &["text"]),
    ("cjs", &["text"]),
    ("jsx", &["text"]),
    ("ts", &["text"]),
    ("tsx", &["text"]),
    ("go", &["text"]),
    ("java", &["text"]),
    ("kt", &["text"]),
    ("kts", &["text"]),
    ("swift", &["text"]),
    ("c", &["text"]),
    ("h", &["text"]),
    ("cpp", &["text"]),
    ("cc", &["text"]),
    ("cxx", &["text"]),
    ("hpp", &["text"]),
    ("cs", &["text"]),
    ("rb", &["text"]),
    ("php", &["text"]),
    ("lua", &["text"]),
    ("r", &["text"]),
    ("sql", &["text"]),
    ("sh", &["text"]),
    ("bash", &["text"]),
    ("zsh", &["text"]),
    ("fish", &["text"]),
    ("json", &["text"]),
    ("jsonl", &["text"]),
    ("ndjson", &["text"]),
    ("yaml", &["text"]),
    ("yml", &["text"]),
    ("toml", &["text"]),
    ("xml", &["text"]),
    ("ini", &["text"]),
    ("cfg", &["text"]),
    ("conf", &["text"]),
    ("proto", &["text"]),
    ("graphql", &["text"]),
    ("gql", &["text"]),
    ("css", &["text"]),
    ("scss", &["text"]),
    ("env", &["text"]),
    ("properties", &["text"]),
    ("mmd", &["text"]),
    // Delimited tables
    ("csv", &["csv"]),
    ("tsv", &["csv"]),
    ("psv", &["csv"]),
    // Hypertext
    ("html", &["html"]),
    ("htm", &["html"]),
    // PDF
    ("pdf", &["pdf"]),
    // OOXML / ODF / e-book / notebook
    ("docx", &["docx"]),
    ("pptx", &["pptx"]),
    ("odt", &["odt"]),
    ("odp", &["odp"]),
    ("epub", &["epub"]),
    ("ipynb", &["ipynb"]),
    // zip container (facade-level converter)
    ("zip", &["zip"]),
    ("cbz", &["zip"]),
    // LaTeX source
    ("tex", &["latex"]),
    ("latex", &["latex"]),
    // pandoc-backed formats (pandoc feature)
    ("adoc", &["pandoc"]),
    ("asciidoc", &["pandoc"]),
    ("opml", &["pandoc"]),
    ("mediawiki", &["pandoc"]),
    ("wiki", &["pandoc"]),
    ("textile", &["pandoc"]),
    // Media family
    ("png", &["media"]),
    ("jpg", &["media"]),
    ("jpeg", &["media"]),
    ("gif", &["media"]),
    ("bmp", &["media"]),
    ("webp", &["media"]),
    ("tiff", &["media"]),
    ("tif", &["media"]),
    ("svg", &["media"]),
    ("wav", &["media"]),
    ("mp3", &["media"]),
    ("ogg", &["media"]),
    ("flac", &["media"]),
    ("aac", &["media"]),
    ("m4a", &["media"]),
    ("wma", &["media"]),
    ("opus", &["media"]),
    ("mp4", &["media"]),
    ("avi", &["media"]),
    ("mkv", &["media"]),
    ("mov", &["media"]),
    ("wmv", &["media"]),
    ("flv", &["media"]),
    ("webm", &["media"]),
    ("m4v", &["media"]),
    ("srt", &["media"]),
    ("vtt", &["media"]),
    ("ass", &["media"]),
    ("ssa", &["media"]),
    ("lrc", &["media"]),
    // HEIC/HEIF
    ("heic", &["heic"]),
    ("heif", &["heic"]),
    // Structured data
    ("parquet", &["parquet"]),
    ("sqlite", &["sqlite"]),
    ("db", &["sqlite"]),
    ("sqlite3", &["sqlite"]),
    ("bib", &["bibtex"]),
    // Containers; `.tgz`/`.tar.gz` need the extractor and the decoder
    ("tar", &["tar"]),
    ("tgz", &["tar", "gzip"]),
    ("tar.gz", &["tar", "gzip"]),
    ("gz", &["gzip"]),
    ("xz", &["xz"]),
    ("zst", &["zstd"]),
    ("7z", &["sevenz"]),
    // Legacy binary Office (soffice)
    ("doc", &["legacy-office"]),
    ("rtf", &["legacy-office"]),
    ("ppt", &["legacy-office"]),
    ("pps", &["legacy-office"]),
    ("pot", &["legacy-office"]),
    // E-mail
    ("eml", &["email"]),
];

/// Capability features a conversion of `ext` requires, if known.
pub fn features_for_extension(ext: &str) -> Option<&'static [&'static str]> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    EXTENSION_FEATURES
        .iter()
        .find(|(candidate, _)| *candidate == ext)
        .map(|(_, features)| *features)
}

/// Capability features compiled into this build.
///
/// Compile-time truth (each entry is behind its own `cfg`), so a build that
/// lacks a feature reports it honestly instead of probing the environment.
pub fn enabled_features() -> Vec<&'static str> {
    let mut features = Vec::new();
    macro_rules! collect {
        ($($feature:literal),+ $(,)?) => {
            $(
                #[cfg(feature = $feature)]
                features.push($feature);
            )+
        };
    }
    collect!(
        "text",
        "csv",
        "html",
        "pdf",
        "pdf-local",
        "pdf-cloud",
        "docx",
        "pptx",
        "odt",
        "odp",
        "epub",
        "ipynb",
        "zip",
        "latex",
        "pandoc",
        "media",
        "heic",
        "audio",
        "legacy-office",
        "email",
        "parquet",
        "sqlite",
        "bibtex",
        "tar",
        "gzip",
        "xz",
        "zstd",
        "sevenz",
    );
    features
}

/// Capability features `ext` needs that this build did not compile in.
///
/// `None` when the extension has no capability feature (genuinely
/// unsupported format) or when every required feature is already enabled
/// (the failure is not about a missing build option).
pub fn missing_features(ext: &str) -> Option<Vec<&'static str>> {
    let required = features_for_extension(ext)?;
    let enabled = enabled_features();
    let missing: Vec<&'static str> = required
        .iter()
        .copied()
        .filter(|f| !enabled.contains(f))
        .collect();
    (!missing.is_empty()).then_some(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_engine_declared_extensions_to_features() {
        assert_eq!(features_for_extension("pdf"), Some(&["pdf"][..]));
        assert_eq!(features_for_extension("DOCX"), Some(&["docx"][..]));
        assert_eq!(features_for_extension(".html"), Some(&["html"][..]));
        assert_eq!(features_for_extension("srt"), Some(&["media"][..]));
        assert_eq!(features_for_extension("7z"), Some(&["sevenz"][..]));
        assert_eq!(features_for_extension("eml"), Some(&["email"][..]));
        assert_eq!(features_for_extension("tgz"), Some(&["tar", "gzip"][..]));
        assert_eq!(features_for_extension("tar.gz"), Some(&["tar", "gzip"][..]));
    }

    /// Alias spellings the engines do not declare must NOT get a hint —
    /// otherwise the user rebuilds and still fails.
    #[test]
    fn undeclared_alias_spellings_get_no_hint() {
        for alias in [
            "docm", "dotx", "dotm", "pptm", "potx", "potm", "xhtml", "ltx", "msg", "pq", "bibtex",
            "sub", "gzip", "zstd", "xyz", "xlsx",
        ] {
            assert_eq!(features_for_extension(alias), None, "{alias}");
        }
    }

    #[test]
    fn enabled_features_matches_the_compiled_set() {
        let enabled = enabled_features();
        assert_eq!(enabled.contains(&"text"), cfg!(feature = "text"));
        assert_eq!(enabled.contains(&"pdf"), cfg!(feature = "pdf"));
        assert_eq!(enabled.contains(&"media"), cfg!(feature = "media"));
    }

    #[test]
    fn missing_features_names_only_absent_ones() {
        assert_eq!(missing_features("xyz"), None);
        #[cfg(not(feature = "pdf"))]
        assert_eq!(missing_features("pdf"), Some(vec!["pdf"]));
        #[cfg(feature = "pdf")]
        assert_eq!(missing_features("pdf"), None);
        // `.tgz` always requires both, independent of which is enabled.
        let tgz = missing_features("tgz");
        match (cfg!(feature = "tar"), cfg!(feature = "gzip")) {
            (true, true) => assert_eq!(tgz, None),
            (true, false) => assert_eq!(tgz, Some(vec!["gzip"])),
            (false, true) => assert_eq!(tgz, Some(vec!["tar"])),
            (false, false) => assert_eq!(tgz, Some(vec!["tar", "gzip"])),
        }
    }
}
