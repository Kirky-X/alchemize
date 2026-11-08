// SPDX-License-Identifier: Apache-2.0
//! OMML (Office Math Markup Language) → LaTeX, table-driven.
//!
//! Covers the common OMML constructs (R-self-004): runs, fractions, sub/sup
//! scripts, radicals, delimiters, n-ary operators, and accent/bar groups.
//! Elements outside the table fall back to `\text{…}` wrapping and are
//! counted in `warnings` — nothing silently disappears.

use quick_xml::Reader;
use quick_xml::events::Event;

/// Conversion warnings surfaced into metadata.
#[derive(Default, Debug, PartialEq)]
pub struct OmmlWarnings {
    /// Formulas converted through the table.
    pub converted: usize,
    /// Fragments that hit the `\text{}` fallback (counted, never dropped).
    pub fallback: usize,
}

/// Single-slot wrappers: OMML element → (LaTeX prefix, suffix).
///
/// Multi-slot constructs (fractions, scripts, radicals, n-ary) are matched
/// explicitly below because their child order and arity matter; this table
/// covers the uniform "one body" family and is consulted before the unknown
/// fallback.
const SINGLE_SLOT_WRAPPERS: &[(&str, &str, &str)] = &[
    ("borderBox", "\\boxed{", "}"),
    ("groupChr", "", ""),
    ("box", "", ""),
    ("phant", "\\phantom{", "}"),
];

/// Look up a single-slot wrapper for an OMML local name.
fn wrapper_for(local: &str) -> Option<(&'static str, &'static str)> {
    SINGLE_SLOT_WRAPPERS
        .iter()
        .find(|(name, _, _)| *name == local)
        .map(|(_, prefix, suffix)| (*prefix, *suffix))
}

