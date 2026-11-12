// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Message catalog — `locales/{en,zh}/messages.ftl` embedded at compile
//! time via [`include_str!`] and registered into the sdforge i18n host
//! registry (the library-layer i18n base). The disk/embedded pair is the
//! same file, so it is in sync by construction (守卫测试
//! `test_locales_dir_matches_embedded` 兜底）。
//!
//! Lookup chain (sdforge `translate_for`): host registration for the
//! requested locale → host registration for `"en"` → the key itself (never
//! panics, never empty). This crate registers both locales verbatim, so the
//! chain behaves exactly like the previous dual-bundle implementation:
//! requested language (zh-* → zh bucket) → English → the key.

use std::sync::OnceLock;

/// Embedded English FTL resource.
pub(crate) const EN_FTL: &str = include_str!("../../locales/en/messages.ftl");
/// Embedded Chinese FTL resource.
pub(crate) const ZH_FTL: &str = include_str!("../../locales/zh/messages.ftl");

/// Registration runs once (idempotent re-registration; the guard only
/// skips the repeated parse).
static REGISTERED: OnceLock<()> = OnceLock::new();

/// Translate `key` under the current locale, falling back to English, then
/// to the key itself.
pub fn translate(key: &str, args: &[(&str, String)]) -> String {
    let lang = crate::i18n::locale::current_locale();
    translate_for(&lang, key, args)
}

/// Translate `key` for an explicit language tag. Any language whose primary
/// subtag is `zh` resolves to the Chinese catalog (`zh-CN`/`zh-TW`/`zh-HK`
/// → `zh`); everything else resolves to English. Unsupported/unknown
/// languages therefore fall back to English, then to the key.
pub fn translate_for(lang: &str, key: &str, args: &[(&str, String)]) -> String {
    ensure_registered();
    let locale = match lang.split('-').next() {
        Some("zh") => "zh",
        _ => "en",
    };
    let owned: Vec<(String, String)> = args
        .iter()
        .map(|(n, v)| ((*n).to_string(), v.clone()))
        .collect();
    let refs: Vec<(&str, String)> = owned.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
    sdforge::i18n::translate_for(locale, key, &refs)
}

/// Register both locale catalogs into the sdforge host registry (once,
/// idempotent).
///
/// sdforge lookup order: host registration (highest priority) → built-in
/// en catalog → the key itself. Registering every key under both `"en"` and
/// `"zh"` makes the effective chain "requested locale → en → key", matching
/// the pre-switch dual-bundle semantics byte for byte. Keys are dash ids
/// here (no dot mapping — this catalog's FTL ids are the caller-facing keys).
pub fn ensure_registered() {
    REGISTERED.get_or_init(|| {
        register_locale("en", EN_FTL);
        register_locale("zh", ZH_FTL);
    });
}

/// Register one locale's catalog entries.
fn register_locale(locale: &str, ftl: &'static str) {
    for (key, value) in parse_entries(ftl) {
        sdforge::i18n::register_translation(locale, key, &value);
    }
}

/// Parse single-line `key = value` entries (skip blanks and `#` comments).
///
/// The catalog convention is single-line patterns only (guard tests assert
/// no continuation lines), so the value is the trimmed line remainder.
/// `{ $name }` placeholders are canonicalized to the compact `{$name}` form:
/// sdforge's missing-argument rendering echoes the registered template
/// placeholder verbatim, and the compact form matches Fluent's missing-arg
/// echo byte for byte (same normalization as sdforge's kit bridge).
fn parse_entries(ftl: &'static str) -> impl Iterator<Item = (&'static str, String)> {
    ftl.lines().filter_map(|line| {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        let (id, raw_value) = trimmed.split_once('=')?;
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        let value = raw_value.trim();
        let value = compact_placeholders(value);
        Some((id, value))
    })
}

/// Canonicalize `{ $name }` placeholders to `{$name}`; non-`$` braces pass
/// through untouched (advance one char, keep contents — sdforge
/// `format_template` semantics).
fn compact_placeholders(value: &str) -> String {
    if !value.contains('{') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find('{') {
        let Some(close_rel) = rest[open..].find('}') else {
            // No closing brace: keep the remainder verbatim (catalog guards
            // assert this never happens).
            out.push_str(rest);
            return out;
        };
        let inner = rest[open + 1..open + close_rel].trim();
        let placeholder = inner
            .strip_prefix('$')
            .map(str::trim)
            .filter(|name| !name.is_empty());
        match placeholder {
            Some(name) => {
                out.push_str(&rest[..open]);
                out.push_str("{$");
                out.push_str(name);
                out.push('}');
                rest = &rest[open + close_rel + 1..];
            }
            None => {
                out.push_str(&rest[..open + 1]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out
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
        // patterns are indented or start with `#`).
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

    /// Catalog convention guard: no continuation lines (all messages are
    /// single-line `key = value`), so the line parser equals Fluent parsing
    /// for this catalog.
    #[test]
    fn catalog_is_single_line_only() {
        for (name, ftl) in [("en", EN_FTL), ("zh", ZH_FTL)] {
            let continuations: Vec<&str> = ftl
                .lines()
                .filter(|l| !l.is_empty() && l.starts_with([' ', '\t']))
                .collect();
            assert!(
                continuations.is_empty(),
                "{name} catalog must stay single-line; found: {continuations:?}"
            );
        }
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

    /// Missing-argument echo matches Fluent semantics: the compact
    /// `{$name}` placeholder text is preserved.
    #[test]
    fn missing_args_echo_compact_placeholder() {
        assert_eq!(
            translate_for("en", "unsupported-format", &[]),
            "unsupported format: {$ext}"
        );
        assert_eq!(
            translate_for("zh", "unsupported-format", &[]),
            "不支持的格式：{$ext}"
        );
    }

    /// zh-* variants resolve to the Chinese catalog (primary-subtag rule).
    #[test]
    fn zh_variants_resolve_to_chinese_catalog() {
        for tag in ["zh", "zh-CN", "zh-TW", "zh-HK", "zh-Hans"] {
            assert_eq!(
                translate_for(tag, "test-args", &[("n", "7".into())]),
                "值 7",
                "tag {tag} must use the zh catalog"
            );
        }
    }

    #[test]
    fn isolating_is_disabled_en_matches_key_text() {
        // The en catalog must render exactly the FTL pattern text (no
        // Unicode isolates around the placeable).
        assert_eq!(
            translate_for("en", "unsupported-format", &[("ext", "xyz".into())]),
            "unsupported format: xyz"
        );
    }

    /// Registration is verbatim: every registered (locale, key) serves its
    /// parsed catalog value (post placeholder-canonicalization).
    #[test]
    fn registration_serves_catalog_verbatim() {
        ensure_registered();
        for (locale, ftl) in [("en", EN_FTL), ("zh", ZH_FTL)] {
            for (key, value) in parse_entries(ftl) {
                assert_eq!(
                    sdforge::i18n::translate_for(locale, key, &[]),
                    value,
                    "sdforge registry must serve ({locale}, {key}) verbatim"
                );
            }
        }
    }
}
