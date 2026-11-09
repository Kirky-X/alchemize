// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 行内管线行为单测。期望值 = vendor 引擎真值探针实测（TEMP-DIFF-PROBE），
//! 构造-事件序列与 machine（T007+）将产出的序列一致。

use super::*;

fn rendered(f: impl FnOnce(&mut Writer)) -> String {
    let mut w = Writer::new();
    f(&mut w);
    w.finish().into_string()
}

// -- 文本管线（T003） ----------------------------------------------------------

#[test]
fn plain_text_passthrough() {
    assert_eq!(rendered(|w| w.text("hello world")), "hello world");
}

#[test]
fn ascii_whitespace_folds_to_single_space() {
    assert_eq!(rendered(|w| w.text("a  \n\t  b")), "a b");
}

#[test]
fn leading_whitespace_at_line_start_dropped() {
    assert_eq!(rendered(|w| w.text("   text")), "text");
}

#[test]
fn nbsp_preserved_verbatim() {
    assert_eq!(rendered(|w| w.text("a\u{a0}b")), "a\u{a0}b");
    // 行首保留缓冲中的 ASCII 空格被剥，NBSP 本体保留
    assert_eq!(rendered(|w| w.text("\u{a0}x")), "\u{a0}x");
}

#[test]
fn constant_escape_set() {
    assert_eq!(
        rendered(|w| w.text("\\ ` * _ [ ] < > ~ $")),
        "\\\\ \\` \\* \\_ \\[ \\] \\< \\> \\~ \\$"
    );
}

#[test]
fn ampersand_entity_escaped() {
    assert_eq!(rendered(|w| w.text("a&b")), "a&amp;b");
}

#[test]
fn hash_escapes_at_line_start_and_heading() {
    assert_eq!(rendered(|w| w.text("# tag")), "\\# tag");
    assert_eq!(rendered(|w| w.text("a # tag")), "a # tag");
    let mut w = Writer::new();
    w.heading = true;
    w.text("a # b");
    assert_eq!(w.finish().into_string(), "a \\# b");
}

#[test]
fn line_start_markers_escape() {
    assert_eq!(rendered(|w| w.text("+ plus")), "\\+ plus");
    assert_eq!(rendered(|w| w.text("- dash")), "\\- dash");
    assert_eq!(rendered(|w| w.text("= eq")), "\\= eq");
    assert_eq!(rendered(|w| w.text("a - b")), "a - b");
}

#[test]
fn ordered_list_forgery_escaped() {
    // 引擎只转义 ". )"（数字不转义）；'-1.' 的 '-' 在行首被转义后
    // "仅数字行"状态终止，'.' 不再转义。真值：<p>1. one</p> => "1\\. one"
    assert_eq!(rendered(|w| w.text("1. one")), "1\\. one");
    assert_eq!(rendered(|w| w.text("42. answer")), "42\\. answer");
    assert_eq!(rendered(|w| w.text("1.")), "1\\.");
    assert_eq!(rendered(|w| w.text("point 1.")), "point 1.");
    assert_eq!(rendered(|w| w.text("-1. negative")), "\\-1. negative");
}

#[test]
fn digit_state_resets_after_block_boundary() {
    assert_eq!(
        rendered(|w| {
            w.text("intro");
            let mut block = Output::new();
            block.push_static("---");
            w.append_block(2, 2, block);
            w.text("9. tail");
        }),
        "intro\n\n---\n\n9\\. tail"
    );
}

#[test]
fn boundary_materializes_two_newlines() {
    assert_eq!(
        rendered(|w| {
            w.text("a");
            w.boundary(2);
            w.text("b");
        }),
        "a\n\nb"
    );
}

#[test]
fn boundary_on_empty_output_produces_no_leading_newlines() {
    assert_eq!(
        rendered(|w| {
            w.boundary(2);
            w.text("b");
        }),
        "b"
    );
}

#[test]
fn pending_space_dropped_before_boundary() {
    assert_eq!(
        rendered(|w| {
            w.text("a ");
            w.boundary(2);
            w.text("b");
        }),
        "a\n\nb"
    );
}

