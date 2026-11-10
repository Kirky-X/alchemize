// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `html-deep` 引擎 —— 自研 HTML→Markdown 转换链（feature `html-deep`）。
//!
//! 主路径为本仓自研迭代状态机引擎（`html_deep/engine`，Apache-2.0，行为
//! 规格经差分验证，见 specmark 变更 `selfhost-html-engine`）；本文件是
//! 转换器适配层：链接绝对化、主内容提取（dom_smoothie）、math 保真路由、
//! 空输出防呆 → htmd 兜底的双失败诚实链。
//!
//! 与引擎的分工：
//! - 引擎只做"只读 DOM → Markdown"（不可失败、迭代遍历、无实体解码）；
//! - 本层做可选预处理（`--base-url` 链接绝对化、`--extract-main` 主内容
//!   提取 + 读屏器子树过滤）与降级链（提取失败/空输出诚实回退）。

mod engine;

use super::html::HtmlConverter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// 读屏器隐藏文本的约定类名片段（小写匹配）。这些元素对人类不可见、仅供
/// 辅助技术（如 BBC 的 `VisuallyHidden-styles__…` 哈希类），主内容提取前
/// 整体删除。用「包含」而非「精确等值」：styled-components/CSS Modules 会
/// 把约定词包进哈希类名；这些短语本身无歧义，误伤面可忽略。
fn is_screen_reader_class(token: &str) -> bool {
    let t = token.to_ascii_lowercase();
    [
        "visually-hidden",
        "visuallyhidden",
        "sr-only",
        "screen-reader",
    ]
    .iter()
    .any(|c| t.contains(c))
}

/// 删除读屏器隐藏子树（约定类名命中即删，DOM 级 detach）。
///
/// 仅用于主内容提取路径——全页转换保真优先，不动任何内容。
/// 无命中时原样返回输入串（不重序列化，零改写）。
fn strip_screen_reader_only(html: &str) -> String {
    let mut doc = scraper::Html::parse_document(html);
    let mut doomed = Vec::new();
    for node in doc.tree.nodes() {
        if let scraper::node::Node::Element(el) = node.value()
            && let Some(class) = el.attr("class")
            && class.split_ascii_whitespace().any(is_screen_reader_class)
        {
            doomed.push(node.id());
        }
    }
    if doomed.is_empty() {
        return html.to_string();
    }
    // 先收集后删除：detach 已断链的 id 再 get_mut 仍是孤儿节点，幂等安全。
    for id in doomed {
        if let Some(mut n) = doc.tree.get_mut(id) {
            n.detach();
        }
    }
    doc.html()
}

/// 把 HTML 中相对的 href/src 重写为基于 `base_url` 的绝对 URL。
///
/// 锚点/协议相对/数据/邮件等非相对目标原样保留；base 解析失败时原样返回。
fn absoluteize_links(html: &str, base_url: &str) -> String {
    use regex::Regex;

    let base = match url::Url::parse(base_url) {
        Ok(b) => b,
        Err(e) => {
            // 诚实降级：base 非法时链接不改写，但操作者必须知道（否则会误以为
            // 链接已绝对化）。
            tracing::warn!(%e, "html-deep: base_url is not a valid URL; links left unchanged");
            return html.to_string();
        }
    };
    let attr_re = match Regex::new(r#"(?i)(\s(?:href|src)=")([^"]*)(")"#) {
        Ok(r) => r,
        Err(_) => return html.to_string(),
    };
    attr_re
        .replace_all(html, |caps: &regex::Captures| {
            let value = &caps[2];
            let is_special = value.is_empty()
                || value.starts_with('#')
                || value.starts_with("//")
                || value.starts_with("mailto:")
                || value.starts_with("tel:")
                || value.starts_with("javascript:")
                || value.starts_with("data:");
            if is_special {
                return caps[0].to_string();
            }
            match base.join(value) {
                Ok(absolute) => format!("{}{}{}", &caps[1], absolute, &caps[3]),
                // join 失败（如 base 与 value 组合非法）保留原值
                Err(_) => caps[0].to_string(),
            }
        })
        .into_owned()
}

