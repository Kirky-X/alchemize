# 📖 Alchemize 用户指南

从安装到进阶的完整使用教程。命令与行为均以当前源码为准；API 细节见 [API 参考](API_REFERENCE.md)，设计背景见[架构文档](ARCHITECTURE.md)。

## 📋 目录

- [简介](#-简介)
- [快速开始](#-快速开始)
- [核心概念](#-核心概念)
- [配置](#️-配置)
- [进阶用法](#-进阶用法)
- [最佳实践](#-最佳实践)
- [故障排查](#-故障排查)
- [延伸阅读](#-延伸阅读)

## 🎯 简介

Alchemize 把任意文档炼成 Markdown：一份 PDF、一个 Word、一段音频甚至一个 zip 压缩包，一条命令得到 Markdown 文本。它按「本地优先、云端兜底」的引擎链自动降级，每次产出都标注质量层级与实际引擎，不吞错、不静默。默认特性构建零外部服务依赖，可完全离线运行。

## 🚀 快速开始

### 📌 前置条件

- **Rust 1.91+**（`Cargo.toml` 的 `rust-version`，edition 2024）
- 可选：`ffmpeg`（视频抽轨）、`pandoc`（LaTeX 高保真路径）——均在 PATH 探测，缺失时对应能力降级或报错

```bash
# 检查 Rust 版本
rustc --version
# 期望输出：rustc 1.97.1（或更高）
```

### 📦 安装

```bash
git clone https://github.com/Kirky-X/alchemize.git
cd alchemize

# 默认特性（零外部服务依赖）
cargo build --release          # 二进制：target/release/alch

# 全功能（MinerU 云端 + 本地 ASR）
cargo build --release --features full

# 安装到本地
cargo install --path . --features full
```

### 💡 第一步

```bash
# 转换到 stdout
alch convert paper.pdf

# 写文件（父目录自动创建），stderr 给摘要行
alch convert report.docx -o report.md
# converted report.docx -> report.md [local_full] (engine: docx-deep)

# 看看哪些引擎可用、支持哪些格式
alch doctor
alch formats
```

## 🔑 核心概念

### 注册表与 fallback 链

每个扩展名的候选引擎按 `priority()` 降序排成链：前序返回 `Err` 自动尝试次优；`Unavailable` 引擎直接跳过并把原因记入 attempts。`--engine NAME` 把偏好引擎提到队首（精确名优先、无精确命中时按前缀匹配，大小写不敏感，未命中静默回落不报错）。

### 质量分层

每次产出的 `quality` 字段（stderr 摘要行 `[quality]` 同源）：

| 层级 | 含义 | 典型来源 |
| --- | --- | --- |
| `api` | 远端高保真 | MinerU 云端 |
| `local_full` | 本地全功能 | 原生解析器（docx-deep/odt/odp/pptx-deep 等） |
| `local_fallback` | 本地降级 | pdf-extract 文本链、无 pandoc 的 LaTeX 围栏 |

降级产出同样算成功（退出码 0）——是否可接受由质量标签判断，工具不替你取舍。

### 引擎与扩展名

| 引擎名 | 扩展名 | 说明 |
| --- | --- | --- |
| `text` | txt/text/log/md + ~45 代码/配置扩展名 | 围栏代码块（语言标签映射） |
| `csv` | csv/tsv/psv | 分隔符自探测（采样前 5 个非空行；tsv 强制制表符、psv 强制竖线） |
| `html` / `epub` / `ipynb` | html/htm、epub、ipynb | 结构化文本解析 |
| `docx` / `pptx` | OOXML 族 | 原生 XML（表格 xlsx/xls/xlsb/ods 不支持） |
| `pdf` | pdf | pdf-extract 文本 + TOC 剔除/结构启发 |
| `html-deep`（feature `html`） | html/htm | 自研迭代状态机 → htmd 兜底；`--extract-main` 主内容提取、`--base-url` 链接绝对化；含 `<math` 自动路由 htmd 保真 |
| `mineru`（feature `pdf-cloud`+配置） | pdf | 云端高保真，链尾 |
| `latex-source` | tex/latex | pandoc 探测，缺失时围栏兜底 |
| `image` / `audio` / `video` / `subtitle` | 图片/音频/视频/字幕族 | 见下文进阶用法 |
| `zip`（facade） | zip | 递归容器，嵌套 zip 不支持 |

### 资产模式

| 模式 | 行为 |
| --- | --- |
| `dir`（默认） | 内嵌图片按文档序抽取，写输出同目录 `images/`，重名加序号前缀，markdown 用相对路径引用 |
| `inline` | base64 data URL 内联，单文件交付 |

## ⚙️ 配置

### 🧱 环境变量总表

| 变量 | 作用 |
| --- | --- |
| `ALCHEMIZE_MINERU_URL` | 设置后注册 MinerU 引擎（本地/网关服务基址） |
| `MINERU_API_TOKEN` | 云端 Precision API bearer token |
| `ALCHEMIZE_TRANSCRIBE_MODELS` | 转录模型 bundle 目录（缺省探测 `~/.cache/alchemize/models/transcribe/`） |
| `ORT_DYLIB_PATH` | onnxruntime 动态库路径 |
| `ALCHEMIZE_ASR_TEST_WAV` | 仅集成测试使用 |
| `ALCHEMIZE_LOG` | 日志级别 `off`/`error`/`warn`/`info`/`debug`/`trace`（默认 `off` 静默）；设级后全部级别只写 stderr |

### 📜 日志

CLI/MCP 的日志前端由 inklog 装配，唯一开关是 `ALCHEMIZE_LOG`：

```bash
ALCHEMIZE_LOG=debug alch convert paper.pdf > paper.md   # 日志全在 stderr，stdout 管道纯净
```

- 默认（未设或 `off`）：不装配任何 subscriber，输出形态与无日志版本逐字节一致。
- 设级后：`error` 到 `trace` 全部级别（含 `info`/`debug`）只写 stderr，stdout 只承载转换产物。
- 非法值：stderr 打一行提示后按 `off` 继续，不静默吞错。
- 同一开关对 `alch-mcp` 生效；MCP 的 stdout 承载 JSON-RPC，日志同样只走 stderr。

### 📥 MinerU 端点

```bash
export ALCHEMIZE_MINERU_URL="http://localhost:8888"   # 本地/网关 MinerU 服务
alch convert paper.pdf
```

不设置时 MinerU 引擎根本不注册，链内只有本地引擎。超时默认 300 秒；云端 Precision API 需要令牌时用 `MINERU_API_TOKEN`。

### 🔢 ASR 模型 bundle

feature `audio` 的本地转录（Qwen3-ASR + silero VAD）需要模型 bundle 与 onnxruntime 动态库：

1. 目录布局：`qwen3-conv_frontend.onnx`、`qwen3-encoder.int8.onnx`、`qwen3-decoder.int8.onnx`、`silero_vad.onnx`、`tokenizer/`。
2. `ALCHEMIZE_TRANSCRIBE_MODELS` 指向该目录（不设置则探测 `~/.cache/alchemize/models/transcribe/`）。
3. `ORT_DYLIB_PATH` 指向 onnxruntime 动态库。

bundle 缺失时引擎如实报 Unavailable 并附获取指引——**不会自动联网下载**。

### 💻 命令行工具

CLI 由 sdforge `#[forge]` 端点封装（`src/forge_app.rs`）：一份端点定义同时出 CLI 与 MCP 工具面。

```bash
alch convert <INPUT> [-o <PATH>] [--engine <NAME>] [--assets <dir|inline>] [--no-remote] [--format <EXT>]
alch doctor      # 每个已注册引擎一行：AVAILABLE 或 UNAVAILABLE + 原因
alch formats     # 全部支持扩展名，排序去重空格分隔
```

`--format` 双语义：**convert 语境**内是扩展名覆盖（旧契约）；**根级**（子命令之前）是 sdforge 输出格式 `text|json`（如 `alch --format json formats`）。

注意：**未编译进特性的引擎、未配置端点的 MinerU 不会出现在 doctor 列表中**（根本不注册）；媒体引擎的依赖缺失在转换时降级，doctor 仍显示 AVAILABLE。退出码：`0` 成功（含降级）、`1` 其他错误、`2` 不支持格式、`3` 全链失败（stderr 逐行列出各引擎失败原因）。

## 🚧 进阶用法

### ✅ 质量与降级判读

`local_fallback` 不等于失败：PDF 本地文本链是预期行为。判读顺序：先看 stderr 摘要行 `[quality]`，再按需升级特性（配 MinerU）。库使用者读 `metadata.converter` 与 `quality` 字段做同样判断。

### ☁️ MinerU 云端与本地服务

MinerU 排链尾：本地链（pdf-extract）失败后才尝试云端（产出 `api` 级）。要完全离线用 `--no-remote`。agent/precision 双客户端按文件大小自动选择（precision 自身 200 MB 上限）；响应体积按用途分级限量。

### 📝 字幕与时间码

srt/vtt/ass/ssa 原生解析（aspasia），输出时间码列表，不经 ffmpeg。

### 👀 视频处理模式

库 API `with_video_mode` 三档：`Gpu` / `AudioOnly`（默认，ffmpeg 抽轨转音频链）/ `Disabled`。ffmpeg 不在 PATH 时视频引擎报 `VideoUnavailable`。

### 🔐 注入自定义 Provider

图片视觉与音频转录都是端口：`with_vision_provider(Arc<dyn VisionProvider>)`、`with_transcribe_provider(Arc<dyn TranscribeProvider>)`。不注入时对应转换降级为元数据 markdown（含尺寸/时长/哈希），无内置云兜底。

### 🧲 主内容提取与链接绝对化（feature `html`）

```bash
# 只转正文（readability 提取，去导航/页脚噪声；失败诚实回退全页）
alch convert saved-page.html --extract-main

# 相对链接改写为绝对（#anchor、mailto: 等保留）
alch convert saved-page.html --base-url "https://origin.example/path/"
```

引擎内部为自研状态机 → htmd 兜底链：引擎主路径输出空但有可见文本时自动落 htmd；含 `<math` 标签的页面自动路由既有 htmd 引擎保 LaTeX。

**适用指引**（真实存档实测：Wikipedia 1MB 页 -71%、docs.rs 索引页 -89%、营销首页 -30%、法律文本页 -2%）：

| 场景 | 建议 | 理由 |
| --- | --- | --- |
| 网页存档转 RAG 语料 / 只读正文 | 开 | 导航、页脚、Edit 链接等噪声剔除（实测 wiki 页 "Edit section" 56→0，正文锚点零丢失） |
| 手写文档 / 结构即内容（含侧栏目录语义） | 不开 | readability 启发式可能裁掉你需要的结构，保真优先 |
| 法律/合同文本存档 | 不开或先对比 | 提取产出为正文 markdown（段落与图片保留），但页面级排版结构会变平（-2% 的许可页正文完整） |

判据很简单：页面有明确「正文块」语义（article、正文密度高）→ 开；否则不开。提取失败或无正文可提时**自动回退全页转换**（不报错、不空串），因此开着也不会丢内容——只是产出形态不同。质量标签可区分两条路径：提取路径为 `local_fallback`、全页路径为 `local_full`，下游可按标签分拣。

### 🤖 MCP 服务（feature `mcp`）

```bash
cargo build --release --features mcp   # 产出 alch-mcp
# 在 MCP 客户端配置中以 stdio 方式接入：
# { "command": "/path/to/alch-mcp" }
```

`alch-mcp` 经 sdforge（rmcp，stdio 传输——传输面无网络出口，convert 工具本身的网络能力随 `pdf-cloud` 特性）把 convert / doctor / formats 暴露为 MCP 工具，参数 schema 由端点签名生成；与 CLI 共享同一份端点定义。

### 🔭 作为库集成

```rust
use alchemize::{AlchemizeBuilder, Input};
use alchemize::application::builder::AssetMode;

let alch = AlchemizeBuilder::new()
    .with_asset_mode(AssetMode::Inline)
    .with_no_remote()               // 离线环境
    .build();
let result = alch.convert_pref(&Input::Path("report.docx".into()), Some("docx-deep")).await?;
println!("{} [{}]", result.title.as_deref().unwrap_or("-"), result.quality);
```

完整 API 见[API 参考](API_REFERENCE.md#-使用示例)。

## 🌟 最佳实践

### ✅ 推荐用法

- 默认特性先行，确有需要再上 `full`——特性越少攻击面与依赖越小。
- 批量转换用管道与脚本组合（stdout 模式对管道友好）。
- `--format <ext>` 只在扩展名不可信时使用；其余场景信任自动检测。

### 🔒 安全实践

- 不可信环境加 `--no-remote`，进程零网络出口。
- 凭据来自环境变量或 `MineruConfig::api_token`；不要把 `MINERU_API_TOKEN` 写进 shell 历史（用 env 文件或 secret 管理器）。
- 细节见[安全文档](SECURITY.md)。

## 🔧 故障排查

| 现象 | 处置 |
| --- | --- |
| 退出码 2 `unsupported format: <ext>` | `alch formats` 确认扩展名；或用 `--format` 显式指定；需要的引擎可能未编译进特性 |
| 退出码 3 全链失败 | stderr 逐行列出各引擎失败原因，按原因处理（端点不可达/文件损坏/依赖缺失） |
| 产出质量只有 `local_fallback` | 属预期降级；要更高保真配 MinerU |
| 图片只有元数据 | 未注入 VisionProvider；无内置兜底是设计行为 |
| 转换报 `AudioUnavailable` | doctor 不显示 provider 级缺失；核对 bundle 布局与 `ORT_DYLIB_PATH` |
| `zip` 转换报不支持嵌套 | zip 递归只支持一层容器内的普通文件；嵌套 zip 属显式不支持 |
| latex 没走 pandoc | pandoc 不在 PATH，已围栏兜底（质量 `local_fallback`）；装 pandoc 后自动升级 |

## 🎯 延伸阅读

- [📘 API 参考](API_REFERENCE.md) — 库集成与自定义引擎
- [🏗️ 架构文档](ARCHITECTURE.md) — fallback 链与模块划分的设计依据
- [❓ FAQ](FAQ.md) — 高频问题速查
- [🔒 安全文档](SECURITY.md) — 信任边界与加固清单
- [🤝 贡献指南](CONTRIBUTING.md) — 想改代码？从这里开始
