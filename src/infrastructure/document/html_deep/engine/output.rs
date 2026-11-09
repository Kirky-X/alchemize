// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Markdown 输出片段树。
//!
//! 设计约束（行为规格 §3 Output）：
//! - 长度/末字符/尾随换行数 O(1) 维护——writer 的边界物化与块拼接决策依赖这三个量；
//! - [`Output::into_string`] 迭代展平为单一 `String`（预分配总长，深嵌套不递归）；
//! - [`Drop`] 迭代化——嵌套深度不变成栈深度；
//! - 复制量线性：静态片段零拷贝（`&'static str`），块级片段以所有权移动拼装。

/// 树中的一段文本或一个嵌套块。
enum Part {
    /// 编译期常量片段（标签语法、围栏标记等），拼装时零拷贝。
    Static(&'static str),
    /// 运行期文本。
    Text(String),
    /// 已物化的嵌套块（子 writer 产物）。
    Block(Box<Output>),
}

/// 片段树：只追加、可整体消费为 `String`。
pub(crate) struct Output {
    parts: Vec<Part>,
    len: usize,
    end: Option<char>,
    newlines: usize,
}

impl Output {
    pub(crate) fn new() -> Self {
        Self {
            parts: Vec::new(),
            len: 0,
            end: None,
            newlines: 0,
        }
    }

    /// 是否为空（O(1)）。
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 末字符（O(1)）；空输出返回 None。
    pub(crate) fn last_char(&self) -> Option<char> {
        self.end
    }

    /// 末尾连续 `\n` 的数量（O(1)）。
    pub(crate) fn trailing_newlines(&self) -> usize {
        self.newlines
    }

    fn record_tail(&mut self, appended: &str, prev_newlines: usize) {
        if appended.is_empty() {
            return;
        }
        let bytes = appended.as_bytes();
        // '\n' 是 ASCII，按字节从尾部数安全。
        let mut k = 0usize;
        while k < bytes.len() && bytes[bytes.len() - 1 - k] == b'\n' {
            k += 1;
        }
        if k == bytes.len() {
            // 追加段全为换行：尾随 run 跨越拼接边界延续。
            self.newlines = prev_newlines + k;
        } else {
            self.newlines = k;
        }
        // 末字符：从尾部回扫 UTF-8 字符边界（新增段必然非空）。
        let mut start = bytes.len() - 1;
        while start > 0 && (bytes[start] & 0b1100_0000) == 0b1000_0000 {
            start -= 1;
        }
        self.end = appended[start..].chars().next();
    }

    /// 从编译期常量构造单片段输出。
    pub(crate) fn from_static(s: &'static str) -> Self {
        let mut out = Self::new();
        out.push_static(s);
        out
    }

    /// 从运行期文本构造单片段输出。
    pub(crate) fn from_text(s: String) -> Self {
        let mut out = Self::new();
        out.push_text(s);
        out
    }

    /// 追加编译期常量片段。
    pub(crate) fn push_static(&mut self, s: &'static str) {
        let prev = self.newlines;
        self.len += s.len();
        self.parts.push(Part::Static(s));
        self.record_tail(s, prev);
    }

    /// 追加运行期文本。
    pub(crate) fn push_text(&mut self, s: String) {
        let prev = self.newlines;
        self.len += s.len();
        self.record_tail(&s, prev);
        self.parts.push(Part::Text(s));
    }

    /// 追加一个已物化的嵌套块（所有权移动；空块为无操作）。
    pub(crate) fn push_block(&mut self, block: Output) {
        if block.is_empty() {
            return;
        }
        self.len += block.len;
        // 尾状态取块自身聚合值：块的末字符/尾随换行就是拼接后的输出尾部。
        self.end = block.end;
        self.newlines = block.newlines;
        self.parts.push(Part::Block(Box::new(block)));
    }

