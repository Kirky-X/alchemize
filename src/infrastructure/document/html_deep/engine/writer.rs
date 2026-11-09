// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 行内输出管线：文本/样式/代码事件 → 转义与空白折叠 → 片段树。
//!
//! 核心机制（行为规格 §3）：
//! - **延迟格式化**：文本事件进缓冲；`push_style`/`pop_style`/`boundary` 等
//!   事件先冲刷缓冲再改状态——标记永远落在真实内容边界上；
//! - **样式栈**：期望栈（desired）与已发射栈（emitted）分离；闭合惰性发生
//!   （下一 `prepare_inline` 或 `finish` 的 [`Writer::close_to`]）；空链接经
//!   `end_link` 强制物化保住目的地；
//! - **空白三路分流**：ASCII 空白折叠（`space` 记号）；Unicode 空白保留
//!   （`preserved_spaces` 缓冲，行首剥掉其中 ASCII 空格）；可见字符按转义表发射；
//! - **边界**：`boundary` 记账，下一可见内容物化时抵扣已有尾随换行；
//!   空输出不产生前导换行；
//! - **行内代码**：`code` 累积缓冲；`code_with_edges`（EndCode 路径）先冲刷
//!   旧缓冲——相邻 code span 由此分开，第二个在 finish 时经相邻检测切 `<code>`；
//! - **finish**：闭合全部样式、trim 尾部 ASCII 空白。

use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, GeneralCategoryGroup},
};

use super::output::Output;

/// 行内样式。`Link` 携带序号（空链接判定）、目的地与标题。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    Strong,
    Em,
    Strike,
    Sup,
    Sub,
    Link {
        serial: usize,
        dest: String,
        title: Option<String>,
    },
}

/// 已发射的样式标记。
struct OpenStyle {
    style: Style,
    /// 开标记在 `output` 中的字节偏移（闭合需 HTML 时原地重写开标记）。
    start: usize,
    /// 开标记是否为 HTML 标签（探测字符决策；闭合跟随）。
    html: bool,
    /// 子样式已闭合（嵌套闭合边界被打断时保守改写 HTML）。
    has_closed_child: bool,
    /// 子链含 sup/sub HTML 标签（打断闭合 run）。
    has_closed_script: bool,
}

