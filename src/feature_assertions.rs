// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Compile-time guards for the feature surface.
//!
//! `full` must contain every conversion feature. A missing member would
//! otherwise stay invisible: under `--features full` the feature-gated
//! registration assertions in `engines.rs` compile the "engine must exist"
//! branch in and the "must be absent" branch out, so a forgotten `full`
//! member silently ships a truncated umbrella. These guards turn that into
//! a compile error. New format features must be added here (sync point 7 in
//! the Cargo.toml feature comment).

macro_rules! full_membership_guards {
    ($($feature:literal),+ $(,)?) => {
        $(
            #[cfg(all(feature = "full", not(feature = $feature)))]
            compile_error!(concat!(
                "feature `full` must include `",
                $feature,
                "` (see Cargo.toml)"
            ));
        )+
    };
}

full_membership_guards!(
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
