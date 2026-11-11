# 更新日志

alchemize 所有显著变更记录于此文件。

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 破坏性变更

- **default 收窄为 `["cli", "text"]`**：默认构建只编译 CLI 骨架与纯文本/代码围栏转换；PDF/docx/html 等格式的转换器与依赖全部按 feature 门控。未启用对应 feature 时 `alch convert x.pdf` 没有候选引擎，返回 `unsupported format`（退出码 2）。需要显式 `--features pdf,docx,...`，或 `--features full` 恢复全量能力。`[[bin]] alch` 同步新增 `required-features = ["cli"]`。
- **新增 15 个格式 feature**（此前这些转换器无条件编译，或 `html` 只含深链）：`text`/`csv`/`html`/`pdf`/`docx`/`pptx`/`odt`/`odp`/`epub`/`ipynb`/`zip`/`latex`/`pandoc`/`media`/`heic`。`html` 现在同时含 htmd 基础链与自研深链，无 `html` feature 时 html 转换完全不可用。既有 feature 调整：`pdf-local = ["pdf", ...]`、`audio = ["media", ...]`、`legacy-office` 增加 quick-xml/zip/tempfile、媒体族统一为 `media`、`heic` 独立。
- **依赖全部 optional 化**：除核心 9 个（`serde`/`serde_json`/`thiserror`/`tracing`/`tokio`/`async-trait`/`anyhow`/`base64`/`sha2`）外，全部依赖 `optional = true` 且只经 feature 的 `dep:` 引用激活；声明统一为 `default-features = false` + 显式 features。
- **版本升级**：`base64 0.23`、`sha2 0.11`、`image 0.25`、`pdfium-render 0.9（`scaled_font_size` 替代 `font_size`，旋转/缩放文本的抽取结果可能与 0.5 略有差异）`（bindings 选 `pdfium_6295`；运行时 e2e 待更新 pdfium 二进制）、`zstd 0.14`、`icu_properties 2.4`；`rusqlite` 保持 `0.39`（`0.41` 与 limiteron→dbnexus→sqlx 的 `libsqlite3-sys <0.38` 硬冲突）；其余 `cargo update` 到最新。
- **Cargo feature 重命名为能力名并按格式拆分**：8 个 feature 调整——6 个重命名 + 2 个按格式拆分；`full` 伞面成员同步为新 14 名集合；不保留旧名别名，旧 `--features` 组合需按新名更新。

| 旧 feature | 新 feature |
| --- | --- |
| `html-deep` | `html` |
| `pdfium` | `pdf-local` |
| `mineru` | `pdf-cloud` |
| `asr` | `audio` |
| `libreoffice` | `legacy-office` |
| `eml` | `email` |
| `containers`（拆分） | `tar` / `gzip` / `xz` / `zstd` / `sevenz`（`.tgz` 需 `tar`+`gzip`） |
| `data`（拆分） | `parquet` / `sqlite` / `bibtex` |

- `full` 伞面成员更新为：`pdf-cloud`+`pdf-local`+`audio`+`email`+`legacy-office`+`html`+`tar`+`gzip`+`xz`+`zstd`+`sevenz`+`parquet`+`sqlite`+`bibtex`（14 个；后续依赖门控变更再扩为 28 个转换类 feature 全集）。
- 拆分收益口径：单格式构建相比旧捆绑减少 6–9 个依赖节点（容器族）与 10–31 个（数据族），冷编耗时降 2%–17%；其中 `sqlite`/`bibtex` 收益明显（15%–17%），`parquet` 约 3%，`gzip` 依赖树收益为零（flate2 本就在树内，收益在编译面/二进制面）。
- `.tar.gz` 组合拼写的降级语义：仅开 `gzip` 时，`.tar.gz` 经 gz 单流解压后将内层 tar 字节降级为附件（内容保留）；`.tgz` 需 `tar`+`gzip` 同时开启才可转换，未齐备时报不支持。

