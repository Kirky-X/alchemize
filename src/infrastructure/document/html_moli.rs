// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `html-moli` 引擎 —— 吸收自 crawlrs 的 html→markdown 能力（feature `moli`）。
//!
//! 来源（crawlrs，Apache-2.0，本文件保留原版权年份区间）：
//! - `src/domain/services/markdown/{converter_chain,moli_adapter,moli_converter}.rs`
//!   —— moli 主 + htmd 兜底的转换链语义（空输出防呆、迭代式 DOM 遍历、诚实降级）
//! - `src/workers/markdown_post_processor.rs` —— absoluteize_links 与
//!   escape_leading_markdown_chars（逐函数移植）
//! - 主内容提取：dom_smoothie（readability 移植，crates.io 依赖）
//!
//! 与 crawlrs 的差异：
//! - 兜底不是独立 htmd 服务，而是复用本仓 HtmlConverter（含 LaTeX 数学保真）；
//! - 含数学特征（alttext）的输入整体路由 HtmlConverter——moli 会展平数学，
//!   路由比兼容更诚实；
//! - extract_main / base_url 为显式选项（保真优先的文档转换器，提取默认关）。

use moli_html2md::{Dom, NodeKind};
use scraper::node::Node;

use super::html::HtmlConverter;
use crate::application::ports::Converter as _;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// `scraper::Html`（html5ever 树）→ `moli_html2md::Dom` 适配器
/// （移植自 crawlrs moli_adapter.rs，测试随迁）。
///
/// 契约（上游 `Dom` trait，只读视图、不二次解码实体）：
/// - `node_kind`：Document/Fragment → Document；Element → 小写局部名；Text → 已解码文本；
///   注释/doctype/处理指令 → Other；**懒加载占位图 → Other**（见 `is_placeholder_img`）
/// - `first_child` / `next_sibling`：ego_tree 直接映射（scraper 内部树，NodeId Copy）
/// - `attribute`：大小写不敏感匹配（scraper 属性名已小写化，双保险）
/// - `control_value` / `has_block_layout` / `has_text_boundary`：保留 trait 默认实现
///   （None/false）——静态 HTML 无计算样式，接了就是假数据（诚实降级）
pub struct ScraperDom<'a> {
    doc: &'a scraper::Html,
}

impl<'a> ScraperDom<'a> {
    pub fn new(doc: &'a scraper::Html) -> Self {
        Self { doc }
    }
}

/// 懒加载占位图判定（解析器级，ScraperDom 专属视图约定）。
///
/// 懒加载站点（BBC 等）的每个图片位先给一张禁用-JS 场景的灰色占位图
/// （`hide-when-no-script` 类），JS 就绪后由浏览器隐藏并换上真图。静态转换
/// 不执行 JS，两张 img 都会被遍历——把占位图判成 Other 让 moli 状态机
/// 整体跳过（不产出、不进子树），等效于浏览器「有 JS 视图」。
///
/// 判据（URL 关键词，方案 A）：src 含 `placeholder` / `spacer`，或 data: URI
/// 小图（≤64 字节的 base64 载荷约一张 1×1 像素）。各站占位图命名高度趋同，
/// 误伤面可忽略；srcSet/src 为真图的节点不命中。
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
            .map(|payload| payload.len() <= 100)
            .unwrap_or(false)
}

impl Dom for ScraperDom<'_> {
    type NodeId = ego_tree::NodeId;

    fn node_kind(&self, node: Self::NodeId) -> NodeKind<'_> {
        match self.doc.tree.get(node).map(|n| n.value()) {
            Some(Node::Document) | Some(Node::Fragment) => NodeKind::Document,
            // scraper 的 Element::name() 即 html5ever 归一化后的小写局部名
            Some(Node::Element(el)) if is_placeholder_img(el) => NodeKind::Other,
            Some(Node::Element(el)) => NodeKind::Element(el.name()),
            // scraper 的 Text 节点在解析时已完成实体解码（StrTendril 明文）
            Some(Node::Text(text)) => NodeKind::Text(text.text.as_ref()),
            Some(Node::Comment(_))
            | Some(Node::Doctype(_))
            | Some(Node::ProcessingInstruction(_)) => NodeKind::Other,
            // 无效 NodeId：按契约归 Other（不 panic——上游约定 invalid IDs 走 Other）
            None => NodeKind::Other,
        }
    }

    fn first_child(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.doc
            .tree
            .get(node)
            .and_then(|n| n.first_child())
            .map(|n| n.id())
    }

    fn next_sibling(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.doc
            .tree
            .get(node)
            .and_then(|n| n.next_sibling())
            .map(|n| n.id())
    }

    fn attribute(&self, node: Self::NodeId, name: &str) -> Option<&str> {
        self.doc
            .tree
            .get(node)?
            .value()
            .as_element()?
            .attrs()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }
}

