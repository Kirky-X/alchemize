// SPDX-License-Identifier: Apache-2.0
//! LaTeX source (.tex/.latex) to Markdown converter via pandoc, with
//! math-delimiter normalization and div-noise stripping (both ported from
//! the liuxiang pipeline).

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult, VideoProcessingMode};

/// Converts LaTeX source files to markdown.
///
/// pandoc available → full conversion (quality `LocalFull`): stdin → GFM,
/// `--extract-media` assets collected into `ConvertResult.assets` with
/// references rewritten to `images/…`. pandoc missing → source wrapped in a
/// fenced block (quality `LocalFallback`).
pub struct LatexSourceConverter {
    pandoc_available: bool,
}

impl LatexSourceConverter {
    /// Probe pandoc at construction time.
    pub fn new() -> Self {
        Self {
            pandoc_available: crate::infrastructure::tools::pandoc::PandocTool::probe()
                == Availability::Available,
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn unused(_mode: VideoProcessingMode) {}
}

impl Default for LatexSourceConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for LatexSourceConverter {
    fn name(&self) -> &'static str {
        "latex-source"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read LaTeX input: {e}")))?;

        let latex = String::from_utf8_lossy(&bytes).into_owned();

        if !self.pandoc_available {
            let markdown = format!("```latex\n{latex}\n```\n");
            let warnings = crate::domain::latex_validate::validate_markdown_math(&markdown);
            return Ok(ConvertResult::with_metadata(
                markdown,
                ContentType::Document,
                serde_json::json!({"source_format": "tex", "engine": "fenced-fallback", "latex_warnings": warnings}),
            )
            .with_quality(ConversionQuality::LocalFallback));
        }

        let media_dir = tempfile::tempdir()
            .map_err(|e| AlchemizeError::other(format!("tempdir failed: {e}")))?;
        let raw = crate::infrastructure::tools::pandoc::PandocTool::latex_to_gfm(
            &latex,
            Some(media_dir.path()),
        )
        .await
        .map_err(|e| AlchemizeError::other(format!("pandoc failed: {e}")))?;

        let mut markdown = normalize_pandoc_math(&raw);
        markdown = strip_latex_div_noise(&markdown);
        // pandoc prefixes extracted media with the temp dir; point at images/.
        let media_root = media_dir.path().display().to_string();
        markdown = markdown.replace(&format!("{media_root}/"), "images/");

        let mut assets = Vec::new();
        collect_media_files(media_dir.path(), "", &mut assets);

        let warnings = crate::domain::latex_validate::validate_markdown_math(&markdown);
        let metadata = serde_json::json!({
            "source_format": "tex",
            "engine": "pandoc",
            "asset_count": assets.len(),
            "latex_warnings": warnings,
        });

        let mut result = ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
            .with_quality(ConversionQuality::LocalFull);
        for asset in assets {
            result = result.with_asset(asset);
        }
        Ok(result)
    }

    fn supported_extensions(&self) -> &[&str] {
        &["tex", "latex"]
    }

    fn availability(&self) -> Availability {
        // The converter always converts: without pandoc it degrades to a
        // fenced LaTeX block (quality LocalFallback). Unavailable is reserved
        // for engines that cannot produce output at all.
        Availability::Available
    }
}

/// Recursively collect media files under `root`, mapping each to an asset
/// whose filename is its path relative to `root` under `images/`.
fn collect_media_files(root: &std::path::Path, rel: &str, out: &mut Vec<crate::domain::asset::Asset>) {
    let dir = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            let sub_rel = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            collect_media_files(root, &sub_rel, out);
        } else if path.is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            let filename = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if let Ok(bytes) = std::fs::read(&path) {
                out.push(crate::domain::asset::Asset::new(filename, bytes));
            }
        }
    }
}

/// Normalize pandoc's GFM math delimiters to standard Markdown math.
///
/// - Inline `` $`…`$ `` → `$…$`
/// - Block ```` ```math \begin{equation}… ```` fences → `$$…$$` with the
///   equation environment wrappers and `\label{…}` removed
/// - Stray `\label{…}` removed everywhere
pub fn normalize_pandoc_math(md: &str) -> String {
    let mut out = md.to_string();

    // Inline: $`...`$ -> $...$ (backslash-escaped backticks stay literal).
    out = replace_inline_dollar_backtick(&out);

    // Block: fenced math environments -> $$...$$
    out = replace_fenced_environments(&out);

    // Stray \label{...}
    out = strip_command_arg(&out, "\\label");

    out
}

fn replace_inline_dollar_backtick(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let chars: Vec<(usize, char)> = md.char_indices().collect();
    let mut ci = 0usize;
    while ci < chars.len() {
        let (i, ch) = chars[ci];
        if ch == '$' && ci + 1 < chars.len() && chars[ci + 1].1 == '`' {
            let start_inner = chars[ci + 1].0 + '`'.len_utf8();
            let mut j = ci + 2;
            let mut inner_end = None;
            while j + 1 < chars.len() {
                if chars[j].1 == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j].1 == '`' && chars[j + 1].1 == '$' {
                    inner_end = Some(chars[j].0);
                    break;
                }
                j += 1;
            }
            if let Some(end) = inner_end {
                out.push('$');
                out.push_str(&md[start_inner..end]);
                out.push('$');
                // advance past the closing backtick+$
                ci = j + 2;
                continue;
            }
        }
        out.push(ch);
        ci += 1;
    }
    out
}

