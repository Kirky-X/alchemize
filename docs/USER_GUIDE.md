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
# 期望输出：rustc 1.91（或更高）
```

### 📦 安装

```bash
git clone https://github.com/Kirky-X/alchemize.git
cd alchemize

# 默认特性（零外部服务依赖）
cargo build --release          # 二进制：target/release/alch

# 全功能（MinerU 云端 + anydoc office 后端 + 本地 ASR）
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
# converted report.docx -> report.md [local_full] (engine: anydoc)

# 看看哪些引擎可用、支持哪些格式
alch doctor
alch formats
```

## 🔑 核心概念

### 注册表与 fallback 链

每个扩展名的候选引擎按 `priority()` 降序排成链：前序返回 `Err` 自动尝试次优；`Unavailable` 引擎直接跳过并把原因记入 attempts。`--engine NAME` 把偏好引擎提到队首（前缀匹配、大小写不敏感，未命中静默回落不报错）。

### 质量分层

每次产出的 `quality` 字段（stderr 摘要行 `[quality]` 同源）：

| 层级 | 含义 | 典型来源 |
| --- | --- | --- |
| `api` | 远端高保真 | MinerU 云端 |
| `local_full` | 本地全功能 | anydoc、原生解析器 |
| `local_fallback` | 本地降级 | pdf-extract 文本链、无 pandoc 的 LaTeX 围栏 |

降级产出同样算成功（退出码 0）——是否可接受由质量标签判断，工具不替你取舍。

### 引擎与扩展名

| 引擎名 | 扩展名 | 说明 |
| --- | --- | --- |
| `text` | txt/text/log/md + ~45 代码/配置扩展名 | 围栏代码块（语言标签映射） |
| `csv` | csv/tsv | 分隔符自探测（首行 tab 多于逗号按 TSV） |
| `html` / `epub` / `ipynb` | html/htm、epub、ipynb | 结构化文本解析 |
| `docx` / `pptx` / `xlsx` | OOXML 族（+xls/ods） | 原生 XML + calamine |
| `pdf` | pdf | pdf-extract 文本 + TOC 剔除/结构启发 |
| `anydoc`（feature） | office 全格式 + 旧版二进制 Office | priority 15 压过原生解析 |
| `mineru`（feature+配置） | pdf | 云端高保真，链尾 |
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

### 📥 MinerU 端点

```bash
export ALCHEMIZE_MINERU_URL="http://localhost:8888"   # 本地/网关 MinerU 服务
alch convert paper.pdf
```

不设置时 MinerU 引擎根本不注册，链内只有本地引擎。超时默认 300 秒；云端 Precision API 需要令牌时用 `MINERU_API_TOKEN`。

### 🔢 ASR 模型 bundle

feature `asr` 的本地转录（Qwen3-ASR + silero VAD）需要模型 bundle 与 onnxruntime 动态库：

1. 目录布局：`qwen3-conv_frontend.onnx`、`qwen3-encoder.int8.onnx`、`qwen3-decoder.int8.onnx`、`silero_vad.onnx`、`tokenizer/`。
2. `ALCHEMIZE_TRANSCRIBE_MODELS` 指向该目录（不设置则探测 `~/.cache/alchemize/models/transcribe/`）。
3. `ORT_DYLIB_PATH` 指向 onnxruntime 动态库。

bundle 缺失时引擎如实报 Unavailable 并附获取指引——**不会自动联网下载**。

### 💻 命令行工具

```bash
alch convert <INPUT> [-o <PATH>] [--engine <NAME>] [--assets <dir|inline>] [--no-remote] [--format <EXT>]
alch doctor      # 每个已注册引擎一行：AVAILABLE 或 UNAVAILABLE + 原因
alch formats     # 全部支持扩展名，排序去重空格分隔
```

注意：**未编译进特性的引擎、未配置端点的 MinerU 不会出现在 doctor 列表中**（根本不注册）；媒体引擎的依赖缺失在转换时降级，doctor 仍显示 AVAILABLE。退出码：`0` 成功（含降级）、`1` 其他错误、`2` 不支持格式、`3` 全链失败（stderr 逐行列出各引擎失败原因）。

## 🚧 进阶用法

### ✅ 质量与降级判读

`local_fallback` 不等于失败：PDF 无 `anydoc` 时走文本链是预期行为。判读顺序：先看 stderr 摘要行 `[quality]`，再按需升级特性（装 `anydoc`/配 MinerU）。库使用者读 `metadata.converter` 与 `quality` 字段做同样判断。

### ☁️ MinerU 云端与本地服务

MinerU 排链尾：本地链（anydoc → pdf-extract）失败后才尝试云端（产出 `api` 级）。要完全离线用 `--no-remote`。agent/precision 双客户端按文件大小自动选择（precision 自身 200 MB 上限）；响应体积按用途分级限量。

### 📝 字幕与时间码

srt/vtt/ass/ssa 原生解析（aspasia），输出时间码列表，不经 ffmpeg。

### 👀 视频处理模式

库 API `with_video_mode` 三档：`Gpu` / `AudioOnly`（默认，ffmpeg 抽轨转音频链）/ `Disabled`。ffmpeg 不在 PATH 时视频引擎报 `VideoUnavailable`。

### 🔐 注入自定义 Provider

图片视觉与音频转录都是端口：`with_vision_provider(Arc<dyn VisionProvider>)`、`with_transcribe_provider(Arc<dyn TranscribeProvider>)`。不注入时对应转换降级为元数据 markdown（含尺寸/时长/哈希），无内置云兜底。

### 🔭 作为库集成

```rust
use alchemize::{AlchemizeBuilder, Input};
use alchemize::application::builder::AssetMode;

let alch = AlchemizeBuilder::new()
    .with_asset_mode(AssetMode::Inline)
    .with_no_remote()               // 离线环境
    .build();
let result = alch.convert_pref(&Input::Path("report.docx".into()), Some("anydoc")).await?;
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
- 凭据只走环境变量；不要把 `MINERU_API_TOKEN` 写进 shell 历史（用 env 文件或 secret 管理器）。
- 细节见[安全文档](SECURITY.md)。

## 🔧 故障排查

| 现象 | 处置 |
| --- | --- |
| 退出码 2 `unsupported format: <ext>` | `alch formats` 确认扩展名；或用 `--format` 显式指定；需要的引擎可能未编译进特性 |
| 退出码 3 全链失败 | stderr 逐行列出各引擎失败原因，按原因处理（端点不可达/文件损坏/依赖缺失） |
| 产出质量只有 `local_fallback` | 属预期降级；要更高保真装 `anydoc`/配 MinerU |
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
