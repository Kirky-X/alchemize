// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! HTML to Markdown converter using htmd.

use async_trait::async_trait;
use htmd::HtmlToMarkdown;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts HTML files to markdown using the htmd crate.
///
/// Handles headings, paragraphs, lists, links, images, tables, and code
/// blocks. LaTeXML-style math/bibliography preprocessing lands with the
/// fidelity pass.
pub struct HtmlConverter {
    converter: HtmlToMarkdown,
}

impl HtmlConverter {
    /// Create a new HTML converter.
    pub fn new() -> Self {
        Self {
            converter: HtmlToMarkdown::new(),
        }
    }

    /// Synchronous core of the conversion: everything after UTF-8 decoding
    /// (style/script strip → math preprocessing → htmd → placeholder
    /// restore → result assembly). The async port delegates here after
    /// `to_bytes`, so both ports share one body and their outputs are
    /// byte-identical by construction.
    pub(crate) fn convert_str(&self, html: &str) -> Result<ConvertResult, AlchemizeError> {
        // Real-world HTML carries <style>/<script> whose text would leak
        // into the body; strip them before any other processing.
        let html = strip_style_and_script(html);

        // LaTeXML-style math must survive as LaTeX: extract before htmd
        // would flatten the element tree, and restore after — htmd escapes
        // backslashes inside `$…$`, so the LaTeX travels as a placeholder.
        let (preprocessed, fragments) = preprocess_math(&html);

        let markdown = self
            .converter
            .convert(&preprocessed)
            .map_err(|e| AlchemizeError::other(format!("HTML conversion failed: {e}")))?;

        let markdown = restore_math(&markdown, &fragments);

        let markdown = if markdown.trim().is_empty() {
            "*HTML document contains no extractable content.*\n".to_string()
        } else {
            markdown.trim().to_string() + "\n"
        };

        let metadata = serde_json::json!({
            "source_format": "html",
            "original_size": html.len(),
            "output_size": markdown.len(),
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }
}

impl Default for HtmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Remove `<style>…</style>` and `<script>…</script>` blocks (case-insensitive
/// open/close tags). Their content is not prose and must not reach the body.
pub fn strip_style_and_script(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let lower = rest.to_ascii_lowercase();
        let style = lower.find("<style");
        let script = lower.find("<script");
        let next = match (style, script) {
            (None, None) => {
                out.push_str(rest);
                return out;
            }
            (Some(a), None) => Some((a, "style")),
            (None, Some(b)) => Some((b, "script")),
            (Some(a), Some(b)) => Some(if a < b { (a, "style") } else { (b, "script") }),
        };
        let (pos, tag) = next.expect("tag kind");
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let lower_after = after.to_ascii_lowercase();
        let close = format!("</{tag}");
        match lower_after.find(&close) {
            Some(rel) => {
                let tail = &after[rel..];
                match tail.find('>') {
                    Some(gt) => rest = &tail[gt + 1..],
                    None => return out,
                }
            }
            None => return out,
        }
    }
}

/// Replace every `<math>…</math>` element with an alphanumeric placeholder,
/// returning the rewritten HTML plus the LaTeX fragments (delimiter included)
/// keyed by placeholder index.
///
/// Extraction priority mirrors the liuxiang `_get_latex` rule: `alttext`
/// first, then the `annotation[encoding=x-tex]` child. Block-display math
/// becomes `$$…$$`; inline stays `$…$`. Placeholders survive htmd untouched
/// (no escapable characters); [`restore_math`] swaps them back afterwards.
pub fn preprocess_math(html: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(html.len());
    let mut fragments: Vec<String> = Vec::new();
    let mut rest = html;
    loop {
        let Some(start_rel) = rest.find("<math") else {
            out.push_str(rest);
            return (out, fragments);
        };
        let (head, tail) = rest.split_at(start_rel);
        out.push_str(head);
        let Some(tag_end_rel) = tail.find('>') else {
            out.push_str(tail);
            return (out, fragments);
        };
        let open_tag = &tail[..tag_end_rel];
        let body_start = tag_end_rel + 1;
        let close_rel = tail[body_start..].find("</math>");
        let Some(close_rel) = close_rel else {
            out.push_str(tail);
            return (out, fragments);
        };
        let body = &tail[body_start..body_start + close_rel];
        let after = &tail[body_start + close_rel + "</math>".len()..];

        let latex = extract_alttext(open_tag).or_else(|| extract_annotation(body));
        match latex {
            Some(tex) if !tex.is_empty() => {
                let fragment = if open_tag.contains("display=\"block\"") {
                    format!("$${tex}$$")
                } else {
                    format!("${tex}$")
                };
                out.push_str(&format!("ALCHEMIZEMATH{}Z", fragments.len()));
                fragments.push(fragment);
            }
            _ => {
                out.push_str(&strip_tags(body));
            }
        }
        rest = after;
    }
}

/// Swap `ALCHEMIZEMATH{i}Z` placeholders back to their LaTeX fragments.
pub fn restore_math(markdown: &str, fragments: &[String]) -> String {
    let mut out = markdown.to_string();
    for (i, fragment) in fragments.iter().enumerate() {
        let placeholder = format!("ALCHEMIZEMATH{i}Z");
        out = out.replace(&placeholder, fragment);
    }
    out
}

/// Read `alttext="…"` from the open tag, unescaping HTML entities.
fn extract_alttext(open_tag: &str) -> Option<String> {
    let idx = open_tag.find("alttext=")?;
    let after = &open_tag[idx + "alttext=".len()..];
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = after[1..].find(quote)? + 1;
    Some(unescape_entities(&after[1..end]))
}

/// Read the `<annotation encoding="application/x-tex">…</annotation>` body.
fn extract_annotation(body: &str) -> Option<String> {
    let start = body.find("encoding=\"application/x-tex\"")?;
    let tag_open_end = body[start..].find('>')? + start;
    let content = &body[tag_open_end + 1..];
    let end = content.find("</annotation>")?;
    Some(unescape_entities(&content[..end]))
}

/// Strip tags from a fragment, keeping text content (public: shared by the
/// LaTeX converter for figcaption bodies).
pub use super::shared::strip_tags;

/// Unescape the common XML entities found inside alttext payloads.
fn unescape_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[async_trait]
impl Converter for HtmlConverter {
    fn name(&self) -> &'static str {
        "html"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read HTML input: {e}")))?;

        let html = String::from_utf8(bytes)
            .map_err(|e| AlchemizeError::other(format!("HTML content is not valid UTF-8: {e}")))?;

        self.convert_str(&html)
    }

    /// 链上唯一 await 是 `to_bytes`（Bytes 变体即 clone）——全链同步能力成立。
    fn is_sync_capable(&self) -> bool {
        true
    }

    fn convert_sync(&self, input: &Input) -> Result<ConvertResult, AlchemizeError> {
        match input {
            Input::Bytes { data, .. } => {
                let html = String::from_utf8(data.clone()).map_err(|e| {
                    AlchemizeError::other(format!("HTML content is not valid UTF-8: {e}"))
                })?;
                self.convert_str(&html)
            }
            Input::Path(_) => Err(AlchemizeError::NotSyncCapable(
                "path input requires the async port; the sync port serves in-memory Input::Bytes only"
                    .into(),
            )),
        }
    }

    fn supported_extensions(&self) -> &[&str] {
        &["html", "htm"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert_eq!(
            HtmlConverter::new().supported_extensions(),
            &["html", "htm"]
        );
    }

    #[tokio::test]
    async fn convert_simple_html() {
        let converter = HtmlConverter::new();
        let input = Input::bytes(
            "<h1>Title</h1><p>Hello world</p>".as_bytes().to_vec(),
            "test.html",
        );
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Title"));
        assert!(result.markdown.contains("Hello world"));
    }

    #[tokio::test]
    async fn convert_html_with_list() {
        let converter = HtmlConverter::new();
        let input = Input::bytes(
            "<ul><li>Item 1</li><li>Item 2</li></ul>"
                .as_bytes()
                .to_vec(),
            "test.html",
        );
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Item 1"));
        assert!(result.markdown.contains("Item 2"));
    }

    #[tokio::test]
    async fn convert_empty_html() {
        let converter = HtmlConverter::new();
        let input = Input::bytes(
            "<html><body></body></html>".as_bytes().to_vec(),
            "test.html",
        );
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("no extractable content"));
    }

    // -- style/script stripping -----------------------------------------------

    #[test]
    fn style_block_removed_from_body() {
        let html = "<html><head><style>:root { --x: red; }</style></head><body><p>Real prose</p></body></html>";
        let out = strip_style_and_script(html);
        assert!(!out.contains("--x: red"));
        assert!(out.contains("Real prose"));
    }

    #[test]
    fn script_block_removed_case_insensitive() {
        let html = "<SCRIPT>alert(1)</SCRIPT><p>text</p>";
        let out = strip_style_and_script(html);
        assert!(!out.contains("alert"));
        assert!(out.contains("text"));
    }

    #[test]
    fn unterminated_style_keeps_prefix() {
        let html = "before<style>unclosed";
        let out = strip_style_and_script(html);
        assert!(out.contains("before"));
        assert!(!out.contains("unclosed"));
    }

    // -- LaTeXML math preprocessing -------------------------------------------

    #[test]
    fn math_alttext_inline_becomes_dollar() {
        let html = r#"<p>value <math alttext="x+y" display="inline"><mi>x</mi></math> here</p>"#;
        let (out, fragments) = preprocess_math(html);
        assert!(
            out.contains("ALCHEMIZEMATH0Z"),
            "placeholder present: {out}"
        );
        assert!(!out.contains("<math"));
        let restored = restore_math(&out, &fragments);
        assert!(restored.contains("$x+y$"), "got: {restored}");
    }

    #[test]
    fn math_block_becomes_double_dollar() {
        let html = r#"<math alttext="E=mc^2" display="block"><mi>E</mi></math>"#;
        let (out, fragments) = preprocess_math(html);
        let restored = restore_math(&out, &fragments);
        assert!(restored.contains("$$E=mc^2$$"), "got: {restored}");
    }

    #[test]
    fn math_annotation_fallback_when_no_alttext() {
        let html = r#"<math><semantics><mi>x</mi><annotation encoding="application/x-tex">x^2</annotation></semantics></math>"#;
        let (out, fragments) = preprocess_math(html);
        let restored = restore_math(&out, &fragments);
        assert!(restored.contains("$x^2$"), "got: {restored}");
    }

    #[test]
    fn math_alttext_takes_priority_over_annotation() {
        let html = r#"<math alttext="\alpha"><annotation encoding="application/x-tex">\beta</annotation></math>"#;
        let (out, fragments) = preprocess_math(html);
        let restored = restore_math(&out, &fragments);
        assert!(restored.contains(r"$\alpha$"));
        assert!(!restored.contains("beta"));
    }

    #[test]
    fn math_entities_unescaped() {
        let html = r#"<math alttext="a &lt; b"><mi>a</mi></math>"#;
        let (out, fragments) = preprocess_math(html);
        let restored = restore_math(&out, &fragments);
        assert!(restored.contains("$a < b$"), "got: {restored}");
    }

    #[test]
    fn math_without_latex_falls_back_to_text() {
        let html = r#"<math><mi>E</mi><mo>=</mo><mn>2</mn></math>"#;
        let (out, _) = preprocess_math(html);
        assert!(!out.contains("<math"));
        assert!(out.contains("E"), "plain text kept, got: {out}");
    }

    #[test]
    fn math_unterminated_left_untouched() {
        let html = "<math alttext=\"x\">";
        let (out, fragments) = preprocess_math(html);
        assert!(out.contains("<math"), "unterminated left untouched");
        assert!(fragments.is_empty());
    }

    #[tokio::test]
    async fn convert_latexml_document_end_to_end() {
        let converter = HtmlConverter::new();
        let html = r#"<html><body><p>Given <math alttext="\alpha+\beta" display="inline"><mi>α</mi></math> we get</p><math alttext="c^2 = a^2 + b^2" display="block"><mi>c</mi></math></body></html>"#;
        let input = Input::bytes(html.as_bytes().to_vec(), "paper.html");
        let result = converter.convert(input).await.unwrap();
        assert!(
            result.markdown.contains(r"$\alpha+\beta$"),
            "md: {}",
            result.markdown
        );
        assert!(
            result.markdown.contains("$$c^2 = a^2 + b^2$$"),
            "md: {}",
            result.markdown
        );
    }

    // -- 同步口 ---------------------------------------------------------------

    /// 两个端口共用同一 convert_str 函数体：含 style/script/math/链接/表格
    /// 的语料上，async 与 sync 输出（markdown+quality+metadata）逐字节相等。
    #[tokio::test]
    async fn sync_port_matches_async_output_byte_for_byte() {
        let converter = HtmlConverter::new();
        let html = r##"<html><head><style>:root{--x:red}</style></head><body><p>Given <math alttext="\alpha+\beta" display="inline"><mi>α</mi></math> we get</p><math alttext="c^2 = a^2 + b^2" display="block"><mi>c</mi></math><script>alert(1)</script><a href="#anchor">link</a><table><tr><td>cell</td></tr></table></body></html>"##;
        let input = Input::bytes(html.as_bytes().to_vec(), "paper.html");
        let async_out = converter.convert(input.clone()).await.unwrap();
        let sync_out = converter.convert_sync(&input).unwrap();
        assert_eq!(async_out.markdown, sync_out.markdown);
        assert_eq!(async_out.quality, sync_out.quality);
        assert_eq!(async_out.metadata, sync_out.metadata);
    }

    /// Path 输入在同步口显式拒绝（facade 已先行守卫，此处为实现层防御）。
    #[test]
    fn convert_sync_rejects_path_input() {
        let converter = HtmlConverter::new();
        let err = converter
            .convert_sync(&Input::Path("/absent/page.html".into()))
            .unwrap_err();
        assert!(
            matches!(err, AlchemizeError::NotSyncCapable(_)),
            "expected NotSyncCapable, got {err:?}"
        );
    }
}
