# 📋 更新日志

alchemize 所有显著变更记录于此文件。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added

- 文档套件：[架构文档](ARCHITECTURE.md)、[API 参考](API_REFERENCE.md)、[用户指南](USER_GUIDE.md)、[FAQ](FAQ.md)、[贡献指南](CONTRIBUTING.md)、[安全文档](SECURITY.md)（本文件除外共 6 篇）。
- 全部 .rs 源文件与测试文件补充版权与 SPDX 头（Apache-2.0）。

## [0.1.0] - 2026-11-07

首个功能完整版本：单 crate DDD 三层架构，任意文档 → Markdown。

### Added

- **转换引擎注册表**：`Converter` 端口 + 优先级 fallback 链（`application/registry.rs`）；`metadata.converter` 记录实际引擎，`quality` 如实标注 `api`/`local_full`/`local_fallback` 三级；全链失败返回逐引擎 attempts 明细。
- **文档格式**：pdf（pdf-extract 本地文本 + TOC 剔除/结构启发）、docx/pptx/xlsx/xls/ods（原生 XML + calamine）、odt/odp/rtf、html/htm、epub、csv/tsv（分隔符自探测）、txt/text/log/md 与 ~45 扩展名代码围栏、ipynb、tex/latex（pandoc 探测 + 围栏兜底）、zip 递归容器（嵌套 zip 显式不支持）。
- **`anydoc` 后端**（feature）：office 全格式含旧版二进制 Office（doc/ppt/…），priority 15 压过原生解析；图片按文档序导出。
- **`mineru` 云端 PDF**（feature）：Agent/Precision 双客户端按文件大小选择、limiteron 轮询退避、响应体积上限、SSRF 校验（scheme/环回/私网/链路本地/域名白名单/签名）；`ALCHEMIZE_MINERU_URL` 配置端点，本地优先、云端兜底（priority 5）。
- **媒体转换**：图片（VisionProvider 注入或元数据降级）、音频/视频（ffmpeg 抽轨 → 转录链）、字幕（srt/vtt/ass/ssa 原生解析）、视频帧去重、视频处理模式（Gpu/AudioOnly/Disabled）。
- **`asr` 本地转录**（feature）：vecboost 门面（Qwen3-ASR ONNX 三件套 + silero VAD），模型 bundle 缺失时如实报 Unavailable 不自动联网；输出 VAD 段级文本 + 时间戳，显式声明非说话人识别；另备 Whisper 兼容 API 后端（显式注入激活）。
- **CLI `alch`**：`convert`（-o/--engine/--assets dir|inline/--no-remote/--format）、`doctor`（逐引擎可用性诊断）、`formats`（支持扩展名清单）；退出码 0/1/2/3 语义化。
- **多编码支持**：UTF-8 直读 → GBK/Big5/Shift-JIS/EUC-JP/ISO-2022-JP/EUC-KR/windows-1252 探测（CJK 优先）→ lossy UTF-8 兜底。
- **密钥防泄漏**：`MineruConfig::api_token` 在 Debug/Display/serde 三通路脱敏（单测钉住）。

### 质量

- 测试：default 档 317 例、`--features full` 档 396 例，全部通过。
- 覆盖率：90.49%（门禁 ≥90%）。
- 真实文件套件：11 个 fixtures（macOS 文档/词表/vecboost 源文件），验证中暴露并修复 HTML 正文中 CSS 泄漏缺陷。
- 规格驱动交付：迁移全程走 specmark 规格（五大能力域 delta specs + 24/24 任务）。

### 致谢

- [firecrawl/anydoc](https://github.com/firecrawl/anydoc)（MIT）——office 全格式后端依赖。
- [PDFMathTranslate](https://github.com/PDFMathTranslate/PDFMathTranslate)（AGPL-3.0）——仅吸收设计思想（版面区域分类、图像页 OCR fallback、段落重组），零代码复制。
- [mnemis](https://github.com/Kirky-X/mnemis)（自有）——转换器骨架迁移源。

[Unreleased]: https://github.com/Kirky-X/alchemize/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Kirky-X/alchemize/releases/tag/v0.1.0