- **SQLite Path 输入改为私有临时副本**：所有输入在打开前 staging 到 `tempfile`（0600、随机名、独占创建）的私有副本，隔断攻击者同目录 `-wal`/`-shm` 注入（RUSTSEC-2026-0459 向量）。语义影响：Path 输入不再读取伴生 `-wal` 中未 checkpoint 的事务（此前就地打开会读到）；staging 需要 TMPDIR 容纳与数据库等大的临时副本。

### 新增

- **PDF 解压炸弹防护**：vendor `lopdf 0.42.0` 分叉（`third_party/lopdf`，`[patch.crates-io]` 接入，MIT 随包携带）给 FlateDecode/LZWDecode 流解压加每流 64 MiB 输出上限（对齐容器族 `MAX_ENTRY_BYTES`），单一收口覆盖 parse 阶段 ObjStm/XRefStm 与 pdf-extract 内容流/ICC 流全部路径；`decompress_predictor` 增加 DecodeParms 几何护栏；分叉线程本位违规标记供宿主在 pdf-extract 吞错后显性拒绝（`AlchemizeError::Pdf`），转换器与 CLI 端到端验证 100 MB 膨胀炸弹被拒（峰值 RSS 79 MB）。
- **PDF 兜底档切换 lopdf 0.47 直连并退役分叉**：移除 `pdf-extract` 依赖与 `third_party/lopdf` vendor 分叉，供应链回归 crates.io 单一上游 `lopdf 0.47`；兜底档文本抽取切到 lopdf 自带 `extract_text_chunks_with_limit`（每页 64 MiB）并新增 256 MiB 全文档累计文本预算（page-farm DoS 守卫），图片路径改 `decompressed_content_with_limit`（每流 64 MiB）；解压错误由 lopdf 原生传播、类型级映射为 `AlchemizeError::Pdf`（替代线程本位标记），抽取段包 `spawn_blocking` 对齐同库惯例。差分基准（Apple EULA / arxiv 论文 / fork fixture 三语料）判定兜底档质量可接受。
- **MCP 工具面**（feature `mcp`）：`alch-mcp` 二进制经 sdforge（rmcp，stdio）把 convert/doctor/formats 暴露为 MCP 工具。
- **html-deep 自研引擎**（feature `html`）：只读 DOM → Markdown 迭代状态机——空输出防呆、迭代式 DOM 防栈击穿、诚实降级；可选主内容提取（dom_smoothie，`--extract-main`）与链接绝对化（`--base-url`）；含 `<math` 标签的页面自动路由 htmd 保真，`$..$` 文本 TeX 跨度原样保留。convert 端点新增 `--base-url`/`--extract-main`。
- 文档套件（七篇专题文档）与全部源文件版权头（Apache-2.0 SPDX）。
- 文档骨架与 confers 对齐：七篇逐篇映射 confers 章节序列（概述→设计原则→系统架构→模块划分→数据流→安全设计→性能设计→相关文档等，逐篇见「📋 目录」）。
- 文档导航节：README.md「📚 文档」表格与 README_EN.md「Docs」表格。

### 修复

- `MineruConfig` 手工实现 `Debug`：修复 derived Debug 明文打印 `api_token` 的泄密面（Display/serde 原已脱敏），单测补钉 Debug 通路断言。
- README.md 对外失实修正：PDF fallback 链序（原写 MinerU 链首，实际本地链在前、MinerU 链尾）；退出码清单补 `1`（其他错误）。
- gz/xz/zst 包装内层转换结果时保留内层 `assets` 与 `metadata`：原实现只取 markdown，内层 docx/odt 抽出的图片被丢弃、`images/...` 引用悬空。
- CSV 参差行改为扩宽表格（合成 `column_N` 表头）而非截断；容器与 zip 统一过滤 `._*`/`__MACOSX` 元数据条目；`MinerUHttpConfig` 手工 `Debug` 脱敏。
- 围栏注入防护统一：文本/代码/ipynb/eml/pandoc/LaTeX/GFM 渲染与容器、zip 路径全部改用 `fence_for`（按正文最长反引号串加长、兼容 lone CR/CRLF、容忍 ≤3 空格缩进），避免不可信内容提前闭合代码块。