/// 纯文本分支的最小转义——先做 HTML 实体化（`& < >`：dom_smoothie 的
/// text_content 是已解码纯文本，字面 `<script>` 直出会在启用原生 HTML 的
/// Markdown 渲染器中执行——安全审查 MEDIUM 修复），再转义行首会误渲染为
/// 结构的字符（ATX 标题 #、引用 >、无序列表 - + *、有序列表 "1."）。
fn escape_leading_markdown_chars(text: &str) -> String {
    fn escape_html(ch: char, out: &mut String) {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }

    fn needs_escape(trimmed: &str) -> bool {
        if trimmed.starts_with('#') || trimmed.starts_with('>') {
            return true;
        }
        for marker in ["- ", "+ ", "* "] {
            if trimmed.starts_with(marker) {
                return true;
            }
        }
        // 有序列表：行首 "数字."（如 "1. " / "42. "）
        match trimmed.split_once('.') {
            Some((num, rest)) if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) => {
                rest.starts_with(' ') || rest.is_empty()
            }
            _ => false,
        }
    }

    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let leading = line.len() - line.trim_start().len();
        // 整行 HTML 实体化；行首结构标记中仅 `>` 会被实体化成惰性的
        // `&gt;`（无需反斜杠），其余（#/-/+/=/数字.）仍需反斜杠前缀。
        let first_visible = line[leading..].chars().next();
        let prefix_backslash =
            needs_escape(line.trim_start()) && first_visible.is_some_and(|c| c != '>');
        if prefix_backslash {
            out.push_str(&line[..leading]);
            out.push('\\');
        } else {
            out.push_str(&line[..leading]);
        }
        for ch in line[leading..].chars() {
            escape_html(ch, &mut out);
        }
    }
    out
}

/// 输入 HTML 是否含可见文本（决定空输出是否异常、兜底是否该触发）。
///
/// 从 `<body>` 子树扫描（head 内 title 不算可见正文），跳过 script/style/
/// template/noscript；存在非空白 Text 即视为可见内容。**显式栈迭代**：
/// DOM 深度不得进入 Rust 调用栈——递归实现会在 2MiB 栈上被万级嵌套
/// （release 实测 20000 层）击穿并 abort 进程。
fn has_visible_text(html: &str) -> bool {
    let doc = scraper::Html::parse_document(html);
    let root = doc.tree.root();
    let body = root
        .descendants()
        .find(|n| matches!(n.value(), scraper::node::Node::Element(e) if e.name() == "body"));
    let start = body.unwrap_or(root);
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        match node.value() {
            scraper::node::Node::Text(t) => {
                if !t.text.trim().is_empty() {
                    return true;
                }
            }
            scraper::node::Node::Element(el) => {
                const SKIP: [&str; 4] = ["script", "style", "template", "noscript"];
                if SKIP.contains(&el.name()) {
                    continue;
                }
                stack.extend(node.children());
            }
            scraper::node::Node::Document | scraper::node::Node::Fragment => {
                stack.extend(node.children())
            }
            _ => {}
        }
    }
    false
}

/// html-deep 引擎：自研引擎主 + HtmlConverter（htmd，含 math 保真）兜底的
/// 转换链，可选主内容提取（dom_smoothie）与链接绝对化。
pub struct HtmlDeepConverter {
    html: HtmlConverter,
    base_url: Option<String>,
    extract_main: bool,
}

impl HtmlDeepConverter {
    /// `base_url`：提供时对输入 HTML 预处理链接绝对化；`extract_main`：开启时
    /// 先经 dom_smoothie 提取主内容（失败/空诚实回退全页）。
    pub fn new(base_url: Option<String>, extract_main: bool) -> Self {
        Self {
            html: HtmlConverter::new(),
            base_url,
            extract_main,
        }
    }