#[test]
fn append_block_orders_inline_tail_and_block() {
    let mut w = Writer::new();
    w.text("intro");
    let mut block = Output::new();
    block.push_static("---");
    w.append_block(2, 2, block);
    w.text("after");
    assert_eq!(w.finish().into_string(), "intro\n\n---\n\nafter");
}

#[test]
fn empty_block_degrades_to_boundary() {
    assert_eq!(
        rendered(|w| {
            w.text("a");
            w.append_block(2, 2, Output::new());
            w.text("b");
        }),
        "a\n\nb"
    );
}

#[test]
fn finish_trims_all_trailing_whitespace() {
    assert_eq!(rendered(|w| w.text("text  \n\t ")), "text");
}

// -- 样式栈（T004）——期望值来自真值探针 ----------------------------------------

#[test]
fn nested_strong_em_plain_close() {
    // 真值：<strong><em>abc</em></strong> => "***abc***"
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Strong);
            w.push_style(Style::Em);
            w.text("abc");
            w.pop_style();
            w.pop_style();
        }),
        "***abc***"
    );
}

#[test]
fn interleaved_styles_close_plain() {
    // 真值：<strong>a<em>b</em>c</strong> => "**a*b*c**"
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Strong);
            w.text("a");
            w.push_style(Style::Em);
            w.text("b");
            w.pop_style();
            w.text("c");
            w.pop_style();
        }),
        "**a*b*c**"
    );
}

#[test]
fn strong_em_trailing_structure() {
    // 真值：<strong>a <em>b</em></strong> => "**a *b***"
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Strong);
            w.text("a ");
            w.push_style(Style::Em);
            w.text("b");
            w.pop_style();
            w.pop_style();
        }),
        "**a *b***"
    );
}

#[test]
fn em_strong_nested() {
    // 真值：<em>a <strong>b</strong> c</em> => "*a **b** c*"
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Em);
            w.text("a ");
            w.push_style(Style::Strong);
            w.text("b");
            w.pop_style();
            w.text(" c");
            w.pop_style();
        }),
        "*a **b** c*"
    );
}

#[test]
fn same_style_push_rejected() {
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Strong);
            w.text("a");
            assert!(!w.push_style(Style::Strong));
            w.text("b");
            w.pop_style();
            w.pop_style();
        }),
        "**ab**"
    );
}

#[test]
fn sup_sub_html_markers() {
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Sup);
            w.text("2");
            w.pop_style();
        }),
        "<sup>2</sup>"
    );
}

#[test]
fn sup_inside_strong_keeps_plain_close() {
    // 真值：<strong>a<sup>2</sup></strong> => "**a<sup>2</sup>**"
    // sup 的 HTML 不改写父样式闭合（仅开标记已是 HTML / 闭合边界才改写）。
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Strong);
            w.text("a");
            w.push_style(Style::Sup);
            w.text("2");
            w.pop_style();
            w.pop_style();
        }),
        "**a<sup>2</sup>**"
    );
}

#[test]
fn probe_char_html_open_between_word_and_punct() {
    // 真值：word<strong><em>x</em></strong> => "word<strong>*x*</strong>"
    // 词字符后紧跟标点探测字符（内层 '*'）：开标记改 HTML。
    assert_eq!(
        rendered(|w| {
            w.text("word");
            w.push_style(Style::Strong);
            w.push_style(Style::Em);
            w.text("x");
            w.pop_style();
            w.pop_style();
        }),
        "word<strong>*x*</strong>"
    );
}

#[test]
fn link_with_dest_and_title() {
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Link {
                serial: 1,
                dest: "https://e.x/a b".into(),
                title: Some("ti\"tle".into()),
            });
            w.text("t");
            w.pop_style();
            w.end_link(1);
        }),
        "[t](https://e.x/a%20b \"ti\\\"tle\")"
    );
}

