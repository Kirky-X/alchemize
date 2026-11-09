// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 表格识别、单元格收集与 GFM 产出（行为规格 §6）。
//!
//! 关键决策：
//! - **识别只看 sections/rows/cells 本身**（深度 +1/+2）——嵌套表由主状态机
//!   在转换单元格内容时自行决策，绝不递归重扫（互不 disjoint 重复）；
//! - **span 拒绝策略**：GFM 无单元格跨行跨列，带有效 span 的表退化（绝不
//!   按 span 值分配网格或静默左移后续单元格）；
//! - **短行保持短**：稀疏行不 padding 到表头宽度（防 O(行×列) 膨胀）；
//! - **列对齐**：每列第一个被扫到的单元格 `align` 属性决定分隔行。

use std::collections::VecDeque;

use scraper::node::Node as SNode;

use super::output::Output;

type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// 待转换单元格：内容根、深度、所属行（行锚点延迟发射用）。
pub(super) struct Cell<'a> {
    pub(super) node: NodeRef<'a>,
    pub(super) depth: usize,
    pub(super) row: Option<NodeRef<'a>>,
}

/// 表格收集器。
pub(super) struct Table<'a> {
    captions: Vec<Cell<'a>>,
    cells: std::vec::IntoIter<Cell<'a>>,
    content: Vec<Output>,
    in_cell: bool,
    rows: Vec<usize>,
    separators: Vec<&'static str>,
    has_header: bool,
    /// GFM 丢弃 section/column 元素；其 id 目标延迟到下一单元格边界发射。
    targets: VecDeque<(usize, NodeRef<'a>)>,
}

/// 表有 role=presentation/none（跳过表格判定，直接普通元素处理）。
pub(super) fn is_presentational(node: NodeRef<'_>) -> bool {
    node.value()
        .as_element()
        .and_then(|el| el.attr("role"))
        .is_some_and(|role| {
            let role = role.trim();
            role.eq_ignore_ascii_case("presentation") || role.eq_ignore_ascii_case("none")
        })
}