    /// 迭代展平为单一 `String`：预分配总长，深嵌套不递归。
    pub(crate) fn into_string(mut self) -> String {
        let mut out = String::with_capacity(self.len);
        // 手写 Drop 使字段不可移出；mem::take 换出 parts，Drop 落在空壳上。
        let mut stack = vec![std::mem::take(&mut self.parts).into_iter()];
        while let Some(iter) = stack.last_mut() {
            match iter.next() {
                Some(Part::Static(s)) => out.push_str(s),
                Some(Part::Text(s)) => out.push_str(&s),
                Some(Part::Block(b)) => {
                    let mut block = *b;
                    // 手写 Drop 禁止字段移出；掏空后入栈，空壳即刻 Drop。
                    stack.push(std::mem::take(&mut block.parts).into_iter());
                }
                None => {
                    stack.pop();
                }
            }
        }
        out
    }
}

#[cfg(test)]
impl Output {
    /// 总字节数（O(1)；测试断言聚合字段用）。
    pub(crate) fn len(&self) -> usize {
        self.len
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // 递归 Drop 会让嵌套深度变成栈深度；此处迭代下沉所有嵌套块。
        let mut queue: Vec<Box<Output>> = std::mem::take(&mut self.parts)
            .into_iter()
            .filter_map(|p| match p {
                Part::Block(b) => Some(b),
                _ => None,
            })
            .collect();
        while let Some(mut b) = queue.pop() {
            queue.extend(
                std::mem::take(&mut b.parts)
                    .into_iter()
                    .filter_map(|p| match p {
                        Part::Block(inner) => Some(inner),
                        _ => None,
                    }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_has_neutral_fields() {
        let out = Output::new();
        assert_eq!(out.len(), 0);
        assert!(out.is_empty());
        assert_eq!(out.last_char(), None);
        assert_eq!(out.trailing_newlines(), 0);
        assert_eq!(out.into_string(), "");
    }

    #[test]
    fn static_and_text_parts_flatten_in_order() {
        let mut out = Output::new();
        out.push_static("**");
        out.push_text("bold".to_string());
        out.push_static("**");
        assert_eq!(out.len(), 8);
        assert_eq!(out.last_char(), Some('*'));
        assert_eq!(out.into_string(), "**bold**");
    }

    #[test]
    fn trailing_newlines_count_runs_across_boundaries() {
        let mut out = Output::new();
        out.push_text("a\n".to_string());
        assert_eq!(out.trailing_newlines(), 1);
        out.push_text("\n".to_string());
        // 全换行追加段延续跨边界 run
        assert_eq!(out.trailing_newlines(), 2);
        out.push_text("b".to_string());
        assert_eq!(out.trailing_newlines(), 0);
        out.push_text("\n\n".to_string());
        assert_eq!(out.trailing_newlines(), 2);
        assert_eq!(out.last_char(), Some('\n'));
        assert_eq!(out.into_string(), "a\n\nb\n\n");
    }

    #[test]
    fn block_append_aggregates_tail_state_and_len() {
        let mut inner = Output::new();
        inner.push_text("cell\n".to_string());
        let mut outer = Output::new();
        outer.push_static("| ");
        outer.push_block(inner);
        assert_eq!(outer.len(), "| ".len() + "cell\n".len());
        assert_eq!(outer.last_char(), Some('\n'));
        assert_eq!(outer.trailing_newlines(), 1);
        assert_eq!(outer.into_string(), "| cell\n");
    }

    #[test]
    fn empty_block_is_noop() {
        let mut out = Output::new();
        out.push_text("keep".to_string());
        out.push_block(Output::new());
        assert_eq!(out.len(), 4);
        assert_eq!(out.into_string(), "keep");
    }

    #[test]
    fn multibyte_last_char_survives() {
        let mut out = Output::new();
        out.push_text("汉字".to_string());
        assert_eq!(out.last_char(), Some('字'));
        assert_eq!(out.into_string(), "汉字");
    }

    #[test]
    fn deep_nesting_drops_without_stack_overflow() {
        // 5 万层嵌套：递归 Drop 会爆 2MiB 测试线程栈，迭代 Drop 必须存活。
        let mut out = Output::new();
        out.push_text("seed".to_string());
        for _ in 0..50_000 {
            let mut wrapped = Output::new();
            wrapped.push_block(std::mem::replace(&mut out, Output::new()));
            out = wrapped;
        }
        assert_eq!(out.into_string(), "seed");
    }

    #[test]
    fn deep_nesting_into_string_is_iterative() {
        let mut out = Output::new();
        out.push_static("S");
        for _ in 0..50_000 {
            let mut wrapped = Output::new();
            wrapped.push_block(std::mem::replace(&mut out, Output::new()));
            out = wrapped;
        }
        assert_eq!(out.into_string(), "S");
    }
}