/// 输入 HTML 是否含可见文本（决定空输出是否异常、兜底是否该触发）。
///
/// 移植自 crawlrs converter_chain.rs：从 `<body>` 子树扫描（head 内 title 不算
/// 可见正文），跳过 script/style/template/noscript；存在非空白 Text 即视为可见
/// 内容。**显式栈迭代**：DOM 深度不得进入 Rust 调用栈——递归实现会在 2MiB 栈上
/// 被万级嵌套（release 实测 20000 层）击穿并 abort 进程。
fn has_visible_text(html: &str) -> bool {
    let doc = scraper::Html::parse_document(html);
    let root = doc.tree.root();
    let body = root
        .descendants()
        .find(|n| matches!(n.value(), Node::Element(e) if e.name() == "body"));
    let start = body.unwrap_or(root);
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        match node.value() {
            Node::Text(t) => {
                if !t.text.trim().is_empty() {
                    return true;
                }
            }
            Node::Element(el) => {
                const SKIP: [&str; 4] = ["script", "style", "template", "noscript"];
                if SKIP.contains(&el.name()) {
                    continue;
                }
                stack.extend(node.children());
            }
            Node::Document | Node::Fragment => stack.extend(node.children()),
            _ => {}
        }
    }
    false
}

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
        if let Node::Element(el) = node.value() {
            if let Some(class) = el.attr("class") {
                if class.split_ascii_whitespace().any(is_screen_reader_class) {
                    doomed.push(node.id());
                }
            }
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

/// 把 HTML 中相对的 href/src 重写为基于 `base_url` 的绝对 URL
/// （移植自 crawlrs markdown_post_processor.rs）。
///
/// 锚点/协议相对/数据/邮件等非相对目标原样保留；base 解析失败时原样返回。
fn absoluteize_links(html: &str, base_url: &str) -> String {
    use regex::Regex;

    let base = match url::Url::parse(base_url) {
        Ok(b) => b,
        Err(e) => {
            // 诚实降级：base 非法时链接不改写，但操作者必须知道（否则会误以为
            // 链接已绝对化）。
            tracing::warn!(%e, "html-moli: base_url is not a valid URL; links left unchanged");
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

/// 纯文本分支的最小 Markdown 转义——仅转义行首会误渲染为结构的字符
/// （ATX 标题 #、引用 >、无序列表 - + *、有序列表 "1."）
/// （移植自 crawlrs markdown_post_processor.rs，逐字保留）。
fn escape_leading_markdown_chars(text: &str) -> String {
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
        if needs_escape(line.trim_start()) {
            out.push_str(&line[..leading]);
            out.push('\\');
            out.push_str(&line[leading..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// html-moli 引擎：moli-html2md 主 + HtmlConverter（htmd，含 math 保真）兜底
/// 的转换链，可选主内容提取（dom_smoothie）与链接绝对化。
pub struct HtmlMoliConverter {
    html: HtmlConverter,
    base_url: Option<String>,
    extract_main: bool,
}

impl HtmlMoliConverter {
    /// `base_url`：提供时对输入 HTML 预处理链接绝对化；`extract_main`：开启时
    /// 先经 dom_smoothie 提取主内容（失败/空诚实回退全页）。
    pub fn new(base_url: Option<String>, extract_main: bool) -> Self {
        Self {
            html: HtmlConverter::new(),
            base_url,
            extract_main,
        }
    }

    /// dom_smoothie 主内容提取：产出纯文本（非空），失败返回 Err。
    /// （移植自 crawlrs dom_smoothie_extractor 的调用形态。）
    /// dom_smoothie 主内容提取：产出（清理后正文 HTML，纯文本）。两者任一非空
    /// 即提取成功——HTML 分支可保段落/图片/链接结构，纯文本仅作其降级。
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

    /// StructuredHtml 分支：清理后正文 HTML → moli 状态机转换（保留段落/
    /// 图片/链接结构）。同步段独立作用域（ego_tree 非 Send，不跨 await）。
    fn convert_structured_html(&self, content_html: &str) -> Option<String> {
        let doc = scraper::Html::parse_document(content_html.trim());
        let root = doc.tree.root();
        let dom = ScraperDom::new(&doc);
        let converter = moli_html2md::Converter::new(moli_html2md::Options {
            max_depth: usize::MAX,
            ..Default::default()
        });
        let markdown = converter.convert(&dom, root.id());
        if markdown.trim().is_empty() {
            None
        } else {
            Some(markdown.trim().to_string() + "\n")
        }
    }
}

#[async_trait::async_trait]
impl crate::application::ports::Converter for HtmlMoliConverter {
    fn name(&self) -> &'static str {
        "html-moli"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read HTML input: {e}")))?;
        let html = String::from_utf8(bytes)
            .map_err(|e| AlchemizeError::other(format!("HTML content is not valid UTF-8: {e}")))?;

        // math 保真路由：moli 会展平 LaTeX 数学；检测条件与 html.rs
        // preprocess_math 的触发条件（`<math` 标签）严格一致，保证「preprocess
        // 会处理的输入必然路由到 math 保真路径」。
        if html.contains("<math") {
            return self.html.convert(input).await;
        }

        // 链接绝对化：仅在显式提供 base_url 时改写（缺省不动任何链接）。
        let html = match &self.base_url {
            Some(base) if !base.is_empty() => absoluteize_links(&html, base),
            _ => html,
        };

        // 主内容提取（显式开启）：crawlrs MainContentSource 三分支同构——
        // ① StructuredHtml：清理后正文 HTML → moli 转换（段落/图片/链接保留）；
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
                        "html-moli: extraction produced empty output, falling back to full page"
                    );
                }
                Err(reason) => {
                    tracing::warn!(reason = %reason, "html-moli: main-content extraction failed, falling back to full page");
                }
            }
        }

        // 全页转换：moli 主路径（html5ever 错误恢复解析，convert 本身不可失败）。
        // 同步段独立作用域：scraper 的 ego_tree 为 Rc 树（非 Send），不得跨越
        // 下方 htmd 兜底的 await 点——本块结束即 drop。
        let primary = {
            let doc = scraper::Html::parse_document(html.trim());
            let root = doc.tree.root();
            let dom = ScraperDom::new(&doc);
            // max_depth 上限解除：moli 遍历是迭代式（无 Rust 栈风险），默认 1000
            // 会静默丢弃深层子树（crawlrs 实测）。
            let converter = moli_html2md::Converter::new(moli_html2md::Options {
                max_depth: usize::MAX,
                ..Default::default()
            });
            converter.convert(&dom, root.id())
        };

        self.finish_full_page(&html, input, primary).await
    }

    fn supported_extensions(&self) -> &[&str] {
        &["html", "htm"]
    }

    /// priority 15：压过既有 html/htmd 引擎（10），与 anydoc 同级——
    /// feature moli 构建下 moli 链是 html 的首选路径。
    fn priority(&self) -> u8 {
        15
    }
}

impl HtmlMoliConverter {
    /// 全页转换尾部：兜底决策（空输出防呆 → htmd 兜底 → 双失败诚实报错）
    /// 与结果装配。`primary` 由调用方传入 moli 输出——独立成函数使兜底决策
    /// 可注入空 primary 直接单测（moli 本身鲁棒，天然触发器罕见）。
    pub(crate) async fn finish_full_page(
        &self,
        html: &str,
        input: Input,
        primary: String,
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
            "html-moli: primary produced empty output with visible text, falling back to htmd"
        );
        // 兜底路径的 HtmlConverter 对空输出恒注入占位文本（*HTML document
        // contains no extractable content.*）——「双空」的可观察终态即占位文本
        // （LocalFallback 如实降标），不存在返回空串的路径。
        let fallback = self.html.convert(input).await?;
        Ok(ConvertResult::with_metadata(
            fallback.markdown,
            ContentType::Document,
            fallback.metadata,
        )
        .with_quality(ConversionQuality::LocalFallback))
    }
}

#[cfg(all(test, feature = "moli"))]
mod tests {
    use super::*;

    fn engine() -> HtmlMoliConverter {
        HtmlMoliConverter::new(None, false)
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
    /// html 引擎）且质量如实标注 LocalFallback。
    #[tokio::test]
    async fn fallback_triggers_on_empty_primary_with_visible_text() {
        let html = "<html><body><p>real visible words</p></body></html>";
        let result = engine()
            .finish_full_page(html, page_input(html), String::new())
            .await
            .unwrap();
        // 兜底触发后走 htmd 正常转换（占位文本仅在 htmd 也输出空时出现）；
        // 质量必须如实降标 LocalFallback——不得冒充 LocalFull。
        assert_eq!(result.markdown, "real visible words\n");
        assert_eq!(result.quality, ConversionQuality::LocalFallback);
    }

    /// escape_leading_markdown_chars 确定性单测（逐字移植自 crawlrs）：
    /// 六类行首标记转义、行首空白保留、非行首不转义。
    #[test]
    fn escape_leading_verbatim_semantics() {
        assert_eq!(escape_leading_markdown_chars("1. one"), "\\1. one");
        assert_eq!(escape_leading_markdown_chars("42. answer"), "\\42. answer");
        assert_eq!(escape_leading_markdown_chars("1."), "\\1.");
        assert_eq!(escape_leading_markdown_chars("# head"), "\\# head");
        assert_eq!(escape_leading_markdown_chars("> quote"), "\\> quote");
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
    }

    /// 防呆负例：纯脚本页面主输出空 → 正确抑制，不触发兜底（无占位文本、
    /// 质量仍为 LocalFull——这是 moli 路径的正常空输出）。
    #[tokio::test]
    async fn script_only_page_suppresses_without_fallback() {
        let html = "<html><body><script>yo();</script></body></html>";
        let result = engine()
            .finish_full_page(html, page_input(html), String::new())
            .await
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
        let extracted = HtmlMoliConverter::new(None, true)
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

    /// 懒加载占位图判定：URL 关键词与 data: 小图命中；真图与非 img 不命中。
    #[test]
    fn is_placeholder_img_judgement() {
        fn el_from(html: &str) -> scraper::node::Element {
            let doc = scraper::Html::parse_fragment(html);
            // parse_fragment 会包出 html/body 包装元素——必须精确选 img，
            // find_map 首个 Element 是包装节点。
            let found = doc.tree.nodes().find_map(|n| match n.value() {
                Node::Element(el) if el.name() == "img" => Some(el.clone()),
                _ => None,
            });
            if found.is_none() {
                eprintln!("PROBE el_from: no img in {html:?}");
            }
            found.expect("test helper: input must contain an <img>")
        }
        // BBC 灰色占位图（真页形态）
        assert!(is_placeholder_img(&el_from(
            r#"<img src="https://static.files.bbci.co.uk/bbcdotcom/web/x/grey-placeholder.png" aria-label="image unavailable"/>"#,
        )));
        // 通用 spacer
        assert!(is_placeholder_img(&el_from(
            r#"<img src="/assets/spacer.gif"/>"#,
        )));
        // 1×1 data: 透明图（懒加载占位另一常见形态）
        assert!(is_placeholder_img(&el_from(
            r#"<img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"/>"#,
        )));
        // 真图：srcSet 站点的 src 是真 URL
        assert!(!is_placeholder_img(&el_from(
            r#"<img src="https://ichef.bbci.co.uk/news/480/cpsprodpb/x.jpg.webp" alt="real photo"/>"#,
        )));
        // 误伤面：非 img 元素与无 src 的 img 绝不命中（el_from 专找 img，
        // div 负例需独立构造）。
        let doc = scraper::Html::parse_fragment(r#"<div src="/x/spacer.gif"/>"#);
        let div = doc
            .tree
            .nodes()
            .find_map(|n| match n.value() {
                Node::Element(el) => Some(el.clone()),
                _ => None,
            })
            .unwrap();
        assert!(!is_placeholder_img(&div), "non-img must never match");
        assert!(!is_placeholder_img(&el_from(r#"<img alt="no src"/>"#)));
    }

    /// 解析器级端到端：全页路径下占位图也不产出（ScraperDom 视图层判定，
    /// 两条 moli 路径——提取与全页——同时生效，无需各自过滤）。
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
}
