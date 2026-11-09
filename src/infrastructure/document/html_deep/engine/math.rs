// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 作者手写 TeX 跨度识别（行为规格 §5a）。
//!
//! 在普通 Markdown 转义**之前**定位完整的 TeX 跨度并原样保留——引擎不做
//! MathML→LaTeX 转换，作者已写的 TeX 是活行为。`$` 边界遵循"非空白开启/
//! 非尾随空白/非货币数字闭合"规则，防止价格吞掉两个金额之间的散文。
//!
//! 性能不变量：扫描步数 ≤ 字节数（线性，防恶意输入拖垮转换）。

/// 定位文本中的完整 TeX 跨度：`(起始字节, 结束字节, 归一化载荷含定界符)`。
/// 候选按起点排序、重叠者保留先到者；显式 `\(\)`/`\[\]` 占据区间但载荷为
/// 原样字面（防区间内 `$` 对再匹配）。
pub(super) fn spans(text: &str) -> Vec<(usize, usize, String)> {
    let mut steps = 0usize;
    spans_counted(text, &mut steps)
}

fn spans_counted(text: &str, steps: &mut usize) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut backslashes = 0;
    let mut dollar: Option<usize> = None;
    let mut double_dollar: Option<usize> = None;
    let mut paren: Option<usize> = None;
    let mut bracket: Option<usize> = None;
    let mut candidates: Vec<(usize, usize, Option<String>)> = Vec::new();
    while index < bytes.len() {
        *steps += 1;
        if bytes[index] == b'$' && backslashes % 2 == 0 {
            if bytes.get(index + 1) == Some(&b'$') {
                if let Some(start) = double_dollar.take() {
                    accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                } else {
                    double_dollar = Some(index);
                }
                index += 2;
                backslashes = 0;
                continue;
            }
            if let Some(start) = dollar.take() {
                // 单 $ 闭合失败（尾空白/货币数字）→ 允许重开。
                if !accept_candidate(text, start, index + 1, 1, true, &mut candidates)
                    && text[index + 1..]
                        .chars()
                        .next()
                        .is_some_and(|ch| !ch.is_whitespace())
                {
                    dollar = Some(index);
                }
            } else if text[index + 1..]
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_whitespace())
            {
                dollar = Some(index);
            }
            index += 1;
            backslashes = 0;
            continue;
        }
        if bytes[index] == b'\\' && backslashes % 2 == 0 {
            match bytes.get(index + 1) {
                Some(b'(') => {
                    paren.get_or_insert(index);
                }
                Some(b'[') => {
                    bracket.get_or_insert(index);
                }
                Some(b')' | b']') => {
                    let start = if bytes[index + 1] == b')' {
                        paren.take()
                    } else {
                        bracket.take()
                    };
                    if let Some(start) = start {
                        accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                        // 完整显式跨度拥有其内部定界符（载荷字面化时，
                        // 内部 $ 对不能再把片段拆成公式）。
                        dollar = dollar.filter(|inner| *inner < start);
                        double_dollar = double_dollar.filter(|inner| *inner < start);
                        paren = paren.filter(|inner| *inner < start);
                        bracket = bracket.filter(|inner| *inner < start);
                    }
                }
                _ => {}
            }
            if matches!(bytes.get(index + 1), Some(b'(' | b'[' | b')' | b']')) {
                index += 2;
                backslashes = 0;
                continue;
            }
        }
        if bytes[index] == b'\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
        index += 1;
    }
    candidates.sort_unstable_by_key(|candidate| candidate.0);
    let mut end = 0;
    candidates.retain(|candidate| {
        let keep = candidate.0 >= end;
        if keep {
            end = candidate.1;
        }
        keep
    });
    candidates
        .into_iter()
        .filter_map(|(start, end, normalized)| normalized.map(|value| (start, end, value)))
        .collect()
}

