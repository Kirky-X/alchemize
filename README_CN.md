[English](README.md) | 中文

<div align="center">

<img src="docs/assets/alchemize.png" alt="Alchemize Logo" width="240">

[![Rust 2024](https://img.shields.io/badge/Rust-2024-edded?logo=rust&style=for-the-badge)](https://www.rust-lang.org/) [![Rustc 1.97.1+](https://img.shields.io/badge/Rustc-1.97.1+-orange.svg?style=for-the-badge)](https://www.rust-lang.org/) [![license](https://img.shields.io/badge/license-Apache--2.0-blue?style=for-the-badge)](LICENSE)

**把任意文档炼成 Markdown**

[✨ 功能特性](#-功能特性) • [🚀 快速开始](#-快速开始) • [📚 文档](#-文档) • [🔒 安全](#-安全) • [🤝 参与贡献](#-参与贡献)

</div>

---

<div align="center">

### 🎯 一处输入，任意格式，炼成 Markdown

给 `alch` 一个文件——PDF、Office、音视频、zip 压缩包——它按优先级 fallback 链自动选引擎，产出带质量标注的 Markdown：

<table style="width:100%; border-collapse: collapse">
<tr>
<td align="center" width="25%">🧪<br><b>任意格式</b><br><span style="color:#64748B">文档 · 代码 · 图片 · 音视频 · 字幕 · zip</span></td>
<td align="center" width="25%">🔗<br><b>fallback 链</b><br><span style="color:#64748B">priority 降序 · 失败自动降级 · 降级显性</span></td>
<td align="center" width="25%">🏷️<br><b>质量分层</b><br><span style="color:#64748B">api / local_full / local_fallback · 引擎可溯</span></td>
<td align="center" width="25%">🏠<br><b>本地优先</b><br><span style="color:#64748B">默认零外部服务 · --no-remote 全离线</span></td>
</tr>
</table>

</div>

---

## 📋 目录

<details open>
<summary>📑 目录</summary>

- [✨ 功能特性](#-功能特性)
- [🚀 快速开始](#-快速开始)
- [📚 文档](#-文档)
- [🔒 安全](#-安全)
- [🤝 参与贡献](#-参与贡献)
- [📋 更新日志](#-更新日志)
- [📄 许可证](#-许可证)
- [🙏 致谢](#-致谢)
- [📞 联系与支持](#-联系与支持)
- [⭐ Star 历史](#-star-历史)

</details>

---

## ✨ 功能特性

<table style="width:100%; border-collapse: collapse">
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">📄 <b>文档全格式</b><br><span style="color:#64748B">PDF（pdfium/纯 Rust/MinerU）、docx/pptx、odt/odp、旧版二进制 Office（doc/ppt/rtf，LibreOffice）、html/epub、csv/tsv/psv、LaTeX/org/rst/asciidoc（pandoc）、ipynb</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧬 <b>代码围栏化</b><br><span style="color:#64748B">py/rs/js/ts/go/java/c/cpp/json/yaml/toml/xml/sh 等 ~45 扩展名，带语言标签的围栏代码块</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🖼️ <b>图片处理</b><br><span style="color:#64748B">注入 VisionProvider 做描述与 OCR；不注入则降级为元数据 markdown（含尺寸/哈希）</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🎙️ <b>音视频转录</b><br><span style="color:#64748B">自研 vecboost 门面（Qwen3-ASR ONNX + silero VAD）完全本地推理；输出 VAD 段级文本 + 时间戳</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">⏱️ <b>字幕解析</b><br><span style="color:#64748B">srt/vtt/ass/ssa 原生解析，输出时间码列表，不经 ffmpeg</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">📦 <b>zip 递归</b><br><span style="color:#64748B">压缩包内文件按扩展名走各自引擎；嵌套 zip 显式不支持，无界自递归在结构上不可能</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🌐 <b>多编码探测</b><br><span style="color:#64748B">UTF-8 直读 → GBK/Big5/Shift-JIS 等 CJK 优先探测 → lossy 兜底；csv 分隔符自探测</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧭 <b>引擎偏好与诊断</b><br><span style="color:#64748B"><code>--engine</code> 精确名优先、前缀回退（未命中静默回落）；<code>doctor</code> 逐引擎可用性、<code>formats</code> 全格式清单</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🖼 <b>资产双模式</b><br><span style="color:#64748B"><code>dir</code>（图片写 <code>images/</code> 相对引用）与 <code>inline</code>（base64 单文件交付）</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🛡️ <b>安全防线</b><br><span style="color:#64748B">MinerU URL SSRF 校验（HTTPS-only/私网拒绝/域名白名单）、响应分級限量、凭据三通路脱敏</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">☁️ <b>MinerU 云端高保真</b><br><span style="color:#64748B">Agent/Precision 双客户端按文件大小选择、轮询退避；本地优先、云端兜底（链尾）</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧰 <b>CLI 诊断三件套</b><br><span style="color:#64748B"><code>convert</code>（-o/--engine/--assets/--no-remote/--format）、<code>doctor</code>、<code>formats</code>；退出码 0/1/2/3 语义化</span></td>
</tr>
</table>

除上述核心能力外，视频抽轨与帧去重、视频处理模式（Gpu/AudioOnly/Disabled）、Whisper 兼容云端后端（显式注入激活）、LaTeX 数学定界符校验，以及可选的转换结果去重缓存（`cache`，TTL + 单飞，作用于 pdf-cloud/audio）等能力也均已内置或以独立特性标志提供；完整矩阵见 [用户指南 · 特性标志](docs/cn/USER_GUIDE.md#-特性标志)。

---

## 🚀 快速开始

### 📦 安装

```bash
# 检查 Rust 版本（要求 1.97.1+）
rustc --version

# 默认特性（CLI + 纯文本转换，约 220 crates，其中 157 来自 CLI 骨架，零外部服务依赖；破坏性变更见「特性标志」）
cargo build --release          # 二进制：target/release/alch

# 按需开启格式（示例：PDF + Word + HTML；需要哪些格式就开哪些 feature）
cargo build --release --features pdf,docx,html

# 全功能（全部 30 个转换类 feature：MinerU 云端 + 自研 office 深解析 + 容器/数据/邮件 + LibreOffice + 本地 ASR）
cargo build --release --features full

# 安装到本地
cargo install --path . --features full
# 注意：--no-default-features 时必须带 cli，否则不产出 alch 二进制
```

### 💡 最小示例

```bash
# 转换到 stdout（PDF 需以 --features pdf 构建）
alch convert paper.pdf

# 写文件 + 引擎偏好 + 资产模式（父目录自动创建；Word 需 --features docx）
alch convert report.docx -o report.md --engine docx-deep --assets dir|inline

# stderr 摘要行：质量层级与实际引擎一目了然
# converted report.docx -> report.md [local_full] (engine: docx-deep)
```

```bash
# 引擎诊断 / 支持格式
alch doctor
alch formats
```

退出码：`0` 成功（含降级产出）；`1` 其他错误；`2` 不支持格式；`3` 全链失败（stderr 列出各引擎失败原因）。

更多示例（批量转换、离线模式、自定义视觉/转录提供方、注册表装配）见 [用户指南 · CLI 示例](docs/cn/USER_GUIDE.md#-cli-示例) 与 [API 参考 · 使用示例](docs/cn/API_REFERENCE.md#-使用示例)。

### 📊 支持格式矩阵与核心概念

完整格式矩阵见 [用户指南 · 支持格式矩阵](docs/cn/USER_GUIDE.md#-支持格式矩阵)；注册表与 fallback 链、质量分层、本地优先等核心概念见 [用户指南 · 核心概念](docs/cn/USER_GUIDE.md#-核心概念)。

---

## 📚 文档

| 文档 | 说明 |
|------|------|
| [📖 用户指南](docs/cn/USER_GUIDE.md) | 从安装到进阶的完整使用教程 |
| [📘 API 参考](docs/cn/API_REFERENCE.md) | 全部公开 API、CLI 接口与环境变量 |
| [🏗️ 架构文档](docs/cn/ARCHITECTURE.md) | 设计原则、模块划分与 fallback 链机制 |
| [❓ FAQ](docs/cn/FAQ.md) | 常见问题解答 |
| [🔒 安全文档](docs/cn/SECURITY.md) | 信任边界、SSRF 防护与漏洞报告 |
| [🤝 贡献指南](docs/cn/CONTRIBUTING.md) | 开发环境、TDD 工作流与提交约定 |
| [📊 基准测试](docs/cn/benchmarks.md) | 真实文件性能基准（criterion） |
| [📋 更新日志](docs/cn/CHANGELOG.md) | 版本变更记录、开发路线图与致谢 |

能力域规格真源（转换/降级链/媒体/CLI 输出等可验证行为）在 `specmark/specs/`。

---

## 🔒 安全

### 🛡️ 安全设计

四道防线：不可信输入的结构化错误处理（不 panic）；MinerU 下载 URL 的 SSRF 校验（HTTPS-only、私网拒绝、域名白名单）；远端响应分级限量；凭据三通路脱敏。详见 [🔒 安全文档](docs/cn/SECURITY.md)。

### 🚨 报告安全漏洞

**不要**开公开 Issue。使用 GitHub 私密安全通告（**Security → Report a vulnerability**），流程与响应承诺见[安全文档 · 漏洞报告流程](docs/cn/SECURITY.md#-漏洞报告流程)。

---

## 🤝 参与贡献

### 🛠️ 开发环境

Rust 1.97.1+，`cargo build --features full` 一键起。详见 [🤝 贡献指南](docs/cn/CONTRIBUTING.md#-环境准备)。

### 💖 贡献方式

- 新格式引擎：实现 `Converter` trait + 在 `engines.rs` 装配 + 钉住行为的测试
- 缺陷修复：先写暴露缺陷的失败测试再修（TDD）
- 文档与规格：文档沿用 confers 骨架版式；行为变更同步 `specmark/specs/`

### 🐛 报告 Bug 与提交 PR

- 报告 Bug：开 [GitHub Issue](https://github.com/Kirky-X/alchemize/issues)，附最小复现文件（脱敏）、`alch --version`、期望与实际输出。
- 功能建议：开 Issue 并标注 `enhancement`；涉及网络出口或凭据处理的提案请先读[安全文档](docs/cn/SECURITY.md)。
- 提交 PR：Conventional Commits（feat/fix/test/docs/chore）；提交前本地门禁：`cargo fmt --check` + `cargo clippy --all-features` 零新增 + 三档测试全绿。

---

## 📋 更新日志

版本变更与[开发路线图](docs/cn/CHANGELOG.md#开发路线图)见 [docs/cn/CHANGELOG.md](docs/cn/CHANGELOG.md)。

---

## 📄 许可证

Apache-2.0（[LICENSE](LICENSE)）。全部源文件带 SPDX 头；自其他仓库迁移的文件保留原版权年份区间。

---

## 🙏 致谢

核心依赖与特别感谢名单见 [更新日志 · 致谢](docs/cn/CHANGELOG.md#致谢)。

---

## 📞 联系与支持

<table style="width:100%; max-width: 600px">
<tr>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/alchemize/issues"><b style="color:#991B1B">Issues</b></a><br>
<span style="color:#64748B">报告问题和 Bug</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/alchemize/security/advisories"><b style="color:#1E40AF">安全通告</b></a><br>
<span style="color:#64748B">私密漏洞报告</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/alchemize"><b style="color:#1E293B">GitHub</b></a><br>
<span style="color:#64748B">查看源代码</span>
</td>
</tr>
</table>

---

## ⭐ Star 历史

[![Star History Chart](https://api.star-history.com/svg?repos=Kirky-X/alchemize&type=Date)](https://star-history.com/#Kirky-X/alchemize&Date)

如果这个项目对您有帮助，请考虑给它一个 ⭐️！