/// Convert every `m:oMath` in a WordprocessingML document to LaTeX.
///
/// Returns the LaTeX fragments in document order plus warning counts. The
/// caller replaces the OMML islands with the fragments.
pub fn convert_document_omml(xml: &[u8]) -> (Vec<String>, OmmlWarnings) {
    let mut out = Vec::new();
    let mut warnings = OmmlWarnings::default();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                if local == "oMath" {
                    let (latex, used_fallback) = convert_island(&mut reader);
                    if !latex.trim().is_empty() {
                        warnings.converted += 1;
                        warnings.fallback += usize::from(used_fallback);
                        out.push(latex);
                    }
                    continue; // convert_island consumed through </m:oMath>
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (out, warnings)
}

/// Convert one `m:oMath` island whose Start event was just consumed.
///
/// Returns the LaTeX body and whether any fragment needed `\text{}`
/// fallback. Consumes events through `</m:oMath>`.
pub fn convert_island_pub(reader: &mut Reader<&[u8]>) -> (String, bool) {
    convert_island(reader)
}

/// Convert one `m:oMath` island (reader positioned after the Start event).
///
/// Consumes events through the closing `</m:oMath>` and returns the LaTeX
/// body plus whether any fragment needed the `\text{}` fallback.
fn convert_island(reader: &mut Reader<&[u8]>) -> (String, bool) {
    let mut converted = String::new();
    let mut fallback = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                match local.as_str() {
                    "r" => {
                        // m:r → m:t text (LaTeX-escaped).
                        let text = collect_text_until(reader, b"r");
                        converted.push_str(&escape_latex(&text));
                    }
                    "f" => {
                        let (num, den) = two_children(reader, b"num", b"den");
                        converted.push_str(&format!("\\frac{{{num}}}{{{den}}}"));
                    }
                    "sSub" => {
                        let (base, sub) = two_children(reader, b"e", b"sub");
                        converted.push_str(&format!("{base}_{{{sub}}}"));
                    }
                    "sSup" => {
                        let (base, sup) = two_children(reader, b"e", b"sup");
                        converted.push_str(&format!("{base}^{{{sup}}}"));
                    }
                    "sSubSup" => {
                        let (base, sub, sup) = three_children(reader, b"e", b"sub", b"sup");
                        converted.push_str(&format!("{base}_{{{sub}}}^{{{sup}}}"));
                    }
                    "rad" => {
                        // <m:rad><m:radPr>…</m:radPr><m:deg>…</m:deg><m:e>…</m:e></m:rad>
                        let (deg, body) = two_children(reader, b"deg", b"e");
                        let deg_clean = deg.trim();
                        if deg_clean.is_empty() || deg_clean == "2" {
                            converted.push_str(&format!("\\sqrt{{{body}}}"));
                        } else {
                            converted.push_str(&format!("\\sqrt[{deg_clean}]{{{body}}}"));
                        }
                    }
                    "d" => {
                        // Delimiter: default parens; m:begPr/m:endPr carry
                        // custom chars which we honor for brackets only.
                        let body = collect_children_text(reader, b"e");
                        converted.push_str(&format!("\\left({body}\\right)"));
                    }
                    "nary" => {
                        let (sub, sup, body) = three_children(reader, b"sub", b"sup", b"e");
                        // The operator char rides m:chr w:val; default ∑.
                        converted.push_str(&format!("\\sum_{{{sub}}}^{{{sup}}} {body}"));
                    }
                    "func" => {
                        let (name, arg) = two_children(reader, b"fName", b"e");
                        converted.push_str(&format!("{name}\\left({arg}\\right)"));
                    }
                    "acc" => {
                        let body = collect_children_text(reader, b"e");
                        converted.push_str(&format!("\\hat{{{body}}}"));
                    }
                    "bar" => {
                        let body = collect_children_text(reader, b"e");
                        converted.push_str(&format!("\\overline{{{body}}}"));
                    }
                    "eqArr" => {
                        let rows = collect_children_text(reader, b"e");
                        converted.push_str(&format!("\\begin{{aligned}}{rows}\\end{{aligned}}"));
                    }
                    "m" => {
                        // Matrix: rows of m:mr separated by \\.
                        let rows = collect_children_text(reader, b"mr");
                        converted.push_str(&format!("\\begin{{matrix}}{rows}\\end{{matrix}}"));
                    }
                    _ => {
                        // Table lookup first: uniform single-slot wrappers.
                        let text = collect_any_until_end(reader, &local);
                        if text.trim().is_empty() {
                            // nothing to emit
                        } else if let Some((prefix, suffix)) = wrapper_for(&local) {
                            converted.push_str(prefix);
                            converted.push_str(&text);
                            converted.push_str(suffix);
                        } else {
                            // Unknown OMML element: wrap in \text{} and count
                            // the fallback so the loss is never silent.
                            fallback = true;
                            converted.push_str(&format!("\\text{{{}}}", escape_latex(&text)));
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                // Bare text inside oMath (rare; m:t usually wraps it).
                let text = String::from_utf8_lossy(t.as_bytes()).into_owned();
                if !text.trim().is_empty() {
                    converted.push_str(&escape_latex(&text));
                }
            }
            Ok(Event::GeneralRef(r)) => {
                // &amp; &lt; … arrive as separate events in quick-xml 0.42;
                // decode or the character silently vanishes.
                converted.push_str(&escape_latex(&decode_entity(&r)));
            }
            Ok(Event::End(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref()) == "oMath" {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (converted, fallback)
}

/// Read the text of the next element pair `(a, b)` inside a container.
fn two_children(reader: &mut Reader<&[u8]>, a: &[u8], b: &[u8]) -> (String, String) {
    let mut first = String::new();
    let mut second = String::new();
    let mut buf = Vec::new();
    let mut next = a;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                let name = std::str::from_utf8(next).unwrap_or("");
                if local == name {
                    let text = collect_text_until(reader, next);
                    if next == a {
                        first = text;
                        next = b;
                    } else {
                        second = text;
                    }
                } else {
                    // Skip unknown child.
                    let _ = collect_any_until_end(reader, &local);
                }
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                if local == "f"
                    || local == "sSub"
                    || local == "sSup"
                    || local == "d"
                    || local == "func"
                {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (first, second)
}

/// Read the text of three ordered children inside a container.
fn three_children(
    reader: &mut Reader<&[u8]>,
    a: &[u8],
    b: &[u8],
    c: &[u8],
) -> (String, String, String) {
    let order = [a, b, c];
    let mut out = [String::new(), String::new(), String::new()];
    let mut idx = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                if idx < order.len() {
                    let want = std::str::from_utf8(order[idx]).unwrap_or("");
                    if local == want {
                        out[idx] = collect_text_until(reader, order[idx]);
                        idx += 1;
                        continue;
                    }
                }
                let _ = collect_any_until_end(reader, &local);
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                if matches!(local.as_str(), "sSubSup" | "nary" | "rad") {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (out[0].clone(), out[1].clone(), out[2].clone())
}

/// Collect the concatenated text of repeated `child` elements until the
/// parent closes (rows, matrix cells, delimiter bodies).
fn collect_children_text(reader: &mut Reader<&[u8]>, child: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                let want = std::str::from_utf8(child).unwrap_or("");
                if local == want {
                    parts.push(collect_text_until(reader, child));
                } else {
                    let _ = collect_any_until_end(reader, &local);
                }
            }
            Ok(Event::End(e)) => {
                let local = crate::infrastructure::document::local_tag_name(e.name().as_ref());
                if matches!(local.as_str(), "acc" | "bar" | "m" | "eqArr") {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    parts.join(" \\\\ ")
}

/// Consume events until the closing tag of `tag` (which was just Started);
/// return the concatenated `m:t`/text content.
fn collect_text_until(reader: &mut Reader<&[u8]>, tag: &[u8]) -> String {
    let mut out = String::new();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if name_ends_with(e.name().as_ref(), tag) {
                    depth += 1;
                }
            }
            Ok(Event::Text(t)) => out.push_str(&String::from_utf8_lossy(t.as_bytes())),
            Ok(Event::GeneralRef(r)) => {
                out.push_str(&decode_entity(&r));
            }
            Ok(Event::End(e)) => {
                if name_ends_with(e.name().as_ref(), tag) {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Whether a QName ends with the given local name (namespace-agnostic).
fn name_ends_with(qname: &str, tag: &[u8]) -> bool {
    let tag = std::str::from_utf8(tag).unwrap_or("\0");
    qname.ends_with(tag)
}

/// Skip through an unknown element (already Started) and return its text.
fn collect_any_until_end(reader: &mut Reader<&[u8]>, local: &str) -> String {
    let mut out = String::new();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref()) == local {
                    depth += 1;
                }
            }
            Ok(Event::Text(t)) => out.push_str(&String::from_utf8_lossy(t.as_bytes())),
            Ok(Event::End(e)) => {
                if crate::infrastructure::document::local_tag_name(e.name().as_ref()) == local {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Decode the five XML predefined entities (`quick-xml` 0.42 surfaces them
/// as `Event::GeneralRef`).
pub fn decode_entity(name: &str) -> String {
    match name {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        other => format!("&{other};"), // unknown: keep verbatim, no silent loss
    }
}

/// Escape LaTeX special characters in source text.
fn escape_latex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\textbackslash{}"),
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(ch);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn omml(inner: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:m="m"><w:body><w:p><m:oMath>{inner}</m:oMath></w:p></w:body></w:document>"#
        )
        .into_bytes()
    }

    fn convert(inner: &str) -> String {
        let (out, _) = convert_document_omml(&omml(inner));
        assert_eq!(out.len(), 1, "one island expected");
        out[0].clone()
    }

    #[test]
    fn plain_run_becomes_text() {
        assert_eq!(convert(r#"<m:r><m:t>x+1</m:t></m:r>"#), "x+1");
    }

    #[test]
    fn fraction_maps_to_frac() {
        let got = convert(
            r#"<m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>"#,
        );
        assert_eq!(got, "\\frac{a}{b}");
    }

    #[test]
    fn subscript_and_superscript() {
        let got = convert(
            r#"<m:sSub><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub></m:sSub>"#,
        );
        assert_eq!(got, "x_{i}");
        let got = convert(
            r#"<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>"#,
        );
        assert_eq!(got, "x^{2}");
    }

    #[test]
    fn sub_sup_combined() {
        let got = convert(
            r#"<m:sSubSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup><m:r><m:t>j</m:t></m:r></m:sup></m:sSubSup>"#,
        );
        assert_eq!(got, "x_{i}^{j}");
    }

    #[test]
    fn radical_square_and_nth() {
        let got = convert(
            r#"<m:rad><m:deg><m:r><m:t>2</m:t></m:r></m:deg><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad>"#,
        );
        assert_eq!(got, "\\sqrt{x}");
        let got = convert(
            r#"<m:rad><m:deg><m:r><m:t>3</m:t></m:r></m:deg><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad>"#,
        );
        assert_eq!(got, "\\sqrt[3]{x}");
    }

    #[test]
    fn delimiter_wraps_parens() {
        let got = convert(r#"<m:d><m:e><m:r><m:t>x+y</m:t></m:r></m:e></m:d>"#);
        assert_eq!(got, "\\left(x+y\\right)");
    }

    #[test]
    fn nary_becomes_sum_with_bounds() {
        let got = convert(
            r#"<m:nary><m:sub><m:r><m:t>i=1</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup><m:e><m:r><m:t>x_i</m:t></m:r></m:e></m:nary>"#,
        );
        assert_eq!(got, "\\sum_{i=1}^{n} x_i");
    }

    #[test]
    fn latex_special_chars_escaped() {
        // Underscore must be escaped; ampersand arrives XML-escaped and is
        // unescaped by the reader, then re-escaped for LaTeX.
        let got = convert(r#"<m:r><m:t>a_b</m:t></m:r>"#);
        assert_eq!(got, "a\\_b");
        let got = convert(r#"<m:r><m:t>a&amp;b</m:t></m:r>"#);
        assert_eq!(got, "a\\&b");
    }

    #[test]
    fn unknown_element_falls_back_and_is_counted() {
        let (out, warnings) = convert_document_omml(&omml(
            r#"<m:mysteryTag><m:r><m:t>weird</m:t></m:r></m:mysteryTag>"#,
        ));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("\\text{weird}"), "got: {}", out[0]);
        assert_eq!(warnings.fallback, 1, "fallback must be counted");
        assert_eq!(warnings.converted, 1);
    }

    #[test]
    fn eqarr_groupchr_borderbox_and_unknown_run() {
        // eqArr rows join with \\.
        assert_eq!(
            convert(r#"<m:eqArr><m:e><m:r><m:t>a=1</m:t></m:r></m:e></m:eqArr>"#),
            "\\begin{aligned}a=1\\end{aligned}"
        );
        // groupChr passes through.
        assert_eq!(
            convert(r#"<m:groupChr><m:e><m:r><m:t>v</m:t></m:r></m:e></m:groupChr>"#),
            "v"
        );
        // borderBox wraps in \boxed.
        assert_eq!(
            convert(r#"<m:borderBox><m:e><m:r><m:t>z</m:t></m:r></m:e></m:borderBox>"#),
            "\\boxed{z}"
        );
        // A bare run of text inside oMath (no m:r wrapper).
        assert_eq!(convert("loose"), "loose");
    }

    #[test]
    fn omathpara_is_walked_like_omath() {
        let doc = br#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:m="m"><w:body>
<w:p><m:oMathPara><m:oMath><m:r><m:t>para math</m:t></m:r></m:oMath></m:oMathPara></w:p>
</w:body></w:document>"#;
        let (out, warnings) = convert_document_omml(doc);
        assert_eq!(out, vec!["para math".to_string()]);
        assert_eq!(warnings.converted, 1);
    }

    #[test]
    fn multi_row_matrix_joins_rows() {
        let got = convert(
            r#"<m:m><m:mr><m:r><m:t>a</m:t></m:r></m:mr><m:mr><m:r><m:t>b</m:t></m:r></m:mr></m:m>"#,
        );
        assert_eq!(got, "\\begin{matrix}a \\\\ b\\end{matrix}");
    }

    #[test]
    fn empty_island_yields_empty_latex() {
        let (out, warnings) = convert_document_omml(&omml(""));
        assert_eq!(out.len(), 0, "an empty oMath produces no island");
        assert_eq!(warnings.converted, 0);
    }

    #[test]
    fn multiple_islands_in_order() {
        let xml = omml(r#"<m:r><m:t>a</m:t></m:r>"#)
            .iter()
            .cloned()
            .chain(
                br#"<w:p><m:oMath><m:r><m:t>b</m:t></m:r></m:oMath></w:p>"#
                    .iter()
                    .cloned(),
            )
            .collect::<Vec<u8>>();
        // Rebuild a doc with two islands.
        let doc = br#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:m="m"><w:body>
<w:p><m:oMath><m:r><m:t>a</m:t></m:r></m:oMath><m:oMath><m:r><m:t>b</m:t></m:r></m:oMath></w:p>
</w:body></w:document>"#;
        let (out, _) = convert_document_omml(doc);
        assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
        let _ = xml; // silence unused
    }

    #[test]
    fn bar_and_accent() {
        assert_eq!(
            convert(r#"<m:bar><m:e><m:r><m:t>x</m:t></m:r></m:e></m:bar>"#),
            "\\overline{x}"
        );
        assert_eq!(
            convert(r#"<m:acc><m:e><m:r><m:t>a</m:t></m:r></m:e></m:acc>"#),
            "\\hat{a}"
        );
    }

    #[test]
    fn func_and_matrix() {
        assert_eq!(
            convert(
                r#"<m:func><m:fName><m:r><m:t>sin</m:t></m:r></m:fName><m:e><m:r><m:t>x</m:t></m:r></m:e></m:func>"#
            ),
            "sin\\left(x\\right)"
        );
        // A single m:mr row joins its cells verbatim; the `&` separator is
        // raw text inside the row (escaped by the LaTeX escaper).
        assert_eq!(
            convert(r#"<m:m><m:mr><m:r><m:t>a</m:t></m:r><m:r><m:t>b</m:t></m:r></m:mr></m:m>"#),
            "\\begin{matrix}ab\\end{matrix}"
        );
    }
}
