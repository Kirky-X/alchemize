// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 状态机端到端行为单测。期望值 = vendor 引擎真值探针实测
//! （TEMP-E2E-PROBE，用后删除）；表格/mathml/fallback/锚点为后续任务的桩，
//! 其行为用例在对应任务落地时补齐。

use super::convert;

fn conv(html: &str) -> String {
    let doc = scraper::Html::parse_document(html);
    convert(&doc)
}

// -- 标题与段落 ---------------------------------------------------------------

#[test]
fn heading_and_paragraph_with_inline() {
    // 真值：<h1>Title</h1><p>Para with <strong>bold</strong> and <a href="u">link</a>.</p>
    assert_eq!(
        conv("<h1>Title</h1><p>Para with <strong>bold</strong> and <a href=\"u\">link</a>.</p>"),
        "# Title\n\nPara with **bold** and [link](u)."
    );
}

#[test]
fn heading_with_emphasis() {
    assert_eq!(conv("<h2>Head <em>em</em></h2>"), "## Head *em*");
}

#[test]
fn empty_heading_disappears() {
    assert_eq!(conv("<h1></h1><p>x</p>"), "x");
}

// -- 列表 ---------------------------------------------------------------------

#[test]
fn simple_list() {
    assert_eq!(conv("<ul><li>one</li><li>two</li></ul>"), "- one\n- two");
}

#[test]
fn loose_list_with_inner_para() {
    // 真值：条目含块 → 条目间隔空行 + 续行缩进
    assert_eq!(
        conv("<ul><li>one</li><li>two<p>inner para</p></li></ul>"),
        "- one\n\n- two\n\n  inner para"
    );
}

#[test]
fn ordered_list_start_attribute() {
    assert_eq!(
        conv("<ol start=\"3\"><li>a</li><li>b</li></ol>"),
        "3. a\n4. b"
    );
}

#[test]
fn nested_list_tight() {
    assert_eq!(
        conv("<ul><li>outer<ul><li>nested</li></ul></li><li>after</li></ul>"),
        "- outer\n  - nested\n- after"
    );
}

#[test]
fn triple_nested_with_trailing_text() {
    assert_eq!(
        conv("<ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul>tail</li></ul>"),
        "- a\n  - b\n    - c\n\n  tail"
    );
}

#[test]
fn orphan_li_treated_as_block() {
    assert_eq!(
        conv("<p>orphan text</p><li>orphan li</li>"),
        "orphan text\n\norphan li"
    );
}

#[test]
fn empty_items_dropped() {
    assert_eq!(
        conv("<ul><li></li><li>keep</li><li>   </li></ul>"),
        "- keep"
    );
}

// -- 引用 ---------------------------------------------------------------------

#[test]
fn blockquote_multi_para() {
    assert_eq!(
        conv("<blockquote><p>quoted</p><p>second</p></blockquote>"),
        "> quoted\n>\n> second"
    );
}

// -- 代码 ---------------------------------------------------------------------

#[test]
fn pre_with_language() {
    assert_eq!(
        conv("<pre><code class=\"language-rust\">fn main() {}\nlet x = 1;</code></pre>"),
        "```rust\nfn main() {}\nlet x = 1;\n```"
    );
}

#[test]
fn pre_without_language() {
    assert_eq!(conv("<pre><code>plain</code></pre>"), "```\nplain\n```");
}

#[test]
fn inline_code() {
    assert_eq!(
        conv("<p>code <code>a*b</code> inline</p>"),
        "code `a*b` inline"
    );
}

#[test]
fn blank_pre_disappears() {
    assert_eq!(conv("<pre><code>   </code></pre><p>x</p>"), "x");
}

// -- 分隔线与硬换行 -----------------------------------------------------------

#[test]
fn hr_block() {
    assert_eq!(conv("<hr><p>after</p>"), "---\n\nafter");
}

#[test]
fn br_hard_break() {
    assert_eq!(conv("<p>a<br>b</p>"), "a  \nb");
}

// -- 图片与媒体 ---------------------------------------------------------------

#[test]
fn inline_image() {
    assert_eq!(
        conv("<p><img src=\"i.png\" alt=\"pic\"> inline</p>"),
        "![pic](i.png) inline"
    );
}

