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

- **Rust 1.97.1+**（`Cargo.toml` 的 `rust-version`，edition 2024）
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

# 默认特性（仅 CLI + 纯文本转换，零外部服务依赖；破坏性变更）
cargo build --release          # 二进制：target/release/alch

# 按需开启格式（示例：PDF + Word；未开启的格式返回 unsupported，退出码 2）
cargo build --release --features pdf,docx

# 全功能（MinerU 云端 + 本地 ASR）
cargo build --release --features full

# 安装到本地
cargo install --path . --features full
```

默认 `default = ["cli", "text"]`：`cargo build` 只编译 CLI 骨架与纯文本/代码围栏转换。要转 PDF、Word、HTML 等格式，必须显式开启对应 feature（见[引擎与扩展名](#引擎与扩展名)的 feature 列）。

### 💡 第一步

```bash
# 转换到 stdout（PDF 需以 --features pdf 构建）
alch convert paper.pdf

# 写文件（父目录自动创建），stderr 给摘要行（Word 需 --features docx）
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

| 引擎名 | 扩展名 | 需要的 feature | 说明 |
| --- | --- | --- | --- |
| `text` | txt/text/log/md + ~45 代码/配置扩展名 | `text`（default 成员） | 围栏代码块（语言标签映射） |
| `csv` | csv/tsv/psv | `csv` | 分隔符自探测（采样前 5 个非空行；tsv 强制制表符、psv 强制竖线） |
| `html` / `html-deep` | html/htm | `html` | `html` 为 htmd 基础链、`html-deep` 为自研深链（两条链同属 `html` feature）；深链支持 `--extract-main` 主内容提取与 `--base-url` 链接绝对化，含 `<math` 自动路由 htmd 保真 |
| `epub` | epub | `epub` | 结构化文本解析 |
| `ipynb` | ipynb | `ipynb` | 笔记本单元格转 markdown |
| `docx` / `docx-deep` | docx | `docx` | OOXML 原生 XML（表格 xlsx/xls/xlsb/ods 不支持） |
| `pptx` / `pptx-deep` | pptx | `pptx` | 同上 |
| `odt-deep` / `odp-deep` | odt / odp | `odt` / `odp` | OpenDocument 深解析 |
| `legacy-office` / `legacy-ppt` | doc, rtf / ppt,pps,pot | `legacy-office` | 外部 soffice 转换后进深解析链 |
| `pdf` | pdf | `pdf` | lopdf 文本抽取 + TOC 剔除/结构启发 |
| `pdfium` | pdf | `pdf-local` | 本地高保真（自动带入 `pdf`）；动态库缺失降级 |
| `mineru` | pdf | `pdf-cloud` + 端点配置 | 云端高保真，链尾 |
| `latex-source` | tex/latex | `latex` | pandoc 探测，缺失时围栏兜底 |
| `pandoc-generic` | org/rst/adoc/asciidoc/opml/mediawiki 等 | `pandoc` | 通用标记语言 |
| `image` / `audio` / `video` / `subtitle` | 图片/音频/视频/字幕族 | `media` | 见下文进阶用法；转录后端另开 `audio` |
| `heic` | heic, heif | `heic` | sips 或元数据降级 |
| `zip`（facade） | zip, cbz | `zip` | 递归容器，嵌套 zip 不支持 |

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
- 同一开关对 `alch mcp` 子命令生效；MCP 的 stdout 承载 JSON-RPC，日志同样只走 stderr。

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

### 🛡️ 审计安全姿态（`doctor --security`）

`alch doctor --security` 在普通 doctor 报告基础上扩展四段视图，展示当前构建的安全边界：解析面、网络出口、外部子进程、预算。它只读编译期特性与环境变量，不发起网络调用。

```bash
alch doctor --security
alch --format json doctor --security   # 同样四段，以 JSON 行数组呈现
```

输出样例（未编译 `pdf-cloud` 的构建）：

