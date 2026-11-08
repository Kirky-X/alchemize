# 更新日志

alchemize 所有显著变更记录于此文件。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 新增

- **MCP 工具面**（feature `mcp`）：`alch-mcp` 二进制经 sdforge（rmcp，stdio）把 convert/doctor/formats 暴露为 MCP 工具。
- **html-moli 引擎**（feature `moli`）：吸收 crawlrs 的 moli-html2md→htmd 转换链（third_party vendor，MIT OR Apache-2.0）——空输出防呆、迭代式 DOM 防栈击穿、诚实降级；可选主内容提取（dom_smoothie，`--extract-main`）与链接绝对化（`--base-url`）；含 `<math` 的页面自动路由 htmd 保真。convert 端点新增 `--base-url`/`--extract-main`。
- 文档套件（七篇专题文档）与全部源文件版权头（Apache-2.0 SPDX）。
- 文档骨架与 confers 对齐：七篇逐篇映射 confers 章节序列（概述→设计原则→系统架构→模块划分→数据流→安全设计→性能设计→相关文档等，逐篇见「📋 目录」）。
- 文档导航节：README.md「📚 文档」表格与 README_EN.md「Docs」表格。

### 修复

- `MineruConfig` 手工实现 `Debug`：修复 derived Debug 明文打印 `api_token` 的泄密面（Display/serde 原已脱敏），单测补钉 Debug 通路断言。
- README.md 两处对外失实修正：PDF fallback 链序（原写 MinerU 链首，实际 anydoc→pdf-extract→MinerU 链尾）；退出码清单补 `1`（其他错误）。

### 变更

- **CLI 改为 sdforge 封装**：convert/doctor/formats 改写为 `#[forge]` 端点（`src/forge_app.rs`），`alch` 经 CliBuilder + 自定义 runner 驱动；退出码 0/1/2/3、位置参数、stdout/stderr 形态、`-o`/`--no-remote`/`--format` 契约全部保留（tests/cli.rs 零改动全绿）。MSRV 1.91 → 1.97.1（sdforge 要求）；直接依赖 clap 移除（sdforge 自带）。
- 文档表述对齐代码事实：domain 层 IO 例外（`Input::to_bytes` 读盘、`DirSink` 落盘）、doctor/availability 语义（未注册引擎不出现、媒体依赖缺失走转换时降级）、模块树补齐 engines/shared/subtitle_converter。

## [0.1.0] - 2026-11-07

首个功能完整版本：单 crate DDD 三层架构，任意文档 → Markdown。

### 新增

- **转换引擎注册表**：`Converter` 端口 + 优先级 fallback 链；`metadata.converter` 记录实际引擎，`quality` 如实标注 `api`/`local_full`/`local_fallback` 三级；全链失败返回逐引擎 attempts 明细。
- **文档格式**：pdf（pdf-extract 本地文本 + TOC 剔除/结构启发）、docx/pptx/xlsx/xls/ods（原生 XML + calamine）、odt/odp/rtf、html/htm、epub、csv/tsv（分隔符自探测）、txt/text/log/md 与 ~45 扩展名代码围栏、ipynb、tex/latex（pandoc 探测 + 围栏兜底）、zip 递归容器（嵌套 zip 显式不支持）。
- **anydoc 后端**（feature `anydoc`）：office 全格式含旧版二进制 Office（doc/ppt/…），priority 15 压过原生解析；图片按文档序经自研 GFM 渲染器导出。
- **MinerU 云端 PDF**（feature `mineru`）：Agent/Precision 双客户端按文件大小选择、limiteron 轮询退避、响应体积分级上限、SSRF 校验（scheme/环回/私网/CGNAT/链路本地/域名白名单/签名）；`ALCHEMIZE_MINERU_URL` 配置端点，本地优先、云端兜底（priority 5 链尾）。
- **媒体转换**：图片（VisionProvider 注入或元数据降级）、音频/视频（ffmpeg 抽轨 → 转录链）、字幕（srt/vtt/ass/ssa 原生解析）、视频帧去重、视频处理模式（Gpu/AudioOnly/Disabled）。
- **ASR 本地转录**（feature `asr`）：vecboost 门面（Qwen3-ASR ONNX 三件套 + silero VAD），模型 bundle 缺失时如实报 Unavailable 不自动联网；输出 VAD 段级文本 + 时间戳，显式声明非说话人识别；另备 Whisper 兼容 API 后端（显式注入激活）。
- **CLI `alch`**：`convert`（-o/--engine/--assets dir|inline/--no-remote/--format）、`doctor`（逐引擎可用性诊断）、`formats`（支持扩展名清单）；退出码 0/1/2/3 语义化。
- **多编码支持**：UTF-8 直读 → GBK/Big5/Shift-JIS/EUC-JP/ISO-2022-JP/EUC-KR/windows-1252 探测（CJK 优先）→ lossy UTF-8 兜底。
- **密钥防泄漏**：`MineruConfig::api_token` 脱敏（单测钉住）。

### 安全

- MinerU 下载 URL 全量过 `ValidatedFileUrl` SSRF 校验（HTTPS-only、拒环回/RFC1918/CGNAT/链路本地/IPv4 映射 IPv6、域名白名单、预签名要求）。
- 远端响应体积分级限量（64KB/8KB/50MB/50MB，预分配上限 8MB）。
- 凭据只经环境变量传递，无内置密钥、无加密存储需求。

### 质量

- 测试：default 档 317 例、`--features full` 档 396 例，全部通过。
- 覆盖率：90.49%（门禁 ≥90%）。
- 真实文件套件：12 项测试，素材为 9 个仓库内 fixtures 与 1 个外部视频源，验证中暴露并修复 HTML 正文中 CSS 泄漏缺陷。
- 规格驱动交付：迁移全程走 specmark 规格（五大能力域 delta specs + 24/24 任务）。

### 致谢

- [firecrawl/anydoc](https://github.com/firecrawl/anydoc)（MIT）——office 全格式后端依赖。
- [PDFMathTranslate](https://github.com/PDFMathTranslate/PDFMathTranslate)（AGPL-3.0）——仅吸收设计思想（版面区域分类、图像页 OCR fallback、段落重组），零代码复制。
- mnemis（自有，未公开）——转换器骨架迁移源。

[Unreleased]: https://github.com/Kirky-X/alchemize/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Kirky-X/alchemize/releases/tag/v0.1.0
