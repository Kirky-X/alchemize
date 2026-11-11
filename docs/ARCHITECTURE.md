# 🏗️ Alchemize 架构文档

Alchemize 是一个把任意文档炼成 Markdown 的 Rust 工具（CLI `alch` + 同名库），采用单 crate DDD 三层架构：`domain` 承载类型与规则（无网络/进程 IO），`application` 承载端口与优先级 fallback 注册表，`infrastructure` 承载各格式转换器与远端后端。本文档基于 `src/` 实际代码结构，介绍设计原则、模块划分、数据流以及安全与性能设计。

## 📋 目录

- [概述](#-概述)
- [设计原则](#-设计原则)
- [系统架构](#️-系统架构)
- [模块划分](#-模块划分)
- [数据流](#-数据流)
- [安全设计](#-安全设计)
- [性能设计](#-性能设计)
- [相关文档](#-相关文档)

## 🎯 概述

Alchemize 的目标是「一处输入，任意格式，产出 Markdown」。它内置一张按优先级组织的转换器注册表：同一扩展名的多个引擎按 `priority()` 降序形成 fallback 链，前序失败自动落到次优，`Unavailable` 的引擎跳过并记录原因；每个产出都如实标注质量层级（`api` / `local_full` / `local_fallback`）与实际引擎名。

三个入口共享同一套核心：

- **CLI `alch`**：`convert`（单文件转换）/ `doctor`（逐引擎可用性诊断）/ `formats`（支持扩展名清单）。
- **库 API**：`AlchemizeBuilder` 流式配置 → `Alchemize` 门面（`convert` / `convert_pref`）。
- **扩展点**：实现 `Converter` / `VisionProvider` / `TranscribeProvider` 端口即可注入自定义引擎。

## 📐 设计原则

1. **domain 无网络/进程 IO**：`src/domain/` 不发网络请求、不起子进程；文件读写仅有两处边界内例外——`Input::to_bytes` 读盘、`DirSink` 落盘（资产写出）。其余领域逻辑可脱离环境测试。
2. **端口-适配器分离**：`application/ports.rs` 定义 `Converter` / `VisionProvider` / `TranscribeProvider` / `ExternalTool` 四个 trait，基础设施层实现它们；注册表只依赖 trait，不依赖具体引擎。
3. **降级显性化**：引擎不可用（`Availability::Unavailable`）直接跳过并记录原因，不静默重试；全链失败返回 `AllBackendsFailed { ext, attempts }` 携带逐引擎失败明细。
4. **本地优先**：云端引擎（MinerU）优先级低于本地链，仅在本地链失败后尝试；`--no-remote` 可整体排除云端引擎。
5. **结果可观测**：成功结果在 `metadata.converter` 记录实际引擎名，`quality` 字段如实标注来源层级，禁止低优结果冒充高质量。

## 🏛️ 系统架构

```mermaid
flowchart TB
    subgraph CLI["src/main.rs（CLI：clap）"]
        CMD["alch convert · doctor · formats"]
    end
    subgraph APP["application/builder.rs — Alchemize 门面"]
        BUILDER["AlchemizeBuilder（流式配置）→ Alchemize"]
        REG["ConverterRegistry（优先级 fallback 注册表）"]
        ZIP["ZipConverter（门面层递归，共享注册表）"]
        BUILDER --> REG
        BUILDER -.-> ZIP
    end
    subgraph INFRA["infrastructure/"]
        ENG["engines.rs 引擎装配（feature → 引擎列表）"]
        DOC["document/：text csv html-deep html epub docx pptx ipynb<br/>latex_source pdf zip<br/>shared（detect_encoding 等共享原语）"]
        MEDIA["media/：image audio video subtitle subtitle_converter<br/>timeline dedup asr（vecboost · whisper_api）"]
        REMOTE["remote/mineru/：client factory agent precision<br/>backoff http_limit validated_url types wrapper"]
        TOOLS["tools/：pandoc（外部二进制探测）"]
    end
    subgraph DOM["domain/（无网络/进程 IO）"]
        INPUT["input：Input / InputFormat / detect_format"]
        RESULT["result：ConvertResult / ConversionQuality"]
        ASSET["asset：Asset / AssetSink（DirSink / InlineSink）"]
        ERROR["error：AlchemizeError"]
        LATEX["latex_validate（纯规则校验）"]
    end
    CMD --> BUILDER
    REG -- "Converter trait（ports.rs）" --> ENG
    ENG --> DOC
    ENG --> MEDIA
    ENG --> REMOTE
    ENG --> TOOLS
    INFRA --> DOM
```

## 🧩 模块划分

### 公开核心模块（无特性门控）

| 模块 | 职责 |
| --- | --- |
| `domain/feature_map.rs` | 扩展名→能力 feature 映射表（单一事实源）、编译期启用 feature 清单、缺失 feature 数据面（CLI 提示语由 `forge_app` 组装） |
| `domain/input.rs` | `Input`（`Path` / `Bytes{data, filename, mimetype, extension}` 两种载体）、`InputFormat` 枚举、`detect_format()`（魔数嗅探优先、扩展名映射兜底） |
| `domain/result.rs` | `ConvertResult`（markdown + metadata + content_type + quality + title + assets）、`ConversionQuality`、`ContentType`、`VideoProcessingMode` |
| `domain/asset.rs` | `Asset{filename, bytes, role}` 与 `AssetSink`（`DirSink` 写 `images/`、`InlineSink` 转 base64 data URL）——DirSink 落盘是 domain 层唯一写盘点 |
| `domain/error.rs` | `AlchemizeError` 15 个变体与 `SecurityViolationType` |
| `domain/latex_validate.rs` | LaTeX 数学定界符纯规则校验（无 IO） |
| `application/ports.rs` | 四个端口 trait + `Availability` + `Transcript`（VAD 段级转录，明确不含说话人归属）+ `which_program()` PATH 探测 |
| `application/registry.rs` | 优先级降序注册、fallback 链尝试、attempts 记录、`metadata.converter` 注入 |
| `application/builder.rs` | `AlchemizeBuilder` → `Alchemize` 门面（convert / convert_pref / availability / engines / asset_sink_for）；zip 门面特判 |
| `infrastructure/document/{gfm_render,shared}` | 自研 GFM 渲染器；共享原语（`shared::detect_encoding` 单独门控 `any(text,csv,pandoc,media)`），模块本身无外部依赖、保持无条件 |

### 特性门控模块

格式模块由对应格式 feature 门控；`office` 聚合模块的门为 `any(docx, pptx, odt, odp, legacy-office)`。

| feature | 模块 | 说明 |
| --- | --- | --- |
| `text` | `infrastructure/document/text.rs` | 纯文本 + ~45 扩展名代码围栏（default 成员） |
| `csv` | `infrastructure/document/csv.rs` | 分隔符自探测 |
| `html` | `infrastructure/document/html.rs` + `html_deep.rs` | htmd 基础链 + 自研深链（priority 15 压过 html(10)）；`--extract-main`/`--base-url` |
| `pdf` | `infrastructure/document/pdf.rs` | lopdf 兜底档文本抽取 + TOC 剔除/结构启发 |
| `docx`/`pptx`/`odt`/`odp` | `infrastructure/document/office/` | OOXML/ODF 深解析与基础解析（模块门 `any(docx,pptx,odt,odp,legacy-office)`） |
| `epub` | `infrastructure/document/epub.rs` | rbook + zip + htmd |
| `ipynb` | `infrastructure/document/ipynb.rs` | 笔记本单元格转 markdown |
| `zip` | `infrastructure/document/zip.rs`（门面层接线） | zip/cbz 递归容器 |
| `latex`/`pandoc` | `document/latex_source.rs`、`document/pandoc_generic.rs`、`tools/pandoc.rs`（模块门 `any(latex,pandoc)`） | 外部 pandoc 探测 + 围栏兜底；通用标记语言 |
| `media` | `infrastructure/media/`（image/audio/video/subtitle/timeline/dedup） | 图片、ffmpeg 抽轨、字幕原生解析、帧去重 |
| `heic` | `infrastructure/media/heic.rs` | sips 或元数据降级（独立 feature） |
| `pdf-local` | `infrastructure/document/pdfium.rs`、`pdf_images.rs` | pdfium 高保真（自动带入 `pdf`）+ 嵌入图提取 |
| `pdf-cloud` | `infrastructure/remote/mineru/` | 云端 PDF：`client`/`factory`（按文件大小选 agent/precision 客户端）、`backoff`（limiteron 轮询退避）、`http_limit`（响应体积分级上限）、`validated_url`（SSRF 校验）、`wrapper`（Converter 适配，priority 5 链尾） |
| `audio` | `infrastructure/media/asr/` | `vecboost.rs`（自研 transcribe 门面：Qwen3-ASR ONNX 三件套 + silero VAD，path 依赖兄弟仓）与 `whisper_api.rs`（Whisper 兼容云端后端，仅显式注入激活） |
| `legacy-office` | `document/office/legacy_office.rs`、`tools/libreoffice.rs` | 旧版二进制 Office（外部 soffice） |
| `tar`/`gzip`/`xz`/`zstd`/`sevenz` | `infrastructure/containers/` | 容器族；RegistrySlot 弱引用回填 |
| `parquet`/`sqlite`/`bibtex` | `infrastructure/data/` | parquet / sqlite / biblatex |
| `email` | `infrastructure/document/eml.rs` | 邮件（mail-parser） |

feature 关闭的引擎根本不出现在 `engines.rs` 装配列表中（`engines.rs` 的每条注册语句与转换器定义处使用同一 feature 门）；`mineru` 还需配置端点（`ALCHEMIZE_MINERU_URL` 或库 API `with_mineru`）才注册。default 为 `["cli", "text"]`，因此默认构建只有文本转换器。

### crate 组织

alchemize 为**单 crate**（`[package]` + `[lib]` + `[[bin]] alch`），无 workspace 子成员、无子 crate。与兄弟仓的关系是 path 依赖：`vecboost`（feature `audio` 下）——发布顺序约束为 vecboost 先于 alchemize 发到 crates.io。

## 🌊 数据流

### 转换主流程

```mermaid
flowchart TD
    IN["输入文件"] --> FMT["detect_format / can_convert 过滤"]
    FMT --> CHAIN["候选引擎链<br/>priority 降序 · no-remote 过滤 · prefer 提前"]
    CHAIN --> TRY{"逐个尝试<br/>Unavailable 跳过 → attempts 记录"}
    TRY -- "首个成功" --> RES["ConvertResult<br/>metadata.converter 注入实际引擎名"]
    RES --> ASSET["资产分流<br/>AssetMode::Dir → images/ 目录 · Inline → base64 data URL"]
    ASSET --> OUT["markdown → stdout 或 -o 文件<br/>stderr 摘要行：converted X → Y · quality · engine"]
    TRY -- "无候选" --> E2["UnsupportedFormat<br/>（退出码 2）"]
    TRY -- "全链失败" --> E3["AllBackendsFailed · attempts 明细<br/>（退出码 3，stderr 列明细）"]
```

失败终态两条：无候选 → `UnsupportedFormat`（退出码 2）；全链失败 → `AllBackendsFailed{ext, attempts}`（退出码 3，stderr 列明细）。

### 降级链数据流

以 PDF 为例（MinerU 已配置时）：

| 顺序 | 引擎 | priority | quality |
| --- | --- | --- | --- |
| 1 | `pdf`（pdf-extract + TOC 剔除/结构启发） | 10 | `local_fallback` |
| 2 | `mineru`（云端，仅在本地链失败后尝试） | 5 | `api` |

前序引擎返回 `Err` 自动尝试次优；`Unavailable` 引擎直接跳过（原因记入 attempts）；`--engine NAME` 精确名优先、无精确命中时按前缀匹配（大小写不敏感），把命中的引擎提到队首，未命中静默回落。`quality` 由产出引擎如实标注，与尝试顺序解耦。

### zip 递归容器数据流

zip 在门面层特判处理：`ZipConverter` 与主注册表共享同一引擎集（`Arc<ConverterRegistry>` clone），压缩包内普通文件按扩展名走各自引擎；zip 自身**不注册进注册表**，嵌套 zip 直接报不支持——无界自递归在结构上不可能发生。

tar/tgz/gz/xz/zst/7z 走注册表内的容器转换器：它们需要同一个已装配的注册表来转换内层文件，因此构造时只拿一个弱引用槽（`RegistrySlot`），由 `AlchemizeBuilder::build` 在注册表包进 `Arc` 后回填——强引用会形成 Arc 环。嵌套深度按「每次转换链」用 tokio task-local 计数（并发转换互不占用），超过 `MAX_DEPTH=3` 的内层条目降级为附件。调用方自建注册表时若未回填槽，条目降级为附件并记 `metadata.registry_wired=false`，不会让整个压缩包失败。zip 门面有独立的 1 GiB 解压聚合上限，不占容器深度层（嵌套 zip 显式不支持）。

## 🔒 安全设计

四道防线（完整模型见[安全文档](SECURITY.md)）：

1. **输入不可信**：文档输入按不可信构造处理，ZIP 容器（docx/pptx/epub/zip）经声明式依赖解析，解析失败返回结构化错误而非 panic。
2. **SSRF 防护**：MinerU 文件下载 URL 必须通过 `ValidatedFileUrl` 校验（HTTPS-only、拒环回/私网/CGNAT/链路本地、IPv4 映射 IPv6 同分类、域名白名单、签名要求）。
3. **响应限量**：远端响应按用途分级上限（64KB/8KB/50MB/50MB，预分配 8MB），超限返回 `ResponseTooLarge`。
4. **凭据脱敏**：`MineruConfig::api_token` 在 `Debug`/`Display`/serde 三通路全部脱敏（手工 `Debug` impl + 单测钉住）；凭据来自环境变量或 `MineruConfig::api_token`，仅发往通过 `validate_base_url` 的端点。

## ⚡ 性能设计

设计级取舍（本仓暂无 criterion 基准数据，不提供性能数字）：

- **多线程 runtime**：CLI（`main.rs` 显式构建 multi-thread runtime）与 `alch-mcp`（`#[tokio::main]` 默认 multi-thread，MCP 包装器内部 `block_in_place` 要求如此）均为多线程；转换以单文件为粒度，无跨任务共享状态竞争。
- **async-trait 端口**：远端与转录路径全异步，轮询退避经 limiteron `delay_for_attempt`，不忙等。
- **无全局健康缓存**：availability 每次转换即时探测（单次转换内一致），换取无过期窗错误路由。
- **内存有界**：远端响应经分级限量读取器（预分配上限 8MB）；zip 拒绝嵌套递归防无界深度。
- **降级而非失败**：本地链先行省去云端往返；依赖缺失（视觉/转录）降级为元数据 markdown，不阻塞整链。

## 📚 相关文档

- [📘 API 参考](API_REFERENCE.md) — 全部公开 API、CLI 接口与环境变量
- [📖 用户指南](USER_GUIDE.md) — 从安装到进阶的完整使用教程
- [❓ FAQ](FAQ.md) — 常见问题解答
- [🔒 安全文档](SECURITY.md) — 信任边界、SSRF 防护、漏洞报告
- [🤝 贡献指南](CONTRIBUTING.md) — 开发环境、测试与提交约定
- [📋 更新日志](CHANGELOG.md) — 版本变更记录
- 能力域规格真源：`specmark/specs/`（fallback-chain / document-conversion / media-conversion / cli-output / domain-ports / docs-suite / license-headers）
