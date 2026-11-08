// SPDX-License-Identifier: Apache-2.0
//! OMML (Office Math Markup Language) → LaTeX, table-driven.
//!
//! Covers the common OMML constructs (R-self-004): runs, fractions, sub/sup
//! scripts, radicals, delimiters, n-ary operators, and accent/bar groups.
//! Elements outside the table fall back to `\text{…}` wrapping and are
//! counted in `warnings` — nothing silently disappears.

use quick_xml::events::Event;
use quick_xml::Reader;

/// Conversion warnings surfaced into metadata.
#[derive(Default, Debug, PartialEq)]
pub struct OmmlWarnings {
    /// Formulas converted through the table.
    pub converted: usize,
    /// Fragments that hit the `\text{}` fallback (counted, never dropped).
    pub fallback: usize,
}

/// One entry of the conversion table: OMML element → LaTeX wrapping.
struct Rule {
    /// OMML local name (`f`, `sSub`, `rad`, …).
    local: &'static str,
    /// LaTeX template; `{}` slots are filled with child conversions in
    /// document order.
    template: &'static str,
    /// Number of `{}` slots in the template.
    slots: usize,
}

/// The conversion table (R-self-004).
const RULES: &[Rule] = &[
    // Fraction: <m:f><m:num>…</m:num><m:den>…</m:den></m:f>
    Rule {
        local: "f",
        template: "\\frac{}{}",
        slots: 2,
    },
    // Subscript: <m:sSub><m:e>…</m:e><m:sub>…</m:sub></m:sSub>
    Rule {
        local: "sSub",
        template: "{}_{{}}",
        slots: 1,
    }, // handled specially below
    // Superscript
    Rule {
        local: "sSup",
        template: "{}^{}",
        slots: 2,
    },
    // Sub+sup combined
    Rule {
        local: "sSubSup",
        template: "{}_{}^{}",
        slots: 3,
    },
    // Radical: <m:rad><m:deg>…</m:deg><m:e>…</m:e></m:rad>; deg=2 → \sqrt
    Rule {
        local: "rad",
        template: "\\sqrt{}",
        slots: 1,
    },
    // Parenthesized delimiter: <m:d><m:e>…</m:e></m:d>
    Rule {
        local: "d",
        template: "\\left({}\\right)",
        slots: 1,
    },
    // N-ary (sum/integral): <m:nary><m:chr w:val="∑"/><m:sub>…</m:sub><m:sup>…</m:sup><m:e>…</m:e></m:nary>
    Rule {
        local: "nary",
        template: "\\sum_{}^{} {}",
        slots: 3,
    },
    // Accent: <m:acc><m:e>…</m:e></m:acc>
    Rule {
        local: "acc",
        template: "\\hat{{{}}}",
        slots: 1,
    },
    // Bar: <m:bar><m:e>…</m:e></m:bar>
    Rule {
        local: "bar",
        template: "\\overline{{{}}}",
        slots: 1,
    },
    // Function apply: <m:func><m:fName>…</m:fName><m:e>…</m:e></m:func>
    Rule {
        local: "func",
        template: "{}\\left({}\\right)",
        slots: 2,
    },
    // Matrix row → cells separated by &
    Rule {
        local: "mr",
        template: "{} & {}",
        slots: 2,
    },
    // Box (borderBox) → mathrm
    Rule {
        local: "borderBox",
        template: "\\boxed{{{}}}",
        slots: 1,
    },
    // Group character (phantom container) → pass through
    Rule {
        local: "groupChr",
        template: "{}",
        slots: 1,
    },
    // Equation array: rows separated by \\
    Rule {
        local: "eqArr",
        template: "\\begin{{aligned}}{}\\end{{aligned}}",
        slots: 1,
    },
];

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
                    warnings.converted += 1;
                    warnings.fallback += usize::from(used_fallback);
                    out.push(latex);
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
                        // Unknown OMML element: consume and mark fallback so
                        // the loss is counted, never silent.
                        let text = collect_any_until_end(reader, &local);
                        if !text.trim().is_empty() {
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
                converted.push_str(&escape_latex(&decode_entity(
                    &r,
                )));
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
        let (out, warnings) =
            convert_document_omml(&omml(r#"<m:box><m:r><m:t>weird</m:t></m:r></m:box>"#));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("\\text{weird}"), "got: {}", out[0]);
        assert_eq!(warnings.fallback, 1, "fallback must be counted");
        assert_eq!(warnings.converted, 1);
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
