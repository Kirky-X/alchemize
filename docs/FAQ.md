# ❓ Alchemize FAQ

常见问题解答。所有答案基于当前源码的真实行为，不含计划中或臆测的内容。

## 📋 目录

- [通用问题](#-通用问题)
- [安装与配置](#-安装与配置)
- [使用与特性](#️-使用与特性)
- [性能](#-性能)
- [故障排查](#-故障排查)
- [还有其他问题？](#-还有其他问题)

## 🧭 通用问题

### ❓ 什么是 Alchemize？

一个 Rust 文档转 Markdown 工具（CLI `alch` + 同名库）：支持 PDF、Office 全格式、HTML/EPUB、CSV、代码/配置文本、图片、音视频、字幕、zip 递归容器等输入，按优先级 fallback 链自动选择引擎，产出带质量标注的 Markdown。

### ❓ 为什么要使用 Alchemize？

三个理由：① 一条命令统一任意格式到 Markdown，不需要为每种格式找单独工具；② 降级显性——每次产出标注 `api`/`local_full`/`local_fallback` 质量层级与实际引擎名，不吞错、不静默；③ 默认构建零外部服务依赖，`--no-remote` 可完全离线。

### ❓ Alchemize 可以用于生产环境吗？

可以。测试规模 default 档 168 例（仅 core + text）、full 档 912 passed（另 5 ignored）、all-features 档 917 passed（另 5 ignored），行覆盖率 90.49%（门禁 ≥90%），真实文件套件 12 项测试（素材：9 个仓库内 fixtures 与 1 个外部视频源）覆盖 PDF/DOCX/XLSX/RTF 等真实文档。安全模型见[安全文档](SECURITY.md)。

### ❓ 支持哪些平台？

任何 Rust 1.91+ 支持的平台（macOS/Linux/Windows）。外部二进制依赖（ffmpeg/pandoc）按 PATH 探测，缺失时对应能力降级。ASR 本地推理另需 onnxruntime 动态库。

### ❓ 如何参与贡献？

见[贡献指南](CONTRIBUTING.md)：环境准备、TDD 工作流、代码规范、提交与 PR 流程。仓库采用 specmark 规格驱动流程，能力域规格真源在 `specmark/specs/`。

### ❓ 在哪里可以获得帮助？

先查本 FAQ 与[用户指南](USER_GUIDE.md)的故障排查节；再查[架构文档](ARCHITECTURE.md)与[API 参考](API_REFERENCE.md)；仍无解则开 GitHub Issue（附最小复现与 `alch doctor` 输出）。

### ❓ 项目采用什么许可证？

Apache-2.0（`LICENSE` 与 Cargo.toml 一致，全部源文件带 SPDX 头）。致谢与边界：PDFMathTranslate（AGPL-3.0）仅吸收设计思想、零代码复制。

## 📦 安装与配置

### ❓ 如何安装？

```bash
cargo install --path . --features full   # 或 cargo build --release（默认仅 CLI + 文本转换）
```

详见[用户指南](USER_GUIDE.md#-安装)。

### ❓ 如何选择合适的特性组合？

```bash
cargo build                        # 最小构建：仅文本/代码围栏（default = cli + text）
cargo build --features pdf,docx    # 需要哪些格式就开哪些格式 feature
cargo build --features pdf,pdf-cloud # PDF 本地链 + MinerU 云端兜底（还需配置端点）
cargo build --features audio       # 本地转录（自动带入 media；还需模型 bundle + ORT 库）
cargo build --features full        # 全量（28 个转换类 feature）
```

### ❓ 为什么默认不能转 pdf/docx？

这是 `dependency-feature-gating` 引入的破坏性变更：`default = ["cli", "text"]`，默认构建只编译 CLI 骨架与纯文本转换。PDF/Word 等格式的转换器与依赖全部由各自 feature 门控，未启用时不会编译、也不会注册——`alch convert x.pdf` 因此没有候选引擎，返回 `unsupported format`（退出码 2）。需要哪些格式就显式开启（如 `--features pdf,docx`）；要恢复旧的全量默认行为用 `--features full`。

### ❓ 如何最小构建？

```bash
cargo build --release                              # 只含 CLI + 文本/代码围栏
cargo build --release --no-default-features --features cli,text  # 等价显式写法
```

最小构建不拉取任何网络/云/Office/容器依赖；除核心 9 个依赖（`serde`/`serde_json`/`thiserror`/`tracing`/`tokio`/`async-trait`/`anyhow`/`base64`/`sha2`）外，其余依赖全部 `optional`，只随对应 feature 经 `dep:` 激活。

### ❓ 不同特性组合的依赖数量差别有多大？

default 档只含核心 9 依赖 + `encoding_rs`（文本），没有任何网络/云/Office/容器依赖（reqwest/limiteron/futures/lopdf/quick-xml/zip/htmd 均为可选）；`pdf` 引入 lopdf，`pdf-cloud` 引入 reqwest+limiteron+futures，`audio` 引入 vecboost（ort/ONNX 栈，path 依赖）。特性关闭的引擎不参与编译，二进制更小、审计面更小。

### ❓ 系统要求是什么？

Rust 1.91+（`rust-version` 门禁）。可选外部工具：ffmpeg（视频）、pandoc（LaTeX 高保真）。ASR 另需模型 bundle 与 onnxruntime 动态库（路径见[用户指南](USER_GUIDE.md#-asr-模型-bundle)）。

## 🛠️ 使用与特性

### ❓ 如何快速上手基本用法？

```bash
alch convert file.pdf > out.md      # stdout
alch convert file.pdf -o out.md     # 写文件 + stderr 摘要行
alch doctor && alch formats         # 先看引擎与格式
```

### ❓ 支持哪些格式和来源？

文档族（pdf/docx/pptx/odt/odp/rtf/旧版二进制 Office/html/epub/csv/tsv/txt/md/zip 递归/ipynb/tex；表格（xlsx/xls/xlsb/ods）不支持）、代码配置族（~45 扩展名围栏化）、图片（png/jpg/gif/bmp/webp/tiff/svg）、音频（wav/mp3/ogg/opus/flac/aac/m4a/wma）、视频（mp4/avi/mkv/mov/wmv/flv/webm/m4v，ffmpeg 抽轨）、字幕（srt/vtt/ass/ssa）。以上格式均要在构建时开启对应 feature（default 只有 `text`）；以 `alch formats` 实时输出为准。

### ❓ 转换质量怎么判读？有校验功能吗？

没有独立的「校验」功能；质量判读靠产出标注：stderr 摘要行与 `metadata.converter`/`quality` 字段。降级（`local_fallback`）属预期行为而非错误；zip 嵌套、不支持格式等边界属显式报错。

### ❓ `--extract-main` 什么时候该开？

| 场景 | 建议 |
| --- | --- |
| 网页存档 → RAG 语料 / 只要正文 | 开（导航/页脚/Edit 噪声剔除；真实实测 wiki 1MB 页 -71%、"Edit section" 56→0，正文锚点零丢失） |
| 手写文档 / 结构即内容 | 不开（readability 启发式可能裁掉需要的结构） |
| 法律/合同全量存档 | 不开或先对比（提取产出为正文 markdown：段落/图片保留，页面级排版变平） |

安全网：提取失败或无正文可提时**自动回退全页转换**，开着不会丢内容。两条路径可由质量标签区分——提取路径 `local_fallback`、全页路径 `local_full`（stderr 摘要与 `quality` 字段）。实测明细见[用户指南 · 主内容提取](USER_GUIDE.md#-主内容提取与链接绝对化feature-html)。

### ❓ Alchemize 安全吗？

安全模型四道防线：不可信输入的结构化错误处理（不 panic）、MinerU 下载 URL 的 SSRF 校验（HTTPS-only/私网拒绝/域名白名单/签名）、远端响应分级限量、凭据三通路脱敏。默认构建零网络出口。完整说明见[安全文档](SECURITY.md)。

### ❓ 如何报告安全漏洞？

不要开公开 Issue。用 GitHub 私密安全通告（仓库 Security 标签页 → Report a vulnerability），流程与响应承诺见[安全文档](SECURITY.md#-漏洞报告流程)。

## ⚡ 性能

### ❓ Alchemize 有多快？

本仓暂无 criterion 基准数据，不提供性能数字（不编造）。设计层面的取舍见[架构文档·性能设计](ARCHITECTURE.md#-性能设计)：单线程 runtime、异步远端路径、无全局健康缓存、响应内存有界。

### ❓ 内存占用如何？

转换以单文件为粒度流式处理；远端响应读取按用途分级限量（提交 64KB/错误 8KB/轮询与结果 50MB，预分配上限 8MB）；zip 拒绝嵌套递归，无界深度在结构上不可能。builder 的 `with_max_file_size` 当前未被引擎装配消费（已知限制，见 [API 参考](API_REFERENCE.md#-核心-api)）。

## 🔧 故障排查

### ❓ 出现 "unsupported format" 错误

退出码 2。三种可能：扩展名确实不支持（`alch formats` 确认）、需要的引擎未编译进特性、扩展名被误判（用 `--format <ext>` 显式指定）。

### ❓ 出现 "all backends failed" 错误

退出码 3。stderr 逐行列出 `- <engine>: <reason>`：端点不可达就查服务、文件损坏就换源、依赖缺失就补装。全链失败不会产出半成品文件。

## 🎯 还有其他问题？

- 先翻[用户指南](USER_GUIDE.md)与[API 参考](API_REFERENCE.md)
- 行为有疑问时 `alch doctor` + 最小复现文件开 GitHub Issue
- 想动手改代码 → [贡献指南](CONTRIBUTING.md)
