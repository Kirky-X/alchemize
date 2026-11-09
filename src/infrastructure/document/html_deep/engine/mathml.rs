// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! MathML 净化渲染与 MathJax 双拷贝识别（行为规格 §5b）。
//!
//! 本引擎**不做** MathML→LaTeX 转换：MathML 以净化后的单行 MathML 行内 HTML
//! 输出（`alttext` 不在属性白名单内被丢弃——宿主把含 alttext 的文档整体
//! 路由给 math 保真引擎）。仅数学标记与表现属性存活；其他编码、HTML、
//! 脚本与事件处理器不能激活。

use std::fmt::Write as _;

use scraper::node::Node as SNode;

type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// MathJax 双拷贝识别：子节点中恰一个 aria-hidden 视觉拷贝 + 恰一个语义
/// 分支（穿过 ≤ min(limit,9) 层单子包装后到达 `<math>` 元素）。命中返回
/// 包装根——主路径随即跳过隐藏拷贝、沿语义分支走普通遍历（保留链接与
/// 样式上下文）。
pub(super) fn primary_alternative(node: NodeRef<'_>, remaining: usize) -> Option<NodeRef<'_>> {
    let mut primary: Option<NodeRef<'_>> = None;
    let mut alternative = false;
    let mut child = node.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if ignorable(current) {
            continue;
        }
        if is_hidden_alternative(current) {
            alternative = true;
            continue;
        }
        if primary.is_some() {
            return None;
        }
        // 包装层数有界：深层普通文本不触发反复全子树扫描。
        let mut candidate = current;
        let mut found: Option<NodeRef<'_>> = None;
        for _ in 1..remaining.min(9) {
            if matches!(
                candidate.value(),
                SNode::Element(el) if el.name() == "math"
            ) {
                found = Some(current);
                break;
            }
            let mut single: Option<NodeRef<'_>> = None;
            let mut grand = candidate.first_child();
            while let Some(g) = grand {
                grand = g.next_sibling();
                if ignorable(g) {
                    continue;
                }
                if single.is_some() {
                    return None;
                }
                single = Some(g);
            }
            candidate = single?;
        }
        primary = found;
    }
    alternative.then_some(primary).flatten()
}

/// 该节点是否为 aria-hidden 视觉拷贝。
pub(super) fn is_hidden_alternative(node: NodeRef<'_>) -> bool {
    node.value().as_element().is_some_and(|el| {
        el.attr("aria-hidden")
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    })
}

/// 可忽略子节点：注释/doctype/PI、纯空白文本。
fn ignorable(node: NodeRef<'_>) -> bool {
    match node.value() {
        SNode::Text(text) => text.text.trim().is_empty(),
        SNode::Comment(_) | SNode::Doctype(_) | SNode::ProcessingInstruction(_) => true,
        _ => false,
    }
}

/// 渲染任务。
enum Task<'a> {
    Node(NodeRef<'a>, usize),
    Sibling(Option<NodeRef<'a>>, usize),
    Close(&'static str),
}

/// 把 `<math>` 子树净化为单行 MathML 行内 HTML：元素/属性白名单、未知容器
/// 改写 `mrow`（保住分数/上下标的子位）、`semantics`/`maction` 只取首个
/// 元素子（`maction` 外包 `<mrow>` 保单位置）、Text 按上下文转义单行化。
pub(super) fn render(root: NodeRef<'_>, max_depth: usize) -> String {
    let mut output = String::new();
    let mut tasks = vec![Task::Node(root, 0)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::Close(tag) => {
                output.push_str("</");
                output.push_str(tag);
                output.push('>');
            }
            Task::Sibling(Some(node), depth) => {
                tasks.push(Task::Sibling(node.next_sibling(), depth));
                tasks.push(Task::Node(node, depth));
            }
            Task::Sibling(None, _) => {}
            Task::Node(node, depth) => {
                if depth >= max_depth {
                    continue;
                }
                match node.value() {
                    SNode::Text(text) => escape(&text.text, &mut output, true),
                    SNode::Element(el)
                        if matches!(
                            el.name(),
                            "annotation" | "annotation-xml" | "script" | "style"
                        ) => {}
                    SNode::Element(el) if matches!(el.name(), "semantics" | "maction") => {
                        // presentation 是首个元素子；后续 annotation 是替代表示。
                        // maction 显示其首个元素；替换体必须保持单位置参数。
                        if el.name() == "maction" {
                            output.push_str("<mrow>");
                            tasks.push(Task::Close("mrow"));
                        }
                        let mut child = node.first_child();
                        while let Some(c) = child {
                            if matches!(c.value(), SNode::Element(_)) {
                                tasks.push(Task::Node(c, depth + 1));
                                break;
                            }
                            child = c.next_sibling();
                        }
                    }
                    SNode::Element(el) => {
                        // 未知容器仍占分数/上下标/根式的一个子位：保留分组。
                        let name: &'static str = if allowed_tag(el.name()) {
                            static_tag(el.name())
                        } else {
                            "mrow"
                        };
                        output.push('<');
                        output.push_str(name);
                        for attr in ATTRIBUTES {
                            if let Some(value) = el.attr(attr) {
                                output.push(' ');
                                output.push_str(attr);
                                output.push_str("=\"");
                                escape(value, &mut output, false);
                                output.push('"');
                            }
                        }
                        output.push('>');
                        tasks.push(Task::Close(name));
                        tasks.push(Task::Sibling(node.first_child(), depth + 1));
                    }
                    SNode::Document | SNode::Fragment => {
                        tasks.push(Task::Sibling(node.first_child(), depth + 1));
                    }
                    _ => {}
                }
            }
        }
    }
    output
}