    /// dom_smoothie 主内容提取：产出（清理后正文 HTML，纯文本）。两者任一
    /// 非空即提取成功——HTML 分支可保段落/图片/链接结构，纯文本仅作其降级。
    fn extract_main_content(&self, html: &str) -> Result<(String, String), String> {
        let trimmed = html.trim();
        if trimmed.is_empty() {
            return Err("empty html".into());
        }
        let mut readability =
            dom_smoothie::Readability::new(trimmed, self.base_url.as_deref(), None)
                .map_err(|e| format!("dom-smoothie: {e}"))?;
        let article: dom_smoothie::Article = readability
            .parse()
            .map_err(|e| format!("dom-smoothie parse: {e}"))?;
        let content_html = article.content.trim().to_string();
        let text = article.text_content.trim().to_string();
        if content_html.is_empty() && text.is_empty() {
            return Err("dom-smoothie: empty content".into());
        }
        Ok((content_html, text))
    }

    /// StructuredHtml 分支：清理后正文 HTML → 自研引擎转换（保留段落/
    /// 图片/链接结构）。同步段独立作用域（ego_tree 非 Send，不跨 await）。
    fn convert_structured_html(&self, content_html: &str) -> Option<String> {
        let doc = scraper::Html::parse_document(content_html.trim());
        let markdown = super::html_deep::engine::convert(&doc);
        if markdown.trim().is_empty() {
            None
        } else {
            Some(markdown.trim().to_string() + "\n")
        }
    }
}

#[async_trait::async_trait]
impl crate::application::ports::Converter for HtmlDeepConverter {
    fn name(&self) -> &'static str {
        "html-deep"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read HTML input: {e}")))?;
        let html = String::from_utf8(bytes)
            .map_err(|e| AlchemizeError::other(format!("HTML content is not valid UTF-8: {e}")))?;

        self.convert_html(&html)
    }

    /// 链上唯一 await 是 `to_bytes`（Bytes 变体即 clone）——全链同步能力
    /// 成立；同步核见 [`HtmlDeepConverter::convert_html`]。
    fn is_sync_capable(&self) -> bool {
        true
    }

    fn convert_sync(&self, input: &Input) -> Result<ConvertResult, AlchemizeError> {
        match input {
            Input::Bytes { data, .. } => {
                let html = String::from_utf8(data.clone()).map_err(|e| {
                    AlchemizeError::other(format!("HTML content is not valid UTF-8: {e}"))
                })?;
                self.convert_html(&html)
            }
            Input::Path(_) => Err(AlchemizeError::NotSyncCapable(
                "path input requires the async port; the sync port serves in-memory Input::Bytes only"
                    .into(),
            )),
        }
    }

    fn supported_extensions(&self) -> &[&str] {
        &["html", "htm"]
    }

    /// priority 15：压过既有 html/htmd 引擎（10）——
    /// feature html-deep 构建下本链是 html 的首选路径。
    fn priority(&self) -> u8 {
        15
    }
}

impl HtmlDeepConverter {
    /// 全同步转换链：math 保真路由 → 链接绝对化 → 主内容提取（可选）→
    /// 全页引擎转换 → 兜底决策。async `convert` 与同步口 `convert_sync`
    /// 在 UTF-8 解码后都委托到这里——两端口共享同一函数体，输出逐字节
    /// 相同由构造保证。
    pub(crate) fn convert_html(&self, html: &str) -> Result<ConvertResult, AlchemizeError> {
        // math 保真路由：引擎会展平 `<math>`（MathML 净化为行内 HTML，非
        // LaTeX）；检测条件与 html.rs preprocess_math 的触发条件（`<math`
        // 标签）严格一致，保证「preprocess 会处理的输入必然路由到 math
        // 保真路径」。（$..$ 文本无 `<math>` 标签，由引擎自身的 TeX 跨度
        // 保留覆盖。）
        if html.contains("<math") {
            return self.html.convert_str(html);
        }

        // 链接绝对化：仅在显式提供 base_url 时改写（缺省不动任何链接）。
        // 兜底输入源在改写**之前**固定下来：旧 async 实现把原始 Input 传给
        // htmd 兜底（原始解码串），base_url 存在时兜底输出相对链接、
        // original_size 计原始长度——零行为变化要求这里逐字节对齐（架构
        // 审查 HIGH 修复）。
        //
        // Cow 避免无 base_url 默认路径的全输入拷贝：旧实现该分支是 String
        // move（零拷贝），共享核吃 &str 后若物化 owned String 会给两个端口
        // 各加一次 O(n) memcpy（性能审查 HIGH 修复）——Cow::Borrowed 保住
        // 零拷贝，Deref 到 &str 的下游用法（parse/len/finish_full_page）
        // 不变。
        let fallback_source = html;
        let html: std::borrow::Cow<'_, str> = match &self.base_url {
            Some(base) if !base.is_empty() => {
                std::borrow::Cow::Owned(absoluteize_links(html, base))
            }
            _ => std::borrow::Cow::Borrowed(html),
        };

