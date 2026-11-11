// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Fluent message catalog — `locales/{en,zh}/messages.ftl` embedded at
//! compile time via [`include_str!`]; the disk/embedded pair is the same
//! file, so it is in sync by construction (limiteron 房规，守卫测试
//! `test_locales_dir_matches_embedded` 兜底）。
//!
//! Bundles are `fluent_bundle::concurrent::FluentBundle` (Send + Sync),
//! cached in [`OnceLock`] statics. Lookup chain: current language →
//! `"en"` → the key itself (never panics, never empty).

use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// Embedded English FTL resource.
pub(crate) const EN_FTL: &str = include_str!("../../locales/en/messages.ftl");
/// Embedded Chinese FTL resource.
pub(crate) const ZH_FTL: &str = include_str!("../../locales/zh/messages.ftl");

/// Translate `key` under the current locale, falling back to English, then
/// to the key itself.
pub fn translate(key: &str, args: &[(&str, String)]) -> String {
    let lang = crate::i18n::locale::current_locale();
    translate_for(&lang, key, args)
}

/// Translate `key` for an explicit language tag. Unsupported/unknown
/// languages fall back to English, then to the key.
pub fn translate_for(lang: &str, key: &str, args: &[(&str, String)]) -> String {
    format_from_bundle(lang, key, args)
        .or_else(|| format_from_bundle("en", key, args))
        .unwrap_or_else(|| key.to_string())
}

/// Cached concurrent Fluent bundles (thread-safe, built once).
static EN_BUNDLE: OnceLock<FluentBundle<FluentResource>> = OnceLock::new();
static ZH_BUNDLE: OnceLock<FluentBundle<FluentResource>> = OnceLock::new();

/// Format a message from the Fluent catalog for the given language.
///
/// Any language whose primary subtag is `zh` resolves to the Chinese
/// bundle (`zh-CN`/`zh-TW`/`zh-HK` → `zh`); everything else resolves to
/// the English bundle. Missing keys return `None` so callers can apply
/// their own fallback.
fn format_from_bundle(lang: &str, key: &str, args: &[(&str, String)]) -> Option<String> {
    let bundle = match lang.split('-').next() {
        Some("zh") => ZH_BUNDLE.get_or_init(build_zh_bundle),
        _ => EN_BUNDLE.get_or_init(build_en_bundle),
    };

    let msg = bundle.get_message(key)?;
    let pattern = msg.value()?;

    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
        fluent_args.set(*name, FluentValue::from(value.clone()));
    }
    let mut errors = Vec::new();
    let formatted = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
    Some(formatted.to_string())
}

/// Build the English bundle (locale `en-US`).
fn build_en_bundle() -> FluentBundle<FluentResource> {
    build_bundle(EN_FTL, "en-US")
}

/// Build the Chinese bundle (locale `zh-CN`).
fn build_zh_bundle() -> FluentBundle<FluentResource> {
    build_bundle(ZH_FTL, "zh-CN")
}

/// Shared bundle construction: one resource, no overload functions, no
/// custom formatter. Placeable expressions and selectors are supported by
/// fluent-bundle itself.
///
/// Isolating is disabled: Unicode bidi isolates (FSI/PDI) around every
/// placeable would change even the English output byte-for-byte, breaking
/// the spec-pinned CLI contract. Hostile-content defense stays at the
/// diagnostic layer (`sanitize_ext` strips bidi controls from user-supplied
/// extensions before they reach a message).
fn build_bundle(ftl: &str, lang: &str) -> FluentBundle<FluentResource> {
    let res = FluentResource::try_new(ftl.to_string()).expect("embedded FTL must parse");
    let langid: LanguageIdentifier = lang.parse().expect("hardcoded langid must parse");
    let mut bundle = FluentBundle::new_concurrent(vec![langid]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(res)
        .expect("single resource cannot overlap");
    bundle
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard: the embedded resources must stay byte-identical to the
    /// on-disk files (defense against an edit that forgot the disk side).
    #[test]
    fn test_locales_dir_matches_embedded() {
        assert_eq!(
            EN_FTL,
            std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/locales/en/messages.ftl"
            ))
            .expect("locales/en/messages.ftl readable"),
        );
        assert_eq!(
            ZH_FTL,
            std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/locales/zh/messages.ftl"
            ))
            .expect("locales/zh/messages.ftl readable"),
        );
    }

    #[test]
    fn en_and_zh_key_sets_match() {
        // Key extraction by line scan: an FTL message id is a line starting
        // with a non-space token followed by `=` (comments and multiline
        // patterns are indented or start with `#`). Avoids fluent-bundle's
        // AST (not exported under default-features = false).
        fn keys(ftl: &str) -> Vec<String> {
            let mut out: Vec<String> = ftl
                .lines()
                .filter(|l| !l.starts_with(['#', ' ']) && l.contains('='))
                .filter_map(|l| l.split('=').next())
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect();
            out.sort();
            out
        }
        assert_eq!(keys(EN_FTL), keys(ZH_FTL), "en/zh key sets must match");
    }

    #[test]
    fn missing_key_falls_back_to_key_itself() {
        assert_eq!(translate_for("zh", "no-such-key", &[]), "no-such-key");
        assert_eq!(translate_for("en", "no-such-key", &[]), "no-such-key");
        assert_eq!(translate_for("fr", "no-such-key", &[]), "no-such-key");
    }

    #[test]
    fn args_interpolate() {
        assert_eq!(
            translate_for("en", "test-args", &[("n", "42".into())]),
            "value 42"
        );
        assert_eq!(
            translate_for("zh-CN", "test-args", &[("n", "42".into())]),
            "值 42"
        );
    }

    #[test]
    fn isolating_is_disabled_en_matches_key_text() {
        // With isolating off, the en bundle must render exactly the FTL
        // pattern text (no FSI/PDI around the placeable).
        assert_eq!(
            translate_for("en", "unsupported-format", &[("ext", "xyz".into())]),
            "unsupported format: xyz"
        );
    }
}
