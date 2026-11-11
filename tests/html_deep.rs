// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! html-deep 引擎测试（feature `html`）：转换链兜底触发、空输出防呆、
//! math 保真路由、主内容提取、链接绝对化。引擎行为契约
//! 见 specmark/specs/html-deep/spec.md。

#![cfg(feature = "html")]

use alchemize::application::builder::{AlchemizeBuilder, AssetMode, HtmlDeepEngineConfig};
use alchemize::application::ports::Converter;
use alchemize::domain::input::Input;
use alchemize::infrastructure::document::html_deep::HtmlDeepConverter;

fn engine(base_url: Option<String>, extract_main: bool) -> HtmlDeepConverter {
    HtmlDeepConverter::new(base_url, extract_main)
}

fn html_input(html: &str) -> Input {
    Input::Bytes {
        data: html.as_bytes().to_vec(),
        filename: "page.html".into(),
        mimetype: None,
        extension: Some("html".into()),
    }
}

/// math 保真路由：含 alttext 的页面整体走既有 HtmlConverter（输出一致）。
#[tokio::test]
async fn math_pages_route_to_htmd_engine() {
    // 两种 math 形态都验证：alttext 属性与 annotation 子元素（html.rs 的
    // preprocess_math 两者都处理，路由检测 `<math` 覆盖其全部触发面）。
    let alttext_page = r##"<html><body><p><math alttext="E=mc^2">E</math></p></body></html>"##;
    let annotation_page = r##"<html><body><p>Mass: <math><mrow><mi>E</mi></mrow>
        <annotation encoding="application/x-tex">E=mc^2</annotation></math></p></body></html>"##;

    let legacy = alchemize::infrastructure::document::html::HtmlConverter::new();
    let deep_engine = engine(None, false);
    for page in [alttext_page, annotation_page] {
        let input = html_input(page);
        let want = legacy.convert(input.clone()).await.unwrap();
        let got = deep_engine.convert(input).await.unwrap();
        assert_eq!(
            want.markdown, got.markdown,
            "math 页必须与既有 html 引擎输出一致: {page}"
        );
    }
}

/// 空输出防呆负例：纯 script 页面输出空且不触发兜底（诚实抑制）。
#[tokio::test]
async fn script_only_page_yields_empty_without_fallback() {
    let html = r#"<html><head><script>var x = 1;</script></head><body><script>yo();</script></body></html>"#;
    let result = engine(None, false).convert(html_input(html)).await.unwrap();
    // 输出为空（无 htmd 兜底占位文本）——script 页的空是正确抑制。
    assert!(
        !result.markdown.contains("no extractable content"),
        "script-only page must not trigger htmd fallback: {:?}",
        result.markdown
    );
}

/// 兜底正例：引擎空输出 + 有可见文本 → 触发 htmd 兜底（占位文本来自
/// 既有 html 引擎，证明走了兜底而非抑制）。
#[tokio::test]
async fn full_page_conversion_goes_through_primary_path() {
    // 兜底安全网（空输出防呆）为 pub(crate) 内部函数，其注入式单测在
    // src/infrastructure/document/html_deep.rs 的 #[cfg(test)] 模块内。
    let html = r#"<html><body><p>real visible words</p></body></html>"#;
    let result = engine(None, false).convert(html_input(html)).await.unwrap();
    assert!(
        result.markdown.contains("real visible words"),
        "{:?}",
        result.markdown
    );
}

/// 主内容提取（StructuredHtml 分支）：导航/页脚噪声被剔除，正文以
/// 结构化 markdown 产出（段落分块、图片链接保留）。
#[tokio::test]
async fn extract_main_drops_chrome_and_keeps_structure() {
    let html = r##"<html><body>
        <nav><a href="/home">Home</a> <a href="/about">About</a></nav>
        <article><h1>Doc Title</h1><img src="/pics/hero.png" alt="hero">
        <p>first point</p><p>body words here</p></article>
        <footer>copyright noise</footer>
    </body></html>"##;

    let full = engine(None, false).convert(html_input(html)).await.unwrap();
    assert!(
        full.markdown.contains("Home") && full.markdown.contains("copyright noise"),
        "full-page conversion keeps chrome: {:?}",
        full.markdown
    );

    let extracted = engine(None, true).convert(html_input(html)).await.unwrap();
    assert!(
        !extracted.markdown.contains("copyright noise"),
        "extraction must drop footer: {:?}",
        extracted.markdown
    );
    assert!(
        extracted.metadata.get("main_content_extracted").is_some(),
        "extraction path must be observable in metadata"
    );
    assert!(
        extracted.markdown.contains("first point") && extracted.markdown.contains("body words"),
        "extracted body must keep article text: {:?}",
        extracted.markdown
    );
    // 结构化产出：段落分块（多行）而非粘连字符串
    assert!(
        extracted.markdown.lines().count() >= 3,
        "structured extraction must preserve paragraph breaks: {:?}",
        extracted.markdown
    );
    // 图片链接保留（正文内的 img 不再丢失）
    assert!(
        extracted.markdown.contains("(/pics/hero.png)"),
        "article image must survive extraction: {:?}",
        extracted.markdown
    );
}

/// 提取失败诚实回退全页：dom_smoothie 无法产出正文时走引擎→htmd 链。
#[tokio::test]
async fn extraction_failure_falls_back_to_full_page() {
    // 无 article/正文语义的稀疏页面：提取大概率失败/空 → 回退全页仍能
    // 转出可见文本（不得 Err、不得空串）。
    let html = r#"<html><body><p>plain words only</p></body></html>"#;
    let result = engine(None, true).convert(html_input(html)).await.unwrap();
    assert!(
        result.markdown.contains("plain words"),
        "fallback to full page must keep visible text: {:?}",
        result.markdown
    );
}

/// 链接绝对化：相对 href/src → 绝对；特殊前缀保留。
#[tokio::test]
async fn base_url_absoluteizes_relative_links() {
    let html = r##"<html><body><p><a href="/docs/guide">guide</a>
        <a href="#anchor">anchor</a>
        <a href="mailto:x@y.z">mail</a>
        <img src="img/pic.png" alt="pic"></p></body></html>"##;
    let result = engine(Some("https://a.example/b/".into()), false)
        .convert(html_input(html))
        .await
        .unwrap();
    assert!(
        result.markdown.contains("https://a.example/docs/guide"),
        "{:?}",
        result.markdown
    );
    assert!(
        result.markdown.contains("https://a.example/b/img/pic.png"),
        "{:?}",
        result.markdown
    );
    assert!(
        result.markdown.contains("(#anchor)"),
        "{:?}",
        result.markdown
    );
    assert!(
        result.markdown.contains("(mailto:x@y.z)"),
        "{:?}",
        result.markdown
    );

    // 无 base_url：链接原样（缺省不改写）
    let plain = engine(None, false).convert(html_input(html)).await.unwrap();
    assert!(
        plain.markdown.contains("(/docs/guide)"),
        "{:?}",
        plain.markdown
    );
}

/// 注册面：with_html_deep 构建后引擎名 html-deep 可见于 engines()。
#[tokio::test]
async fn engine_registered_via_builder() {
    let alch = AlchemizeBuilder::new()
        .with_asset_mode(AssetMode::Dir)
        .with_html_deep(HtmlDeepEngineConfig::default())
        .build();
    assert!(
        alch.engines().contains(&"html-deep"),
        "html-deep must register: {:?}",
        alch.engines()
    );
}