/// 白名单标签提升为 'static（字面量等价替换）。
fn static_tag(tag: &str) -> &'static str {
    // allowed_tag 已确认 tag ∈ 白名单；白名单字面量即 'static。
    for candidate in ALL_TAGS {
        if *candidate == tag {
            return candidate;
        }
    }
    "mrow"
}

/// 白名单标签全集（static_tag 查表用）。
const ALL_TAGS: &[&str] = &[
    "math",
    "mi",
    "mn",
    "mo",
    "mtext",
    "ms",
    "mspace",
    "mrow",
    "mfrac",
    "msqrt",
    "mroot",
    "mstyle",
    "merror",
    "mpadded",
    "mphantom",
    "mfenced",
    "menclose",
    "msub",
    "msup",
    "msubsup",
    "munder",
    "mover",
    "munderover",
    "mmultiscripts",
    "mprescripts",
    "none",
    "mtable",
    "mtr",
    "mlabeledtr",
    "mtd",
    "maligngroup",
    "malignmark",
];

/// MathML 表现元素白名单。
fn allowed_tag(tag: &str) -> bool {
    matches!(
        tag,
        "math"
            | "mi"
            | "mn"
            | "mo"
            | "mtext"
            | "ms"
            | "mspace"
            | "mrow"
            | "mfrac"
            | "msqrt"
            | "mroot"
            | "mstyle"
            | "merror"
            | "mpadded"
            | "mphantom"
            | "mfenced"
            | "menclose"
            | "msub"
            | "msup"
            | "msubsup"
            | "munder"
            | "mover"
            | "munderover"
            | "mmultiscripts"
            | "mprescripts"
            | "none"
            | "mtable"
            | "mtr"
            | "mlabeledtr"
            | "mtd"
            | "maligngroup"
            | "malignmark"
    )
}

/// 表现属性白名单（27 个）。
const ATTRIBUTES: &[&str] = &[
    "display",
    "mathvariant",
    "displaystyle",
    "scriptlevel",
    "linethickness",
    "bevelled",
    "open",
    "close",
    "separators",
    "notation",
    "accent",
    "accentunder",
    "stretchy",
    "symmetric",
    "largeop",
    "movablelimits",
    "form",
    "fence",
    "separator",
    "rowspan",
    "columnspan",
    "rowalign",
    "columnalign",
    "rowlines",
    "columnlines",
    "lquote",
    "rquote",
];

/// 文本/属性值转义：Markdown 文本类字符数值实体化（防 Markdown 解释），
/// XML 实体化，换行/制表符折叠空格（行内 HTML 片段必须单行）。
fn escape(text: &str, output: &mut String, markdown_text: bool) {
    for ch in text.chars() {
        match ch {
            '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '$' if markdown_text => {
                let _ = write!(output, "&#{};", ch as u32);
            }
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\n' | '\r' | '\t' => output.push(' '),
            _ => output.push(ch),
        }
    }
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
    fn render_keeps_whitelisted_elements_and_attrs() {
        let doc = parse(
            r#"<math display="block"><mfrac linethickness="2"><mi>a</mi><mn>2</mn></mfrac></math>"#,
        );
        let out = render(find(&doc, "math"), 100);
        assert!(out.contains(r#"<math display="block">"#), "{out}");
        assert!(out.contains(r#"<mfrac linethickness="2">"#), "{out}");
        assert!(out.contains("<mi>a</mi>"), "{out}");
        assert!(out.contains("</mfrac>"), "{out}");
    }

    #[test]
    fn unknown_tag_becomes_mrow() {
        let doc = parse(r#"<math><weird>x</weird></math>"#);
        let out = render(find(&doc, "math"), 100);
        assert!(out.contains("<mrow>x</mrow>"), "{out}");
        assert!(!out.contains("weird"), "{out}");
    }

    #[test]
    fn annotation_dropped_and_semantics_takes_first_element() {
        let doc = parse(
            r#"<math><semantics><mi>x</mi><annotation encoding="TeX">x</annotation></semantics></math>"#,
        );
        let out = render(find(&doc, "math"), 100);
        assert!(out.contains("<mi>x</mi>"), "{out}");
        assert!(!out.contains("annotation"), "{out}");
        assert!(!out.contains("TeX"), "{out}");
    }

    #[test]
    fn text_escaped_as_numeric_entities_single_line() {
        let doc = parse(r#"<math><mtext>a*b&#10;c</mtext></math>"#);
        let out = render(find(&doc, "math"), 100);
        assert!(out.contains("a&#42;b c"), "{out}");
    }

    #[test]
    fn mathjax_double_copy_detected() {
        let doc = parse(
            r#"<span><span aria-hidden="true">x²</span><span><math><mi>x</mi></math></span></span>"#,
        );
        // 外层 span 是文档序首个 span（find 直达）；其子含 aria-hidden 拷贝。
        let outer = find(&doc, "span");
        assert!(primary_alternative(outer, 100).is_some());
    }

    #[test]
    fn plain_element_not_double_copy() {
        let doc = parse(r#"<span><span>just text</span></span>"#);
        let span = find(&doc, "span");
        let outer = span.parent().unwrap();
        assert!(primary_alternative(outer, 100).is_none());
    }

    #[test]
    fn two_semantic_branches_rejected() {
        let doc = parse(
            r#"<span><span aria-hidden="true">x</span><math><mi>a</mi></math><math><mi>b</mi></math></span>"#,
        );
        let span = find(&doc, "span");
        assert!(primary_alternative(span, 100).is_none());
    }
}