```text
Parse surfaces:    text, csv, docx, html, pdf-local, ...
Network egress:    ✗ NONE (pdf-cloud not compiled)
External subprocesses:
  pandoc          ✓ on PATH
  soffice         ✗ not found
  sips            ✓ on PATH
  ffmpeg          ✓ on PATH
Budgets:
  per page / stream         64 MiB
  per document             256 MiB
  container chain          512 MiB
  http submit               64 KB
```

编译 `pdf-cloud` 且设置 `ALCHEMIZE_MINERU_URL` 时，出口行显示 MinerU 主机名与一行 `token:` 状态。token 值永不打印，只显示 `present` / `NOT SET`。该信息与 `SECURITY.md` 文档一致，针对当前构建实时渲染；参见[安全最佳实践](SECURITY.md#-安全最佳实践)。

### 🔍 回放转换过程（`convert --explain`）

`alch convert <INPUT> --explain` 在正常结果之后向 stderr 追加引擎链回放。每行显示引擎、结局，以及（对已执行步骤）实测耗时；末行显示汇聚后的信任等级。该旗标不改变 stdout 结果，仅是诊断视图。

```bash
alch convert report.pdf --explain
```

stderr 回放样例：

```text
-- engine chain (3 attempts, 412ms total) --
1. pdfium         skipped   dylib not found
2. pdf            used      388ms
3. mineru         not reached
trust: self_contained
```

`skipped` 表示探测过但未选中；`used` 表示执行并产出结果；`not reached` 表示从未尝试。耗时为整数毫秒（向下取整）。不带 `--explain` 时，输出与普通转换逐字节一致。

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
cargo build --release --features mcp   # 产出 alch（mcp 子命令随 feature 编入）
# 在 MCP 客户端配置中以 stdio 方式接入：
# { "command": "/path/to/alch", "args": ["mcp"] }
```

`alch mcp` 子命令经 sdforge（rmcp，stdio 传输——传输面无网络出口，convert 工具本身的网络能力随 `pdf-cloud` 特性）把 convert / doctor / formats 暴露为 MCP 工具，参数 schema 由端点签名生成；与 CLI 共享同一份端点定义（原独立 `alch-mcp` 二进制已折叠为本子命令）。

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

## 📊 支持格式矩阵

| 类别 | 扩展名 | 引擎（priority） | 需要的 feature |
| --- | --- | --- | --- |
| Word（OOXML） | docx | docx-deep(18) → docx(10) | `docx` |
| Word（旧版） | doc, rtf | legacy-office(15，LibreOffice) | `legacy-office` |
| OpenDocument 文本 | odt | odt-deep(18) | `odt` |
| 演示文稿 | pptx / ppt,pps,pot / odp | pptx-deep(18) → pptx(10) / legacy-ppt(15) / odp-deep(18) | `pptx` / `odp`（旧二进制 `legacy-office`） |
| 电子表格 | —（不支持） | 表格解析器已移除：calamine 的物化面（共享字符串、styles/theme、ODS 重复与文本展开）无法在不可信输入上被前置校验约束；xlsx/xls/xlsb/ods 不再支持 | — |
| PDF | pdf | pdfium(20) → pdf-extract(10) → MinerU(5) | `pdf` / `pdf-local` / `pdf-cloud` |
| 网页 / 电子书 | html, htm / epub | html(htmd) → html-deep(15) / epub | `html`（基础链 + 深链） / `epub` |
| 纯文本 / 代码 | txt, md, log, jsonl, ndjson, env, properties, rmd, qmd, mmd + ~45 代码扩展 | text（围栏化） | `text`（default 成员） |
| 表格文本 | csv, tsv, psv | csv（分隔符自探测） | `csv` |
| 标记语言 | tex, latex / org, rst, adoc, asciidoc, opml, mediawiki, wiki, textile | latex-source / pandoc-generic | `latex` / `pandoc` |
| 笔记本 | ipynb | ipynb | `ipynb` |
| 图片 | png, jpg, jpeg, gif, webp, bmp, tif, tiff, svg / heic, heif | image（Vision 可选） / heic（sips 或元数据） | `media` / `heic` |
| 音频 | wav, mp3, ogg, opus, flac, aac, m4a, wma | audio（ASR 可选） | `media`（+`audio` 转录） |
| 视频 | mp4, m4v, mov, avi, mkv, webm, flv, wmv | video（ffmpeg + ASR/Vision 可选） | `media`（+`audio` 转录） |
| 字幕 | srt, vtt, ass, ssa, lrc | subtitle | `media` |
| 邮件 / 网页存档 | eml / mht, mhtml | eml（头部/正文/附件清单；`html` 开启时正文走 html 链） | `email` |
| Outlook 邮件 | msg | msg→MIME 重建→eml 路径（cfb + 最小 MAPI 层） | `msg`（蕴含 `email`） |
| 帮助文档 | chm | chm（hhc 目录序 → html 链逐主题合并） | `chm`（蕴含 `html`） |
| 结构化数据 | parquet / sqlite, db, sqlite3 / bib | parquet / sqlite / bib | `parquet` / `sqlite` / `bibtex` |
| 归档 | zip, cbz / tar, tgz / gz, xz, zst / 7z | zip 递归 / tar 族递归 / 7z 递归 | `zip` / `tar`/`gzip`/`xz`/`zstd`/`sevenz` |

## 🎨 特性标志

### 📦 功能预设

| 预设 | 命令 | 适用场景 |
| --- | --- | --- |
| 最小化（default） | `cargo build` | 仅 CLI + 纯文本/代码围栏（`default = ["cli", "text"]`）；PDF/Office 等需显式开启格式 feature |
| 常用文档 | `cargo build --features pdf,docx,pptx,odt,odp,html,epub,csv,ipynb,zip` | 文档转换主路径 + zip 递归；按需追加其他格式 |
| 全功能 | `cargo build --features full` | 全部 30 个转换类 feature（含 MinerU 云端 PDF、本地 ASR、容器/数据/邮件/帮助文档） |

### 📋 功能矩阵

**格式族（16 个，一格式一 feature）**

| feature | 门控能力 | 说明 |
| --- | --- | --- |
| `text` | TextConverter | 纯文本与 ~45 扩展名代码围栏；**default 成员** |
| `csv` | CsvConverter | csv/tsv/psv 分隔符自探测 |
| `html` | HtmlConverter（htmd 基础链）+ HtmlDeepConverter（自研深链） | 两条 html 链同属一个 feature；不含 `html` 时 html/htm 转换完全不可用 |
| `pdf` | PdfConverter（lopdf 文本抽取） | PDF 本地兜底档；pdfium 高保真需另开 `pdf-local` |
| `docx` | DocxConverter + DocxDeepConverter | OOXML 深解析 + 基础解析（quick-xml + zip） |
| `pptx` | PptxConverter + PptxDeepConverter | 同上 |
| `odt` | OdtConverter | 同上 |
| `odp` | OdpConverter | 同上 |
| `epub` | EpubConverter | rbook + zip + htmd |
| `ipynb` | IpynbConverter | 仅核心 serde_json |
| `zip` | ZipConverter | zip/cbz 一层递归容器（嵌套 zip 显式不支持） |
| `latex` | LatexSourceConverter | pandoc 探测 + 围栏兜底 |
| `pandoc` | PandocGenericConverter | org/rst/asciidoc/opml/mediawiki 等通用格式 |
| `media` | image / audio / video / subtitle 四转换器 | 图片、音视频抽轨、字幕解析；转录后端另开 `audio` |
| `heic` | HeicConverter | sips 或元数据降级；独立 feature |
| `chm` | ChmConverter（libchm，纯 Rust 含 LZX） | CHM 编译帮助文档→md：`.hhc` 目录序优先（否则路径字典序），逐主题走 html 链；上限 512 主题/16MiB 单条/256MiB 累计，超限 `truncated` 标注；蕴含 `html` |

**后端 / 数据 / 容器 / 工具面**

| feature | 引入 | 说明 |
| --- | --- | --- |
| `pdf-local` | 本地 PDF 高保真引擎（pdfium-render） | 自动带入 `pdf`；运行时探测动态库，缺失降级到 pdf-extract。嵌入图提取走 lopdf 结构遍历（DCT/JPX 直通，样本重编码 PNG，不支持项显性计数） |
| `pdf-cloud` | MinerU 云端 PDF 引擎（reqwest/limiteron） | 端点经 `ALCHEMIZE_MINERU_URL` 配置后才注册 |
| `audio` | vecboost 转录门面 + Whisper 兼容 API | 自动带入 `media`；Qwen3-ASR ONNX + silero VAD，registry 依赖 vecboost |
| `legacy-office` | 旧版二进制 Office 引擎（外部 soffice） | doc/rtf→docx、ppt/pps/pot→pptx 后进深解析链；缺席自动降级 |
| `email` | 邮件引擎（mail-parser） | 头部/正文/附件清单；eml/mht/mhtml 三扩展，正文 HTML 在 `html` 开启时走 html 链 |
| `msg` | Outlook 邮件引擎（cfb + 最小 MAPI 层） | `.msg` OLE 复合文件→重建 MIME→复用 eml 渲染；8 类核心属性 + 附件清单；自动带入 `email` |
| `parquet`/`sqlite`/`bibtex` | 结构化数据引擎，按格式拆分的三个 feature | parquet 流式 10 万行截断、sqlite 只读、biblatex 条目 |
| `tar`/`gzip`/`xz`/`zstd`/`sevenz` | 归档容器族，按格式拆分的五个 feature | 递归转换，魔数检测内层格式；`.tgz` 需 `tar`+`gzip` 同时开启 |
| `cli` | CLI 骨架（sdforge + inklog） | **default 成员**；`[[bin]] alch` 的 required-features 为 `cli` |
| `mcp` | MCP 工具面（`alch mcp` 子命令，stdio 服务） | convert/doctor/formats 以 MCP 工具暴露；隐含 `cli`，不随 default |
| `full` | 全部 30 个转换类 feature（格式族 + 后端/数据/容器；不含 `cli`/`mcp`） | |

**default 行为（破坏性变更）**：`default = ["cli", "text"]` —— 默认构建只编译 CLI 骨架与纯文本转换。`alch convert x.pdf` / `x.docx` 在默认构建下没有候选引擎，返回 `unsupported format`（退出码 2）。需要哪些格式就显式开启对应 feature（如 `--features pdf,docx`）；`--features full` 恢复全部能力。

**依赖说明**：除核心 9 个（`serde`/`serde_json`/`thiserror`/`tracing`/`tokio`/`async-trait`/`anyhow`/`base64`/`sha2`）与 i18n 三件（`fluent-bundle`/`unic-langid`/`sys-locale`）外，全部依赖 `optional = true` 且只经 feature 的 `dep:` 引用激活；关闭 feature 时对应依赖不进入编译图。版本升级：`base64 0.23`、`sha2 0.11`、`image 0.25`、`pdfium-render 0.9`（绑定选 `pdfium_6295`，运行时 e2e 待更新 pdfium 二进制）、`zstd 0.14`、`icu_properties 2.4`；`rusqlite` 保持 `0.39`（`0.41` 与 limiteron→dbnexus→sqlx 的 `libsqlite3-sys <0.38` 硬冲突）。

## 💻 CLI 示例

```bash
# 管道友好：stdout 直出
alch convert paper.pdf | head -50

# 批量：shell 循环
for f in docs/*.docx; do alch convert "$f" -o "md/$(basename "${f%.docx}").md"; done

# 离线/合规环境：进程零网络出口
alch convert --no-remote secret.docx -o secret.md

# 不可信扩展名时强制指定格式
alch convert payload.bin --format pdf
```
