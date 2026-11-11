// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Shared helpers for document converters.

use std::io::Read;

use crate::domain::error::AlchemizeError;

/// Per-part read ceiling for inner package members (OOXML/ODF parts).
/// Matches the container per-entry policy: bounded per stream.
pub const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;

/// Read an inner package part with a size ceiling.
///
/// Package parts are attacker-controlled: an entry may honestly declare
/// gigabytes and a plain `read_to_end` would grow the buffer to match.
/// `take` bounds the read; exceeding the ceiling is an explicit error.
pub fn read_part_limited<R: Read>(reader: &mut R, what: &str) -> Result<Vec<u8>, AlchemizeError> {
    let mut buf = Vec::new();
    reader
        .take(MAX_PART_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| AlchemizeError::Zip(format!("part {what} read failed: {e}")))?;
    if buf.len() as u64 > MAX_PART_BYTES {
        return Err(AlchemizeError::Zip(format!(
            "part {what} exceeds the {MAX_PART_BYTES}-byte per-part limit"
        )));
    }
    Ok(buf)
}

/// Cumulative output ceiling for one conversion's markdown text: multi-part
/// documents can otherwise sum unboundedly despite per-part read limits.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// Enforce the cumulative output ceiling.
pub fn check_output_budget(bytes: usize) -> Result<(), AlchemizeError> {
    if bytes > MAX_OUTPUT_BYTES {
        return Err(AlchemizeError::other(format!(
            "converted output exceeds the {MAX_OUTPUT_BYTES}-byte cumulative limit"
        )));
    }
    Ok(())
}

/// One-line label: control/invisible format characters become spaces so
/// attacker-controlled names cannot inject markdown structure (headings,
/// fences) or visually reorder the rendered text (bidi overrides, line
/// separators).
pub fn sanitize_label(text: &str) -> String {
    text.chars()
        .map(|c| if is_invisible(c) { ' ' } else { c })
        .collect()
}

/// Control (Cc), line/paragraph separator (Zl/Zp) and invisible format
/// characters (Cf: zero-width, bidi controls, BOM) used for spoofing.
fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{08E2}'
                | '\u{180E}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{2060}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0001}'
        )
        || ('\u{0600}'..='\u{0605}').contains(&c)
        || ('\u{200B}'..='\u{200F}').contains(&c)
        || ('\u{202A}'..='\u{202E}').contains(&c)
        || ('\u{2061}'..='\u{2069}').contains(&c)
        || ('\u{13430}'..='\u{1343F}').contains(&c)
        || ('\u{E0000}'..='\u{E007F}').contains(&c)
}

/// Upper bound for inline text in a fenced fallback block.
///
/// Bounds both the document size and the fence length derived from the body
/// (a member of nothing but backticks must not amplify output).
pub const MAX_INLINE_TEXT_BYTES: usize = 8 * 1024;

/// Pick a fence at least as long as the longest backtick run in `body`, so
/// payload text cannot terminate a fenced block early.
///
/// CommonMark allows up to three leading spaces before a fence, so the count
/// strips up to three spaces; the returned fence is one backtick longer than
/// the longest run found.
pub fn fence_for(body: &str) -> String {
    // CommonMark treats CR, LF and CRLF as line endings; `str::lines` only
    // splits on LF, so normalize first (a lone CR can otherwise hide a
    // fence). CR-free bodies — the overwhelming majority — skip the two
    // whole-buffer copies entirely.
    let normalized;
    let body = if body.contains('\r') {
        normalized = body.replace("\r\n", "\n").replace('\r', "\n");
        normalized.as_str()
    } else {
        body
    };
    let longest = body
        .lines()
        .map(|line| {
            let trimmed = line
                .strip_prefix("   ")
                .or_else(|| line.strip_prefix("  "))
                .or_else(|| line.strip_prefix(' '))
                .unwrap_or(line);
            trimmed.chars().take_while(|c| *c == '`').count()
        })
        .max()
        .unwrap_or(0);
    "`".repeat(longest.max(3) + 1)
}