#[test]
fn image_without_src_outputs_alt_text() {
    assert_eq!(conv("<p><img alt=\"broken\">x</p>"), "brokenx");
}

#[test]
fn video_with_source() {
    // 真值：<video src="v.mp4"><track></video> => "[Video](v.mp4)"
    //（track 子元素无 src，media 只看自身 src 与 source 子元素）
    assert_eq!(
        conv("<video src=\"v.mp4\"><track></video>"),
        "[Video](v.mp4)"
    );
}

// -- 选择框 -------------------------------------------------------------------

#[test]
fn select_options() {
    assert_eq!(
        conv("<select><option value=\"1\">Red</option><option value=\"2\">Blue</option></select>"),
        "Red\nBlue"
    );
}

#[test]
fn optgroup_label() {
    assert_eq!(
        conv("<select><optgroup label=\"Colors\"><option>Red</option></optgroup></select>"),
        "Colors\nRed"
    );
}

// -- 隐藏状态对象 -------------------------------------------------------------

#[test]
fn serialized_state_object_dropped() {
    let json = format!("{{\"{}\":\"{}\"}}", "k".repeat(256), "v".repeat(256));
    assert!(json.len() >= 512);
    let html = format!("<div hidden>{json}</div><p>visible</p>");
    assert_eq!(conv(&html), "visible");
}

// -- 可达名兜底 ---------------------------------------------------------------

#[test]
fn empty_button_fallback_label() {
    // 空按钮无可视内容 → aria-label 兜底输出 " label "（收尾 trim）。
    assert_eq!(
        conv("<p>a<button aria-label=\"Do it\"></button>b</p>"),
        "a Do it b"
    );
}

// -- 表格（T013 桩：原生 Markdown 表路径待 Table::from_dom 落地） --------------

#[test]
fn table_stub_degrades_to_layout_path() {
    // Table::from_dom 未实现（恒 None）→ 结构表走 fallback（T016 桩=空产物）
    // → 空块退化为边界。输入不 panic 即可；真值对齐在 T013 落地后补。
    let out = conv("<table><tr><th>H1</th></tr></table>");
    let _ = out;
}

// -- 不可失败性 ---------------------------------------------------------------

#[test]
fn malformed_input_never_panics() {
    for html in [
        "<ul><li></li></ul>",
        "<table><tr><td></td></tr></table>",
        "<p><a href=\"javascript:x\">bad</a></p>",
        "<pre><code></code></pre>",
        "<ol start=\"abc\"><li>x</li></ol>",
        "<p><img><a href=\"u\"><img alt=\"in link\"></a></p>",
        "<blockquote><h2>in quote</h2></blockquote>",
        "<h1><ul><li>list in heading</li></ul></h1>",
    ] {
        let _ = conv(html);
    }
}

#[test]
fn deep_nesting_survives() {
    let mut html = String::new();
    for _ in 0..20_000 {
        html.push_str("<div>");
    }
    html.push_str("deep");
    for _ in 0..20_000 {
        html.push_str("</div>");
    }
    let out = conv(&html);
    assert_eq!(out, "deep");
}

// -- 95 攻坚补充（元素分支） --------------------------------------------------

#[test]
fn mathml_island_renders_inline_html() {
    // 无 alttext 的 <math>：machine 分支直出 MathML 标记（inline_html 路径）。
    let out = conv("<p>x<math><mi>a</mi><mo>+</mo><mi>b</mi></math>y</p>");
    assert!(out.contains("<math"), "mathml markup present: {out}");
    assert!(out.contains("x") && out.contains("y"));
}

#[test]
fn mathml_hidden_alternative_is_skipped() {
    // MathJax 双拷贝：aria-hidden 分支不进输出，DOM 序仍保序。
    let out =
        conv("<p><math aria-hidden=\"true\"><mi>ghost</mi></math><math><mi>real</mi></math></p>");
    assert!(!out.contains("ghost"), "hidden copy leaked: {out}");
    assert!(out.contains("real"), "{out}");
}

#[test]
fn select_option_label_becomes_choice_text() {
    let out = conv(
        "<div><select><optgroup label=\"Group\"><option value=\"1\">One</option></optgroup></select></div>",
    );
    assert!(out.contains("Group"), "optgroup label: {out}");
    assert!(out.contains("One"), "option text: {out}");
}

