// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 锚点系统（行为规格 §2 锚点系统）。
//!
//! 两遍预扫描：①收集被引用目标（`href="#x"` 的 fragment 与 `td/th headers`
//! 引用的 id）；②存在引用时建立 id/name 清单并求交（fragment 原样或
//! percent-decode lossy 后匹配）。转换中命中元素发射 `<span id="…"></span>`，
//! 全局去重由 machine 的 emitted_anchors 集合承担。
//!
//! 契约：`a[name]` 仅 a 元素参与；`th` 的 id 命中 headers 引用时也发射
//! （GFM 会丢 thead/tr 等宿主，锚点延迟到可发射位置）。

use std::collections::HashSet;

use scraper::node::Node as SNode;

use super::structure::subtree_has;

type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// 被引用锚点集合（转换开始时一次预扫描）。
#[derive(Default)]
pub(super) struct Targets {
    /// fragment 命中清单后的被引用 id（含 decode 匹配）。
    referenced: HashSet<String>,
    /// headers 引用且命中 th id 的子集。
    header_ids: HashSet<String>,
    /// 全树元素 id 清单（a[name] 发射时排除与 id 同名者）。
    ids: HashSet<String>,
}

impl Targets {
    pub(super) fn is_empty(&self) -> bool {
        self.referenced.is_empty() && self.header_ids.is_empty()
    }

    /// 该节点应发射的锚点目标：id（被引用，或 th 命中 headers 引用）；
    /// a[name] 被引用且不同名于任一元素 id。
    pub(super) fn targets<'n>(&self, node: NodeRef<'n>) -> Vec<&'n str> {
        let mut found: Vec<&'n str> = Vec::new();
        let Some(el) = node.value().as_element() else {
            return found;
        };
        if let Some(id) = el.attr("id").filter(|id| {
            self.referenced.contains(*id) || (el.name() == "th" && self.header_ids.contains(*id))
        }) {
            found.push(id);
        }
        if el.name() == "a"
            && let Some(name) = el
                .attr("name")
                .filter(|name| self.referenced.contains(*name) && !self.ids.contains(*name))
            && !found.contains(&name)
        {
            found.push(name);
        }
        found
    }

    /// 预扫描：第一遍收集引用，存在引用才做第二遍清单。
    pub(super) fn scan(root: NodeRef<'_>) -> Self {
        let mut fragments: Vec<String> = Vec::new();
        let mut headers: HashSet<String> = HashSet::new();
        // 深度限制固化无上限（迭代遍历）。
        subtree_has(root, usize::MAX, |node| {
            let el = match node.value().as_element() {
                Some(el) => el,
                None => return false,
            };
            if let Some(href) = el
                .attr("href")
                .and_then(|h| h.strip_prefix('#'))
                .filter(|h| !h.is_empty())
            {
                fragments.push(href.to_owned());
            }
            if matches!(el.name(), "td" | "th")
                && let Some(value) = el.attr("headers")
            {
                headers.extend(value.split_ascii_whitespace().map(str::to_owned));
            }
            false
        });
        if fragments.is_empty() && headers.is_empty() {
            return Self::default();
        }
        let mut available = HashSet::new();
        let mut ids = HashSet::new();
        let mut header_ids = HashSet::new();
        subtree_has(root, usize::MAX, |node| {
            let Some(el) = node.value().as_element() else {
                return false;
            };
            if let Some(id) = el.attr("id") {
                available.insert(id.to_owned());
                ids.insert(id.to_owned());
                if el.name() == "th" && headers.contains(id) {
                    header_ids.insert(id.to_owned());
                }
            }
            if el.name() == "a"
                && let Some(name) = el.attr("name")
            {
                available.insert(name.to_owned());
            }
            false
        });
        let mut referenced = HashSet::new();
        for fragment in fragments {
            if available.contains(&fragment) {
                referenced.insert(fragment);
            } else {
                let decoded = decode_fragment(&fragment);
                if available.contains(&decoded) {
                    referenced.insert(decoded);
                }
            }
        }
        Self {
            referenced,
            header_ids,
            ids,
        }
    }
}

/// 子树内全部被引用锚点（math 整树发射用；调用方去重）。
pub(super) fn within(node: NodeRef<'_>, remaining: usize, targets: &Targets) -> Vec<String> {
    let mut found = Vec::new();
    subtree_has(node, remaining, |n| {
        for id in targets.targets(n) {
            found.push(id.to_owned());
        }
        false
    });
    found
}

/// 子树是否含任一被引用锚点（code/pre fallback 判定用）。
pub(super) fn contains(node: NodeRef<'_>, remaining: usize, targets: &Targets) -> bool {
    subtree_has(node, remaining, |n| !targets.targets(n).is_empty())
}