/// 接受候选：单 $ 的拒绝规则（空载荷/尾空白/货币数字）+ Markdown 资源模式。
/// 被拒绝的完整跨度仍占据区间（载荷 None = 原样字面回退）。
#[allow(clippy::too_many_arguments)]
fn accept_candidate(
    text: &str,
    start: usize,
    end: usize,
    delimiter: usize,
    single_dollar: bool,
    candidates: &mut Vec<(usize, usize, Option<String>)>,
) -> bool {
    let content_start = start + delimiter;
    let content_end = end - delimiter;
    let followed_by_digit = text.as_bytes().get(end).is_some_and(u8::is_ascii_digit);
    if content_start == content_end
        || (single_dollar
            && (text[..content_end]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
                || followed_by_digit))
    {
        return false;
    }
    let content = &text[content_start..content_end];
    let span = &text[start..end];
    let has_resource = contains_markdown_resource(content);
    // $ 候选保留货币/资源拒绝与重试规则；显式定界符仍拥有资源类字面回退。
    if has_resource && !span.starts_with('\\') {
        return false;
    }
    let normalized = if has_resource {
        None
    } else {
        normalized_span(span)
    };
    candidates.push((start, end, normalized));
    true
}

/// Markdown 资源模式：`]( `、`][`、`![[`。
fn contains_markdown_resource(text: &str) -> bool {
    text.contains("](") || text.contains("][") || text.contains("![[")
}

/// 归一化 TeX 载荷（行为规格 §5a）：剥定界符；空白 run → 单空格（TeX 语义）；
/// `%` 注释删至行尾（控制词拼接语义）；`\verb|…|` 原样保留；text 类命令
/// 花括号参数跟踪（内含 HTML 起始 → 拒绝）；普通上下文 `<` 后跟字母类
/// → 改写 `< `；拒绝：空输出、花括号不平衡、裸 `$`、结尾悬空反斜杠。
/// 重新包装：`\(` 或单 `$` → `$…$`；`$$` 或 `\[` → `$$…$$`。
fn normalized_span(text: &str) -> Option<String> {
    let delimiter = if text.starts_with("$$") || text.starts_with('\\') {
        2
    } else {
        1
    };
    let mut output = String::new();
    let mut chars = text[delimiter..text.len() - delimiter].chars().peekable();
    let mut escaped = false;
    let mut control_word = false;
    let mut space = false;
    let mut text_depth = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '\\' && !escaped {
            let command: String = chars
                .clone()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            if command == "verb" {
                for _ in 0..command.len() {
                    chars.next();
                }
                flush_space(&mut output, &mut space);
                output.push_str(r"\verb");
                if chars.peek() == Some(&'*') {
                    output.push('*');
                    chars.next();
                }
                let delimiter = chars
                    .next()
                    .filter(|c| !c.is_whitespace() && !c.is_ascii_alphabetic() && *c != '<')?;
                output.push(delimiter);
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if matches!(c, '\n' | '\r') || html_start(c, chars.peek().copied()) {
                        return None;
                    }
                    output.push(c);
                    if c == delimiter {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return None;
                }
                control_word = false;
                continue;
            }
            const TEXT_COMMANDS: &[&str] = &[
                "text",
                "mbox",
                "textrm",
                "textbf",
                "textit",
                "texttt",
                "textsf",
                "textnormal",
                "hbox",
            ];
            if TEXT_COMMANDS.contains(&command.as_str()) {
                let mut probe = chars.clone();
                for _ in 0..command.len() {
                    probe.next();
                }
                while probe.peek().is_some_and(char::is_ascii_whitespace) {
                    probe.next();
                }
                if probe.next() == Some('{') {
                    flush_space(&mut output, &mut space);
                    output.push('\\');
                    output.push_str(&command);
                    output.push('{');
                    chars = probe;
                    text_depth += 1;
                    control_word = false;
                    continue;
                }
            }
        }
        if ch == '%' && !escaped {
            while let Some(part) = chars.next() {
                if part == '\r' {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    break;
                }
                if part == '\n' {
                    break;
                }
            }
            // TeX 删除注释与换行且不插入 token；仅当拼接会改写控制词时补空格。
            space |= control_word && chars.peek().is_some_and(char::is_ascii_alphabetic);
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
            if escaped {
                output.push(' ');
            } else {
                space = true;
            }
            escaped = false;
            control_word = false;
            continue;
        }
        if space {
            control_word = false;
        }
        flush_space(&mut output, &mut space);
        if html_start(ch, chars.peek().copied()) {
            if text_depth > 0 {
                return None;
            }
            output.push_str("< ");
        } else {
            output.push(ch);
        }
        if !escaped && text_depth > 0 {
            if ch == '{' {
                text_depth += 1;
            } else if ch == '}' {
                text_depth -= 1;
            }
        }
        control_word = ch.is_ascii_alphabetic() && (escaped || control_word);
        escaped = ch == '\\' && !escaped;
    }
    if output.is_empty() || text_depth > 0 || !markdown_payload_is_safe(&output) {
        return None;
    }
    let inline = text.starts_with("\\(") || delimiter == 1;
    if inline && output.ends_with(char::is_whitespace) {
        // 空 TeX 组保住前导控制空格，同时行内 Markdown 定界符保持在非空白后。
        output.push_str("{}");
    }
    let marker = if inline { "$" } else { "$$" };
    Some(format!("{marker}{output}{marker}"))
}

