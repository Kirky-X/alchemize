// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 迭代任务栈状态机：把 DOM 遍历决策与 writer 事件编排为 Markdown 输出。
//!
//! 设计（行为规格 §2）：
//! - **任务栈**：`Children(Some(n), d)` 先压兄弟续接 `Children(next_sibling)`
//!   再压 `Visit(n)`——兄弟续接只存一个，遍历内存 O(深度) 而非 O(宽度)；
//!   DOM 深度不变成 Rust 调用栈深度（无递归）。
//! - **writer 栈**：`capture()` 压子 writer（继承期望样式栈）；`EndXxx` 任务
//!   弹出、物化、把内容计数合并给父、产物作为块追加。
//! - **深度上限固化**：本仓恒无上限（迭代遍历无栈风险；静默丢内容的
//!   max_depth 截断语义永不触发）。
//! - **适配器语义固化**：scraper 树无计算样式——`has_block_layout`/
//!   `has_text_boundary`/`control_value` 恒为 false/None（接了就是假数据）。

use std::collections::HashSet;

use scraper::node::Node as SNode;

use super::anchors::{self, Targets};
use super::content;
use super::fallback;
use super::form;
use super::mathml;
use super::media;
use super::output::Output;
use super::structure::{attribute, is_block, is_structural_block, subtree_has};
use super::table::{self, Table};
use super::visibility;
use super::writer::{Style, Writer};

/// 深度上限（固化无限制；保留参数形态以对齐规格描述）。
const MAX_DEPTH: usize = usize::MAX;