pub(crate) struct Writer {
    prefix: Output,
    output: String,
    pending_text: String,
    /// 行内代码缓冲（code 事件累积，围栏延迟发射）。
    code: Option<String>,
    /// 最近一个 Markdown code span 结束于 output 的字节偏移（相邻检测）。
    code_markdown_end: Option<usize>,
    /// 最近一个数学跨度结束于 output 的字节偏移（防 `$…$1` 误配，T015 接入）。
    math_end: Option<usize>,
    /// 折叠中的 ASCII 空白。
    space: bool,
    /// 保留的 Unicode 空白（含其前被折叠的一个 ASCII 空格）。
    preserved_spaces: String,
    /// 本行 ASCII 数字计数（None = 非"纯数字行"）。
    line_digits: Option<usize>,
    pub(crate) heading: bool,
    pub(crate) has_blocks: bool,
    /// 是否产出过任何可见内容。
    pub(crate) content_seen: bool,
    pending_break: usize,
    desired: Vec<Style>,
    emitted: Vec<OpenStyle>,
    /// `emitted` 与 `desired` 的最长公共样式前缀长度（增量维护，prepare_inline
    /// 逐字符调用时免重算）。样式入栈后不可变、两栈只做尾部增删，前缀相等性
    /// 只在 pop（收缩）与 push（新样式恰好衔接且与仍未闭合的旧发射同款时延展）处变化。
    common: usize,
    /// 已发射链接的最大序号（空链接判定：end_link 序号大于它 = 从未发射）。
    last_link: usize,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self {
            prefix: Output::new(),
            output: String::new(),
            pending_text: String::new(),
            code: None,
            code_markdown_end: None,
            math_end: None,
            space: false,
            preserved_spaces: String::new(),
            line_digits: None,
            heading: false,
            has_blocks: false,
            content_seen: false,
            pending_break: 0,
            desired: Vec::new(),
            emitted: Vec::new(),
            common: 0,
            last_link: 0,
        }
    }

    /// 子捕获 writer：继承期望样式栈（嵌套内容延续父上下文），其余归零。
    pub(crate) fn child(&self) -> Self {
        Self {
            desired: self.desired.clone(),
            heading: self.heading,
            ..Self::new()
        }
    }

    /// 子 writer 的可观测状态并入父（内容计数与链接序号）。
    pub(crate) fn merge_child(&mut self, child: &Self) {
        self.last_link = self.last_link.max(child.last_link);
        self.content_seen |= child.content_seen;
    }

    /// HTML fallback 的格式化上下文：是否处于链接内、块级 fallback 是否
    /// 在激活样式内（需要 `<div>` 包裹重建）。
    pub(crate) fn html_context(&self, block: bool) -> super::fallback::FormattingContext {
        super::fallback::FormattingContext {
            in_link: self.in_link(),
            wrapped_block: block && !self.desired.is_empty(),
        }
    }

    /// 硬换行 `  \\n`（br 元素）：闭合样式、物化边界后写入。
    pub(crate) fn hard_break(&mut self) {
        self.content_seen = true;
        self.flush_pending_text();
        self.flush_code();
        self.close_to(0, None);
        self.flush_spaces(false);
        self.flush_breaks();
        self.output.push_str("  \n");
        self.space = false;
        self.line_digits = Some(0);
    }

    /// 追加单个已成型块（hr/代码块/引用块/标题行等：内容即块本体）。
    pub(crate) fn block(&mut self, block: Output, before: usize, after: usize) {
        self.content_seen = true;
        self.append_block(before, after, block);
    }

    // -- 输入侧 ---------------------------------------------------------------

    /// 文本事件：只入缓冲；含非空白字符即计为内容（可达名兜底的
    /// 判定发生在冲刷前，计数必须与缓冲同步——对齐 Content::text 语义）。
    pub(crate) fn text(&mut self, s: &str) {
        if !s.is_empty() {
            self.content_seen |= s.chars().any(|ch| !ch.is_whitespace());
            self.pending_text.push_str(s);
        }
    }

    /// 行内代码事件：累积进缓冲，围栏延迟到格式边界。
    pub(crate) fn code(&mut self, text: &str) {
        self.flush_pending_text();
        if text.starts_with(is_space) {
            self.flush_code();
            self.space = true;
        }
        let text = text.trim_matches(is_space);
        if text.is_empty() {
            return;
        }
        self.content_seen = true;
        if self.code.is_none() {
            self.prepare_inline('`');
            self.code = Some(String::new());
        }
        let code = self.code.get_or_insert_with(String::new);
        let mut space = false;
        for ch in text.chars() {
            if is_space(ch) {
                space = true;
            } else {
                if space {
                    code.push(' ');
                    space = false;
                }
                code.push(ch);
            }
        }
    }

    /// EndCode 路径：先冲刷旧缓冲（相邻 span 由此分开），首尾空白外置为
    /// 普通文本，内部进代码围栏。
    pub(crate) fn code_with_edges(&mut self, text: &str) {
        self.flush_pending_text();
        // 空元素没有可见边缘：保持待定代码，交由下一可见内容决定是否需要间隔。
        if text.is_empty() {
            return;
        }
        self.flush_code();
        let content = text.trim_matches(char::is_whitespace);
        if content.is_empty() {
            self.text(text);
            return;
        }
        let leading = text.len() - text.trim_start_matches(char::is_whitespace).len();
        let trailing = text.trim_end_matches(char::is_whitespace).len();
        if leading > 0 {
            self.text(&text[..leading]);
        }
        self.code(content);
        if trailing < text.len() {
            self.flush_code();
            self.text(&text[trailing..]);
        }
    }

    /// 行内 HTML 片段（净化后的 MathML 等）：原样发射，计为可见内容。
    pub(crate) fn inline_html(&mut self, markup: &str) {
        if !markup.is_empty() {
            self.content_seen = true;
        }
        self.markup(markup);
    }

    /// 锚点标记（span id）：不计为可见内容。
    pub(crate) fn anchor(&mut self, markup: &str) {
        self.markup(markup);
    }

    /// 消费 HTML fallback 产物：块级 → 前后 2 空行的块；行内 → 原样标记。
    /// 含内容时计为可见内容。
    pub(crate) fn html(&mut self, rendered: super::fallback::Html, block: bool) {
        if rendered.has_content {
            self.content_seen = true;
        }
        if rendered.markup.is_empty() {
            return;
        }
        if block {
            let mut out = Output::new();
            out.push_text(rendered.markup);
            self.append_block(2, 2, out);
        } else {
            self.markup(&rendered.markup);
        }
    }

    /// 原样发射标记串（不转义、不折叠）。
    fn markup(&mut self, markup: &str) {
        if markup.is_empty() {
            return;
        }
        self.flush_pending_text();
        self.flush_code();
        self.prepare_inline('<');
        self.output.push_str(markup);
        self.line_digits = None;
    }

    /// 图片语法 `![alt](dest "title")`：alt 为字面文本（空白折叠后逐字转义）。
    pub(crate) fn image(&mut self, alt: &str, src: &str, title: Option<&str>) {
        self.content_seen = true;
        self.flush_pending_text();
        self.flush_code();
        self.prepare_inline('!');
        self.output.push_str("![");
        // alt 是字面文本，不是二次 Markdown 输入：空白折叠为单空格后转义。
        for (index, word) in alt.split(is_space).filter(|w| !w.is_empty()).enumerate() {
            if index > 0 {
                self.output.push(' ');
            }
            for ch in word.chars() {
                match ch {
                    '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' | '$' => {
                        self.output.push('\\');
                        self.output.push(ch);
                    }
                    '&' => self.output.push_str("&amp;"),
                    c => self.output.push(c),
                }
            }
        }
        self.output.push_str("](");
        self.output.push_str(&escape_dest(src));
        if let Some(t) = title.filter(|t| !t.is_empty()) {
            push_title(&mut self.output, t);
        }
        self.output.push(')');
        self.line_digits = None;
    }

    /// 压入样式。返回 false = 拒入（同款已在期望栈、或链接内压链接），
    /// 子树照常遍历但不产标记。sup/sub 例外：总是入栈。
    pub(crate) fn push_style(&mut self, style: Style) -> bool {
        // sup/sub 恒入栈，免做 O(深度) 查重扫描（深嵌套链的平方项来源）。
        let duplicate = match &style {
            Style::Sup | Style::Sub => false,
            _ => self.desired.contains(&style),
        };
        if duplicate || (matches!(style, Style::Link { .. }) && self.in_link()) {
            return false;
        }
        // 先冲刷缓冲再入栈：已缓冲文本必须按旧样式集物化。
        self.flush_pending_text();
        self.flush_code();
        let appended = self.desired.len();
        self.desired.push(style);
        // 前缀延展：新样式恰好落在公共前缀末位，且与仍未闭合的旧发射同款
        // （`**a**` 紧跟 `**b**`——旧 Strong 还开着，新 Strong 续用同一标记）。
        // Link 的 serial 全局唯一，新串绝不与旧发射同款——same_style 返回 false。
        if self.common == appended
            && self.emitted.len() > self.common
            && same_style(&self.emitted[self.common].style, &self.desired[self.common])
        {
            self.common += 1;
        }
        true
    }

    /// 弹出样式：只弹期望栈；闭合在下一内容事件或 finish 惰性发生。
    pub(crate) fn pop_style(&mut self) {
        self.flush_pending_text();
        self.flush_code();
        self.desired.pop();
        self.common = self.common.min(self.desired.len());
    }

    /// 结束链接：空链接（从未发射标记）仍强制物化 `[](dest)`（目的地不丢）。
    pub(crate) fn end_link(&mut self, serial: usize) {
        self.flush_pending_text();
        self.flush_code();
        if self.last_link < serial {
            self.prepare_inline('[');
        }
        self.pop_style();
    }

    /// 请求 n 个换行的段落边界（2 = 空行，1 = 单换行）。闭合并冲刷全部行内状态。
    pub(crate) fn boundary(&mut self, n: usize) {
        self.flush_pending_text();
        self.has_blocks |= n > 1;
        self.flush_code();
        self.close_to(0, None);
        self.flush_spaces(false);
        self.space = false;
        self.pending_break = self.pending_break.max(n);
    }

    // -- 位置状态 -------------------------------------------------------------

    fn is_empty(&self) -> bool {
        self.output.is_empty() && self.prefix.is_empty()
    }

    fn last_char(&self) -> Option<char> {
        self.output.chars().last().or(self.prefix.last_char())
    }

    fn in_link(&self) -> bool {
        self.desired.iter().any(|s| matches!(s, Style::Link { .. }))
    }

    // -- 冲刷 -----------------------------------------------------------------

    /// 物化惰性边界（下一可见内容/块拼接时调用）。
    fn flush_breaks(&mut self) {
        if self.pending_break == 0 {
            return;
        }
        let breaks = std::mem::take(&mut self.pending_break);
        // space 标志不在此重置：折叠空格随边界后的首个可见内容物化
        // （"  **" 用例——重置会丢失边界前最后一个 ASCII 空格）。
        // 空格重置属于 boundary() 的 flush_spaces(false)。
        if !self.is_empty() {
            // 抵扣：已物化输出的尾随换行；输出全为换行时连同 prefix 尾一起算。
            let mut existing = self.output.len() - self.output.trim_end_matches('\n').len();
            if existing == self.output.len() {
                existing += self.prefix.trailing_newlines();
            }
            for _ in existing..breaks {
                self.output.push('\n');
            }
        }
        self.line_digits = Some(0);
    }

    /// 物化保留空格与折叠空格。`trailing_ascii` 控制折叠空格是否随行内内容物化。
    fn flush_spaces(&mut self, trailing_ascii: bool) {
        if !self.preserved_spaces.is_empty() {
            self.flush_breaks();
            // 行首剥掉保留缓冲中的 ASCII 空格（NBSP 等保留）。
            let text = if self.is_empty() || self.last_char() == Some('\n') {
                self.preserved_spaces.trim_start_matches(' ').to_string()
            } else {
                std::mem::take(&mut self.preserved_spaces)
            };
            self.output.push_str(&text);
            self.preserved_spaces.clear();
            self.line_digits = None;
        }
        if trailing_ascii && self.space && !self.is_empty() && self.last_char() != Some('\n') {
            self.output.push(' ');
            self.line_digits = None;
        }
        self.space = false;
    }

    /// 冲刷文本缓冲：TeX 跨度保留（§5a：定界符跨透明行内包装可识别——
    /// 依赖延迟转义）→ 逐字转义。
    fn flush_pending_text(&mut self) {
        if self.pending_text.is_empty() {
            return;
        }
        self.flush_code();
        let text = std::mem::take(&mut self.pending_text);
        let mut consumed = 0;
        for (start, end, normalized) in super::math::spans(&text) {
            self.plain_text(&text[consumed..start]);
            self.prepare_inline('$');
            // 相邻数学跨度之间插注释，防止 `$…$$…$` 误配对。
            if self.math_end == Some(self.output.len()) {
                self.output.push_str("<!-- -->");
            }
            // TeX 命令/下标/对齐符承载语义：Markdown 转义会改变数学阅读器
            // 的语义——原样发射（数学载荷不转义）。
            self.output.push_str(&normalized);
            self.math_end = Some(self.output.len());
            self.line_digits = None;
            consumed = end;
        }
        self.plain_text(&text[consumed..]);
    }

    /// 逐字转义发射（行为规格 §3 转义表）。
    fn plain_text(&mut self, text: &str) {
        for ch in text.chars() {
            if is_space(ch) {
                self.space = true;
                continue;
            }
            if ch.is_whitespace() {
                // Unicode 空白保留：其前折叠的 ASCII 空格一并入保留缓冲。
                if self.space {
                    self.preserved_spaces.push(' ');
                    self.space = false;
                }
                self.preserved_spaces.push(ch);
                continue;
            }
            self.prepare_inline(ch);
            // 数学跨度后紧跟数字：插注释防 `$…$1` 误配（T015 接入 math_end）。
            if ch.is_ascii_digit() && self.math_end == Some(self.output.len()) {
                self.output.push_str("<!-- -->");
            }
            let line_start = self.is_empty() || self.last_char() == Some('\n');
            if line_start {
                self.line_digits = Some(0);
            }
            match ch {
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '$' => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '#' if line_start || self.heading => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '+' | '-' | '=' if line_start => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '.' | ')' if self.line_digits.is_some_and(|d| d > 0) => {
                    self.output.push('\\');
                    self.output.push(ch);
                }
                '&' => self.output.push_str("&amp;"),
                _ => self.output.push(ch),
            }
            self.line_digits = if ch.is_ascii_digit() {
                self.line_digits.map(|n| n + 1)
            } else {
                None
            };
            self.content_seen = true;
        }
    }

    /// 行内代码冲刷：围栏 = 最长反引号 run + 1；围栏碰撞时两侧加空格 padding；
    /// 相邻 Markdown code span 切 `<code>` 防围栏合并。
    fn flush_code(&mut self) {
        if self.code_markdown_end == Some(self.output.len()) {
            self.flush_code_html();
            return;
        }
        if let Some(code) = self.code.take() {
            let fence = "`".repeat(longest_run(&code, '`') + 1);
            let padding = code.starts_with('`')
                || code.ends_with('`')
                || (code.starts_with(' ') && code.ends_with(' ') && code.chars().any(|c| c != ' '));
            self.output.push_str(&fence);
            if padding {
                self.output.push(' ');
            }
            self.output.push_str(&code);
            if padding {
                self.output.push(' ');
            }
            self.output.push_str(&fence);
            self.code_markdown_end = Some(self.output.len());
            self.line_digits = None;
        }
    }

    /// 相邻 code span 的 HTML 形态：`& < >` 实体化 + ASCII 标点反斜杠转义。
    fn flush_code_html(&mut self) {
        if let Some(code) = self.code.take() {
            self.output.push_str("<code>");
            for ch in code.chars() {
                match ch {
                    '&' => self.output.push_str("&amp;"),
                    '<' => self.output.push_str("&lt;"),
                    '>' => self.output.push_str("&gt;"),
                    c if c.is_ascii_punctuation() => {
                        self.output.push('\\');
                        self.output.push(c);
                    }
                    c => self.output.push(c),
                }
            }
            self.output.push_str("</code>");
            self.code_markdown_end = None;
            self.line_digits = None;
        }
    }

    /// 格式化延迟的终点：物化边界、闭合已弹出样式到公共前缀、物化空格、
    /// 立即发射新开标记（探测字符决策是否改 HTML 标签）。
    fn prepare_inline(&mut self, next: char) {
        self.flush_breaks();
        // 公共前缀已增量维护（push/pop/close_to/本函数末尾），逐字符免重算。
        let common = self.common;
        // 新链接会在标签前发射 '['：边界探测把这个字符算进去。
        let link = self.desired[common..]
            .iter()
            .position(|s| matches!(s, Style::Link { .. }))
            .map(|index| common + index);
        let next_after_close = if self.space || !self.preserved_spaces.is_empty() {
            ' '
        } else if link.is_some() {
            '['
        } else {
            next
        };
        self.close_to(common, Some(next_after_close));
        self.flush_spaces(true);
        for index in common..self.desired.len() {
            // 探测字符：紧邻内层样式的标记字符，或真实下一字符。
            let first = match self.desired.get(index + 1) {
                Some(Style::Strong | Style::Em) => '*',
                Some(Style::Strike) => '~',
                Some(Style::Sup | Style::Sub) => '<',
                Some(Style::Link { .. }) => '[',
                None => next,
            };
            // 词字符与标点之间的开标记无法开 CommonMark 跨度 → 改 HTML 标签。
            let html = is_punctuation(first) && self.last_char().is_some_and(is_wordlike);
            let style = self.desired[index].clone();
            let start = self.output.len();
            match &style {
                Style::Strong => self.output.push_str(if html { "<strong>" } else { "**" }),
                Style::Em => self.output.push_str(if html { "<em>" } else { "*" }),
                Style::Strike => self.output.push_str(if html { "<del>" } else { "~~" }),
                Style::Sup => self.output.push_str("<sup>"),
                Style::Sub => self.output.push_str("<sub>"),
                Style::Link { serial, .. } => {
                    self.last_link = self.last_link.max(*serial);
                    // 紧邻 [ 的字面 ! 在链接开标记处转义（插到 ! 之前）。
                    if self.output.ends_with('!') {
                        self.output.insert(self.output.len() - 1, '\\');
                    }
                    self.output.push('[');
                }
            }
            self.emitted.push(OpenStyle {
                style,
                start,
                html,
                has_closed_child: false,
                has_closed_script: false,
            });
        }
        // 全部期望样式已发射：两栈完全同步。
        self.common = self.desired.len();
    }

    /// 闭合 emitted[count..]（行为规格 §3 闭合规则）：
    /// - sup/sub 恒 HTML 闭合；链接闭合为 `](dest "title")`；
    /// - Strong/Em/Strike：同标记相邻闭合合并为单次 run（`***`），run 的左边缘
    ///   是 run 起点前的字符而非刚发射的标记——嵌套 strong/em 因此能以纯
    ///   Markdown 闭合；
    /// - 保守 HTML 改写：子已闭合且（本链含 HTML 标签 ∨ run 将合并）∨ 闭合
    ///   边界（前标点、后词字符）∨ 开标记已是 HTML——开标记原地重写；
    /// - 链接/HTML/sup/sub 结束 run（保留各自标点边缘）。
    fn close_to(&mut self, count: usize, next: Option<char>) {
        // 截断后 emitted[..count] 仍是 desired 的前缀：公共前缀收缩到 count。
        self.common = self.common.min(count);
        // 正在形成的闭合 run：(标记字节, run 左边缘字符)。
        let mut closing_run: Option<(u8, Option<char>)> = None;
        let mut closed_script = false;
        let closed_child = self.emitted.len() > count;
        while self.emitted.len() > count {
            let opened = self
                .emitted
                .pop()
                .unwrap_or_else(|| unreachable!("style stack paired"));
            let delimiters = match &opened.style {
                Style::Strong => Some(("**", "<strong>", "</strong>")),
                Style::Em => Some(("*", "<em>", "</em>")),
                Style::Strike => Some(("~~", "<del>", "</del>")),
                Style::Sup => {
                    self.output.push_str("</sup>");
                    closing_run = None;
                    closed_script = true;
                    None
                }
                Style::Sub => {
                    self.output.push_str("</sub>");
                    closing_run = None;
                    closed_script = true;
                    None
                }
                Style::Link { dest, title, .. } => {
                    self.output.push_str("](");
                    self.output.push_str(&escape_dest(dest));
                    if let Some(t) = title.as_deref().filter(|t| !t.is_empty()) {
                        push_title(&mut self.output, t);
                    }
                    self.output.push(')');
                    closing_run = None;
                    None
                }
            };
            if let Some((marker, open_tag, close_tag)) = delimiters {
                let marker_byte = marker.as_bytes()[0];
                // run 延续时左边缘沿用 run 起点前的字符（刚发射的内层标记不算）。
                let preceding = match closing_run {
                    Some((previous, preceding))
                        if previous == marker_byte && !opened.has_closed_child =>
                    {
                        preceding
                    }
                    _ => self.last_char(),
                };
                let closing_needs_html = (opened.has_closed_child
                    && (closed_script
                        || opened.has_closed_script
                        || closing_run.is_some_and(|(previous, _)| previous == marker_byte)))
                    || (next.is_some_and(is_wordlike) && preceding.is_some_and(is_punctuation));
                if opened.html || closing_needs_html {
                    if !opened.html {
                        // 仅改写本仓产出：祖先开标记都在此偏移之前，偏移仍有效。
                        self.output
                            .replace_range(opened.start..opened.start + marker.len(), open_tag);
                    }
                    self.output.push_str(close_tag);
                    closing_run = None;
                } else {
                    self.output.push_str(marker);
                    closing_run = Some((marker_byte, preceding));
                }
            }
        }
        if closed_child {
            // 子标记可与后继兄弟配对（***a**b**c***d 类），保守边缘检查保留。
            for opened in &mut self.emitted {
                opened.has_closed_child = true;
                opened.has_closed_script |= closed_script;
            }
        }
    }

    // -- 块拼接与收尾 ---------------------------------------------------------

    /// 追加已物化块：boundary(before) → 冲刷 → 行内尾部固化 → 推块 → boundary(after)。
    pub(crate) fn append_block(&mut self, before: usize, after: usize, block: Output) {
        if block.is_empty() {
            self.boundary(before.max(after));
            return;
        }
        self.flush_pending_text();
        self.flush_code();
        self.boundary(before);
        self.flush_breaks();
        if !self.output.is_empty() {
            let tail = std::mem::take(&mut self.output);
            self.prefix.push_text(tail);
        }
        self.prefix.push_block(block);
        self.line_digits = None;
        self.code_markdown_end = None;
        self.math_end = None;
        self.boundary(after);
    }

    /// 冲刷缓冲与代码，不闭合样式（machine 在收尾前弹空样式栈）。
    pub(crate) fn materialize_pending(&mut self) {
        self.flush_pending_text();
        self.flush_code();
    }

    /// 收尾：闭合全部样式、trim 尾部 ASCII 空白，产出片段树。
    pub(crate) fn finish(mut self) -> Output {
        self.flush_pending_text();
        self.flush_code();
        self.close_to(0, None);
        self.flush_spaces(false);
        let end = self.output.trim_end_matches(is_space).len();
        self.output.truncate(end);
        self.prefix.push_text(std::mem::take(&mut self.output));
        self.prefix
    }
}

