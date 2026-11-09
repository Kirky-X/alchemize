// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 不可渲染的序列化页面状态对象判定（行为规格 §2 前置逻辑第 4 步）。
//!
//! 现代站点常把页面状态 JSON 藏在 `display:none` 的元素里（Next.js
//! `__NEXT_DATA__` 等）。这些内容对人类不可见，转换会产出垃圾文本——
//! 判定命中则整树丢弃。
//!
//! 判据（全部满足才算状态对象，防误伤可见内容）：
//! 1. 元素带 `hidden` 属性，或 inline style 含 `display:none`（分号分割、
//!    后声明覆盖前声明，除非前者带 `!important`）；
//! 2. 唯一子节点是文本，trim 后 ≥ 512 字节；
//! 3. 文本以 `{` 或 `[` 开头、以 `}` 或 `]` 结尾（JSON 形态）。

use scraper::node::Node as SNode;

type NodeRef<'a> = ego_tree::NodeRef<'a, scraper::node::Node>;

/// 状态对象的最小体积（防把短小的 hidden 提示文本误判为 JSON）。
const MIN_STATE_BYTES: usize = 512;

/// 该元素是否为"不可渲染的序列化状态对象"（命中则整树丢弃）。
pub(super) fn is_serialized_state(node: NodeRef<'_>) -> bool {
    let Some(el) = node.value().as_element() else {
        return false;
    };
    if !is_non_rendered(el) {
        return false;
    }
    let mut children = node.children();
    let Some(only) = children.next() else {
        return false;
    };
    if children.next().is_some() {
        return false;
    }
    let SNode::Text(text) = only.value() else {
        return false;
    };
    let trimmed = text.text.trim();
    if trimmed.len() < MIN_STATE_BYTES {
        return false;
    }
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

/// hidden 属性或 `display:none`（inline style 分号语义，含 !important 覆盖）。
fn is_non_rendered(el: &scraper::node::Element) -> bool {
    if el.attr("hidden").is_some() {
        return true;
    }
    let Some(style) = el.attr("style") else {
        return false;
    };
    let mut current = false;
    for decl in style.split(';') {
        let Some((prop, value)) = decl.split_once(':') else {
            continue;
        };
        if !prop.trim().eq_ignore_ascii_case("display") {
            continue;
        }
        let value = value.trim();
        let important = value.strip_suffix("!important").map(str::trim);
        if important.is_some_and(|v| v.eq_ignore_ascii_case("none")) {
            // !important 的 none 永久生效，后续声明无法覆盖。
            return true;
        }
        current = value.eq_ignore_ascii_case("none");
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(html: &str) -> scraper::Html {
        scraper::Html::parse_document(html)
    }

    fn first_div<'a>(doc: &'a scraper::Html) -> Option<NodeRef<'a>> {
        doc.tree
            .nodes()
            .find(|n| matches!(n.value(), SNode::Element(e) if e.name() == "div"))
    }

    fn big_json() -> String {
        // ≥512 字节的 JSON 形态文本。
        let items: Vec<String> = (0..64).map(|i| format!("\"k{i}\":{i}")).collect();
        format!("{{{{{}}}}}", items.join(","))
    }

    #[test]
    fn hidden_json_state_object_detected() {
        let json = big_json();
        assert!(json.len() >= MIN_STATE_BYTES);
        let html = format!(r#"<div hidden>{json}</div>"#);
        let doc = parsed(&html);
        assert!(is_serialized_state(first_div(&doc).unwrap()));
    }

    #[test]
    fn display_none_json_state_object_detected() {
        let json = big_json();
        let html = format!(r#"<div style="color:red;display:none">{json}</div>"#);
        let doc = parsed(&html);
        assert!(is_serialized_state(first_div(&doc).unwrap()));
    }

    #[test]
    fn important_none_wins_over_later_display() {
        let json = big_json();
        // 前声明 display:none!important 被后声明 display:block 覆盖不了。
        let html = format!(r#"<div style="display:none!important;display:block">{json}</div>"#);
        let doc = parsed(&html);
        assert!(is_serialized_state(first_div(&doc).unwrap()));
        // 反向：none!important 在后 → 命中；先 none 后 block（无 important）→ 不命中
        let html = format!(r#"<div style="display:block;display:none!important">{json}</div>"#);
        let doc = parsed(&html);
        assert!(is_serialized_state(first_div(&doc).unwrap()));
        let html = format!(r#"<div style="display:none;display:block">{json}</div>"#);
        let doc = parsed(&html);
        assert!(!is_serialized_state(first_div(&doc).unwrap()));
    }

    #[test]
    fn visible_or_small_or_multi_child_not_detected() {
        let json = big_json();
        // 可见元素
        let html = format!(r#"<div>{json}</div>"#);
        let doc = parsed(&html);
        assert!(!is_serialized_state(first_div(&doc).unwrap()));
        // 小体积（<512）
        let html = r#"<div hidden>{"a":1}</div>"#;
        let doc = parsed(html);
        assert!(!is_serialized_state(first_div(&doc).unwrap()));
        // 多个子节点
        let html = format!(r#"<div hidden>{json}<p>x</p></div>"#);
        let doc = parsed(&html);
        assert!(!is_serialized_state(first_div(&doc).unwrap()));
        // 非 JSON 形态
        let long_text = "x".repeat(600);
        let html = format!(r#"<div hidden>{long_text}</div>"#);
        let doc = parsed(&html);
        assert!(!is_serialized_state(first_div(&doc).unwrap()));
    }
}
