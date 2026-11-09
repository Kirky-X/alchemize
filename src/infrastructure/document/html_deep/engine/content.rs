// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 内容级判定：图片标签、可达名兜底、代码语言、代码复制控件（行为规格 §2）。

use scraper::node::Node as SNode;

use super::structure::attribute;

type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// `img` 的可见标签：`alt` 属性优先；缺失时（且元素非 aria-hidden/hidden/
/// presentational）依次回退 `aria-label`、`title`。均无 → 空串。
pub(super) fn image_label(node: NodeRef<'_>) -> String {
    let el = match node.value().as_element() {
        Some(el) => el,
        None => return String::new(),
    };
    if let Some(alt) = el.attr("alt")
        && !alt.is_empty()
    {
        return alt.to_string();
    }
    if !is_presentational_el(el) {
        if let Some(label) = attribute(node, "aria-label").filter(|l| !l.is_empty()) {
            return label;
        }
        if el.attr("alt").is_none()
            && let Some(title) = attribute(node, "title").filter(|t| !t.is_empty())
        {
            return title;
        }
    }
    String::new()
}

/// 可达名兜底标签（行为规格 §2 后置逻辑第 3 步）：元素为 a/button/svg/
/// select/option/optgroup/h1–h6，或 role 首词 ∈ 交互角色表，且非
/// aria-hidden/hidden/presentational → 取 `aria-label` 或 `title`
/// （svg 另看 `<title>` 子文本，剩余深度 > 2 才看）。
pub(super) fn fallback_text(node: NodeRef<'_>, remaining: usize) -> Option<String> {
    let el = node.value().as_element()?;
    let tag = el.name();
    let named_kind = matches!(
        tag,
        "a" | "button"
            | "svg"
            | "select"
            | "option"
            | "optgroup"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    ) || el
        .attr("role")
        .and_then(|role| role.split_whitespace().next())
        .is_some_and(|role| {
            matches!(
                role,
                "button"
                    | "link"
                    | "img"
                    | "heading"
                    | "checkbox"
                    | "radio"
                    | "textbox"
                    | "combobox"
                    | "switch"
                    | "tab"
            )
        });
    if !named_kind || is_presentational_el(el) {
        return None;
    }
    if let Some(label) = attribute(node, "aria-label").filter(|l| !l.is_empty()) {
        return Some(label);
    }
    if let Some(title) = attribute(node, "title").filter(|t| !t.is_empty()) {
        return Some(title);
    }
    if tag == "svg" && remaining > 2 {
        // svg 的 <title> 子元素文本。
        for child in node.children() {
            if let SNode::Element(child_el) = child.value()
                && child_el.name() == "title"
                && let Some(text) = descendant_text(child)
            {
                return Some(text);
            }
        }
    }
    None
}