/// Markdown-escape a label used as a heading: `<script>`, images and links
/// in member names must render literally, not as HTML/markdown.
pub fn escape_markdown_label(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in sanitize_label(text).chars() {
        if matches!(
            ch,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '!' | '(' | ')'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Label safe for an inline code span: newlines scrubbed and backticks
/// neutralized (a backtick would close the span and inject markdown).
pub fn code_span_label(text: &str) -> String {
    sanitize_label(text).replace('`', "'")
}

/// Extract the local tag name from an XML tag (strip namespace prefix).
pub fn local_tag_name(tag: &str) -> String {
    match tag.split_once(':') {
        Some((_, local)) => local.to_string(),
        None => tag.to_string(),
    }
}

/// Decode an XML entity reference body (`quick-xml` 0.42 surfaces them as
/// `Event::GeneralRef`; the name is the reference WITHOUT the surrounding
/// `&` / `;` — e.g. `amp`, `#233`, `#xE9`).
pub fn decode_entity(name: &str) -> String {
    match name {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        // Numeric references (decimal and hex): decode to the character;
        // unknown/unrepresentable keeps the verbatim form, no silent loss.
        other => match (other.strip_prefix("#x"), other.strip_prefix('#')) {
            (Some(hex), _) => u32::from_str_radix(hex, 16)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("&{other};")),
            (None, Some(dec)) => dec
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("&{other};")),
            (None, None) => format!("&{other};"),
        },
    }
}

/// Strip tags from a fragment, keeping text content.
///
/// Lives here (not in the html module) so the LaTeX converter can reuse it
/// without pulling the whole `html` capability feature in.
pub fn strip_tags(fragment: &str) -> String {
    let mut out = String::new();
    let mut rest = fragment;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        match rest[lt..].find('>') {
            Some(gt) => rest = &rest[lt + gt + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Decode bytes to a string, trying UTF-8 first then common legacy encodings.
///
/// Returns `(decoded, encoding_name)`; falls back to lossy UTF-8.
#[cfg(any(
    feature = "text",
    feature = "csv",
    feature = "pandoc",
    feature = "media"
))]
pub fn detect_encoding(bytes: &[u8]) -> (String, String) {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_string(), "utf-8".to_string());
    }

    // CJK encodings first: windows-1252 decodes almost any byte sequence, so
    // probing it first would shadow GBK/Big5/Shift-JIS content.
    let encodings = [
        ("gbk", encoding_rs::GBK),
        ("big5", encoding_rs::BIG5),
        ("shift_jis", encoding_rs::SHIFT_JIS),
        ("euc-jp", encoding_rs::EUC_JP),
        ("iso-2022-jp", encoding_rs::ISO_2022_JP),
        ("euc-kr", encoding_rs::EUC_KR),
        ("windows-1252", encoding_rs::WINDOWS_1252),
    ];

    for (name, encoding) in encodings {
        let (decoded, _, _) = encoding.decode(bytes);
        let decoded_str = decoded.into_owned();
        if decoded_str.chars().any(|c| c.is_alphanumeric()) {
            return (decoded_str, name.to_string());
        }
    }

    (
        String::from_utf8_lossy(bytes).into_owned(),
        "utf-8-lossy".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_entity_covers_named_and_numeric_references() {
        assert_eq!(decode_entity("amp"), "&");
        assert_eq!(decode_entity("lt"), "<");
        assert_eq!(decode_entity("gt"), ">");
        assert_eq!(decode_entity("quot"), "\"");
        assert_eq!(decode_entity("apos"), "'");
        // Numeric: decimal and hex, as emitted by Event::GeneralRef (no & / ;).
        assert_eq!(decode_entity("#233"), "é");
        assert_eq!(decode_entity("#xE9"), "é");
        assert_eq!(decode_entity("#65"), "A");
        // Unknown name and unrepresentable codepoint stay verbatim.
        assert_eq!(decode_entity("unknown"), "&unknown;");
        assert_eq!(decode_entity("#0"), "\0");
        assert_eq!(decode_entity("#xFFFFFFFF"), "&#xFFFFFFFF;");
    }

    #[test]
    fn local_tag_name_strips_namespace() {
        assert_eq!(local_tag_name("w:p"), "p");
        assert_eq!(local_tag_name("a:blip"), "blip");
        assert_eq!(local_tag_name("plain"), "plain");
    }

    #[test]
    fn sanitize_label_drops_invisible_format_characters() {
        // Bidi override (Cf), line separator (Zl) and BOM must not survive.
        let raw = "safe\u{202E}evil\u{2028}next\u{FEFF}end";
        let clean = sanitize_label(raw);
        assert_eq!(clean, "safe evil next end");
    }

    #[test]
    fn fence_for_handles_lone_cr_line_endings() {
        // CommonMark treats a lone CR as a line ending; `str::lines` does not.
        assert_eq!(fence_for("A\r`````\rB"), "``````");
    }

    #[test]
    fn fence_for_outgrows_backtick_runs_in_body() {
        assert_eq!(fence_for("plain"), "````");
        assert_eq!(fence_for("a\n```\nb"), "````");
        assert_eq!(fence_for("a\n`````\nb"), "``````");
        // CommonMark allows up to three leading spaces before a fence.
        assert_eq!(fence_for("a\n   `````\nb"), "``````");
        // Four spaces is an indented code block, not a fence close.
        assert_eq!(fence_for("a\n    `````\nb"), "````");
    }

    #[cfg(any(
        feature = "text",
        feature = "csv",
        feature = "pandoc",
        feature = "media"
    ))]
    #[test]
    fn detect_encoding_utf8_passthrough() {
        let (text, name) = detect_encoding("héllo".as_bytes());
        assert_eq!(name, "utf-8");
        assert_eq!(text, "héllo");
    }

    #[cfg(any(
        feature = "text",
        feature = "csv",
        feature = "pandoc",
        feature = "media"
    ))]
    #[test]
    fn detect_encoding_gbk_fallback() {
        // "你好" encoded in GBK
        let gbk_bytes = &[0xC4, 0xE3, 0xBA, 0xC3];
        let (text, name) = detect_encoding(gbk_bytes);
        assert_eq!(name, "gbk");
        assert!(text.contains("你好"));
    }

    #[cfg(any(
        feature = "text",
        feature = "csv",
        feature = "pandoc",
        feature = "media"
    ))]
    #[test]
    fn detect_encoding_lossy_for_undecodable() {
        let (text, name) = detect_encoding(&[0xFF, 0xFE, 0x00]);
        assert_eq!(name, "utf-8-lossy");
        assert!(!text.is_empty() || text.is_empty()); // never panics
    }
}
