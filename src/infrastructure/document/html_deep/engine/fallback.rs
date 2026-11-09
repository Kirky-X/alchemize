// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! HTML fallback：白名单净化序列化（行为规格 §4）。
//!
//! 触发点（machine 三处）：①code/pre 子树含链接或被引用锚点；②标题子树含
//! 块级元素；③结构表 &&（复杂表 ∨ 含锚点）∨ 结构表 && 有单元格。
//!
//! 产出"保结构、保锚点、白名单净化"的 HTML：脚本/样式/事件处理器/表现属性
//! 永不复制；`href`/`src` 过安全校验；首个锚点 target 变标签 `id` 属性，
//! 其余变前置 `<span id>`；表格系标签独立行书写（防 CommonMark HTML 块被
//! 空行截断）；presentational 表标签降级 `div`。

use std::collections::HashSet;
use std::fmt::Write as _;

use scraper::node::Node as SNode;

use super::anchors::{self, Targets};
use super::content;
use super::mathml;
use super::media;
use super::structure::attribute;
use super::structure::{is_structural_block, owns_boundaries};
use super::table::{self, has_cell_span};
use super::visibility;
use super::writer::is_space;

type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// Markdown 无合并单元格/嵌套表/块级单元格内容。各结构预扫描停在嵌套表处，
/// 嵌套作用域互不相交（各遍历有界线性）。
pub(super) fn needed(node: NodeRef<'_>, remaining: usize) -> bool {
    let mut stack: Vec<(NodeRef<'_>, usize)> = node.children().map(|c| (c, 1)).collect();
    while let Some((current, depth)) = stack.pop() {
        if depth >= remaining {
            continue;
        }
        if let SNode::Element(el) = current.value() {
            let tag = el.name();
            if matches!(
                tag,
                "table"
                    | "pre"
                    | "blockquote"
                    | "ul"
                    | "ol"
                    | "li"
                    | "hr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            ) {
                return true;
            }
            if matches!(tag, "td" | "th")
                && ["rowspan", "colspan"].iter().any(|attr| {
                    el.attr(attr)
                        .is_some_and(|value| has_cell_span(attr, value))
                })
            {
                return true;
            }
        }
        stack.extend(current.children().map(|c| (c, depth + 1)));
    }
    false
}

/// 表格是否带结构语义（保住作者标注的关系，即便没有表头单元格）：
/// role=table/grid/treegrid；行/单元格含 th/thead/caption；td/th 带
/// scope/headers/span。只看本表的单元格（嵌套表不扫）。
pub(super) fn requires_table_structure(node: NodeRef<'_>, remaining: usize) -> bool {
    let Some(el) = node.value().as_element() else {
        return false;
    };
    if el.attr("role").is_some_and(|role| {
        matches!(
            role.trim().to_ascii_lowercase().as_str(),
            "table" | "grid" | "treegrid"
        )
    }) {
        return true;
    }
    let mut stack: Vec<(NodeRef<'_>, usize)> = node.children().map(|c| (c, 1)).collect();
    while let Some((current, depth)) = stack.pop() {
        if depth >= remaining {
            continue;
        }
        if let SNode::Element(el) = current.value() {
            let tag = el.name();
            if tag == "table" {
                continue;
            }
            if matches!(tag, "th" | "thead" | "caption") {
                return true;
            }
            if matches!(tag, "td" | "th")
                && (el.attr("scope").is_some_and(|v| !v.trim().is_empty())
                    || el.attr("headers").is_some_and(|v| !v.trim().is_empty())
                    || ["rowspan", "colspan"].into_iter().any(|name| {
                        el.attr(name)
                            .is_some_and(|value| has_cell_span(name, value))
                    }))
            {
                return true;
            }
        }
        stack.extend(current.children().map(|c| (c, depth + 1)));
    }
    false
}

/// 表格子树是否含 th/td 单元格。
pub(super) fn has_cells(node: NodeRef<'_>, remaining: usize) -> bool {
    let mut stack: Vec<(NodeRef<'_>, usize)> = node.children().map(|c| (c, 1)).collect();
    while let Some((current, depth)) = stack.pop() {
        if depth >= remaining {
            continue;
        }
        if matches!(
            current.value(),
            SNode::Element(el) if matches!(el.name(), "th" | "td")
        ) {
            return true;
        }
        if let Some(sibling) = current.next_sibling() {
            stack.push((sibling, depth));
        }
        stack.extend(current.children().map(|c| (c, depth + 1)));
    }
    false
}

/// 文本模式：普通折叠 / code 数值实体 / pre 保留换行。
#[derive(Clone, Copy)]
enum TextMode {
    Normal,
    Code,
    Pre,
}

/// 渲染任务。
enum Task<'a> {
    Node(NodeRef<'a>, usize, TextMode),
    Siblings(Option<NodeRef<'a>>, usize, TextMode),
    MathSiblings(Option<NodeRef<'a>>, usize, TextMode),
    Close(String),
    EndTable,
    Fallback(String, usize),
}

/// 格式化上下文：调用方是否处于链接内、块级 fallback 是否在激活样式内
/// （`<div>` 包裹重建）。
pub(super) struct FormattingContext {
    pub(super) in_link: bool,
    pub(super) wrapped_block: bool,
}

/// fallback 产物：净化 HTML 片串 + 是否含内容。
pub(super) struct Html {
    pub(super) markup: String,
    pub(super) has_content: bool,
}

/// 渲染子树为净化 HTML。preformatted_code 选项固化 false（换行变空格语义
/// 不启用；CRLF 归一仅在 Code 模式，本仓恒关闭）。
pub(super) fn render(
    root: NodeRef<'_>,
    depth: usize,
    anchor_targets: &Targets,
    emitted_anchors: &mut HashSet<String>,
    formatting: FormattingContext,
) -> Html {
    let mut in_link = formatting.in_link;
    // 深度上限固化无限制（迭代遍历无栈风险）。
    let inline_root = matches!(root.value(), SNode::Element(el) if el.name() == "code");
    // 格式化块接收外层包裹。数值实体保留源换行且不产生截断 CommonMark HTML
    // 块的空行——仅未包裹的 pre 根。
    let keep_source_lines = !formatting.wrapped_block
        && matches!(root.value(), SNode::Element(el) if el.name() == "pre");
    let mut escaper = TextEscaper {
        inline_root,
        keep_newlines: keep_source_lines,
        trailing_cr: false,
    };
    let mut output = String::new();
    let mut content = 0usize;
    let mut table_modes: Vec<bool> = Vec::new();
    let mut tasks = vec![Task::Node(root, depth, TextMode::Normal)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::EndTable => {
                table_modes.pop();
            }
            Task::Fallback(label, before) => {
                if content == before {
                    content += 1;
                    output.push(' ');
                    escaper.write(&label, &mut output, TextMode::Normal);
                    output.push(' ');
                }
            }
            Task::Close(tag) => {
                if tag == "a" {
                    in_link = false;
                }
                if matches!(tag.as_str(), "table" | "thead" | "tbody" | "tfoot" | "tr") {
                    begin_html_line(&mut output);
                }
                output.push_str("</");
                output.push_str(&tag);
                output.push('>');
            }
            Task::Siblings(None, _, _) | Task::MathSiblings(None, _, _) => {}
            Task::Siblings(Some(node), depth, mode) => {
                tasks.push(Task::Siblings(node.next_sibling(), depth, mode));
                tasks.push(Task::Node(node, depth, mode));
            }
            Task::MathSiblings(Some(node), depth, mode) => {
                tasks.push(Task::MathSiblings(node.next_sibling(), depth, mode));
                if !mathml::is_hidden_alternative(node) {
                    tasks.push(Task::Node(node, depth, mode));
                }
            }
            Task::Node(node, depth, mode) => {
                if visibility::is_serialized_state(node) {
                    continue;
                }
                let in_code = !matches!(mode, TextMode::Normal);
                if in_code && content::is_code_control(node) {
                    continue;
                }
                if let SNode::Element(el) = node.value() {
                    let owns = allowed(el.name()) && owns_boundaries(el.name());
                    if !in_code && !owns {
                        // has_block_layout 固化 false：无样式表布局边界。
                        let _ = el;
                    }
                }
                let mode = match node.value() {
                    SNode::Element(el) if el.name() == "pre" => TextMode::Pre,
                    SNode::Element(el)
                        if el.name() == "code" && matches!(mode, TextMode::Normal) =>
                    {
                        escaper.trailing_cr = false;
                        TextMode::Code
                    }
                    _ => mode,
                };
                let in_code = !matches!(mode, TextMode::Normal);
                if let SNode::Element(el) = node.value()
                    && !allowed(el.name())
                {
                    for id in anchor_targets.targets(node) {
                        if emitted_anchors.insert(id.to_owned()) {
                            output.push_str(&anchors::markup(id));
                        }
                    }
                }
                match node.value() {
                    SNode::Text(text) => {
                        content += usize::from(!text.text.is_empty());
                        escaper.write(&text.text, &mut output, mode);
                    }
                    SNode::Element(el)
                        if matches!(
                            el.name(),
                            "head" | "title" | "script" | "style" | "noscript" | "template"
                        ) => {}
                    SNode::Element(el) if el.name() == "math" => {
                        // MathML 不经文本转义器：后续 LF 不能补全先前 CR。
                        escaper.trailing_cr = false;
                        content += 1;
                        for id in anchors::within(node, usize::MAX - depth, anchor_targets) {
                            if emitted_anchors.insert(id.clone()) {
                                output.push_str(&anchors::markup(&id));
                            }
                        }
                        output.push_str(&mathml::render(node, usize::MAX - depth));
                    }
                    SNode::Element(el) if el.name() == "input" => {
                        if let Some(value) = super::form::input_text(node) {
                            content += 1;
                            output.push(' ');
                            escaper.write(&value, &mut output, mode);
                            output.push(' ');
                        }
                    }
                    SNode::Element(el) if el.name() == "textarea" => {
                        // control_value 固化 None：遍历子树（文本已解码）。
                        tasks.push(Task::Siblings(node.first_child(), depth + 1, mode));
                    }
                    SNode::Element(el) => {
                        let tag = el.name();
                        if tag == "table" {
                            table_modes.push(table::is_presentational(node));
                            tasks.push(Task::EndTable);
                        }
                        if tag == "img" {
                            let label = content::image_label(node);
                            let targets: Vec<_> = anchor_targets
                                .targets(node)
                                .into_iter()
                                .filter(|id| emitted_anchors.insert((*id).to_owned()))
                                .collect();
                            if let Some(src) = media::source(node) {
                                content += 1;
                                for id in targets.iter().skip(1) {
                                    output.push_str(&anchors::markup(id));
                                }
                                output.push_str("<img src=\"");
                                escape_attribute(&src, &mut output);
                                output.push_str("\" alt=\"");
                                escape_attribute(&label, &mut output);
                                output.push('"');
                                if let Some(id) = targets.first() {
                                    output.push_str(" id=\"");
                                    escape_attribute(id, &mut output);
                                    output.push('"');
                                }
                                if let Some(title) = el.attr("title") {
                                    output.push_str(" title=\"");
                                    escape_attribute(title, &mut output);
                                    output.push('"');
                                }
                                output.push('>');
                            } else {
                                content += 1;
                                for id in targets {
                                    output.push_str(&anchors::markup(id));
                                }
                                escaper.write(&label, &mut output, mode);
                            }
                            continue;
                        }
                        if matches!(tag, "iframe" | "video" | "audio") {
                            let urls = media::sources(node, tag);
                            let poster = media::poster(node, tag);
                            if !urls.is_empty() || poster.is_some() {
                                content += 1;
                                let block = tag == "audio" && !in_code;
                                output.push_str(if block { "<div>" } else { " " });
                                if let Some(poster) = poster {
                                    output.push_str("<img src=\"");
                                    escape_attribute(&poster, &mut output);
                                    output.push_str("\" alt=\"");
                                    escape_attribute(&media::poster_label(node), &mut output);
                                    output.push_str("\"> ");
                                }
                                let has_source = !urls.is_empty();
                                for url in urls {
                                    if in_link {
                                        escaper.write(&media::label(node, tag), &mut output, mode);
                                        output.push_str(" (");
                                        escaper.write(&url, &mut output, mode);
                                        output.push_str(") ");
                                    } else {
                                        output.push_str("<a href=\"");
                                        escape_attribute(&url, &mut output);
                                        output.push_str("\">");
                                        escaper.write(&media::label(node, tag), &mut output, mode);
                                        output.push_str("</a> ");
                                    }
                                }
                                if block {
                                    output.push_str("</div>");
                                }
                                if has_source {
                                    continue;
                                }
                            }
                        }
                        let expanded = table_modes.last() == Some(&true)
                            && matches!(
                                tag,
                                "table"
                                    | "thead"
                                    | "tbody"
                                    | "tfoot"
                                    | "tr"
                                    | "td"
                                    | "th"
                                    | "caption"
                                    | "colgroup"
                                    | "col"
                            );
                        // 占位锚点展开为非链接目标，使生成后代可保留自己的链接。
                        let link_href = if tag == "a" {
                            media::link_href(node)
                        } else {
                            None
                        };
                        let retained = if expanded && matches!(tag, "colgroup" | "col") {
                            None
                        } else if expanded {
                            Some("div")
                        } else if allowed(tag)
                            && (tag != "a" || (!in_link && link_href.is_some()))
                            && (tag != "span"
                                || !anchor_targets.targets(node).is_empty()
                                || el.attr("title").is_some())
                        {
                            Some(tag)
                        } else {
                            None
                        };
                        if let Some(retained_tag) = retained {
                            if matches!(retained_tag, "br" | "hr") {
                                content += 1;
                            }
                            in_link |= retained_tag == "a";
                            if matches!(
                                retained_tag,
                                "table"
                                    | "thead"
                                    | "tbody"
                                    | "tfoot"
                                    | "tr"
                                    | "td"
                                    | "th"
                                    | "caption"
                                    | "colgroup"
                                    | "col"
                            ) {
                                begin_html_line(&mut output);
                            }
                            let targets: Vec<_> = anchor_targets
                                .targets(node)
                                .into_iter()
                                .filter(|id| emitted_anchors.insert((*id).to_owned()))
                                .collect();
                            for id in targets.iter().skip(1) {
                                output.push_str(&anchors::markup(id));
                            }
                            output.push('<');
                            output.push_str(retained_tag);
                            for name in [
                                "rowspan", "colspan", "scope", "headers", "href", "src", "alt",
                                "title", "start", "reversed", "value", "label", "selected",
                                "multiple", "disabled", "open", "span",
                            ] {
                                if expanded
                                    && matches!(name, "rowspan" | "colspan" | "scope" | "headers")
                                {
                                    continue;
                                }
                                let value = if name == "href" && retained_tag == "a" {
                                    link_href.clone()
                                } else if name == "src" && retained_tag == "img" {
                                    media::source(node)
                                } else {
                                    attribute(node, name)
                                };
                                if let Some(value) = value {
                                    if (name == "href"
                                        && retained_tag != "a"
                                        && !media::safe_link_url_pub(&value))
                                        || (name == "src" && !media::safe_url_pub(&value))
                                    {
                                        continue;
                                    }
                                    output.push(' ');
                                    output.push_str(name);
                                    output.push_str("=\"");
                                    escape_attribute(&value, &mut output);
                                    output.push('"');
                                }
                            }
                            if let Some(id) = targets.first() {
                                output.push_str(" id=\"");
                                escape_attribute(id, &mut output);
                                output.push('"');
                            }
                            let language = if retained_tag == "pre" {
                                content::code_language(node)
                            } else if retained_tag == "code" {
                                content::class_language(el.attr("class")).map(str::to_owned)
                            } else {
                                None
                            };
                            if let Some(language) = language.and_then(|value| {
                                value.split_ascii_whitespace().next().map(str::to_owned)
                            }) {
                                output.push_str(" class=\"language-");
                                escape_attribute(&language, &mut output);
                                output.push('"');
                            }
                            output.push('>');
                            // HTML 解析器丢弃 pre 后的首个 LF：与源文分开补。
                            if retained_tag == "pre" {
                                output.push('\n');
                            }
                            if matches!(retained_tag, "br" | "hr" | "img" | "col") {
                                continue;
                            }
                            tasks.push(Task::Close(retained_tag.to_owned()));
                        } else {
                            // 白名单外的支持标签仍可发射锚点。
                            for id in anchor_targets.targets(node) {
                                if emitted_anchors.insert(id.to_owned()) {
                                    output.push_str(&anchors::markup(id));
                                }
                            }
                        }
                        if mathml::primary_alternative(node, usize::MAX - depth).is_some() {
                            for id in anchors::within(node, usize::MAX - depth, anchor_targets) {
                                if emitted_anchors.insert(id.clone()) {
                                    output.push_str(&anchors::markup(&id));
                                }
                            }
                            tasks.push(Task::MathSiblings(node.first_child(), depth + 1, mode));
                        } else {
                            if matches!(mode, TextMode::Normal)
                                && let Some(label) =
                                    content::fallback_text(node, usize::MAX - depth)
                            {
                                tasks.push(Task::Fallback(label, content));
                            }
                            tasks.push(Task::Siblings(node.first_child(), depth + 1, mode));
                        }
                    }
                    SNode::Document | SNode::Fragment => {
                        tasks.push(Task::Siblings(node.first_child(), depth + 1, mode));
                    }
                    _ => {}
                }
            }
        }
    }
    Html {
        markup: output,
        has_content: content != 0,
    }
}

/// 保留标签白名单：全部结构块 + 行内/选择系/媒体兜底标签。
fn allowed(tag: &str) -> bool {
    is_structural_block(tag)
        || matches!(
            tag,
            "select"
                | "colgroup"
                | "col"
                | "optgroup"
                | "option"
                | "button"
                | "span"
                | "a"
                | "img"
                | "br"
                | "hr"
                | "strong"
                | "b"
                | "em"
                | "i"
                | "s"
                | "strike"
                | "del"
                | "sup"
                | "sub"
                | "code"
        )
}

/// 属性值转义：`|` 数值实体（GFM 管道转义独立于原始 HTML 属性）。
pub(super) fn escape_attribute(text: &str, output: &mut String) {
    for ch in text.chars() {
        if ch == '|' {
            output.push_str("&#124;");
        } else {
            escape_character(ch, output);
        }
    }
}

fn escape_character(ch: char, output: &mut String) {
    match ch {
        '&' => output.push_str("&amp;"),
        '<' => output.push_str("&lt;"),
        '>' => output.push_str("&gt;"),
        '"' => output.push_str("&quot;"),
        // 原始 HTML 块止于空 Markdown 行；实体保留预格式换行而不提前结束块。
        '\n' => output.push_str("&#10;"),
        '\r' => output.push_str("&#13;"),
        _ => output.push(ch),
    }
}

/// 文本转义器：普通模式空白折叠单空格；code 模式 ASCII 标点数值实体
/// （inline_root）；pre 根保留真实换行。
struct TextEscaper {
    inline_root: bool,
    keep_newlines: bool,
    trailing_cr: bool,
}

impl TextEscaper {
    fn write(&mut self, text: &str, output: &mut String, mode: TextMode) {
        let code = !matches!(mode, TextMode::Normal);
        for ch in text.chars() {
            if !code && matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
                if !output.ends_with([' ', '\n']) {
                    output.push(' ');
                }
                continue;
            }
            if self.keep_newlines && code && ch == '\n' {
                output.push(ch);
            } else if code && self.inline_root && ch.is_ascii_punctuation() {
                let _ = write!(output, "&#{};", ch as u32);
            } else {
                escape_character(ch, output);
            }
            let _ = &mut self.trailing_cr;
        }
    }
}

/// 表格系标签独立行书写：剥尾空格、必要时补换行。
fn begin_html_line(output: &mut String) {
    while output.ends_with(' ') {
        output.pop();
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
}

#[allow(unused)]
fn _is_space_used(value: &str) -> bool {
    value.chars().all(is_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(html: &str) -> scraper::Html {
        scraper::Html::parse_document(html)
    }

    fn find<'a>(doc: &'a scraper::Html, tag: &'static str) -> NodeRef<'a> {
        doc.tree
            .nodes()
            .find(|n| matches!(n.value(), SNode::Element(e) if e.name() == tag))
            .unwrap_or_else(|| panic!("fixture must contain <{tag}>"))
    }

    #[test]
    fn needed_detects_block_in_table() {
        // needed 清单不含 p（p 由块边界处理）；含 ul/pre/blockquote 等。
        let doc = parse("<table><tr><td><ul><li>x</li></ul></td></tr></table>");
        assert!(needed(find(&doc, "table"), usize::MAX));
        let doc = parse("<table><tr><td>plain</td></tr></table>");
        assert!(!needed(find(&doc, "table"), usize::MAX));
    }

    #[test]
    fn requires_structure_variants() {
        let doc = parse("<table><thead><tr><td>a</td></tr></thead></table>");
        assert!(requires_table_structure(find(&doc, "table"), usize::MAX));
        let doc = parse("<table role=\"table\"><tr><td>a</td></tr></table>");
        assert!(requires_table_structure(find(&doc, "table"), usize::MAX));
        let doc = parse("<table><tr><td scope=\"row\">a</td></tr></table>");
        assert!(requires_table_structure(find(&doc, "table"), usize::MAX));
        let doc = parse("<table><tr><td>plain</td></tr></table>");
        assert!(!requires_table_structure(find(&doc, "table"), usize::MAX));
    }

    #[test]
    fn has_cells_check() {
        let doc = parse("<table><tr><td>a</td></tr></table>");
        assert!(has_cells(find(&doc, "table"), usize::MAX));
    }

    #[test]
    fn script_never_copied() {
        let doc = parse("<div><script>alert(1)</script><p>keep</p></div>");
        let out = render(
            find(&doc, "div"),
            0,
            &Targets::scan(doc.tree.root()),
            &mut HashSet::new(),
            FormattingContext {
                in_link: false,
                wrapped_block: false,
            },
        );
        assert!(!out.markup.contains("alert"), "{}", out.markup);
        assert!(out.markup.contains("keep"), "{}", out.markup);
        assert!(out.has_content);
    }

    #[test]
    fn anchor_becomes_id_attribute() {
        let doc = parse(r##"<div id="target"><p>body</p></div><a href="#target">go</a>"##);
        let targets = Targets::scan(doc.tree.root());
        let mut emitted = HashSet::new();
        let out = render(
            find(&doc, "div"),
            0,
            &targets,
            &mut emitted,
            FormattingContext {
                in_link: false,
                wrapped_block: false,
            },
        );
        assert!(
            out.markup.starts_with("<div id=\"target\">"),
            "{}",
            out.markup
        );
    }

    #[test]
    fn pre_preserves_source_newlines() {
        let doc = parse("<pre><code>a\nb</code></pre>");
        let out = render(
            find(&doc, "pre"),
            0,
            &Targets::scan(doc.tree.root()),
            &mut HashSet::new(),
            FormattingContext {
                in_link: false,
                wrapped_block: false,
            },
        );
        assert!(out.markup.contains("a\nb"), "{}", out.markup);
        assert!(out.markup.contains("<pre>"), "{}", out.markup);
    }

    #[test]
    fn inline_code_ascii_punct_numeric() {
        let doc = parse("<code>a*b&c</code>");
        let out = render(
            find(&doc, "code"),
            0,
            &Targets::scan(doc.tree.root()),
            &mut HashSet::new(),
            FormattingContext {
                in_link: false,
                wrapped_block: false,
            },
        );
        assert!(out.markup.contains("a&#42;b&#38;c"), "{}", out.markup);
    }
}