impl<'a> Table<'a> {
    /// 从 DOM 识别并收集表格。识别条件（任一命中即 None 退化）：
    /// presentational；table 直接子节点出现 caption/tr/section/colgroup/col/
    /// 可忽略以外的东西；行直接子节点出现 th/td/可忽略以外的东西；任一单元格
    /// 带有效 span；第二个 thead 节段行或表头后更宽的行；空表。
    pub(super) fn from_dom(node: NodeRef<'a>) -> Option<Self> {
        if is_presentational(node) {
            return None;
        }
        let mut t = Self {
            captions: Vec::new(),
            cells: Vec::new().into_iter(),
            content: Vec::new(),
            in_cell: false,
            rows: Vec::new(),
            separators: Vec::new(),
            has_header: false,
            targets: VecDeque::new(),
        };
        let mut cells: Vec<Cell<'a>> = Vec::new();
        let mut child = node.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            match current.value() {
                SNode::Element(el) if el.name() == "caption" && t.rows.is_empty() => {
                    t.captions.push(Cell {
                        node: current,
                        depth: 1,
                        row: None,
                    });
                }
                SNode::Element(el) if el.name() == "tr" => {
                    t.add_row(current, "table", &mut cells)?;
                }
                SNode::Element(el) if matches!(el.name(), "thead" | "tbody" | "tfoot") => {
                    let tag = el.name();
                    t.defer_target(current, cells.len());
                    let mut row = current.first_child();
                    while let Some(r) = row {
                        row = r.next_sibling();
                        match r.value() {
                            SNode::Element(rel) if rel.name() == "tr" => {
                                t.add_row(r, tag, &mut cells)?;
                            }
                            k if ignorable(k) => {}
                            _ => return None,
                        }
                    }
                }
                SNode::Element(el) if matches!(el.name(), "colgroup" | "col") => {
                    t.defer_target(current, cells.len());
                    if el.name() == "colgroup" {
                        let mut column = current.first_child();
                        while let Some(c) = column {
                            column = c.next_sibling();
                            t.defer_target(c, cells.len());
                        }
                    }
                }
                k if ignorable(k) => {}
                _ => return None,
            }
        }
        if t.rows.is_empty() {
            return None;
        }
        t.cells = cells.into_iter();
        Some(t)
    }

    /// 收集一行单元格。行深度固定为 1 层语义（相对 table 的直接子层）。
    fn add_row(
        &mut self,
        row_node: NodeRef<'a>,
        section: &str,
        cells: &mut Vec<Cell<'a>>,
    ) -> Option<()> {
        let start = cells.len();
        let mut all_headers = true;
        let mut child = row_node.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            match current.value() {
                SNode::Element(el) if matches!(el.name(), "th" | "td") => {
                    // GFM 无单元格 span：有效 span → 整表退化。
                    for name in ["colspan", "rowspan"] {
                        if el.attr(name).is_some_and(|span| has_cell_span(name, span)) {
                            return None;
                        }
                    }
                    all_headers &= el.name() == "th";
                    cells.push(Cell {
                        node: current,
                        depth: 1,
                        row: (cells.len() == start).then_some(row_node),
                    });
                    // 每列第一个被扫到的单元格决定对齐；合成表头可随后续行变宽。
                    if !self.has_header && cells.len() - start > self.separators.len() {
                        let align = el.attr("align").unwrap_or_default().trim();
                        self.separators.push(if align.eq_ignore_ascii_case("left") {
                            ":---"
                        } else if align.eq_ignore_ascii_case("right") {
                            "---:"
                        } else if align.eq_ignore_ascii_case("center") {
                            ":---:"
                        } else {
                            "---"
                        });
                    }
                }
                k if ignorable(k) => {}
                _ => return None,
            }
        }
        let count = cells.len() - start;
        if count == 0 {
            self.defer_target(row_node, cells.len());
            return Some(());
        }
        if self.rows.is_empty() {
            // 显式表头（thead 或全 th 行）；缺失时合成空表头，不消耗数据行。
            self.has_header = section == "thead" || (section != "tfoot" && all_headers);
        } else if section == "thead" || (self.has_header && count > self.separators.len()) {
            // GFM 只支持单表头行：更宽的行会被解析器丢列 → 整表退化。
            return None;
        }
        self.rows.push(count);
        Some(())
    }

    /// 取走 caption 节点（先于单元格转换，作普通块处理）。
    pub(super) fn take_captions(&mut self) -> Vec<Cell<'a>> {
        std::mem::take(&mut self.captions)
    }

    /// 延迟锚点宿主（thead/tbody/tfoot/tr/colgroup/col 上带 id 者）。
    fn defer_target(&mut self, node: NodeRef<'a>, cell: usize) {
        if node
            .value()
            .as_element()
            .is_some_and(|el| el.attr("id").is_some())
        {
            self.targets.push_back((cell, node));
        }
    }

    /// 下一个延迟发射的锚点宿主（其位置已被内容填充时弹出）。
    pub(super) fn next_target(&mut self) -> Option<NodeRef<'a>> {
        if self
            .targets
            .front()
            .is_some_and(|(cell, _)| *cell <= self.content.len())
        {
            self.targets.pop_front().map(|(_, node)| node)
        } else {
            None
        }
    }

    /// 下一个待转换单元格。
    pub(super) fn next_cell(&mut self) -> Option<Cell<'a>> {
        let cell = self.cells.next()?;
        self.in_cell = true;
        Some(cell)
    }

    /// 收下单元格内容产物。
    pub(super) fn finish_cell(&mut self, content: Output) {
        self.content.push(content);
        self.in_cell = false;
    }

    /// 全部单元格收齐后产出 GFM 表格（全空 → 空块=整表消失）。
    pub(super) fn finish(self) -> Output {
        if self.content.iter().all(Output::is_empty) {
            return Output::new();
        }
        let mut output = String::new();
        if !self.has_header {
            // 合成空表头（不消耗数据行），沿用各列对齐。
            output.push('|');
            for _ in &self.separators {
                output.push_str("  |");
            }
            separator_row(&mut output, &self.separators);
        }
        let mut content = self.content.into_iter();
        for (row, count) in self.rows.iter().enumerate() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push('|');
            for _ in 0..*count {
                output.push(' ');
                let cell = content
                    .next()
                    .unwrap_or_else(|| unreachable!("every table cell was converted"))
                    .into_string();
                for (line, text) in cell.split('\n').enumerate() {
                    if line > 0 {
                        output.push_str("<br>");
                    }
                    for ch in text.trim_end().chars() {
                        if ch == '|' {
                            // GFM 解析行内前剥掉该转义（含 code span 与链接 URL）。
                            output.push('\\');
                        }
                        output.push(ch);
                    }
                }
                output.push_str(" |");
            }
            if row == 0 && self.has_header {
                separator_row(&mut output, &self.separators);
            }
        }
        Output::from_text(output)
    }
}

/// 分隔行（沿用各列对齐）。
fn separator_row(output: &mut String, separators: &[&str]) {
    output.push_str("\n|");
    for separator in separators {
        output.push(' ');
        output.push_str(separator);
        output.push_str(" |");
    }
}

/// 可忽略节点：注释/doctype/处理指令、script 等容器、纯空白文本。
fn ignorable(value: &SNode) -> bool {
    match value {
        SNode::Comment(_) | SNode::Doctype(_) | SNode::ProcessingInstruction(_) => true,
        SNode::Element(el) => {
            matches!(el.name(), "script" | "style" | "noscript" | "template")
        }
        SNode::Text(text) => text.text.chars().all(super::writer::is_space),
        _ => false,
    }
}