        // 主内容提取（显式开启）：三分支同构——
        // ① StructuredHtml：清理后正文 HTML → 引擎转换（段落/图片/链接保留）；
        // ② PlainText 降级：text_content 行首转义直出（无结构）；
        // ③ 全页回退：提取失败/双产皆空时走完整转换链。
        // 提取前先删读屏器隐藏子树（对人类不可见的 "Site search" 类文本）；
        // 全页转换保真优先，不做此过滤。
        if self.extract_main {
            let html = strip_screen_reader_only(&html);
            match self.extract_main_content(&html) {
                Ok((content_html, text)) => {
                    let markdown = self.convert_structured_html(&content_html).or_else(|| {
                        (!text.is_empty()).then(|| escape_leading_markdown_chars(&text))
                    });
                    if let Some(markdown) = markdown {
                        return Ok(ConvertResult::with_metadata(
                            markdown,
                            ContentType::Document,
                            serde_json::json!({
                                "source_format": "html",
                                "main_content_extracted": true,
                            }),
                        )
                        .with_quality(ConversionQuality::LocalFallback));
                    }
                    // 双产皆空：诚实回退全页（不静默吞）。
                    tracing::warn!(
                        "html-deep: extraction produced empty output, falling back to full page"
                    );
                }
                Err(reason) => {
                    tracing::warn!(reason = %reason, "html-deep: main-content extraction failed, falling back to full page");
                }
            }
        }

        // 全页转换：自研引擎主路径（html5ever 错误恢复解析，convert 本身
        // 不可失败）。整条链是纯同步函数——scraper 的 ego_tree（Rc 树，
        // 非 Send）只活在本函数帧内，不进入任何 async 上下文。
        let primary = {
            let doc = scraper::Html::parse_document(html.trim());
            super::html_deep::engine::convert(&doc)
        };

        self.finish_full_page(&html, primary, fallback_source)
    }

    /// 全页转换尾部：兜底决策（空输出防呆 → htmd 兜底 → 双失败诚实报错）
    /// 与结果装配。`primary` 由调用方传入引擎输出——独立成函数使兜底决策
    /// 可注入空 primary 直接单测（引擎本身鲁棒，天然触发器罕见）。同步
    /// 函数：htmd 兜底走 [`HtmlConverter::convert_str`]。`fallback_source`
    /// 是绝对化**之前**的原始解码串——旧 async 实现的兜底输入即原始串，
    /// 零行为变化要求兜底输出与其逐字节一致；`html`（可能已改写）仅用于
    /// `original_size` 元数据与可见性判定，与旧实现相同。
    pub(crate) fn finish_full_page(
        &self,
        html: &str,
        primary: String,
        fallback_source: &str,
    ) -> Result<ConvertResult, AlchemizeError> {
        if !primary.trim().is_empty() {
            let markdown = primary.trim().to_string() + "\n";
            let output_size = markdown.len();
            return Ok(ConvertResult::with_metadata(
                markdown,
                ContentType::Document,
                serde_json::json!({
                    "source_format": "html",
                    "original_size": html.len(),
                    "output_size": output_size,
                }),
            )
            .with_quality(ConversionQuality::LocalFull));
        }

        if !has_visible_text(html) {
            // 纯脚本/空壳页面的空输出是正确抑制，不触发兜底。
            return Ok(ConvertResult::with_metadata(
                primary,
                ContentType::Document,
                serde_json::json!({
                    "source_format": "html",
                    "original_size": html.len(),
                    "output_size": 0usize,
                }),
            )
            .with_quality(ConversionQuality::LocalFull));
        }

        // 有可见文本而主输出空白：保真异常，htmd 兜底。
        tracing::warn!(
            "html-deep: primary produced empty output with visible text, falling back to htmd"
        );
        // 兜底路径的 HtmlConverter 对空输出恒注入占位文本（*HTML document
        // contains no extractable content.*）——「双空」的可观察终态即占位文本
        // （LocalFallback 如实降标），不存在返回空串的路径。
        let fallback = self.html.convert_str(fallback_source)?;
        Ok(ConvertResult::with_metadata(
            fallback.markdown,
            ContentType::Document,
            fallback.metadata,
        )
        .with_quality(ConversionQuality::LocalFallback))
    }
}

