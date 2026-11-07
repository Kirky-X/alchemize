# 🏗️ Alchemize 架构文档

Alchemize 是一个把任意文档炼成 Markdown 的 Rust 工具（CLI `alch` + 同名库），采用单 crate DDD 三层架构：`domain` 承载类型与规则（无网络/进程 IO），`application` 承载端口与优先级 fallback 注册表，`infrastructure` 承载各格式转换器与远端后端。本文档基于 `src/` 实际代码结构，介绍设计原则、模块划分、fallback 链机制与数据流。

## 📋 目录

- [设计原则](#设计原则)
- [系统架构](#系统架构)
- [模块划分](#模块划分)
- [核心抽象：端口与注册表](#核心抽象端口与注册表)
- [fallback 链与质量分层](#fallback-链与质量分层)
- [数据流](#数据流)
- [关键设计决策](#关键设计决策)
- [测试与质量](#测试与质量)

## 设计原则

1. **domain 无网络/进程 IO**：`src/domain/` 不发网络请求、不起子进程；文件读写仅有两处边界内例外——`Input::to_bytes` 读盘、`DirSink` 落盘（资产写出）。其余领域逻辑可脱离环境测试。
2. **端口-适配器分离**：`application/ports.rs` 定义 `Converter`/`VisionProvider`/`TranscribeProvider`/`ExternalTool` 四个 trait，基础设施层实现它们；注册表只依赖 trait，不依赖具体引擎。
3. **降级显性化**：引擎不可用（`Availability::Unavailable`）直接跳过并记录原因，不静默重试；全链失败返回 `AllBackendsFailed { ext, attempts }` 携带逐引擎失败明细（`src/application/registry.rs:103-146`）。
4. **本地优先**：云端引擎（MinerU）优先级低于本地链，仅在本地链失败后尝试（`src/infrastructure/remote/mineru/wrapper.rs:284-290`）；`--no-remote` 可整体排除云端引擎。
5. **结果可观测**：成功结果在 `metadata.converter` 记录实际引擎名，`quality` 字段如实标注来源层级，禁止低优结果冒充高质量。

## 系统架构

```
┌─────────────────────────────────────────────────────────┐
│  src/main.rs（CLI：clap）                                │
│  alch convert | doctor | formats                         │
└──────────────────────────┬──────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────┐
│  application/builder.rs — Alchemize 门面                 │
│  AlchemizeBuilder（流式配置）→ Alchemize                  │
│  ├─ ConverterRegistry（优先级 fallback 注册表）           │
│  └─ ZipConverter（门面层递归，共享注册表）                 │
└──────────────────────────┬──────────────────────────────┘
                           │ Converter trait（ports.rs）
┌──────────────────────────▼──────────────────────────────┐
│  infrastructure/                                         │
│  ├─ engines.rs 引擎装配（feature→引擎列表）                │
│  ├─ document/  text csv html epub docx pptx xlsx ipynb   │
│  │             latex_source pdf anydoc anydoc_render zip │
│  │             shared（detect_encoding 等共享原语）        │
│  ├─ media/     image audio video subtitle timeline dedup │
│  │             subtitle_converter asr/{vecboost, whisper}│
│  ├─ remote/    mineru/{client, factory, agent,           │
│  │             precision, backoff, http_limit,           │
│  │             validated_url, types, wrapper}            │
│  └─ tools/     pandoc（外部二进制探测）                    │
└──────────────────────────┬──────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────┐
│  domain/  input（Input/InputFormat/detect_format）        │
│           result（ConvertResult/ConversionQuality）       │
│           asset（Asset/AssetSink：DirSink/InlineSink）    │
│           error（AlchemizeError）                         │
│           latex_validate（纯规则校验）                     │
└─────────────────────────────────────────────────────────┘
```

## 模块划分

### domain（领域层，零 IO）

| 文件 | 职责 |
| --- | --- |
| `input.rs` | `Input`（`Path` / `Bytes{data, filename, mimetype, extension}` 两种载体）、`InputFormat` 枚举、`detect_format()`（扩展名映射 + WEBP/WAVE/AVI 魔数嗅探） |
| `result.rs` | `ConvertResult`（markdown + metadata + content_type + quality + title + assets）、`ConversionQuality`（`Api`/`LocalFull`/`LocalFallback`，序列化为 `api`/`local_full`/`local_fallback`）、`ContentType`、`VideoProcessingMode` |
| `asset.rs` | `Asset{filename, bytes, role}` 与 `AssetSink`（`DirSink` 写 `images/` 目录、`InlineSink` 转 base64 data URL）——**边界内例外**：DirSink 落盘是 domain 层唯一的写盘点 |
| `error.rs` | `AlchemizeError` 15 个变体（格式/后端/IO/PDF/ZIP/XML/视频/音频/视觉/MinerU/安全/体积等）与 `SecurityViolationType` |
| `latex_validate.rs` | LaTeX 数学定界符纯规则校验（无 IO） |
| `error.rs` | `AlchemizeError` 15 个变体（格式/后端/IO/PDF/ZIP/XML/视频/音频/视觉/MinerU/安全/体积等）与 `SecurityViolationType` |
| `latex_validate.rs` | LaTeX 数学定界符纯规则校验（无 IO） |

### application（应用层）

| 文件 | 职责 |
| --- | --- |
| `ports.rs` | 四个端口 trait + `Availability` + `Transcript`（VAD 段级转录，明确不含说话人归属）+ `which_program()` PATH 探测 |
| `registry.rs` | `ConverterRegistry`：注册时按 `priority()` 降序插入；转换时过滤 `can_convert` 与 no-remote，按序尝试，Unavailable 跳过并记入 attempts，成功注入 `metadata.converter` |
| `builder.rs` | `AlchemizeBuilder` 流式配置（MinerU/视觉/转录/视频模式/资产模式/云端排除）→ `Alchemize` 门面（convert / convert_pref / availability / engines / asset_sink_for）；zip 在门面层处理 |

### infrastructure（基础设施层）

| 子模块 | 职责 |
| --- | --- |
| `document/` | 各文档格式转换器：`text`（纯文本 + ~45 扩展名代码围栏）、`csv`、`html`、`epub`、`docx`/`pptx`/`xlsx`（原生 XML 解析）、`latex_source`（pandoc 探测 + 围栏兜底）、`pdf`（pdf-extract 本地文本 + TOC 剔除/结构启发）、`anydoc`（feature `anydoc`，office 全格式含旧版二进制格式）、`zip`（递归容器） |
| `media/` | `image`（VisionProvider 或元数据降级）、`audio`/`video`（ffmpeg 抽轨 → 转录链）、`subtitle`（srt/vtt/ass/ssa 原生解析）、`timeline`、`dedup`（视频帧去重）、`asr/`（vecboost 本地 ONNX 转录 + Whisper 兼容 API） |
| `remote/mineru/` | 云端 PDF 后端：`client`/`factory`（按文件大小选 agent/precision 客户端）、`backoff`（轮询退避，limiteron delay_for_attempt）、`http_limit`（响应体积上限）、`validated_url`（SSRF 校验）、`wrapper`（Converter 适配） |
| `tools/` | `pandoc` 外部二进制探测（构建时缺席、运行时探测） |

## 核心抽象：端口与注册表

`Converter` trait（`application/ports.rs`）的五个决策点：

| 方法 | 默认值 | 作用 |
| --- | --- | --- |
| `supported_extensions()` | 必须实现 | 小写无点扩展名表 |
| `can_convert(&Input)` | 扩展名匹配 | 是否进入候选链 |
| `priority()` | 10 | 数值大者先试 |
| `name()` | `"unknown"` | `--engine` 选择名与 `metadata.converter` 记录名 |
| `availability()` | `Available` | 运行时自述（缺 ffmpeg/模型/端点时返回 Unavailable + 原因） |
| `is_remote()` | `false` | `--no-remote` 时从候选链剔除 |

## fallback 链与质量分层

注册表对同一扩展名的多引擎按 `priority()` **降序**尝试（`registry.rs`）：

**PDF 示例**（feature `anydoc` 开、MinerU 已配置时）：

| 顺序 | 引擎 | priority | quality |
| --- | --- | --- | --- |
| 1 | `anydoc` | 15 | `local_full` |
| 2 | `pdf`（pdf-extract + TOC 剔除/结构启发） | 10 | `local_fallback` |
| 3 | `mineru`（云端，仅在本地链失败后尝试） | 5 | `api` |

**office 格式示例**（docx）：`anydoc`(15) → `docx` 原生 XML 解析(10)。

规则：

- 前序引擎返回 `Err` 自动尝试次优；`Unavailable` 引擎直接跳过（原因记入 attempts，不消耗转换尝试）。
- `--engine NAME` 把名称匹配（精确或前缀，大小写不敏感）的引擎提到队首；名称未命中静默回落 priority 路由，不报错。
- `quality` 由产出引擎如实标注（`Api`/`LocalFull`/`LocalFallback`），与尝试顺序解耦——MinerU 排链尾但产出仍是 `api` 级。
- `zip` 是门面层特例：与主注册表共享引擎集做递归转换，但自身不注册进注册表，嵌套 zip 直接报不支持（防无界自递归，`builder.rs:70-78`）。

## 数据流

```
输入文件
  │ detect_format / can_convert 过滤
  ▼
候选引擎链（priority 降序，no-remote 过滤，prefer 提前）
  │ 逐个尝试（Unavailable 跳过 → attempts 记录）
  ▼
首个成功结果 ConvertResult
  │ metadata.converter 注入实际引擎名
  ▼
资产分流：AssetMode::Dir → images/ 目录 / Inline → base64 data URL
  ▼
markdown → stdout 或 -o 文件（stderr 摘要行：converted X → Y [quality] (engine: N)）
```

失败终态两条：无候选 → `UnsupportedFormat`（退出码 2）；全链失败 → `AllBackendsFailed{ext, attempts}`（退出码 3，stderr 列明细）。

## 关键设计决策

1. **zip 出注册表**：zip 转换需要递归调用其他引擎，若自身也是注册表成员会造成无界自递归；放在门面层并共享最终注册表，嵌套 zip 显式报不支持。
2. **MinerU 云端置尾**：云端调用有成本与延迟，本地链（anydoc/pdf-extract）先行；MinerU 产出 `api` 级质量，仅在本地全失败时成为兜底高保真路径（`wrapper.rs:284-290` 注释明确此意图）。
3. **availability 自述制**：`Converter::availability()` 默认 `Available`——媒体引擎的依赖缺失（视觉/转录/ffmpeg）不在注册期探测，而是转换时降级或报错（如 vecboost provider 对模型缺失自述 `Unavailable`，audio 引擎转译为 `AudioUnavailable`）。doctor 聚合的是注册期可用性；注册表不做全局健康缓存（每次转换即时探测）。
4. **转录不虚构说话人**：`Transcript` 只含 VAD 段与文本，`DISCLAIMER` 常量随输出携带——VAD 无说话人身份，归属信息宁缺毋假。
5. **密钥防泄漏**：`MineruConfig::api_token` 在 `Debug`/`Display`/serde 序列化三处全部脱敏（`skip_serializing` + `<redacted>`，builder.rs 单测钉住）。

## 测试与质量

- 单 crate 双档测试：default（不含可选后端）与 `--features full`；full 档 396 例、0 失败。
- 集成测试四套：`tests/cli.rs`（CLI e2e）、`tests/golden.rs`（金样输出）、`tests/real_files.rs` + `tests/fixtures/real/`（真实文件套件 11 fixtures）、`tests/coverage_boost.rs`。
- 覆盖率门禁 ≥90%（当前 90.49%）。
- 规格驱动：`specmark/changes/migrate-mnemis-parser/specs/` 保存五大能力域的可验证规格（cli-output / document-conversion / domain-ports / fallback-chain / media-conversion）。