/// 元素子树的首段可见文本（svg title 读取）。
fn descendant_text(root: NodeRef<'_>) -> Option<String> {
    let mut stack: Vec<NodeRef<'_>> = root.children().collect();
    while let Some(node) = stack.pop() {
        match node.value() {
            SNode::Text(t) => {
                let text = t.text.trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
            SNode::Element(_) | SNode::Document | SNode::Fragment => {
                stack.extend(node.children());
            }
            _ => {}
        }
    }
    None
}

/// class 属性中的 `language-X` 首词（fallback code 标签合成用）。
pub(super) fn class_language(class: Option<&str>) -> Option<&str> {
    class?
        .split_ascii_whitespace()
        .find_map(|part| part.strip_prefix("language-").filter(|s| !s.is_empty()))
}

/// pre/code 的语言标注：取 class 中 `language-X` / 简单语言首词。
/// （default_code_language 选项固化 None。）
pub(super) fn code_language(node: NodeRef<'_>) -> Option<String> {
    let el = node.value().as_element()?;
    // pre 自身或其 code 子元素的 class 均可声明语言。
    let class = el
        .attr("class")
        .map(str::to_owned)
        .or_else(|| code_child_class(node));
    let class = class?;
    let mut found = None;
    for token in class.split_whitespace() {
        if let Some(lang) = token
            .strip_prefix("language-")
            .or_else(|| token.strip_prefix("lang-"))
            .filter(|lang| !lang.is_empty())
        {
            return Some(lang.to_string());
        }
        if found.is_none() && !token.contains('-') {
            found = Some(token.to_string());
        }
    }
    found
}

fn code_child_class(node: NodeRef<'_>) -> Option<String> {
    for child in node.children() {
        if let SNode::Element(el) = child.value()
            && el.name() == "code"
            && let Some(class) = el.attr("class")
        {
            return Some(class.to_string());
        }
    }
    None
}

/// 代码复制控件（pre 内常见）：button/input/select/textarea 或 role=button
/// ——原文收集时跳过，避免把"Copy"按钮文字混进代码。
pub(super) fn is_code_control(node: NodeRef<'_>) -> bool {
    let Some(el) = node.value().as_element() else {
        return false;
    };
    matches!(el.name(), "button" | "input" | "select" | "textarea")
        || el
            .attr("role")
            .is_some_and(|r| r.trim().eq_ignore_ascii_case("button"))
}

/// 标题元素判定（嵌套标题容器分支用）。
pub(super) fn is_heading(node: NodeRef<'_>) -> bool {
    matches!(
        node.value().as_element().map(|e| e.name()),
        Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")
    )
}

/// HTML fallback 触发的块级元素判定（标题子树含它们 → fallback）。
pub(super) fn is_fallback_block(node: NodeRef<'_>) -> bool {
    matches!(
        node.value().as_element().map(|e| e.name()),
        Some("table" | "pre" | "blockquote" | "ul" | "ol" | "hr")
    )
}

/// 元素是否 presentational：aria-hidden=true / hidden 属性 / role=none|presentation。
fn is_presentational_el(el: &scraper::node::Element) -> bool {
    el.attr("aria-hidden").is_some_and(|v| v == "true")
        || el.attr("hidden").is_some()
        || el
            .attr("role")
            .is_some_and(|r| matches!(r.trim(), "none" | "presentation"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(html: &str) -> scraper::Html {
        scraper::Html::parse_document(html)
    }

    fn find<'a>(doc: &'a scraper::Html, tag: &str) -> NodeRef<'a> {
        doc.tree
            .nodes()
            .find(|n| matches!(n.value(), SNode::Element(e) if e.name() == tag))
            .unwrap_or_else(|| panic!("fixture must contain <{tag}>"))
    }

    #[test]
    fn image_label_prefers_alt_then_aria_then_title() {
        let doc = parse(r#"<img alt="alt text" src="u">"#);
        assert_eq!(image_label(find(&doc, "img")), "alt text");
        let doc = parse(r#"<img src="u" aria-label="aria text" title="title text">"#);
        assert_eq!(image_label(find(&doc, "img")), "aria text");
        let doc = parse(r#"<img src="u" title="title text">"#);
        assert_eq!(image_label(find(&doc, "img")), "title text");
        let doc = parse(r#"<img src="u">"#);
        assert_eq!(image_label(find(&doc, "img")), "");
    }

    #[test]
    fn image_label_hidden_img_suppresses_fallback() {
        // alt 缺失且 aria-hidden → 不回退 aria-label/title。
        let doc = parse(r#"<img src="u" aria-hidden="true" title="t">"#);
        assert_eq!(image_label(find(&doc, "img")), "");
    }

    #[test]
    fn fallback_text_interactive_roles() {
        let doc = parse(r#"<div role="button" aria-label="Do it"></div>"#);
        assert_eq!(
            fallback_text(find(&doc, "div"), 10),
            Some("Do it".to_string())
        );
        // role 首词匹配（多词 role）
        let doc = parse(r#"<div role="switch checkbox" title="t"></div>"#);
        assert_eq!(fallback_text(find(&doc, "div"), 10), Some("t".to_string()));
        // 非交互元素不兜底
        let doc = parse(r#"<p title="x">body</p>"#);
        assert_eq!(fallback_text(find(&doc, "p"), 10), None);
        // presentational 不兜底
        let doc = parse(r#"<button role="presentation" aria-label="x"></button>"#);
        assert_eq!(fallback_text(find(&doc, "button"), 10), None);
    }

    #[test]
    fn svg_title_child_read() {
        let doc = parse(r#"<svg><title>Chart</title><rect/></svg>"#);
        assert_eq!(
            fallback_text(find(&doc, "svg"), 10),
            Some("Chart".to_string())
        );
    }

    #[test]
    fn code_language_from_class() {
        let doc = parse(r#"<pre><code class="language-rust">fn main()</code></pre>"#);
        assert_eq!(code_language(find(&doc, "pre")), Some("rust".to_string()));
        let doc = parse(r#"<pre class="language-python"><code>x</code></pre>"#);
        assert_eq!(code_language(find(&doc, "pre")), Some("python".to_string()));
        let doc = parse(r#"<pre><code>x</code></pre>"#);
        assert_eq!(code_language(find(&doc, "pre")), None);
    }

    #[test]
    fn code_control_detection() {
        let doc = parse(r#"<pre><button>Copy</button><code>x</code></pre>"#);
        assert!(is_code_control(find(&doc, "button")));
        let doc = parse(r#"<span role="button">icon</span>"#);
        assert!(is_code_control(find(&doc, "span")));
        let doc = parse(r#"<code>x</code>"#);
        assert!(!is_code_control(find(&doc, "code")));
    }
}
