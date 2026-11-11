// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Locale detection (unconditional dependency — no feature gate).
//!
//! Detection chain (first non-empty wins, POSIX env values normalized
//! `zh_CN.UTF-8` → `zh-CN`): `ALCHEMIZE_LANG` → `LC_ALL` → `LC_MESSAGES`
//! → `LANG` → `sys-locale` primary language → `"en"`. Never empty.
//!
//! The chain runs once and is cached in a [`OnceLock`]: `sys-locale` hits
//! CoreFoundation FFI on macOS per call, and env reads take the process
//! env lock — both are pointlessly repeated for a process-lifetime
//! property. This crate deliberately never mutates the locale environment
//! at runtime, so a cached first detection is faithful.

use std::sync::OnceLock;

/// Detect the current UI language as a BCP-47 tag.
///
/// POSIX env values carry an underscore and an optional codeset suffix;
/// both are normalized so `zh_CN.UTF-8` becomes `zh-CN`. A `C`/`POSIX`
/// locale means "unlocalized" and resolves to English.
pub fn current_locale() -> String {
    static LOCALE: OnceLock<String> = OnceLock::new();
    LOCALE.get_or_init(detect_locale).clone()
}

/// Run the detection chain once (uncached worker of [`current_locale`]).
fn detect_locale() -> String {
    for key in ["ALCHEMIZE_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(raw) = std::env::var(key)
            && let Some(tag) = normalize_posix_lang(&raw)
        {
            return tag;
        }
    }
    if let Some(lang) = sys_locale::get_locale() {
        return lang;
    }
    "en".to_string()
}

/// Normalize one POSIX-style locale value into a BCP-47 tag, or `None`
/// when the value is empty or means "unlocalized" (`C`, `POSIX`).
fn normalize_posix_lang(raw: &str) -> Option<String> {
    let head = raw.split('.').next().unwrap_or(raw);
    let head = head.trim();
    if head.is_empty() || head == "C" || head == "POSIX" {
        return None;
    }
    Some(head.replace('_', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_values_normalize() {
        assert_eq!(
            normalize_posix_lang("zh_CN.UTF-8").as_deref(),
            Some("zh-CN")
        );
        assert_eq!(normalize_posix_lang("en_US").as_deref(), Some("en-US"));
        assert_eq!(normalize_posix_lang("C"), None);
        assert_eq!(normalize_posix_lang("POSIX"), None);
        assert_eq!(normalize_posix_lang(""), None);
    }

    #[test]
    fn detection_never_panics_and_returns_nonempty() {
        // Reads whatever the host environment holds; only the invariant
        // matters here (the chain itself is exercised via translate_for
        // in catalog tests — edition 2024 makes env mutation unsafe and
        // is deliberately not used).
        let lang = current_locale();
        assert!(!lang.is_empty());
    }
}