/// HTML 非负整数解析：非法默认 1；rowspan 0 延伸到行组末尾（都算有效 span）。
/// 由此值绝不分配网格。
pub(super) fn has_cell_span(name: &str, value: &str) -> bool {
    let value = value.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let value = value.strip_prefix('+').unwrap_or(value);
    let mut digits = value.bytes().take_while(u8::is_ascii_digit).peekable();
    if digits.peek().is_none() {
        return false;
    }
    let count = digits.fold(0u32, |count, digit| {
        count
            .saturating_mul(10)
            .saturating_add(u32::from(digit - b'0'))
    });
    count > 1 || (name == "rowspan" && count == 0)
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
    fn simple_table_with_header() {
        let doc =
            parse("<table><tr><th>H1</th><th>H2</th></tr><tr><td>a</td><td>b</td></tr></table>");
        let mut table = Table::from_dom(find(&doc, "table")).expect("structural table");
        let mut out = String::new();
        while let Some(cell) = table.next_cell() {
            let _ = cell;
            table.finish_cell(Output::from_text("x".to_string()));
        }
        out.push_str(&table.finish().into_string());
        assert_eq!(out, "| x | x |\n| --- | --- |\n| x | x |");
    }

    #[test]
    fn spans_and_foreign_children_reject() {
        let doc = parse("<table><tr><td colspan=\"2\">a</td></tr></table>");
        assert!(Table::from_dom(find(&doc, "table")).is_none());
        let doc = parse("<table><tr><td>a</td><td>b</td></tr></table>");
        assert!(Table::from_dom(find(&doc, "table")).is_some());
        // rows 之后出现 caption → 退化（caption 仅在行前收集）。
        let doc = parse("<table><tr><td>a</td></tr><caption>late</caption></table>");
        assert!(Table::from_dom(find(&doc, "table")).is_none());
        // 行首 caption 正常收集，不退化。
        let doc = parse("<table><caption>T</caption><tr><td>a</td></tr></table>");
        assert!(Table::from_dom(find(&doc, "table")).is_some());
    }

    #[test]
    fn presentational_tables_skipped() {
        let doc = parse("<table role=\"presentation\"><tr><td>a</td></tr></table>");
        assert!(Table::from_dom(find(&doc, "table")).is_none());
    }

    #[test]
    fn thead_tbody_sections() {
        let doc = parse(
            "<table><thead><tr><th>H</th></tr></thead><tbody><tr><td>a</td></tr></tbody></table>",
        );
        let mut table = Table::from_dom(find(&doc, "table")).expect("sectioned table");
        while table.next_cell().is_some() {
            table.finish_cell(Output::from_text("c".to_string()));
        }
        assert_eq!(table.finish().into_string(), "| c |\n| --- |\n| c |");
    }

    #[test]
    fn alignment_from_first_cell() {
        let doc = parse(
            "<table><tr><th align=\"left\">H</th><th align=\"right\">H</th><th align=\"center\">H</th></tr><tr><td>a</td><td>b</td><td>c</td></tr></table>",
        );
        let mut table = Table::from_dom(find(&doc, "table")).expect("aligned table");
        while table.next_cell().is_some() {
            table.finish_cell(Output::from_text("x".to_string()));
        }
        assert_eq!(
            table.finish().into_string(),
            "| x | x | x |\n| :--- | ---: | :---: |\n| x | x | x |"
        );
    }

    #[test]
    fn synthetic_header_when_missing() {
        let doc = parse("<table><tr><td>a</td><td>b</td></tr></table>");
        let mut table = Table::from_dom(find(&doc, "table")).expect("headerless table");
        while table.next_cell().is_some() {
            table.finish_cell(Output::from_text("c".to_string()));
        }
        assert_eq!(
            table.finish().into_string(),
            "|  |  |\n| --- | --- |\n| c | c |"
        );
    }

    #[test]
    fn pipe_escaped_and_multiline_br() {
        let doc = parse("<table><tr><th>H</th></tr><tr><td>a|b</td></tr></table>");
        let mut table = Table::from_dom(find(&doc, "table")).expect("pipe table");
        while let Some(cell) = table.next_cell() {
            let _ = cell;
            table.finish_cell(Output::from_text("a|b\nc".to_string()));
        }
        assert_eq!(
            table.finish().into_string(),
            "| a\\|b<br>c |\n| --- |\n| a\\|b<br>c |"
        );
    }

    #[test]
    fn empty_cells_table_vanishes() {
        let doc = parse("<table><tr><td></td></tr></table>");
        let mut table = Table::from_dom(find(&doc, "table")).expect("empty-cell table");
        while let Some(cell) = table.next_cell() {
            let _ = cell;
            table.finish_cell(Output::new());
        }
        assert!(table.finish().into_string().is_empty());
    }

    #[test]
    fn second_thead_rejects() {
        let doc = parse(
            "<table><thead><tr><th>H</th></tr></thead><thead><tr><th>H2</th></tr></thead></table>",
        );
        assert!(Table::from_dom(find(&doc, "table")).is_none());
    }

    #[test]
    fn span_value_semantics() {
        // 非法/无数字 = 无 span；1 = 无；>1 = 有；rowspan 0 = 有。
        assert!(!has_cell_span("colspan", "1"));
        assert!(!has_cell_span("colspan", "abc"));
        assert!(!has_cell_span("colspan", ""));
        assert!(has_cell_span("colspan", "2"));
        assert!(has_cell_span("rowspan", "0"));
        assert!(has_cell_span("colspan", " 2"));
        assert!(has_cell_span("colspan", "+2"));
    }
}