#[test]
fn link_inside_link_rejected() {
    // 真值：<a href="u">a</a><a href="u">b</a> => "[a](u)[b](u)"；
    // 链接内压链接拒入后，外层目的地保留。
    assert_eq!(
        rendered(|w| {
            assert!(w.push_style(Style::Link {
                serial: 1,
                dest: "outer".into(),
                title: None
            }));
            w.text("a");
            assert!(!w.push_style(Style::Link {
                serial: 2,
                dest: "inner".into(),
                title: None
            }));
            w.text("b");
            w.end_link(2);
            w.pop_style();
            w.end_link(1);
        }),
        "[ab](outer)"
    );
}

#[test]
fn adjacent_links() {
    // 真值：<a href="u">a</a><a href="u">b</a> => "[a](u)[b](u)"
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Link {
                serial: 1,
                dest: "u".into(),
                title: None,
            });
            w.text("a");
            w.end_link(1);
            w.push_style(Style::Link {
                serial: 2,
                dest: "u".into(),
                title: None,
            });
            w.text("b");
            w.end_link(2);
        }),
        "[a](u)[b](u)"
    );
}

#[test]
fn empty_link_keeps_dest() {
    // 真值：<a href="u"></a> => "[](u)"
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Link {
                serial: 1,
                dest: "u".into(),
                title: None,
            });
            w.end_link(1);
        }),
        "[](u)"
    );
}

#[test]
fn link_open_escapes_leading_exclamation() {
    // 真值：wow!<a href="u">t</a> => "wow\\![t](u)"
    assert_eq!(
        rendered(|w| {
            w.text("wow!");
            w.push_style(Style::Link {
                serial: 1,
                dest: "u".into(),
                title: None,
            });
            w.text("t");
            w.pop_style();
            w.end_link(1);
        }),
        "wow\\![t](u)"
    );
}

#[test]
fn strike_marker() {
    assert_eq!(
        rendered(|w| {
            w.push_style(Style::Strike);
            w.text("gone");
            w.pop_style();
        }),
        "~~gone~~"
    );
}

// -- 行内代码与图片（T005）——期望值来自真值探针 --------------------------------

#[test]
fn inline_code_fence() {
    assert_eq!(
        rendered(|w| {
            w.code("a ` b");
            w.text(" after");
        }),
        "``a ` b`` after"
    );
}

#[test]
fn code_padding_rules() {
    // EndCode 路径（code_with_edges）：真值 <code>`x</code> => "`` `x ``"
    // （围栏碰撞时两侧 padding）；首尾空白外置为普通文本。
    assert_eq!(rendered(|w| w.code_with_edges("x")), "`x`");
    assert_eq!(rendered(|w| w.code_with_edges("`x")), "`` `x ``");
    // 首尾空白外置：行首 ASCII 空格被剥、尾部被 finish trim
    assert_eq!(rendered(|w| w.code_with_edges(" x ")), "`x`");
}

#[test]
fn adjacent_code_spans_become_html() {
    // EndCode 序列真值：<code>a</code><code>b</code> => "`a`<code>b</code>"
    // （第二个 span 的 flush_code 撞上相邻偏移 → HTML 形态）
    assert_eq!(
        rendered(|w| {
            w.code_with_edges("a");
            w.code_with_edges("b");
        }),
        "`a`<code>b</code>"
    );
}

#[test]
fn code_whitespace_folds() {
    assert_eq!(rendered(|w| w.code("a  \n b")), "`a b`");
}

#[test]
fn image_syntax_with_alt_escaping() {
    assert_eq!(
        rendered(|w| w.image("a [b] c", "u u", None)),
        "![a \\[b\\] c](u%20u)"
    );
}

#[test]
fn image_title_quoted_and_escaped() {
    assert_eq!(
        rendered(|w| w.image("alt", "u", Some("t\\t"))),
        "![alt](u \"t\\\\t\")"
    );
}

#[test]
fn image_alt_whitespace_folds() {
    assert_eq!(rendered(|w| w.image("a  \n b", "u", None)), "![a b](u)");
}