#[cfg(all(test, feature = "html-deep"))]
mod tests {
    use super::*;
    use crate::application::ports::Converter as _;

    fn engine() -> HtmlDeepConverter {
        HtmlDeepConverter::new(None, false)
    }

    fn page_input(html: &str) -> Input {
        Input::Bytes {
            data: html.as_bytes().to_vec(),
            filename: "page.html".into(),
            mimetype: None,
            extension: Some("html".into()),
        }
    }

    /// 兜底正例：主输出空 + 有可见文本 → htmd 兜底（占位文本来自既有
    /// html 引擎）且质量如实标注 LocalFallback。同步化后为纯同步单测。
    #[test]
    fn fallback_triggers_on_empty_primary_with_visible_text() {
        let html = "<html><body><p>real visible words</p></body></html>";
        let result = engine()
            .finish_full_page(html, String::new(), html)
            .unwrap();
        // 兜底触发后走 htmd 正常转换（占位文本仅在 htmd 也输出空时出现）；
        // 质量必须如实降标 LocalFallback——不得冒充 LocalFull。
        assert_eq!(result.markdown, "real visible words\n");
        assert_eq!(result.quality, ConversionQuality::LocalFallback);
    }

    /// 兜底输入源回归锚点（架构审查 HIGH 修复）：base_url 存在时 htmd 兜底
    /// 转换的是 absoluteize **之前**的原始解码串——旧 async 行为。兜底输出
    /// 必须保持相对链接（未被改写），`original_size` 计原始长度。
    #[test]
    fn fallback_with_base_url_uses_pre_absoluteize_source() {
        let html = r##"<html><body><a href="page.html">doc</a></body></html>"##;
        let engine = HtmlDeepConverter::new(Some("https://example.com/docs/".to_string()), false);
        let result = engine.finish_full_page(html, String::new(), html).unwrap();
        assert!(
            result.markdown.contains("(page.html)"),
            "fallback must see the pre-absoluteize string: {:?}",
            result.markdown
        );
        assert!(
            !result
                .markdown
                .contains("https://example.com/docs/page.html"),
            "fallback output must not be absoluteized: {:?}",
            result.markdown
        );
        assert_eq!(result.metadata["original_size"], html.len());
    }