/// 载荷 Markdown 安全检查：裸 `$`、不平衡花括号、结尾悬空反斜杠 → 拒绝。
fn markdown_payload_is_safe(text: &str) -> bool {
    let mut escaped = false;
    let mut brace_depth = 0usize;
    for ch in text.chars() {
        // math 阅读器在 \verb 内也跟踪花括号上下文。
        match (ch, escaped) {
            ('$', false) => return false,
            ('{', false) => brace_depth += 1,
            ('}', false) if brace_depth == 0 => return false,
            ('}', false) => brace_depth -= 1,
            _ => {}
        }
        escaped = ch == '\\' && !escaped;
    }
    !escaped && brace_depth == 0
}

fn flush_space(output: &mut String, pending: &mut bool) {
    if *pending && !output.is_empty() {
        output.push(' ');
    }
    *pending = false;
}

/// HTML 起始序列：`<` 后跟字母 / `/` / `!` / `?`。
fn html_start(ch: char, next: Option<char>) -> bool {
    ch == '<'
        && next.is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, '/' | '!' | '?'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> String {
        spans(text)
            .into_iter()
            .next()
            .map(|(_, _, v)| v)
            .unwrap_or_default()
    }

    #[test]
    fn inline_dollar_spans_normalized() {
        assert_eq!(one("$x_i$"), "$x_i$");
        assert_eq!(one("a $\\alpha$ b"), "$\\alpha$");
        assert_eq!(one("$ x $"), "", "开启空白 → 拒绝");
        assert_eq!(one("$1"), "", "货币数字 → 拒绝");
    }

    #[test]
    fn explicit_paren_spans_rewrapped() {
        assert_eq!(one("\\(x+1\\)"), "$x+1$");
        assert_eq!(one("\\[x+1\\]"), "$$x+1$$");
    }

    #[test]
    fn currency_does_not_swallow_prose() {
        // 两个金额之间的散文不被吞。
        assert_eq!(one("costs $5 and $7 total"), "");
    }

    #[test]
    fn dollar_rejection_retries() {
        // "a $x b$ c"：首个 $ 的载荷含尾空白 → 闭合失败重开，第二个配对成功。
        assert_eq!(one("a $x b$ c"), "$x b$");
    }

    #[test]
    fn comment_stripped_to_line_end() {
        assert_eq!(one("$x% note\n y$"), "$x y$");
    }

    #[test]
    fn html_in_text_command_rejects() {
        assert_eq!(one("$\\text{<b>"), "");
        // 普通上下文的 < 改写 "< "
        assert_eq!(one("$a<b$"), "$a< b$");
    }

    #[test]
    fn markdown_resource_rejects_dollar() {
        assert_eq!(one("$[x](y$"), "", "资源模式 → $ 定界拒绝");
        // 显式定界符：载荷字面化（占据区间但不产出）
        assert_eq!(one("\\([x](y\\)"), "");
    }

    #[test]
    fn verbatim_preserved() {
        // verb 载荷原样保留；载荷内的裸 $ 会被 payload 安全检查拒绝
        //（markdown_payload_is_safe 对 $ 不豁免 verb）——真源语义。
        assert_eq!(one("$\\verb|a+b|c$"), "$\\verb|a+b|c$");
        assert_eq!(one("$\\verb|a$b|c$"), "");
    }

    #[test]
    fn scan_is_linear() {
        for text in ["\\".repeat(8_192), "\\(x ".repeat(4_096)] {
            let mut steps = 0;
            let _ = spans_counted(&text, &mut steps);
            assert!(
                steps <= text.len(),
                "{steps} steps for {} bytes",
                text.len()
            );
        }
    }

    #[test]
    fn overlapping_candidates_first_wins() {
        // $$ 与 $ 重叠：先到者保留。
        assert_eq!(one("$$a$"), "");
        assert_eq!(one("$$a$$"), "$$a$$");
    }

    #[test]
    fn empty_payload_rejected() {
        assert_eq!(one("$$"), "");
        assert_eq!(one("\\(\\)"), "");
    }
}