/// fragment 的 percent-decode（UTF-8 lossy；非法 % 序列原样保留）。
pub(super) fn decode_fragment(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'%' && offset + 2 < bytes.len() {
            let hex = |byte: u8| (byte as char).to_digit(16);
            if let (Some(high), Some(low)) = (hex(bytes[offset + 1]), hex(bytes[offset + 2])) {
                decoded.push((high * 16 + low) as u8);
                offset += 3;
                continue;
            }
        }
        decoded.push(bytes[offset]);
        offset += 1;
    }
    String::from_utf8(decoded)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

/// 锚点标记 `<span id="…"></span>`，值转义 `& " < > \n \r |`。
pub(super) fn markup(id: &str) -> String {
    let mut out = String::from("<span id=\"");
    for ch in id.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '|' => out.push_str("&#124;"),
            c => out.push(c),
        }
    }
    out.push_str("\"></span>");
    out
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
    fn referenced_id_emits_target() {
        let doc = parse(r##"<a href="#sec">go</a><h3 id="sec">Section</h3>"##);
        let targets = Targets::scan(doc.tree.root());
        assert_eq!(targets.targets(find(&doc, "h3")), vec!["sec"]);
        assert!(contains(doc.tree.root(), usize::MAX, &targets));
    }

    #[test]
    fn unreferenced_id_no_target() {
        let doc = parse(r#"<h3 id="sec">Section</h3>"#);
        let targets = Targets::scan(doc.tree.root());
        assert!(targets.is_empty());
        assert_eq!(targets.targets(find(&doc, "h3")), Vec::<&str>::new());
    }

    #[test]
    fn percent_decoded_fragment_matches() {
        // href="#a%20b" 解码为 "a b" 后命中 id="a b"。
        let doc = parse(r##"<a href="#a%20b">go</a><h3 id="a b">S</h3>"##);
        let targets = Targets::scan(doc.tree.root());
        assert_eq!(targets.targets(find(&doc, "h3")), vec!["a b"]);
    }

    #[test]
    fn a_name_referenced_and_not_id_shadowed() {
        let doc = parse(r##"<a href="#old">go</a><a name="old">anchor</a>"##);
        let targets = Targets::scan(doc.tree.root());
        // find 命中的是首个 a（只有 href）；带 name 的锚点需按 name 属性选。
        let named = doc
            .tree
            .nodes()
            .find(|n| {
                matches!(n.value(), SNode::Element(e) if e.name() == "a" && e.attr("name").is_some())
            })
            .unwrap();
        assert_eq!(targets.targets(named), vec!["old"]);
        // name 与某元素 id 同名 → 不发射（id 已覆盖）
        let doc = parse(r##"<a href="#x">go</a><a name="x">anchor</a><p id="x">p</p>"##);
        let targets = Targets::scan(doc.tree.root());
        assert_eq!(targets.targets(find(&doc, "a")), Vec::<&str>::new());
    }

    #[test]
    fn headers_reference_matches_th_id() {
        let doc = parse(r#"<table><tr><th id="h1">A</th><td headers="h1">x</td></tr></table>"#);
        let targets = Targets::scan(doc.tree.root());
        assert_eq!(targets.targets(find(&doc, "th")), vec!["h1"]);
    }

    #[test]
    fn within_collects_all_referenced() {
        let doc = parse(r##"<a href="#a">1</a><a href="#b">2</a><p id="a"></p><p id="b"></p>"##);
        let targets = Targets::scan(doc.tree.root());
        let mut got = within(doc.tree.root(), usize::MAX, &targets);
        got.sort();
        assert_eq!(got, vec!["a", "b"]);
    }

    #[test]
    fn decode_fragment_edge_cases() {
        assert_eq!(decode_fragment("a%20b"), "a b");
        assert_eq!(decode_fragment("%E4%B8%AD"), "中");
        assert_eq!(decode_fragment("%zz"), "%zz");
        assert_eq!(decode_fragment("%2"), "%2");
        assert_eq!(decode_fragment("plain"), "plain");
    }

    #[test]
    fn markup_escapes_specials() {
        assert_eq!(markup("x"), "<span id=\"x\"></span>");
        assert_eq!(
            markup("a\"b&c<d>"),
            "<span id=\"a&quot;b&amp;c&lt;d&gt;\"></span>"
        );
    }

    #[test]
    fn scan_skips_script_content() {
        // script 内的 "#x" 字符串不是属性，天然不命中——确认扫描不 panic。
        let doc = parse(r##"<script>var a = "#x";</script><p id="x">t</p>"##);
        let targets = Targets::scan(doc.tree.root());
        assert!(targets.is_empty());
    }
}