fn replace_fenced_environments(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    loop {
        let Some(fence_start) = rest.find("```") else {
            out.push_str(rest);
            return out;
        };
        let line_end = match rest[fence_start..].find('\n') {
            Some(rel) => fence_start + rel,
            None => {
                out.push_str(rest);
                return out;
            }
        };
        let info = rest[fence_start + 3..line_end].trim();
        if info != "math" && info != "latex" && info != "tex" && !info.is_empty() {
            out.push_str(&rest[..line_end + 1]);
            rest = &rest[line_end + 1..];
            continue;
        }
        let Some(close_rel) = rest[line_end + 1..].find("\n```") else {
            out.push_str(rest);
            return out;
        };
        let body = &rest[line_end + 1..line_end + 1 + close_rel];
        let after = &rest[line_end + 1 + close_rel + 4..];

        if body.contains("\\begin{equation}")
            || body.contains("\\begin{align")
            || body.contains("\\begin{gather")
            || body.contains("\\begin{eqnarray}")
            || body.contains("\\begin{multline}")
            || body.contains("\\begin{cases}")
        {
            let inner = strip_environment_wrappers(body);
            out.push_str("$$\n");
            out.push_str(inner.trim());
            out.push_str("\n$$");
            // skip the rest of the closing fence line
            let skip = after.find('\n').map(|n| n + 1).unwrap_or(after.len());
            out.push('\n');
            rest = &after[skip..];
        } else {
            out.push_str(&rest[..line_end + 1]);
            rest = &rest[line_end + 1..];
        }
    }
}

const ENVIRONMENTS: &[&str] = &[
    "equation", "equation*", "eqnarray", "eqnarray*", "align", "align*", "gather", "gather*",
    "multline", "multline*", "cases", "split",
];

fn strip_environment_wrappers(body: &str) -> String {
    let mut inner = body.to_string();
    for env in ENVIRONMENTS {
        inner = strip_command_arg_pair(&inner, "begin", env);
        inner = strip_command_arg_pair(&inner, "end", env);
    }
    inner
}

