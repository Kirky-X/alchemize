// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 媒体资源解析：img srcset（逐字节扫描/描述符校验/最大候选）、
//! iframe/video/audio 源、URL scheme 白名单与链接黑名单（行为规格 §7）。

use scraper::node::Node as SNode;

use super::structure::attribute;

type NodeRef<'a> = ego_tree::NodeRef<'a, SNode>;

/// `a` 的安全链接目的地：href 非空且过 `safe_link_url` 黑名单。
pub(super) fn link_href(node: NodeRef<'_>) -> Option<String> {
    let href = attribute(node, "href")?;
    if href.is_empty() || !safe_link_url(&href) {
        return None;
    }
    Some(href)
}

/// 子树谓词：元素为 `a` 且带安全 href（code/pre fallback 判定用）。
pub(super) fn node_has_safe_link(node: NodeRef<'_>) -> bool {
    matches!(node.value().as_element().map(|e| e.name()), Some("a")) && link_href(node).is_some()
}

/// img 的资源地址，四级优先（行为规格 §7）：`srcset` 最大候选 → `src`
/// → `data-srcset` 最大候选 → 私有懒加载属性（data-src/data-lazy-src/
/// data-original）。全部校验后取第一个可用者。
pub(super) fn source(node: NodeRef<'_>) -> Option<String> {
    for (attr, srcset) in [
        ("srcset", true),
        ("src", false),
        ("data-srcset", true),
        ("data-src", false),
        ("data-lazy-src", false),
        ("data-original", false),
    ] {
        match attribute(node, attr) {
            Some(value) if !value.trim().is_empty() => {
                if srcset {
                    if let Some(src) = largest_srcset_candidate(&value) {
                        return Some(src);
                    }
                } else if let Some(src) = direct_url(&value) {
                    return Some(src);
                }
            }
            _ => {}
        }
    }
    None
}

/// srcset 解析（HTML 语法，逐字节扫描）：URL 止于空白；尾随逗号 = 无描述符；
/// 描述符止于逗号（括号内逗号除外）。w/h 需正整数、x 需非负浮点（无前导 +）；
/// 合法组合 = 仅 w、w+h、仅密度；重复后缀/非法 token 整条丢弃。
fn parse_srcset(value: &str) -> Vec<SrcsetCandidate> {
    let bytes = value.as_bytes();
    let mut offset = 0;
    let mut candidates = Vec::new();
    while offset < bytes.len() {
        while offset < bytes.len() && (bytes[offset].is_ascii_whitespace() || bytes[offset] == b',')
        {
            offset += 1;
        }
        let url_start = offset;
        while offset < bytes.len() && !bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        if url_start == offset {
            break;
        }
        let mut url_end = offset;
        while url_end > url_start && bytes[url_end - 1] == b',' {
            url_end -= 1;
        }
        let url = &value[url_start..url_end];
        let ended_with_comma = url_end < offset;
        if ended_with_comma {
            if !url.is_empty() {
                candidates.push(SrcsetCandidate {
                    url: url.to_string(),
                    width: None,
                    density: None,
                });
            }
            continue;
        }
        let descriptor_start = offset;
        let mut in_parentheses = false;
        while offset < bytes.len() {
            match bytes[offset] {
                b'(' => in_parentheses = true,
                b')' => in_parentheses = false,
                b',' if !in_parentheses => break,
                _ => {}
            }
            offset += 1;
        }
        let descriptor = value[descriptor_start..offset].trim();
        if let Some((width, _height, density)) = descriptor_values(descriptor)
            && !url.is_empty()
        {
            candidates.push(SrcsetCandidate {
                url: url.to_string(),
                width,
                density,
            });
        }
    }
    candidates
}

/// 描述符 token 解析：重复后缀拒绝；合法组合 = (w[,h]) 或仅密度。
fn descriptor_values(descriptor: &str) -> Option<(Option<u64>, Option<u64>, Option<f64>)> {
    if descriptor.is_empty() {
        return Some((None, None, None));
    }
    let mut width = None;
    let mut height = None;
    let mut density = None;
    for token in descriptor.split_ascii_whitespace() {
        if let Some(num) = token.strip_suffix('w')
            && width.is_none()
        {
            width = Some(valid_positive_integer(num)?);
        } else if let Some(num) = token.strip_suffix('h')
            && height.is_none()
        {
            height = Some(valid_positive_integer(num)?);
        } else if let Some(num) = token.strip_suffix('x')
            && density.is_none()
        {
            density = Some(valid_nonneg_float(num)?);
        } else {
            return None;
        }
    }
    match (width, height, density) {
        (Some(_), _, None) | (None, None, Some(_)) => Some((width, height, density)),
        _ => None,
    }
}

/// 正整数（纯 ASCII 数字且 > 0；前导零合法）。
fn valid_positive_integer(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse::<u64>().ok())
        .flatten()
        .filter(|v| *v > 0)
}