/// ASCII 空白（折叠对象；与 Unicode 保留空白分流的判据）。
pub(crate) fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// 样式同一性（O(1)）：Link 只比 serial——serial 由全局计数器分配，同 serial
/// 即同一次压入（dest/title 必相同），免去长 URL 逐字节比较。
fn same_style(a: &Style, b: &Style) -> bool {
    match (a, b) {
        (Style::Link { serial: s1, .. }, Style::Link { serial: s2, .. }) => s1 == s2,
        _ => a == b,
    }
}

/// CommonMark 口径的标点：ASCII 标点直判，非 ASCII 走 Unicode P/S 分类。
fn is_punctuation(ch: char) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_punctuation();
    }
    GeneralCategoryGroup::Punctuation
        .union(GeneralCategoryGroup::Symbol)
        .contains(CodePointMapData::<GeneralCategory>::new().get(ch))
}

/// 词字符：非空白且非标点。
fn is_wordlike(ch: char) -> bool {
    !ch.is_whitespace() && !is_punctuation(ch)
}

/// 最长连续 run（围栏长度）。
pub(crate) fn longest_run(text: &str, marker: char) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in text.chars() {
        if ch == marker {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// 链接/图片目的地转义：`\ ( )` 反斜杠；空白与尖括号百分号化；`&` 实体化。
fn escape_dest(dest: &str) -> String {
    let mut out = String::with_capacity(dest.len());
    for ch in dest.chars() {
        match ch {
            '\\' | '(' | ')' => {
                out.push('\\');
                out.push(ch);
            }
            ' ' => out.push_str("%20"),
            '\t' => out.push_str("%09"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '&' => out.push_str("&amp;"),
            c => out.push(c),
        }
    }
    out
}

/// 追加 ` "title"`（`\ "` 反斜杠转义）。
fn push_title(out: &mut String, title: &str) {
    out.push_str(" \"");
    for ch in title.chars() {
        match ch {
            '\\' | '"' => {
                out.push('\\');
                out.push(ch);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests;
