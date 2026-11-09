// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! HTML 元素分类表与统一有界子树谓词遍历。
//!
//! 三套分类（行为规格 §2/§9）：
//! - [`is_structural_block`]：文档结构块——产块边界、参与"标题/表格含块 →
//!   HTML fallback"与"复杂表"判定；
//! - [`is_block`]：结构块 ∪ 少数自成一块的媒体/框架元素；
//! - [`owns_boundaries`]：结构块 ∪ 表格系/选择系——这些元素自持文档/列表/
//!   选择边界，状态机不再叠加样式表布局边界。
//!
//! [`subtree_has`] 是六个同构有界 DFS 的统一实现：迭代栈、深度上限受控、
//! 命中即短路。

/// 文档结构块（块边界 + fallback/表格判定参与元素）。
pub(crate) fn is_structural_block(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "caption"
            | "center"
            | "dd"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "html"
            | "legend"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}

/// 块元素：结构块 ∪ 自成一块的媒体/框架元素。
pub(crate) fn is_block(tag: &str) -> bool {
    is_structural_block(tag)
        || matches!(
            tag,
            "audio" | "canvas" | "frameset" | "isindex" | "noframes" | "output"
        )
}

/// 自持文档/列表/选择边界的元素（状态机不叠加样式表布局边界）。
pub(crate) fn owns_boundaries(tag: &str) -> bool {
    is_structural_block(tag) || matches!(tag, "select" | "optgroup" | "option" | "col" | "colgroup")
}

/// 属性查询：大小写不敏感匹配（html5ever 已小写化，双保险），值已解码。
pub(crate) fn attribute(
    node: ego_tree::NodeRef<'_, scraper::node::Node>,
    name: &str,
) -> Option<String> {
    node.value()
        .as_element()?
        .attrs()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.to_string())
}

/// 有界迭代子树谓词：从 `root` 的直接子节点开始迭代下沉（根的子节点算第 1 层，
/// 最多访问第 `max_depth` 层），任一**元素**节点命中 `pred` 即返回 true。
/// 谓词收到元素节点引用；命中即短路，全树最多访问一次。
pub(crate) fn subtree_has<'a, F>(
    root: ego_tree::NodeRef<'a, scraper::node::Node>,
    max_depth: usize,
    pred: F,
) -> bool
where
    F: FnMut(ego_tree::NodeRef<'a, scraper::node::Node>) -> bool,
{
    // (节点, 本层允许的层数)：根的子节点为第 1 层。
    let mut stack: Vec<(ego_tree::NodeRef<'a, scraper::node::Node>, usize)> =
        root.children().map(|c| (c, max_depth)).collect();
    let mut pred = pred;
    while let Some((node, level)) = stack.pop() {
        let is_element = matches!(node.value(), scraper::node::Node::Element(_));
        if is_element && pred(node) {
            return true;
        }
        let descend = match node.value() {
            scraper::node::Node::Element(_)
            | scraper::node::Node::Document
            | scraper::node::Node::Fragment => level > 1,
            _ => false,
        };
        if descend {
            let next = level - 1;
            stack.extend(node.children().map(|c| (c, next)));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::node::Node as SNode;

    fn el_tag_is(tag: &'static str) -> impl Fn(ego_tree::NodeRef<'_, SNode>) -> bool {
        move |n| matches!(n.value(), SNode::Element(e) if e.name() == tag)
    }

    fn parse(html: &str) -> scraper::Html {
        scraper::Html::parse_document(html)
    }

    fn first_element<'a>(doc: &'a scraper::Html, tag: &str) -> ego_tree::NodeRef<'a, SNode> {
        doc.tree
            .nodes()
            .find(|n| matches!(n.value(), SNode::Element(e) if e.name() == tag))
            .unwrap_or_else(|| panic!("fixture must contain <{tag}>"))
    }

    #[test]
    fn classification_tables_cover_decision_matrix() {
        // 结构块抽检（决策表直接涉及的类别）
        for tag in [
            "div",
            "p",
            "table",
            "ul",
            "blockquote",
            "h1",
            "pre",
            "td",
            "li",
        ] {
            assert!(is_structural_block(tag), "{tag} must be structural");
            assert!(is_block(tag), "{tag} must be block");
            assert!(owns_boundaries(tag), "{tag} must own boundaries");
        }
        // 媒体块：block 但不 structural / 不 owns
        for tag in ["audio", "canvas", "frameset"] {
            assert!(is_block(tag), "{tag} must be block");
            assert!(!is_structural_block(tag), "{tag} not structural");
            assert!(!owns_boundaries(tag), "{tag} does not own boundaries");
        }
        // 选择系：owns 但不 structural
        for tag in ["select", "optgroup", "option", "col", "colgroup"] {
            assert!(owns_boundaries(tag), "{tag} must own boundaries");
            assert!(!is_structural_block(tag), "{tag} not structural");
            assert!(!is_block(tag), "{tag} not block");
        }
        // 行内与未收录元素全不命中
        for tag in ["span", "a", "strong", "img", "code", "b", "unknown-x"] {
            assert!(!is_structural_block(tag), "{tag} not structural");
            assert!(!is_block(tag), "{tag} not block");
            assert!(!owns_boundaries(tag), "{tag} does not own boundaries");
        }
    }

    #[test]
    fn subtree_has_finds_nested_predicate() {
        let doc = parse("<div><section><ul><li>x</li></ul></section></div>");
        let root = first_element(&doc, "div");
        assert!(subtree_has(root, 8, el_tag_is("li")));
        assert!(subtree_has(root, 8, el_tag_is("ul")));
        assert!(!subtree_has(root, 8, el_tag_is("table")));
    }

    #[test]
    fn subtree_has_respects_depth_budget() {
        let doc = parse("<div><section><ul><li>x</li></ul></section></div>");
        let root = first_element(&doc, "div");
        // 预算 2：div→section(1)→ul(2) 命中；li 在第 3 层超出
        assert!(subtree_has(root, 2, el_tag_is("ul")));
        assert!(!subtree_has(root, 2, el_tag_is("li")));
    }

    #[test]
    fn subtree_handles_non_element_roots_and_text() {
        let doc = parse("<div>plain <span>text</span></div>");
        let root = first_element(&doc, "div");
        assert!(subtree_has(root, 4, el_tag_is("span")));
        // 纯文本子树：谓词恒假
        let doc2 = parse("<div>words only</div>");
        let root2 = first_element(&doc2, "div");
        assert!(!subtree_has(root2, 4, &|_| true));
    }

    #[test]
    fn deep_subtree_does_not_overflow_stack() {
        // 2 万层嵌套 div：递归 walker 会逼近 2MiB 线程栈，迭代实现必须存活。
        let depth = 20_000;
        let mut html = String::from("<div>");
        for _ in 0..depth {
            html.push_str("<div>");
        }
        html.push_str("<mark>deep</mark>");
        for _ in 0..=depth {
            html.push_str("</div>");
        }
        let doc = parse(&html);
        let root = first_element(&doc, "div");
        assert!(subtree_has(root, depth + 10, el_tag_is("mark")));
    }
}