/// DOM 节点引用别名（Copy，借用文档树）。
type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// 排程任务。压栈序约定：`Children(Some(n), d)` 先压 `Children(next_sibling)`
/// 再压 `Visit(n)`（栈顶先执行）。
enum Task<'a> {
    Visit(NodeRef<'a>, usize),
    Children(Option<NodeRef<'a>>, usize),
    MathChildren(Option<NodeRef<'a>>, usize),
    Boundary,
    ChoiceBoundary,
    /// 可达名兜底：`(label, 压入时的内容计数)`——子树处理完后若无任何可见
    /// 内容则输出 ` label `（空控件的可达名兜底）。
    Fallback(String, bool),
    PopStyle,
    EndLink(usize),
    EndQuote,
    EndHeading(usize),
    EndList(usize),
    EndItem(String),
    /// pre/code 原文收集：`(首子节点, 深度, 是否行内 code)`。
    RawChildren(Option<NodeRef<'a>>, usize, bool),
    /// pre 模式下结构块结束后补换行。
    EndRawBlock,
    EndCode,
    EndPre(Option<String>),
    TableCell,
    EndTableCell,
}

struct List {
    /// 该列表条目 writer 所在的 writer 栈深（紧凑嵌套判定用）。
    output_depth: usize,
    ordered: bool,
    next: i64,
    loose: bool,
}

struct Machine<'a> {
    tasks: Vec<Task<'a>>,
    writers: Vec<Writer>,
    lists: Vec<List>,
    tables: Vec<Table<'a>>,
    raw: String,
    serial: usize,
    anchor_targets: Targets,
    emitted_anchors: HashSet<String>,
}

/// 把解析后的 HTML 文档转换为 Markdown（引擎入口，见模块文档）。
pub(in crate::infrastructure::document::html_deep) fn convert(doc: &scraper::Html) -> String {
    let mut machine = Machine {
        tasks: vec![Task::Visit(doc.tree.root(), 0)],
        writers: vec![Writer::new()],
        lists: Vec::new(),
        tables: Vec::new(),
        raw: String::new(),
        serial: 0,
        anchor_targets: anchors::Targets::scan(doc.tree.root()),
        emitted_anchors: HashSet::new(),
    };
    machine.run();
    machine.take_writer().into_string()
}

impl<'a> Machine<'a> {
    fn run(&mut self) {
        while let Some(task) = self.tasks.pop() {
            match task {
                Task::Visit(node, depth) => self.visit(node, depth),
                Task::Children(Some(node), depth) => {
                    // 只存一个兄弟续接：遍历存储 O(深度)。
                    self.tasks.push(Task::Children(node.next_sibling(), depth));
                    self.tasks.push(Task::Visit(node, depth));
                }
                Task::MathChildren(Some(node), depth) => {
                    self.tasks
                        .push(Task::MathChildren(node.next_sibling(), depth));
                    // MathJax 双拷贝：只遍历非 aria-hidden 分支，DOM 序保序。
                    if !mathml::is_hidden_alternative(node) {
                        self.tasks.push(Task::Visit(node, depth));
                    }
                }
                Task::Children(None, _)
                | Task::MathChildren(None, _)
                | Task::RawChildren(None, _, _) => {}
                Task::Boundary => self.writer().boundary(2),
                Task::ChoiceBoundary => self.writer().boundary(1),
                Task::Fallback(label, before) => {
                    if self.writer().content_seen == before {
                        self.writer().text(" ");
                        self.writer().text(&label);
                        self.writer().text(" ");
                    }
                }
                Task::PopStyle => self.writer().pop_style(),
                Task::EndLink(serial) => self.writer().end_link(serial),
                Task::EndQuote => {
                    let content = self.take_writer().into_string();
                    let mut quote = String::new();
                    for line in content.lines() {
                        if !quote.is_empty() {
                            quote.push('\n');
                        }
                        quote.push('>');
                        if !line.is_empty() {
                            quote.push(' ');
                            quote.push_str(line);
                        }
                    }
                    self.writer().block(Output::from_text(quote), 2, 2);
                }
                Task::EndHeading(level) => {
                    let content = self.take_writer().into_string();
                    if !content.is_empty() {
                        let heading = format!(
                            "{} {}",
                            "#".repeat(level),
                            content
                                .lines()
                                .map(|line| line.trim_matches(super::writer::is_space))
                                .filter(|line| !line.is_empty())
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        self.writer().block(Output::from_text(heading), 2, 2);
                    } else {
                        self.writer().boundary(2);
                    }
                }
                Task::EndList(before) => {
                    self.lists.pop();
                    let content = self.take_writer();
                    let has_blocks = self.writer().has_blocks;
                    self.writer().append_block(before, 2, content);
                    if before == 1 {
                        // 尾随嵌套列表不松父条目；后续内容仍可实现空行边界。
                        self.writer().has_blocks = has_blocks;
                    }
                }
                Task::EndItem(marker) => {
                    self.writer().materialize_pending();
                    let loose = self.writer().has_blocks;
                    let content = self.take_writer().into_string();
                    if content.is_empty() {
                        continue;
                    }
                    let mut item = String::new();
                    for (index, line) in content.lines().enumerate() {
                        if index == 0 {
                            item.push_str(&marker);
                        } else {
                            item.push('\n');
                            if !line.is_empty() {
                                for _ in 0..marker.len() {
                                    item.push(' ');
                                }
                            }
                        }
                        item.push_str(line);
                    }
                    let list = self
                        .lists
                        .last_mut()
                        .unwrap_or_else(|| unreachable!("list item belongs to a list"));
                    let separation = if loose || list.loose { 2 } else { 1 };
                    list.loose |= loose;
                    self.writer()
                        .block(Output::from_text(item), separation, separation);
                }
                Task::RawChildren(Some(node), depth, inline) => self.raw_node(node, depth, inline),
                Task::EndRawBlock => {
                    if !self.raw.is_empty() && !self.raw.ends_with('\n') {
                        self.raw.push('\n');
                    }
                }
                Task::EndCode => {
                    let text = std::mem::take(&mut self.raw);
                    // preformatted 选项固化 false：行内 code 按普通 HTML 空白折叠。
                    self.writer().code_with_edges(&text);
                }
                Task::EndPre(language) => self.end_pre(language),
                Task::TableCell => self.table_cell(),
                Task::EndTableCell => {
                    let content = self.take_writer();
                    let table = self
                        .tables
                        .last_mut()
                        .unwrap_or_else(|| unreachable!("cell belongs to a table"));
                    table.finish_cell(content);
                }
            }
        }
    }

    fn writer(&mut self) -> &mut Writer {
        self.writers
            .last_mut()
            .unwrap_or_else(|| unreachable!("conversion has an output"))
    }

    fn capture(&mut self) {
        let writer = self.writer().child();
        self.writers.push(writer);
    }

    /// 弹出子 writer：物化、内容计数并入父、finish 为产物。
    fn take_writer(&mut self) -> Output {
        let mut writer = self
            .writers
            .pop()
            .unwrap_or_else(|| unreachable!("capture has an output"));
        writer.materialize_pending();
        if let Some(parent) = self.writers.last_mut() {
            parent.merge_child(&writer);
        }
        writer.finish()
    }

    fn children(&mut self, node: NodeRef<'a>, depth: usize) {
        self.tasks.push(Task::Children(node.first_child(), depth));
    }

    fn style(&mut self, style: Style) {
        if self.writer().push_style(style) {
            self.tasks.push(Task::PopStyle);
        }
    }

    fn visit(&mut self, node: NodeRef<'a>, depth: usize) {
        let tag = match node_kind(node) {
            NodeKind::Document => {
                self.children(node, depth + 1);
                return;
            }
            NodeKind::Text(text) => {
                self.writer().text(text);
                return;
            }
            NodeKind::Other => return,
            NodeKind::Element(tag) => tag,
        };
        if visibility::is_serialized_state(node) {
            return;
        }
        // 通用锚点发射（排除有专属发射点的元素；表格在 cell 边界发射）。
        if tag != "table"
            && !matches!(
                tag,
                "head"
                    | "script"
                    | "style"
                    | "noscript"
                    | "template"
                    | "title"
                    | "pre"
                    | "code"
                    | "li"
                    | "blockquote"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            )
        {
            self.emit_node_targets(node);
        }
        match tag {
            "math" => {
                self.emit_anchors_within(node, MAX_DEPTH - depth);
                let markup = mathml::render(node, MAX_DEPTH - depth);
                self.writer().inline_html(&markup);
                return;
            }
            "head" | "script" | "style" | "noscript" | "template" | "title" => return,
            "br" => {
                self.writer().hard_break();
                return;
            }
            "hr" => {
                self.writer().block(Output::from_static("---"), 2, 2);
                return;
            }
            "select" | "optgroup" | "option" => {
                self.writer().boundary(1);
                if matches!(tag, "optgroup" | "option")
                    && let Some(label) = attribute(node, "label").filter(|l| !l.is_empty())
                {
                    self.writer().text(&label);
                    self.writer().boundary(1);
                    if tag == "option" {
                        return;
                    }
                }
                self.tasks.push(Task::ChoiceBoundary);
            }
            "strong" | "b" => self.style(Style::Strong),
            "em" | "i" => self.style(Style::Em),
            "del" | "s" | "strike" => self.style(Style::Strike),
            "sup" => self.style(Style::Sup),
            "sub" => self.style(Style::Sub),
            "a" => {
                if let Some(href) = media::link_href(node) {
                    self.serial += 1;
                    let serial = self.serial;
                    let style = Style::Link {
                        serial,
                        dest: href,
                        title: attribute(node, "title"),
                    };
                    if self.writer().push_style(style) {
                        self.tasks.push(Task::EndLink(serial));
                    }
                }
            }
            "img" => {
                let alt = content::image_label(node);
                let title = attribute(node, "title");
                if let Some(src) = media::source(node) {
                    self.writer().image(&alt, &src, title.as_deref());
                } else {
                    // 资源缺失不得抹掉文本替代（尤其是链接的唯一标签）。
                    self.writer().text(&alt);
                }
                return;
            }
            "input" => {
                if let Some(value) = form::input_text(node) {
                    self.writer().text(" ");
                    self.writer().text(&value);
                    self.writer().text(" ");
                }
                return;
            }
            "textarea" => {
                // control_value 固化 None（静态树无当前值）：遍历子树（文本已解码）。
                self.children(node, depth + 1);
                return;
            }
            "iframe" | "video" | "audio" if self.media(node, tag) => return,
            "code" | "pre" => {
                let remaining = MAX_DEPTH - depth;
                if subtree_has(node, remaining, |n| media::node_has_safe_link(n))
                    || (!self.anchor_targets.is_empty()
                        && anchors::contains(node, remaining, &self.anchor_targets))
                {
                    self.html(node, depth, tag == "pre");
                    return;
                }
                self.raw.clear();
                let end = if tag == "pre" {
                    Task::EndPre(content::code_language(node))
                } else {
                    Task::EndCode
                };
                self.tasks.push(end);
                self.tasks.push(Task::RawChildren(
                    node.first_child(),
                    depth + 1,
                    tag == "code",
                ));
                return;
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let remaining = MAX_DEPTH - depth;
                if subtree_has(node, remaining, |n| content::is_heading(n)) {
                    // 嵌套标题：当普通容器处理。
                    self.writer().boundary(2);
                    self.emit_own_targets(node);
                    self.tasks.push(Task::Boundary);
                    self.children(node, depth + 1);
                    return;
                }
                if subtree_has(node, remaining, |n| content::is_fallback_block(n)) {
                    self.html(node, depth, true);
                    return;
                }
                self.capture();
                self.writer().heading = true;
                self.tasks
                    .push(Task::EndHeading(tag.as_bytes()[1] as usize - b'0' as usize));
            }
            "blockquote" => {
                self.capture();
                self.tasks.push(Task::EndQuote);
            }
            "ul" | "ol" => self.start_list(node, tag == "ol"),
            "li" => self.start_item(),
            "table" => {
                let presentation = table::is_presentational(node);
                let structural_table =
                    !presentation && fallback::requires_table_structure(node, MAX_DEPTH - depth);
                let complex_table = fallback::needed(node, MAX_DEPTH - depth);
                if structural_table
                    && (complex_table
                        || (!self.anchor_targets.is_empty()
                            && anchors::contains(node, MAX_DEPTH - depth, &self.anchor_targets)))
                {
                    self.html(node, depth, true);
                    return;
                }
                self.emit_own_targets(node);
                if !complex_table && let Some(mut t) = Table::from_dom(node) {
                    let captions = t.take_captions();
                    self.tables.push(t);
                    self.tasks.push(Task::TableCell);
                    for caption in captions.into_iter().rev() {
                        self.tasks.push(Task::Visit(caption.node, caption.depth));
                    }
                    return;
                }
                if structural_table && fallback::has_cells(node, MAX_DEPTH - depth) {
                    self.html(node, depth, true);
                    return;
                }
                // 纯布局表：内容退化输出。
                self.writer().boundary(2);
                self.tasks.push(Task::Boundary);
            }
            _ if is_block(tag) => {
                self.writer().boundary(2);
                self.tasks.push(Task::Boundary);
            }
            _ => {}
        }
        if matches!(
            tag,
            "li" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) {
            self.emit_own_targets(node);
        }
        if mathml::primary_alternative(node, MAX_DEPTH - depth).is_some() {
            self.emit_anchors_within(node, MAX_DEPTH - depth);
            self.tasks
                .push(Task::MathChildren(node.first_child(), depth + 1));
            return;
        }
        if let Some(label) = content::fallback_text(node, MAX_DEPTH - depth) {
            let before = self.writer().content_seen;
            self.tasks.push(Task::Fallback(label, before));
        }
        self.children(node, depth + 1);
    }

    // -- 子系统入口 -----------------------------------------------------------

    /// HTML fallback（code/pre、含块标题、复杂/结构表格三类触发）。
    fn html(&mut self, node: NodeRef<'a>, depth: usize, block: bool) {
        let formatting = self.writer().html_context(block);
        // depth 传真实机器深度（render 内部自增 +1 需远离 usize::MAX）；
        // 子系统调用面各自用 usize::MAX - depth 计算剩余预算。
        let rendered = fallback::render(
            node,
            depth,
            &self.anchor_targets,
            &mut self.emitted_anchors,
            formatting,
        );
        self.writer().html(rendered, block);
    }

    /// iframe/video/audio 媒体源处理；无源时返回 false 落回通用处理。
    fn media(&mut self, node: NodeRef<'a>, tag: &str) -> bool {
        let urls = media::sources(node, tag);
        let poster = media::poster(node, tag);
        if urls.is_empty() && poster.is_none() {
            return false;
        }
        let block = is_block(tag);
        if block {
            self.writer().boundary(2);
        } else {
            self.writer().text(" ");
        }
        let label = media::label(node, tag);
        if let Some(poster) = poster {
            self.writer()
                .image(&format!("{label} poster"), &poster, None);
            self.writer().text(" ");
        }
        let has_source = !urls.is_empty();
        for href in urls {
            self.serial += 1;
            let serial = self.serial;
            let opened = self.writer().push_style(Style::Link {
                serial,
                dest: href.clone(),
                title: None,
            });
            self.writer().text(&label);
            if opened {
                self.writer().end_link(serial);
            } else {
                // Markdown 不能嵌链接：目的地以文本保留而非静默丢失。
                self.writer().text(" (");
                self.writer().text(&href);
                self.writer().text(")");
            }
            self.writer().text(" ");
        }
        if block {
            self.writer().boundary(2);
        } else {
            self.writer().text(" ");
        }
        has_source
    }

    fn start_list(&mut self, node: NodeRef<'a>, ordered: bool) {
        // 紧凑嵌套：当前 writer 恰为最内层列表的条目 writer 层。
        let before = if self
            .lists
            .last()
            .is_some_and(|list| self.writers.len() == list.output_depth + 1)
        {
            1
        } else {
            2
        };
        let next = attribute(node, "start")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1);
        self.capture();
        self.lists.push(List {
            output_depth: self.writers.len(),
            ordered,
            next,
            loose: false,
        });
        self.tasks.push(Task::EndList(before));
    }

    fn start_item(&mut self) {
        let Some(list) = self
            .lists
            .last_mut()
            .filter(|list| list.output_depth == self.writers.len())
        else {
            // 孤儿 li：当普通块处理。
            self.writer().boundary(2);
            self.tasks.push(Task::Boundary);
            return;
        };
        let marker = if list.ordered {
            let marker = format!("{}. ", list.next);
            list.next = list.next.saturating_add(1);
            marker
        } else {
            "- ".to_owned()
        };
        self.capture();
        self.tasks.push(Task::EndItem(marker));
    }

    fn raw_node(&mut self, node: NodeRef<'a>, depth: usize, inline: bool) {
        self.tasks
            .push(Task::RawChildren(node.next_sibling(), depth, inline));
        if visibility::is_serialized_state(node) {
            return;
        }
        if content::is_code_control(node) {
            return;
        }
        match node_kind(node) {
            NodeKind::Text(text) => self.raw.push_str(text),
            NodeKind::Element("script" | "style" | "template" | "noscript") => {}
            // 行内 code 换行归一为空白；显式 br 保留为换行；后代格式化不转 Markdown。
            NodeKind::Element("br") => self.raw.push('\n'),
            NodeKind::Document | NodeKind::Element(_) => {
                if !inline
                    && matches!(node_kind(node), NodeKind::Element(tag) if is_structural_block(tag))
                {
                    if !self.raw.is_empty() && !self.raw.ends_with('\n') {
                        self.raw.push('\n');
                    }
                    self.tasks.push(Task::EndRawBlock);
                }
                self.tasks
                    .push(Task::RawChildren(node.first_child(), depth + 1, inline));
            }
            _ => {}
        }
    }

    fn end_pre(&mut self, language: Option<String>) {
        let text = std::mem::take(&mut self.raw);
        if text.trim().is_empty() {
            self.writer().boundary(2);
            return;
        }
        // CommonMark 代码块归一 CRLF/CR；语言名只占 info string 首行。
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let language = language.unwrap_or_default();
        let language = language.split(['\r', '\n']).next().unwrap_or_default();
        let marker = if language.contains('`') { '~' } else { '`' };
        let fence = marker
            .to_string()
            .repeat(3.max(super::writer::longest_run(&text, marker) + 1));
        let mut block = fence.clone();
        for ch in language.chars() {
            match ch {
                '\\' => block.push_str("\\\\"),
                '&' => block.push_str("&amp;"),
                _ => block.push(ch),
            }
        }
        block.push('\n');
        block.push_str(&text);
        if !text.is_empty() && !text.ends_with('\n') {
            block.push('\n');
        }
        block.push_str(&fence);
        self.writer().block(Output::from_text(block), 2, 2);
    }

    fn table_cell(&mut self) {
        let table = self
            .tables
            .last_mut()
            .unwrap_or_else(|| unreachable!("table conversion is active"));
        let mut targets = Vec::new();
        while let Some(node) = table.next_target() {
            targets.push(node);
        }
        if let Some(cell) = table.next_cell() {
            self.capture();
            for node in targets {
                self.emit_own_targets(node);
            }
            if let Some(row) = cell.row {
                self.emit_own_targets(row);
            }
            self.emit_own_targets(cell.node);
            self.tasks.push(Task::TableCell);
            self.tasks.push(Task::EndTableCell);
            self.children(cell.node, cell.depth + 1);
        } else {
            let table = self
                .tables
                .pop()
                .unwrap_or_else(|| unreachable!("table conversion is active"));
            self.writer().append_block(2, 2, table.finish());
            for node in targets {
                self.emit_own_targets(node);
            }
        }
    }

    // -- 锚点 -----------------------------------------------------------------

    fn emit_node_targets(&mut self, node: NodeRef<'a>) {
        let ids: Vec<String> = self
            .anchor_targets
            .targets(node)
            .into_iter()
            .map(str::to_owned)
            .collect();
        for id in ids {
            if self.emitted_anchors.insert(id.clone()) {
                let markup = anchors::markup(&id);
                self.writer().anchor(&markup);
            }
        }
    }

    fn emit_own_targets(&mut self, node: NodeRef<'a>) {
        let ids: Vec<String> = self
            .anchor_targets
            .targets(node)
            .into_iter()
            .map(str::to_owned)
            .collect();
        for id in ids {
            if self.emitted_anchors.insert(id.clone()) {
                let markup = anchors::markup(&id);
                self.writer().anchor(&markup);
            }
        }
    }

    fn emit_anchors_within(&mut self, node: NodeRef<'a>, remaining: usize) {
        for id in anchors::within(node, remaining, &self.anchor_targets) {
            if self.emitted_anchors.insert(id.clone()) {
                self.writer().anchor(&anchors::markup(&id));
            }
        }
    }
}

// -- ego_tree 视图层 ----------------------------------------------------------

/// 节点渲染类别（替代 Dom::NodeKind：scraper 树直接视图）。
enum NodeKind<'a> {
    Document,
    Element(&'a str),
    Text(&'a str),
    Other,
}

fn node_kind<'a>(node: NodeRef<'a>) -> NodeKind<'a> {
    match node.value() {
        SNode::Document | SNode::Fragment => NodeKind::Document,
        SNode::Element(el) if is_placeholder_img(el) => NodeKind::Other,
        SNode::Element(el) => NodeKind::Element(el.name()),
        // scraper 文本节点在解析时已完成实体解码。
        SNode::Text(text) => NodeKind::Text(&text.text),
        SNode::Comment(_) | SNode::Doctype(_) | SNode::ProcessingInstruction(_) => NodeKind::Other,
    }
}

/// 懒加载占位图判定（解析器级视图约定）。
///
/// 懒加载站点（BBC 等）的每个图片位先给一张禁用-JS 场景的灰色占位图，
/// JS 就绪后由浏览器隐藏并换上真图。静态转换不执行 JS，两张 img 都会被
/// 遍历——把占位图判成 Other 让状态机整体跳过（不产出、不进子树），
/// 等效于浏览器「有 JS 视图」。
///
/// 判据（URL 关键词）：src 含 `placeholder` / `spacer`，或 data: URI 小图
/// （1×1 像素的 base64 载荷 ≤ 100 字符）。各站占位图命名高度趋同，误伤面
/// 可忽略；srcSet/src 为真图的节点不命中。
fn is_placeholder_img(el: &scraper::node::Element) -> bool {
    if el.name() != "img" {
        return false;
    }
    let Some(src) = el.attr("src") else {
        return false;
    };
    let lower = src.to_ascii_lowercase();
    if lower.contains("placeholder") || lower.contains("spacer") {
        return true;
    }
    // data:image/...;base64,<载荷>：1×1 透明 PNG/GIF 的 base64 载荷 ≤ 100 字符
    lower.starts_with("data:image/")
        && lower
            .rsplit("base64,")
            .next()
            .is_some_and(|payload| payload.len() <= 100)
}

#[cfg(test)]
mod tests;