/// 非负浮点：无前导 `+`，有限，≥ 0。
fn valid_nonneg_float(value: &str) -> Option<f64> {
    if value.starts_with('+') {
        return None;
    }
    let parsed: f64 = value.parse().ok()?;
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

/// srcset 候选：URL 与描述符。
struct SrcsetCandidate {
    url: String,
    width: Option<u64>,
    density: Option<f64>,
}

/// 最大 srcset 候选：width 优先、其次 density、默认 1.0 最大者（过 safe_url）。
fn largest_srcset_candidate(value: &str) -> Option<String> {
    parse_srcset(value)
        .into_iter()
        .filter(|candidate| safe_url(&candidate.url))
        .max_by(|left, right| {
            let score = |candidate: &SrcsetCandidate| {
                candidate
                    .width
                    .map(|v| v as f64)
                    .or(candidate.density)
                    .unwrap_or(1.0)
            };
            score(left).total_cmp(&score(right))
        })
        .map(|candidate| candidate.url)
}

/// 直接 URL 校验：非空、非 #、非 about:blank、过 safe_url。
fn direct_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('#') || value == "about:blank" {
        return None;
    }
    safe_url(value).then(|| value.to_string())
}

/// iframe/video/audio 的源清单：自身 `src`（不看 srcset）→ 子 `<source>`
/// 的 `src`（iframe 不看子），去重保序。
pub(super) fn sources(node: NodeRef<'_>, tag: &str) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(src) = direct_attribute(node, "src") {
        urls.push(src);
    }
    if tag != "iframe" {
        for child in node.children() {
            if let SNode::Element(el) = child.value()
                && el.name() == "source"
                && let Some(src) = attribute(child, "src").as_deref().and_then(direct_url)
                && !urls.contains(&src)
            {
                urls.push(src);
            }
        }
    }
    urls
}

/// video 的海报帧：`poster` 属性按图片 URL 校验。
pub(super) fn poster(node: NodeRef<'_>, tag: &str) -> Option<String> {
    if tag != "video" {
        return None;
    }
    attribute(node, "poster").as_deref().and_then(direct_url)
}

/// fallback 属性校验用包装：链接目的地黑名单。
pub(super) fn safe_link_url_pub(url: &str) -> bool {
    safe_link_url(url)
}

/// fallback 属性校验用包装：URL scheme 白名单（图片上下文）。
pub(super) fn safe_url_pub(url: &str) -> bool {
    safe_url(url)
}

/// 海报帧 alt 文本：视频标签 + " poster"。
pub(super) fn poster_label(node: NodeRef<'_>) -> String {
    format!("{} poster", label(node, "video"))
}

/// 媒体标签：aria-label → title → 默认名（Video/Audio/Embedded content）。
pub(super) fn label(node: NodeRef<'_>, tag: &str) -> String {
    if let Some(l) = attribute(node, "aria-label").filter(|l| !l.is_empty()) {
        return l;
    }
    if let Some(t) = attribute(node, "title").filter(|t| !t.is_empty()) {
        return t;
    }
    match tag {
        "video" => "Video",
        "audio" => "Audio",
        _ => "Embedded content",
    }
    .to_string()
}

/// 属性直取校验（sources/poster 用）。
fn direct_attribute(node: NodeRef<'_>, name: &str) -> Option<String> {
    attribute(node, name).as_deref().and_then(direct_url)
}

/// URL scheme 白名单（无 `:` / 相对形 → 放行；图片允许 data:image/）。
fn safe_url(url: &str) -> bool {
    let cleaned: String = url.chars().filter(|c| !c.is_ascii_control()).collect();
    if !cleaned.contains(':') {
        return true;
    }
    let Some((scheme, _)) = cleaned.split_once(':') else {
        return true;
    };
    if scheme.contains(['/', '#', '?']) {
        return true;
    }
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto" | "tel" | "ftp"
    ) || cleaned.to_ascii_lowercase().starts_with("data:image/")
}

