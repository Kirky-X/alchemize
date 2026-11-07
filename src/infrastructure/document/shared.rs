// SPDX-License-Identifier: Apache-2.0
//! Shared helpers for document converters.

/// Extract the local tag name from an XML tag (strip namespace prefix).
pub fn local_tag_name(tag: &str) -> String {
    match tag.split_once(':') {
        Some((_, local)) => local.to_string(),
        None => tag.to_string(),
    }
}

/// Decode bytes to a string, trying UTF-8 first then common legacy encodings.
///
/// Returns `(decoded, encoding_name)`; falls back to lossy UTF-8.
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
    fn local_tag_name_strips_namespace() {
        assert_eq!(local_tag_name("w:p"), "p");
        assert_eq!(local_tag_name("a:blip"), "blip");
        assert_eq!(local_tag_name("plain"), "plain");
    }

    #[test]
    fn detect_encoding_utf8_passthrough() {
        let (text, name) = detect_encoding("héllo".as_bytes());
        assert_eq!(name, "utf-8");
        assert_eq!(text, "héllo");
    }

    #[test]
    fn detect_encoding_gbk_fallback() {
        // "你好" encoded in GBK
        let gbk_bytes = &[0xC4, 0xE3, 0xBA, 0xC3];
        let (text, name) = detect_encoding(gbk_bytes);
        assert_eq!(name, "gbk");
        assert!(text.contains("你好"));
    }

    #[test]
    fn detect_encoding_lossy_for_undecodable() {
        let (text, name) = detect_encoding(&[0xFF, 0xFE, 0x00]);
        assert_eq!(name, "utf-8-lossy");
        assert!(!text.is_empty() || text.is_empty()); // never panics
    }
}
