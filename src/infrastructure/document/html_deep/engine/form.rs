// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 表单控件的文本产出（行为规格 §2 input/textarea 分支）。
//!
//! `control_value` 固化 None（静态树无计算后的当前值）：空文本控件不把
//! name 当字段值输出，文本框用源码默认（`value` 属性）。

use scraper::node::Node as SNode;

use super::structure::attribute;

type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// input 控件的文本产出：
/// - checkbox/radio → `☑`/`☐`（`checked` 属性存在与否）；
/// - hidden/file/password → 无产出；
/// - image → alt/aria-label/title；
/// - 按钮（submit/button/reset）→ aria-label/title；
/// - 其余文本类 → `value` 属性（非空）。
///
/// 全部无值 → None（machine 不输出）。
pub(super) fn input_text(node: NodeRef<'_>) -> Option<String> {
    let el = node.value().as_element()?;
    let input_type = el.attr("type").unwrap_or("text").to_ascii_lowercase();
    match input_type.as_str() {
        "checkbox" => {
            return Some(
                if el.attr("checked").is_some() {
                    "☑"
                } else {
                    "☐"
                }
                .to_string(),
            );
        }
        "radio" => {
            return Some(
                if el.attr("checked").is_some() {
                    "☑"
                } else {
                    "☐"
                }
                .to_string(),
            );
        }
        "hidden" | "file" | "password" => return None,
        "image" => {
            return labeled(el, node);
        }
        "submit" | "button" | "reset" => {
            return labeled(el, node);
        }
        _ => {}
    }
    attribute(node, "value").filter(|v| !v.is_empty())
}

/// image/按钮控件的可达名：alt → aria-label → title。
fn labeled(el: &scraper::node::Element, node: NodeRef<'_>) -> Option<String> {
    if let Some(alt) = el.attr("alt").filter(|a| !a.is_empty()) {
        return Some(alt.to_string());
    }
    if let Some(label) = attribute(node, "aria-label").filter(|l| !l.is_empty()) {
        return Some(label);
    }
    attribute(node, "title").filter(|t| !t.is_empty())
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
    fn checkbox_states() {
        let doc = parse(r#"<input type="checkbox" checked>"#);
        assert_eq!(input_text(find(&doc, "input")), Some("☑".to_string()));
        let doc = parse(r#"<input type="checkbox">"#);
        assert_eq!(input_text(find(&doc, "input")), Some("☐".to_string()));
        let doc = parse(r#"<input type="radio">"#);
        assert_eq!(input_text(find(&doc, "input")), Some("☐".to_string()));
    }

    #[test]
    fn hidden_file_password_empty() {
        for t in ["hidden", "file", "password"] {
            let doc = parse(&format!(r#"<input type="{t}" value="secret">"#));
            assert_eq!(input_text(find(&doc, "input")), None, "{t} must be empty");
        }
    }

    #[test]
    fn text_input_uses_value_default() {
        let doc = parse(r#"<input value="hello">"#);
        assert_eq!(input_text(find(&doc, "input")), Some("hello".to_string()));
        let doc = parse(r#"<input>"#);
        assert_eq!(input_text(find(&doc, "input")), None);
    }

    #[test]
    fn image_and_button_labels() {
        let doc = parse(r#"<input type="image" alt="Go">"#);
        assert_eq!(input_text(find(&doc, "input")), Some("Go".to_string()));
        let doc = parse(r#"<input type="submit" aria-label="Send">"#);
        assert_eq!(input_text(find(&doc, "input")), Some("Send".to_string()));
        let doc = parse(r#"<input type="button" title="OK">"#);
        assert_eq!(input_text(find(&doc, "input")), Some("OK".to_string()));
    }
}