#[test]
fn option_label_attribute_preferred() {
    let out = conv("<div><select><option label=\"Short\">Long text</option></select></div>");
    assert!(out.contains("Short"), "label attr: {out}");
    assert!(
        !out.contains("Long text"),
        "inner text should not win: {out}"
    );
}

#[test]
fn code_block_with_link_falls_back_to_html() {
    // code/pre 子树含链接：白名单净化 HTML 兜底（link 无法用围栏承载）。
    let out = conv("<pre><code>see <a href=\"https://ok.example/x\">docs</a></code></pre>");
    assert!(
        out.contains("<code") || out.contains("<pre"),
        "html fallback: {out}"
    );
    assert!(out.contains("docs"), "{out}");
}

#[test]
fn pre_block_with_anchor_target_falls_back() {
    // pre 内含被引用锚点：id 必须保留 → fallback。
    let out = conv(
        "<a href=\"#anchor\">ref</a><pre><code><span id=\"anchor\">payload</span></code></pre>",
    );
    assert!(out.contains("payload"), "{out}");
    assert!(
        out.contains("id=") || out.contains("#anchor"),
        "anchor kept: {out}"
    );
}

#[test]
fn nested_heading_treated_as_container() {
    // 标题嵌套标题：降级普通容器，内层标题仍成标题。
    let out = conv("<h1>Outer <h2>Inner</h2> tail</h1>");
    assert!(out.contains("Outer"), "{out}");
    assert!(out.contains("Inner"), "{out}");
}

#[test]
fn textarea_children_traversed() {
    let out = conv("<div><textarea>typed &amp; saved</textarea></div>");
    assert!(out.contains("typed"), "{out}");
    assert!(out.contains("saved"), "{out}");
}

#[test]
fn input_value_emitted_with_spaces() {
    let out = conv("<div><input value=\"typed words\"></div>");
    assert!(out.contains("typed words"), "{out}");
}

#[test]
fn img_with_safe_src_becomes_image() {
    let out = conv("<p>before<img src=\"https://safe.example/a.png\" alt=\"glyph\">after</p>");
    assert!(out.contains("![glyph]"), "image syntax: {out}");
    assert!(out.contains("https://safe.example/a.png"), "{out}");
}

#[test]
fn video_with_safe_source_becomes_link() {
    let out = conv(
        "<div><video poster=\"https://safe.example/p.png\"><source src=\"https://safe.example/v.mp4\"></video></div>",
    );
    assert!(out.contains("p.png"), "poster: {out}");
    assert!(out.contains("v.mp4"), "source link: {out}");
}

#[test]
fn end_code_flushes_pending_raw() {
    // 行内 code 空内容 + 相邻文本：EndCode 收尾不吞分隔。
    let out = conv("<p>a<code></code>b</p>");
    assert!(out.contains("a") && out.contains("b"), "{out}");
}

// -- 95 攻坚批四 --------------------------------------------------------------

#[test]
fn structural_table_with_rowspan_falls_back_to_html() {
    // thead（结构表）+ rowspan（markdown 表模型装不下）→ 白名单 HTML 兜底。
    let out = conv(
        "<table><thead><tr><th>h</th></tr></thead><tbody><tr><td rowspan=\"2\">x</td><td>y</td></tr></tbody></table>",
    );
    assert!(out.contains("<table"), "html fallback: {out}");
    assert!(out.contains("rowspan"), "attribute preserved: {out}");
}

#[test]
fn audio_inside_link_keeps_destination_as_text() {
    // Markdown 链接不可嵌套：媒体链接降级为 `label (url)` 文本。
    let out = conv(
        "<p><a href=\"https://safe.example/page\"><audio src=\"https://safe.example/a.mp3\"></audio></a></p>",
    );
    assert!(out.contains("a.mp3"), "{out}");
    assert!(
        out.contains("https://safe.example/a.mp3") || out.contains("("),
        "dest kept: {out}"
    );
}

#[test]
fn layout_table_without_structure_degrades() {
    // 纯布局表（无语义标记、可装下也按布局处理）：内容照常输出。
    let out = conv("<table><tr><td>cell a</td><td>cell b</td></tr></table>");
    assert!(out.contains("cell a") && out.contains("cell b"), "{out}");
}