    /// escape_leading_markdown_chars 确定性单测：
    /// 六类行首标记转义、行首空白保留、非行首不转义。
    #[test]
    fn escape_leading_verbatim_semantics() {
        assert_eq!(escape_leading_markdown_chars("1. one"), "\\1. one");
        assert_eq!(escape_leading_markdown_chars("42. answer"), "\\42. answer");
        assert_eq!(escape_leading_markdown_chars("1."), "\\1.");
        assert_eq!(escape_leading_markdown_chars("# head"), "\\# head");
        assert_eq!(escape_leading_markdown_chars("> quote"), "&gt; quote");
        assert_eq!(escape_leading_markdown_chars("- bullet"), "\\- bullet");
        assert_eq!(escape_leading_markdown_chars("+ plus"), "\\+ plus");
        assert_eq!(escape_leading_markdown_chars("* star"), "\\* star");
        // 行首空白保留（反斜杠插在空白之后）
        assert_eq!(
            escape_leading_markdown_chars("  - indented"),
            "  \\- indented"
        );
        // 非行首/非标记不转义
        assert_eq!(escape_leading_markdown_chars("point 1."), "point 1.");
        assert_eq!(
            escape_leading_markdown_chars("-1. negative"),
            "-1. negative"
        );
        assert_eq!(escape_leading_markdown_chars("plain"), "plain");
        // HTML 实体化：已解码的裸标签不得直出（XSS 防护）
        assert_eq!(
            escape_leading_markdown_chars("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
        assert_eq!(escape_leading_markdown_chars("a&b"), "a&amp;b");
        assert_eq!(escape_leading_markdown_chars("# <tag>"), "\\# &lt;tag&gt;");
    }

    /// 防呆负例：纯脚本页面主输出空 → 正确抑制，不触发兜底（无占位文本、
    /// 质量仍为 LocalFull——这是引擎路径的正常空输出）。同步化后为纯同步单测。
    #[test]
    fn script_only_page_suppresses_without_fallback() {
        let html = "<html><body><script>yo();</script></body></html>";
        let result = engine()
            .finish_full_page(html, String::new(), html)
            .unwrap();
        assert!(
            !result.markdown.contains("no extractable content"),
            "script-only page must not trigger fallback: {:?}",
            result.markdown
        );
        assert_eq!(result.quality, ConversionQuality::LocalFull);
    }

    /// 读屏器隐藏文本过滤（主内容提取前置）：BBC 真页形态——styled-components
    /// 哈希类名 `VisuallyHidden-styles__…`，约定词被包进哈希，必须包含匹配。
    #[test]
    fn strip_screen_reader_hashed_class() {
        let html = r#"<html><body>
            <div class="DrawerBackgroundStyled-sc-1abc">
              <label for="s"><span class="VisuallyHidden-styles__VisuallyHiddenStyled-sc-69248fc2-0 fWJrIe">Site search</span></label>
              <input id="s" type="text">
            </div>
            <article><p>real words</p></article>
        </body></html>"#;
        let out = strip_screen_reader_only(html);
        assert!(
            !out.contains("Site search"),
            "screen-reader span must be removed: {out}"
        );
        assert!(
            out.contains("real words"),
            "visible content must survive: {out}"
        );
    }

    /// 常规约定类名（Tailwind `sr-only` 等）整子树删除；多 token class 里
    /// 只有约定 token 命中时也删。
    #[test]
    fn strip_screen_reader_plain_class() {
        let html = r#"<html><body>
            <p class="sr-only">skip to content</p>
            <p class="mt-2 visually-hidden">hidden hint</p>
            <p class="mt-2">visible text</p>
        </body></html>"#;
        let out = strip_screen_reader_only(html);
        assert!(!out.contains("skip to content"), "{out}");
        assert!(!out.contains("hidden hint"), "{out}");
        assert!(out.contains("visible text"), "{out}");
    }

    /// 无命中：输入原样返回（零改写、不重序列化）；`hidden`/`Search` 等
    /// 非约定词不误伤。
    #[test]
    fn strip_screen_reader_no_match_is_identity() {
        let html = r#"<html><body>
            <p class="hidden">keep me</p>
            <p class="SearchBox">Search box label</p>
            <p>plain</p>
        </body></html>"#;
        assert_eq!(strip_screen_reader_only(html), html);
    }

    /// 提取路径端到端：--extract-main 时读屏文本不再进入产出（全页路径
    /// 不受影响，由既有 full-page 用例覆盖）。
    #[tokio::test]
    async fn extract_main_drops_screen_reader_text() {
        let html = r#"<html><body>
            <nav><span class="sr-only">Site search</span><a href="/">Home</a></nav>
            <article><h1>Title</h1><p>body words</p></article>
        </body></html>"#;
        let extracted = HtmlDeepConverter::new(None, true)
            .convert(page_input(html))
            .await
            .unwrap();
        assert!(
            !extracted.markdown.contains("Site search"),
            "screen-reader text must not reach extraction output: {:?}",
            extracted.markdown
        );
        assert!(
            extracted.markdown.contains("body words"),
            "{:?}",
            extracted.markdown
        );
    }

    /// 懒加载占位图判定已在引擎层（ScraperDom 视图约定随引擎吸收迁移）；
    /// 此处验证解析器级端到端：全页路径下占位图也不产出。
    #[tokio::test]
    async fn placeholder_img_dropped_in_full_page_path() {
        let html = r#"<html><body>
            <figure>
              <img src="https://cdn.example/img/grey-placeholder.png" aria-label="image unavailable"/>
              <img src="https://cdn.example/photo.jpg" alt="real photo"/>
              <figcaption>caption text</figcaption>
            </figure>
        </body></html>"#;
        let result = engine().convert(page_input(html)).await.unwrap();
        assert!(
            !result.markdown.contains("grey-placeholder"),
            "placeholder img must not reach markdown: {:?}",
            result.markdown
        );
        assert!(
            result.markdown.contains("(https://cdn.example/photo.jpg)"),
            "real image must survive: {:?}",
            result.markdown
        );
    }

    // -- 同步口 ---------------------------------------------------------------

    /// 两端口共享 convert_html 函数体：math 路由、extract_main、全页、
    /// 脚本抑制四类语料上 async 与 sync 输出逐字节相等。
    #[tokio::test]
    async fn sync_port_matches_async_output_byte_for_byte() {
        let corpora: [(&str, bool); 4] = [
            // math 保真路由（htmd 路径）
            (
                r#"<html><body><p>Given <math alttext="\alpha+\beta" display="inline"><mi>α</mi></math> we get</p><math alttext="c^2 = a^2 + b^2" display="block"><mi>c</mi></math></body></html>"#,
                false,
            ),
            // extract_main 主路径（读屏器过滤 + 引擎转换）
            (
                r#"<html><body><nav><span class="sr-only">Site search</span><a href="/">Home</a></nav><article><h1>Title</h1><p>body words</p></article></body></html>"#,
                true,
            ),
            // 全页主路径
            (
                r#"<html><body><h1>Page</h1><p>words with <a href="/rel">relative link</a></p></body></html>"#,
                false,
            ),
            // 脚本抑制路径（空输出不触发兜底）
            ("<html><body><script>yo();</script></body></html>", false),
        ];
        for (html, extract_main) in corpora {
            let engine = HtmlDeepConverter::new(None, extract_main);
            let input = page_input(html);
            let async_out = engine.convert(input.clone()).await.unwrap();
            let sync_out = engine.convert_sync(&input).unwrap();
            assert_eq!(async_out.markdown, sync_out.markdown, "corpus: {html}");
            assert_eq!(async_out.quality, sync_out.quality, "corpus: {html}");
            assert_eq!(async_out.metadata, sync_out.metadata, "corpus: {html}");
        }
    }

    /// base_url 链接绝对化路径同样两端口一致。
    #[tokio::test]
    async fn sync_matches_async_with_base_url() {
        let html =
            r#"<html><body><a href="page.html">doc</a><img src="img/pic.png"/></body></html>"#;
        let engine = HtmlDeepConverter::new(Some("https://example.com/docs/".to_string()), false);
        let input = page_input(html);
        let async_out = engine.convert(input.clone()).await.unwrap();
        let sync_out = engine.convert_sync(&input).unwrap();
        assert_eq!(async_out.markdown, sync_out.markdown);
        assert_eq!(async_out.metadata, sync_out.metadata);
        assert!(
            async_out
                .markdown
                .contains("(https://example.com/docs/page.html)"),
            "absoluteization must have run: {:?}",
            async_out.markdown
        );
    }

    /// Path 输入在同步口显式拒绝（实现层防御，facade 已先行守卫）。
    #[test]
    fn convert_sync_rejects_path_input() {
        let err = engine()
            .convert_sync(&Input::Path("/absent/page.html".into()))
            .unwrap_err();
        assert!(
            matches!(err, AlchemizeError::NotSyncCapable(_)),
            "expected NotSyncCapable, got {err:?}"
        );
    }
}