/// Remove `\cmd{arg}` occurrences (single-line arg, no nested braces).
fn strip_command_arg(text: &str, cmd: &str) -> String {
    let needle = format!("{cmd}{{");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(&needle) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + needle.len()..];
        match after.find('}') {
            Some(end) => {
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[pos..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Remove `\begin{env}` / `\end{env}` for one environment name.
fn strip_command_arg_pair(text: &str, cmd: &str, env: &str) -> String {
    text.replace(&format!("\\{cmd}{{{env}}}"), "")
}

/// Strip pandoc's LaTeX-environment div noise and convert image tags.
pub fn strip_latex_div_noise(md: &str) -> String {
    let mut out = remove_ccsxml_blocks(md);
    out = strip_div_tags(&out);
    out = convert_img_tags(&out);
    out = convert_embed_tags(&out);
    out = strip_figure_tags(&out);
    out = convert_figcaption(&out);
    out = collapse_blank_lines(&out);
    out
}

fn remove_ccsxml_blocks(md: &str) -> String {
    let marker = "<div class=\"CCSXML\">";
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(pos) = rest.find(marker) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        match after.find("</div>") {
            Some(end) => rest = &after[end + "</div>".len()..],
            None => {
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_div_tags(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(pos) = rest.find('<') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let Some(gt) = after.find('>') else {
            out.push_str(after);
            return out;
        };
        let tag = &after[..gt + 1];
        if tag.starts_with("<div") {
            // Drop the tag, keep content.
            rest = &after[gt + 1..];
        } else if tag.starts_with("</div") {
            out.push('\n');
            rest = &after[gt + 1..];
        } else {
            out.push_str(tag);
            rest = &after[gt + 1..];
        }
    }
    out.push_str(rest);
    out
}

fn src_of_tag(tag: &str) -> Option<String> {
    let idx = tag.find("src=")?;
    let after = &tag[idx + 4..];
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = after[1..].find(quote)? + 1;
    Some(after[1..end].to_string())
}

fn convert_img_tags(md: &str) -> String {
    convert_tag_to_image(md, "<img")
}

fn convert_embed_tags(md: &str) -> String {
    convert_tag_to_image(md, "<embed")
}

fn convert_tag_to_image(md: &str, open: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(pos) = rest.find(open) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let Some(gt) = after.find('>') else {
            out.push_str(after);
            return out;
        };
        let tag = &after[..gt + 1];
        if let Some(src) = src_of_tag(tag) {
            out.push_str(&format!("![figure]({src})"));
        } else {
            out.push_str(tag);
        }
        rest = &after[gt + 1..];
    }
    out.push_str(rest);
    out
}

fn strip_figure_tags(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(pos) = rest.find('<') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let Some(gt) = after.find('>') else {
            out.push_str(after);
            return out;
        };
        let tag = &after[..gt + 1];
        if tag.starts_with("<figure") {
            rest = &after[gt + 1..];
        } else if tag.starts_with("</figure") {
            out.push('\n');
            rest = &after[gt + 1..];
        } else {
            out.push_str(tag);
            rest = &after[gt + 1..];
        }
    }
    out.push_str(rest);
    out
}

fn convert_figcaption(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(pos) = rest.find("<figcaption") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let (Some(open_end), Some(close_rel)) = (after.find('>'), after.find("</figcaption>"))
        else {
            out.push_str(after);
            return out;
        };
        let inner = &after[open_end + 1..close_rel];
        let plain = crate::infrastructure::document::html::strip_tags(inner);
        out.push_str(&format!("\n*{plain}*\n"));
        rest = &after[close_rel + "</figcaption>".len()..];
    }
    out.push_str(rest);
    out
}

fn collapse_blank_lines(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut run = 0usize;
    for ch in md.chars() {
        if ch == '\n' {
            run += 1;
            if run <= 2 {
                out.push(ch);
            }
        } else {
            run = 0;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert_eq!(
            LatexSourceConverter::new().supported_extensions(),
            &["tex", "latex"]
        );
    }

    // -- normalize_pandoc_math ------------------------------------------------

    #[test]
    fn inline_dollar_backtick_normalized() {
        let md = "value $`x + y`$ here";
        assert_eq!(normalize_pandoc_math(md), "value $x + y$ here");
    }

    #[test]
    fn inline_escaped_backtick_kept() {
        let md = r"odd $`a \` b`$ tail";
        let out = normalize_pandoc_math(md);
        assert!(out.contains("$a \\` b$"), "got: {out}");
    }

    #[test]
    fn fenced_equation_becomes_display_math() {
        let md = "```math\n\\begin{equation}\nE = mc^2\n\\label{eq:e}\n\\end{equation}\n```\n";
        let out = normalize_pandoc_math(md);
        assert!(out.contains("$$"), "display math present: {out}");
        assert!(out.contains("E = mc^2"));
        assert!(!out.contains("\\label"));
        assert!(!out.contains("\\begin{equation}"));
    }

    #[test]
    fn stray_label_removed() {
        assert_eq!(normalize_pandoc_math(r"a\label{eq:x}b"), "ab");
    }

    #[test]
    fn plain_code_fence_untouched() {
        let md = "```rust\nfn main() {}\n```\n";
        assert_eq!(normalize_pandoc_math(md), md);
    }

    // -- strip_latex_div_noise --------------------------------------------------

    #[test]
    fn ccsxml_block_removed_entirely() {
        let md = "before\n<div class=\"CCSXML\">junk inner</div>\nafter";
        let out = strip_latex_div_noise(md);
        assert!(!out.contains("CCSXML"));
        assert!(!out.contains("junk"));
        assert!(out.contains("before"));
        assert!(out.contains("after"));
    }

    #[test]
    fn div_tags_stripped_content_kept() {
        let md = "<div class=\"center\">\ncontent line\n</div>\n";
        let out = strip_latex_div_noise(md);
        assert!(!out.contains("<div"));
        assert!(out.contains("content line"));
    }

    #[test]
    fn img_tag_becomes_markdown_image() {
        let md = "<img src=\"media/fig1.png\" alt=\"fig\" />";
        let out = strip_latex_div_noise(md);
        assert!(out.contains("![figure](media/fig1.png)"), "got: {out}");
    }

    #[test]
    fn embed_tag_becomes_markdown_image() {
        let md = "<embed src=\"media/fig2.pdf\" />";
        let out = strip_latex_div_noise(md);
        assert!(out.contains("![figure](media/fig2.pdf)"), "got: {out}");
    }

    #[test]
    fn figcaption_becomes_italic_caption() {
        let md = "<figure>\n<img src=\"media/x.png\" />\n<figcaption>Result plot</figcaption>\n</figure>\n";
        let out = strip_latex_div_noise(md);
        assert!(out.contains("*Result plot*"), "got: {out}");
        assert!(!out.contains("<figcaption"));
        assert!(!out.contains("<figure"));
    }

    #[test]
    fn blank_lines_collapsed() {
        let md = "a\n\n\n\n\nb";
        let out = strip_latex_div_noise(md);
        assert_eq!(out, "a\n\nb");
    }

    // -- fallback branch ---------------------------------------------------------

    #[tokio::test]
    async fn fenced_fallback_when_pandoc_missing() {
        let converter = LatexSourceConverter {
            pandoc_available: false,
        };
        let input = Input::bytes(b"\\section{Hi}".to_vec(), "paper.tex");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.starts_with("```latex\n"));
        assert_eq!(result.quality, ConversionQuality::LocalFallback);
        // The converter always converts (fenced fallback is built in):
        // availability stays Available; pandoc status shows in metadata.
        assert_eq!(converter.availability(), Availability::Available);
    }
}