### 变更

- **移除表格解析器**：xlsx/ods 支持与 `calamine` 依赖整体移除——其物化面（共享字符串逐格克隆、styles/theme、ODS 重复与 `text:c` 展开）无法在不可信输入上被前置校验约束。xlsx/xls/xlsb/ods 一律返回 `UnsupportedFormat`。
- **容器转换器构造面收敛**：tar/tgz/gz/xz/zst/7z 只保留 `with_registry_slot(slot)`；嵌套深度改为按转换链 task-local 计数（并发转换互不占用）；槽未回填时条目降级为附件并记 `metadata.registry_wired=false`（原为硬报错）。
- `default_converters` 增加容器注册表槽参数；`AlchemizeBuilder::build()` 自动回填。
- **CLI 改为 sdforge 封装**：convert/doctor/formats 改写为 `#[forge]` 端点（`src/forge_app.rs`），`alch` 经 CliBuilder + 自定义 runner 驱动；退出码 0/1/2/3、位置参数、stdout/stderr 形态、`-o`/`--no-remote`/`--format` 契约全部保留（tests/cli.rs 零改动全绿）。MSRV 1.91 → 1.97.1（sdforge 要求）；直接依赖 clap 移除（sdforge 自带）。
- 文档表述对齐代码事实：domain 层 IO 例外（`Input::to_bytes` 读盘、`DirSink` 落盘）、doctor/availability 语义（未注册引擎不出现、媒体依赖缺失走转换时降级）、模块树补齐 engines/shared/subtitle_converter。

### 移除

- 表格解析器 `xlsx`/`ods`（`calamine` 依赖）、`XlsxConverter` 及其 fixtures/基准行。
- 容器转换器 `new(registry)` / `with_arc(registry)` 公开构造（测试路径保留为 `#[cfg(test)]`）。

## [0.1.0] - 2026-11-07

首个功能完整版本：单 crate DDD 三层架构，任意文档 → Markdown。

### 新增

- **转换引擎注册表**：`Converter` 端口 + 优先级 fallback 链；`metadata.converter` 记录实际引擎，`quality` 如实标注 `api`/`local_full`/`local_fallback` 三级；全链失败返回逐引擎 attempts 明细。
- **文档格式**：pdf（pdf-extract 本地文本 + TOC 剔除/结构启发）、docx/pptx/xlsx/xls/ods（原生 XML + calamine）、odt/odp/rtf、html/htm、epub、csv/tsv（分隔符自探测）、txt/text/log/md 与 ~45 扩展名代码围栏、ipynb、tex/latex（pandoc 探测 + 围栏兜底）、zip 递归容器（嵌套 zip 显式不支持）。
- **anydoc 后端**（feature `anydoc`）：office 全格式含旧版二进制 Office（doc/ppt/…），priority 15 压过原生解析；图片按文档序经自研 GFM 渲染器导出。
- **MinerU 云端 PDF**（feature `pdf-cloud`）：Agent/Precision 双客户端按文件大小选择、limiteron 轮询退避、响应体积分级上限、SSRF 校验（scheme/环回/私网/CGNAT/链路本地/域名白名单/签名）；`ALCHEMIZE_MINERU_URL` 配置端点，本地优先、云端兜底（priority 5 链尾）。
- **媒体转换**：图片（VisionProvider 注入或元数据降级）、音频/视频（ffmpeg 抽轨 → 转录链）、字幕（srt/vtt/ass/ssa 原生解析）、视频帧去重、视频处理模式（Gpu/AudioOnly/Disabled）。
- **ASR 本地转录**（feature `audio`）：vecboost 门面（Qwen3-ASR ONNX 三件套 + silero VAD），模型 bundle 缺失时如实报 Unavailable 不自动联网；输出 VAD 段级文本 + 时间戳，显式声明非说话人识别；另备 Whisper 兼容 API 后端（显式注入激活）。
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