/// 链接目的地黑名单（去控制字符后小写前缀）。链接语境对 `data:` 全禁——
/// SVG 等 data 子类型可携带脚本，且顶层 data: 导航本就被现代浏览器阻止；
/// 图片语境的 `data:image/` 由 safe_url 单独放行。
fn safe_link_url(url: &str) -> bool {
    let cleaned: String = url.chars().filter(|c| !c.is_ascii_control()).collect();
    let lower = cleaned.to_ascii_lowercase();
    !lower.starts_with("javascript:")
        && !lower.starts_with("vbscript:")
        && !lower.starts_with("data:")
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
    fn link_href_blocks_scripts() {
        for bad in [
            r#"<a href="javascript:alert(1)">x</a>"#,
            r#"<a href="vbscript:x">x</a>"#,
            r#"<a href="data:text/html,<script>">x</a>"#,
            r#"<a href="">x</a>"#,
        ] {
            let doc = parse(bad);
            assert_eq!(link_href(find(&doc, "a")), None, "{bad}");
        }
        // data: 全禁（链接语境，含 SVG 子类型）
        for bad in [
            r#"<a href="data:image/svg+xml,<svg onload=x>">"#,
            r#"<a href="data:foo,bar">"#,
        ] {
            let doc = parse(bad);
            assert_eq!(link_href(find(&doc, "a")), None, "{bad}");
        }
        // file scheme 从资源白名单移除
        let doc = parse(r#"<img src="file:///etc/passwd.png">"#);
        assert_eq!(source(find(&doc, "img")), None);
        let doc = parse(r#"<a href="https://e.x/page">x</a>"#);
        assert_eq!(
            link_href(find(&doc, "a")),
            Some("https://e.x/page".to_string())
        );
    }

    #[test]
    fn src_validation_rules() {
        let doc = parse(r#"<img src="https://e.x/i.png">"#);
        assert!(source(find(&doc, "img")).is_some());
        for bad in [
            "<img src=\"#anchor\">",
            "<img src=\"about:blank\">",
            "<img src=\"  \">",
        ] {
            let doc = parse(bad);
            assert_eq!(source(find(&doc, "img")), None, "{bad}");
        }
        // 图片允许 data:image/，拒绝其他 data:
        let doc = parse(r#"<img src="data:image/png;base64,AAAA">"#);
        assert!(source(find(&doc, "img")).is_some());
        let doc = parse(r#"<img src="data:text/plain,hi">"#);
        assert_eq!(source(find(&doc, "img")), None);
    }

    #[test]
    fn srcset_picks_largest_width() {
        let doc = parse(r#"<img srcset="small.png 200w, big.png 800w" src="fallback.png">"#);
        assert_eq!(source(find(&doc, "img")), Some("big.png".to_string()));
    }

    #[test]
    fn srcset_falls_back_to_src() {
        let doc = parse(r#"<img srcset="" src="fallback.png">"#);
        assert_eq!(source(find(&doc, "img")), Some("fallback.png".to_string()));
    }

    #[test]
    fn srcset_density_candidates() {
        let doc = parse(r#"<img srcset="a.png 1x, b.png 2.5x">"#);
        assert_eq!(source(find(&doc, "img")), Some("b.png".to_string()));
    }

    #[test]
    fn srcset_invalid_candidates_dropped() {
        // 非法描述符整条丢弃：+2w 非法（前导+）、2w3x 组合不合法。
        let doc = parse(r#"<img srcset="a.png +2w, b.png 3x">"#);
        assert_eq!(source(find(&doc, "img")), Some("b.png".to_string()));
        let doc = parse(r#"<img srcset="a.png 2w 3x">"#);
        assert_eq!(source(find(&doc, "img")), None);
        let doc = parse(r#"<img srcset="a.png 2w 3x" src="fb.png">"#);
        assert_eq!(source(find(&doc, "img")), Some("fb.png".to_string()));
    }

    #[test]
    fn srcset_no_descriptor_defaults_one() {
        let doc = parse(r#"<img srcset="a.png, b.png 2x">"#);
        assert_eq!(source(find(&doc, "img")), Some("b.png".to_string()));
    }

    #[test]
    fn lazyload_attrs_consulted_in_order() {
        let doc = parse(r#"<img data-src="lazy.png">"#);
        assert_eq!(source(find(&doc, "img")), Some("lazy.png".to_string()));
        let doc = parse(r#"<img data-lazy-src="lazy2.png" data-original="orig.png">"#);
        assert_eq!(source(find(&doc, "img")), Some("lazy2.png".to_string()));
    }

    #[test]
    fn iframe_ignores_source_children() {
        let doc = parse(r#"<iframe><source src="s.mp4"></iframe>"#);
        assert!(sources(find(&doc, "iframe"), "iframe").is_empty());
        let doc = parse(r#"<video><source src="s.mp4"></video>"#);
        assert_eq!(
            sources(find(&doc, "video"), "video"),
            vec!["s.mp4".to_string()]
        );
    }

    #[test]
    fn media_label_defaults() {
        let doc = parse(r#"<video src="v.mp4"></video>"#);
        assert_eq!(label(find(&doc, "video"), "video"), "Video");
        let doc = parse(r#"<audio src="a.mp3"></audio>"#);
        assert_eq!(label(find(&doc, "audio"), "audio"), "Audio");
        let doc = parse(r#"<iframe src="i"></iframe>"#);
        assert_eq!(label(find(&doc, "iframe"), "iframe"), "Embedded content");
        let doc = parse(r#"<video aria-label="Demo"></video>"#);
        assert_eq!(label(find(&doc, "video"), "video"), "Demo");
    }
}
