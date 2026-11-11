# 📘 Alchemize API 参考

本文档覆盖 alchemize 库公开 API 与 `alch` CLI 完整接口。所有签名与行为以 `src/` 源码为准（lib.rs 再导出清单、main.rs clap 定义、各实现文件）。

## 📋 目录

- [概述](#-概述)
- [核心 API](#-核心-api)
- [配置/扩展 API](#-配置扩展-api)
- [错误类型](#-错误类型)
- [特性门控 API](#-特性门控-api)
- [使用示例](#-使用示例)
- [最佳实践](#-最佳实践)
- [相关文档](#-相关文档)

## 🎯 概述

### API 设计原则

- **门面优先**：`AlchemizeBuilder` → `Alchemize` 是唯一推荐入口；`ConverterRegistry` 公开但面向自定义装配场景。
- **显式降级**：依赖缺失不是 `Err` 而是 `Unavailable{reason}` 自述或元数据降级产出；全链失败才返回携带逐引擎 attempts 的错误。
- **可观测结果**：`ConvertResult` 携带 `metadata.converter`（实际引擎名）与 `quality`（来源层级），调用方无需猜测产出来自哪条链。
- **trait 驱动扩展**：自定义引擎/视觉/转录能力全部经 `application::ports` 的 trait 注入，无全局注册表、无反射。

### 📦 特性说明

根再导出（`src/lib.rs`）：

```rust
pub use application::builder::{Alchemize, AlchemizeBuilder, MineruConfig};
pub use application::registry::ConverterRegistry;
pub use domain::error::AlchemizeError;
pub use domain::input::{Input, InputFormat, detect_format};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

| feature | 门控 API | 默认 |
| --- | --- | --- |
| `pdf-cloud` | MinerU 引擎注册 + `MineruConfig` 装配 | 关 |
| `audio` | vecboost 转录门面 + Whisper 兼容后端 | 关 |
| `full` | 以上全部 14 个转换能力 feature 全集合（详见 Cargo.toml） | 关 |

## 🧱 核心 API

### 转换门面与构建器

`AlchemizeBuilder::new()` → 流式配置 → `build()` 得到 `Alchemize`：

| Builder 方法 | 默认 | 说明 |
| --- | --- | --- |
| `with_mineru(MineruConfig)` | 无 | 配置后且 feature `pdf-cloud` 开启时注册 `mineru` 引擎（priority 5，链尾） |
| `with_vision_provider(Arc<dyn VisionProvider>)` | 无 | 图片描述/OCR；不注入则图片转元数据 markdown，无内置兜底 |
| `with_transcribe_provider(Arc<dyn TranscribeProvider>)` | 无 | 音频引擎转录；feature `audio` 下默认用 vecboost 门面 |
| `with_video_mode(VideoProcessingMode)` | `AudioOnly` | `Gpu` / `AudioOnly`（抽轨转音频链）/ `Disabled` |
| `with_asset_mode(AssetMode)` | `Dir` | 资产落盘模式 |
| `with_no_remote()` | false | `is_remote()==true` 的引擎不进入候选链 |
| `with_max_file_size(usize)` | 104857600 | 输入大小上限（字节）：facade 在任何分发前拦截超限输入；ZIP 转换器用同一上限作为嵌套条目上限。tar/tgz/7z 的内层条目不受此值约束（单条目 64 MiB、整包 256 MiB）；gz/xz/zst 单流解压上限固定 64 MiB。MinerU precision 客户端自身另有 200 MB 上限 |
| `build()` | — | 组装默认引擎集并返回 `Alchemize` |

`Alchemize` 方法：

| 方法 | 签名 | 说明 |
| --- | --- | --- |
| `convert` | `async fn convert(&self, input: &Input) -> Result<ConvertResult, AlchemizeError>` | 按 fallback 链转换 |
| `convert_pref` | `async fn convert_pref(&self, input: &Input, prefer: Option<&str>) -> Result<ConvertResult, AlchemizeError>` | 指定偏好引擎（名称精确或前缀匹配，大小写不敏感；未命中静默回落 priority 路由） |
| `availability` | `fn availability(&self) -> Vec<(String, Vec<String>, Availability)>` | 每引擎 `(名称, 扩展名表, 可用性)`，`doctor` 数据源；zip 单列一行 |
| `engines` | `fn engines(&self) -> Vec<&'static str>` | 已注册引擎名（zip 不在内） |
| `engine_count` | `fn engine_count(&self) -> usize` | 注册表成员数（zip 不计） |
| `is_no_remote` | `fn is_no_remote(&self) -> bool` | 构建时是否排除了云端引擎 |
| `asset_sink_for` | `fn asset_sink_for(&self, output_root: &Path) -> Box<dyn AssetSink>` | 按资产模式构造 sink（Dir → `<root>/images/`） |

`MineruConfig`（feature `pdf-cloud`）：

```rust
pub struct MineruConfig {
    pub base_url: String,            // 默认 "http://localhost:8888"（本地 MinerU 服务）
    pub agent_base_url: String,      // 默认 "https://mineru.net"（云端 Agent API）
    pub precision_base_url: String,  // 默认 "https://mineru.net"（云端 Precision API）
    pub api_token: Option<String>,   // 云端 bearer token；serde skip_serializing
    pub timeout_secs: u64,           // 默认 300
}
```

`api_token` 在 `Debug`（手工 impl）、`Display`、serde 序列化三条通路全部脱敏（单测 `mineru_config_display_redacts_token` 钉住）。

### 注册表

`ConverterRegistry`（面向自定义装配，门面内部亦使用）：

| 方法 | 说明 |
| --- | --- |
| `new()` / `new_no_remote()` | 空注册表（后者排除云端引擎） |
| `register(Box<dyn Converter>)` | 按 priority 降序插入 |
| `convert(&Input)` / `convert_pref(&Input, Option<&str>)` | 候选链尝试，语义同门面 |
| `len()` / `is_empty()` | 成员数 |
| `availability()` | doctor 数据源 |
| `converters()` | `&dyn Converter` 迭代器 |

`infrastructure::engines::default_converters(...)`（面向自定义装配；签名随 feature 变化）：

```rust
pub fn default_converters(
    mineru: &Option<MineruConfig>,
    vision: Option<Arc<dyn VisionProvider>>,
    transcribe: Option<Arc<dyn TranscribeProvider>>,
    video_mode: VideoProcessingMode,
    html_deep: &Option<HtmlDeepEngineConfig>,
    container_registry_slot: &Arc<OnceLock<Weak<ConverterRegistry>>>,
) -> Vec<Box<dyn Converter>>
```

装配完成后必须执行 `container_registry_slot.set(Arc::downgrade(&registry))`；否则 tar/tgz/gz/xz/zst/7z 的内层条目降级为附件（`metadata.registry_wired=false`）。`AlchemizeBuilder::build()` 已包含这一步。返回列表中不包含任何电子表格引擎（xlsx/xls/xlsb/ods 不支持）。

### 输入与格式检测

```rust
pub enum Input {
    Path(PathBuf),
    Bytes { data: Vec<u8>, filename: String, mimetype: Option<String>, extension: Option<String> },
}
```

`Bytes` 载体中 `mimetype`/`extension` 显式覆盖优先于文件名推断。`detect_format(&Input) -> Option<InputFormat>`：**魔数嗅探优先**（仅 `Input::Bytes` 载体；覆盖 `%PDF`、ZIP 族的 ODF/docx/pptx/xlsx/epub 细分、GIF/PNG/JPEG、RIFF（WEBP/WAV/AVI）、Ogg/FLAC 等，实现见 `domain/input.rs` 的 `detect_magic_bytes`），嗅探不出再按扩展名映射兜底；`Input::Path` 无字节可嗅探，直接走扩展名。无法识别返回 `None`。`InputFormat` 为 22 成员枚举（含 `Unknown`）。

### 结果与资产

```rust
pub struct ConvertResult {
    pub markdown: String,
    pub metadata: serde_json::Value,   // 注册表注入 metadata.converter = 实际引擎名
    pub content_type: ContentType,     // Document | Image | Audio | Video
    pub quality: ConversionQuality,
    pub title: Option<String>,         // ≤20 token，取自内容前缀或 "Untitled"
    pub assets: Vec<Asset>,            // 按文档序抽取的二进制资产
}
```

`ConversionQuality`：`Api`（远端高保真）｜ `LocalFull`（默认，本地全功能）｜ `LocalFallback`（本地降级）。serde 序列化为 `api`/`local_full`/`local_fallback`，质量由产出引擎如实标注、与尝试顺序解耦。

`Asset { filename: String, bytes: Vec<u8>, role: AssetRole }`；`AssetSink` 两个实现：`DirSink`（写 `<输出同目录>/images/`，重名加序号前缀）与 `InlineSink`（base64 data URL 内联）。

## 🔌 配置/扩展 API

`application::ports` 是实现自定义引擎的扩展点。

### Converter Trait

```rust
#[async_trait]
pub trait Converter: Send + Sync {
    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError>;
    fn supported_extensions(&self) -> &[&str];
    fn can_convert(&self, input: &Input) -> bool { /* 默认：扩展名匹配 */ }
    fn priority(&self) -> u8 { 10 }
    fn name(&self) -> &'static str { "unknown" }
    fn availability(&self) -> Availability { Availability::Available }
    fn is_remote(&self) -> bool { false }
}
```

| 方法 | 默认值 | 作用 |
| --- | --- | --- |
| `supported_extensions()` | 必须实现 | 小写无点扩展名表 |
| `can_convert(&Input)` | 扩展名匹配 | 是否进入候选链 |
| `priority()` | 10 | 数值大者先试 |
| `name()` | `"unknown"` | `--engine` 选择名与 `metadata.converter` 记录名 |
| `availability()` | `Available` | 运行时自述（缺依赖时 `Unavailable{reason}`，doctor 与 attempts 展示原因） |
| `is_remote()` | `false` | `--no-remote` 时从候选链剔除 |

注意 availability 语义：能降级产出的引擎应恒返回 `Available`（如 latex-source 无 pandoc 出围栏兜底），把「能否产出」与「产出质量」分开——否则注册表会把它当不可用跳过。

### VisionProvider Trait

```rust
#[async_trait]
pub trait VisionProvider: Send + Sync {
    async fn describe(&self, image_data: &[u8], detail: DetailLevel) -> anyhow::Result<String>;
    async fn ocr(&self, image_data: &[u8]) -> anyhow::Result<String>;
    fn name(&self) -> &'static str;
}
```

图片引擎的描述与 OCR 来源；不注入时图片转换降级为元数据 markdown（含尺寸/哈希），alchemize 不内置任何视觉后端。

### TranscribeProvider Trait

```rust
#[async_trait]
pub trait TranscribeProvider: Send + Sync {
    async fn transcribe(&self, audio_wav16k: &[u8]) -> Result<Transcript, AlchemizeError>;
    fn availability(&self) -> Availability;
    fn name(&self) -> &'static str;
}
```

输入为 16kHz 单声道 WAV 字节。`Transcript { language, duration, segments: Vec<TranscriptSegment{start_sec, end_sec, text}> }`；`full_text()` 拼接段文本；`DISCLAIMER` 常量声明「分段由 VAD 产生，非说话人识别」。

### ExternalTool Trait

```rust
pub trait ExternalTool: Send + Sync {
    fn program(&self) -> &str;
    fn probe(&self) -> Availability { /* PATH 探测，无 shell 调用 */ }
}
pub fn which_program(program: &str) -> bool;
```

外部二进制（pandoc/ffmpeg）统一走 PATH 探测：构建时缺席、运行时发现。

## 🚨 错误类型

### `AlchemizeError`

`domain::error::AlchemizeError`（thiserror）主要变体：

| 变体 | 触发场景 |
| --- | --- |
| `UnsupportedFormat(String)` | 无候选引擎（CLI 退出码 2） |
| `NoConverterAccepted` | 无转换器接受输入 |
| `AllBackendsFailed { ext, attempts }` | 全链失败，`attempts: Vec<(引擎名, 失败原因)>`（CLI 退出码 3） |
| `FileRead { path, source }` | 输入读取失败 |
| `Pdf(String)` / `Zip(String)` / `Xml(String)` | 本地解析失败 |
| `VideoUnavailable` / `AudioUnavailable` / `VisionUnavailable` | 依赖缺失（ffmpeg/转录/视觉） |
| `MineruUnavailable` / `MineruConversion` | 云端服务不可达/转换失败 |
| `SecurityViolation { kind, message }` | SSRF 校验拒绝 |
| `ResponseTooLarge { actual, limit }` | HTTP 响应体超限 |
| `Other(String)` | 其他转换器错误 |

### `SecurityViolationType`

URL 校验拒绝的六类原因：`InvalidScheme`（非 HTTPS）、`LoopbackAddress`、`PrivateAddress`（RFC1918）、`LinkLocalAddress`、`HostnameNotAllowed`（白名单外）、`MissingSignature`（预签名缺失）。

### 相关类型定义

`Availability::Available` ｜ `Availability::Unavailable { reason: String }`（equality 可比较，doctor 与 attempts 展示 reason）；`VideoProcessingMode::{Gpu, AudioOnly, Disabled}`（默认 `AudioOnly`）；`AssetMode::{Dir, Inline}`（默认 `Dir`）。

## 🚪 特性门控 API

### MinerU（`pdf-cloud` 特性）

- 注册条件：`with_mineru(MineruConfig)` 且 feature 编译开启；CLI 侧由 `ALCHEMIZE_MINERU_URL` 触发。
- 装配产物：`mineru` 引擎（仅 `pdf` 扩展名），priority 5 链尾，产出 `quality=api`。
- 内部契约：agent/precision 双客户端按文件大小选择（factory）；轮询退避经 limiteron；响应体积分级限量；下载 URL 过 `ValidatedFileUrl` SSRF 校验。
- 环境变量：`ALCHEMIZE_MINERU_URL`（端点）、`MINERU_API_TOKEN`（precision bearer token）。

### ASR（`audio` 特性）

- 装配产物：转录提供方接入 audio/video 引擎。默认 provider 为 vecboost 门面（Qwen3-ASR ONNX 三件套 + silero VAD，本地推理）；Whisper 兼容云端后端仅在显式注入配置时激活。
- 模型探测：`ALCHEMIZE_TRANSCRIBE_MODELS` → 缺省 `~/.cache/alchemize/models/transcribe/`（布局 `qwen3-conv_frontend.onnx`、`qwen3-encoder.int8.onnx`、`qwen3-decoder.int8.onnx`、`silero_vad.onnx`、`tokenizer/`）；`ORT_DYLIB_PATH` 指向 onnxruntime 动态库。缺失时 provider 自述 `Unavailable` 并附获取指引，不自动联网。
- 测试专用：`ALCHEMIZE_ASR_TEST_WAV`（集成测试输入路径）。

### HtmlDeep（`html` 特性）

- 装配产物：`html-deep` 引擎（exts html/htm，priority 15 压过 html/htmd）——本仓自研迭代状态机（`src/infrastructure/document/html_deep/engine/`，Apache-2.0）→ htmd 兜底转换链。
- 链语义：自研引擎主路径 →（主输出空且有可见文本）htmd 兜底（复用既有 html 引擎，含 math 保真）→ 双失败 `Err(Other)` 诚实上报；纯脚本/空壳页的空输出是正确抑制。
- math 路由：含 `<math` 标签的输入整体直通既有 html 引擎（引擎会将 MathML 净化为行内 HTML 而非 LaTeX；检测条件与 html.rs preprocess_math 触发条件一致）。
- 可选语义：`base_url`（转换前链接绝对化，`#`/`mailto:`/`data:` 等前缀保留）与 `extract_main`（dom_smoothie 主内容提取，降级纯文本分支行首转义；失败/空诚实回退全页）。默认均关——保真优先。
- 选项面：builder `with_html_deep(HtmlDeepEngineConfig{base_url, extract_main})`；CLI/MCP convert 参数 `--base-url` / `--extract-main`。

### MCP（`mcp` 特性）

- 装配产物：`alch-mcp` 二进制——`sdforge::mcp::serve_stdio(build())` 承载 convert / doctor / formats 三个 MCP 工具（`#[forge(tool_name)]` 注册，参数 schema 由端点签名生成），stdio 传输——传输面无网络出口（convert 工具本身的网络能力随 `pdf-cloud` 特性）。
- CLI 同源：三个端点同时以 `cli = true` 注册进 `alch`（CliBuilder + 自定义 runner，退出码 0/1/2/3 契约见主 spec cli-output）。
- 注意：根级内置全局 `--format text|json`（sdforge 输出格式）；convert 的扩展名覆盖参数名为 `input_format`，CLI 侧由 runner 在 convert 语境把 `--format` 归一为 `--input_format`。

## 💡 使用示例

### 基础转换

```rust
use alchemize::{AlchemizeBuilder, Input};

#[tokio::main]
async fn main() -> Result<(), alchemize::AlchemizeError> {
    let alch = AlchemizeBuilder::new().build();
    let result = alch.convert(&Input::Path("paper.pdf".into())).await?;
    println!("engine: {}", result.metadata["converter"]);
    println!("quality: {}", result.quality);
    println!("{}", result.markdown);
    Ok(())
}
```

### 引擎偏好

```rust
// 把 pdf 引擎提到队首；名称未命中静默回落默认链
let result = alch.convert_pref(&input, Some("pdf")).await?;
```

### 资产模式

```rust
use alchemize::application::builder::AssetMode;

let alch = AlchemizeBuilder::new()
    .with_asset_mode(AssetMode::Inline) // base64 data URL 内联
    .build();
// Dir 模式（默认）经 asset_sink_for 写 <输出>/images/
```

### 自定义视觉提供方

```rust
use std::sync::Arc;
use alchemize::application::ports::VisionProvider;

struct MyVlm;
#[async_trait::async_trait]
impl VisionProvider for MyVlm {
    async fn describe(&self, image: &[u8], _detail: alchemize::domain::result::DetailLevel) -> anyhow::Result<String> {
        Ok("a photo".into())
    }
    async fn ocr(&self, image: &[u8]) -> anyhow::Result<String> {
        Ok("text".into())
    }
    fn name(&self) -> &'static str { "my-vlm" }
}

let alch = AlchemizeBuilder::new()
    .with_vision_provider(Arc::new(MyVlm))
    .build();
```

### 自定义转录提供方

```rust
use alchemize::application::ports::{TranscribeProvider, Transcript, TranscriptSegment};

struct MyAsr;
#[async_trait::async_trait]
impl TranscribeProvider for MyAsr {
    async fn transcribe(&self, wav16k: &[u8]) -> Result<Transcript, alchemize::AlchemizeError> {
        Ok(Transcript {
            language: Some("zh".into()),
            duration: Some(1.5),
            segments: vec![TranscriptSegment { start_sec: 0.0, end_sec: 1.5, text: "你好".into() }],
        })
    }
    fn availability(&self) -> alchemize::application::ports::Availability {
        alchemize::application::ports::Availability::Available
    }
    fn name(&self) -> &'static str { "my-asr" }
}
```

## 🌟 最佳实践

### 错误处理

- 匹配三类终态分别处理：`UnsupportedFormat`（提示用户看 `formats`）、`AllBackendsFailed{attempts}`（把 attempts 逐行呈现给用户）、其余（按消息上报）。
- 不吞 `Err`、不用默认值顶替失败产出；降级产出（`quality=local_fallback`）是 `Ok`，由调用方按 quality 决定是否可接受。

### 引擎选择

- 优先信任默认链（本地优先、云端兜底已是既定策略）；确需强制时用 `convert_pref`/`--engine`（前缀匹配即可），并接受未命中静默回落。
- 自定义引擎的 `priority()`：接入现有链用 <15 的值；`availability()` 区分「能否产出」与「产出质量」，能降级的引擎恒 `Available`。

### 安全注意事项

- 凭据来自环境变量（`MINERU_API_TOKEN`）或 `MineruConfig::api_token`；三通路脱敏，不写入日志。
- 打印/序列化 `MineruConfig` 安全（三通路脱敏），但自定义类型含敏感字段时应效仿手工 `Debug` + 单测钉住的做法。
- 不可信环境开启 `with_no_remote()`，进程零网络出口。

### 常见问题排查

- `metadata` 为 `Null`：直接经 `Converter` 实现产出（未过注册表）才会出现；门面路径必注入 `converter`。
- 引擎没出现在 `availability()`：feature 未编译或（MinerU）未配置端点——根本未注册。
- zip 输入报 `UnsupportedFormat("zip")`：仅当直接调用 `default_converters` 自建注册表（未走 `AlchemizeBuilder`）时出现——zip 转换器在门面层接线，不在注册表内（避免嵌套 zip 自递归）。
- 容器（tar/tgz/gz/xz/zst/7z）内层文件全部降级为附件、`metadata.registry_wired` 为 `false`：直接调用 `default_converters` 后没有回填容器注册表槽。装配完成后须执行 `slot.set(Arc::downgrade(&registry))`；`AlchemizeBuilder::build()` 已自动完成。

## 📚 相关文档

- [🏗️ 架构文档](ARCHITECTURE.md) — 设计原则、模块划分与数据流
- [📖 用户指南](USER_GUIDE.md) — 从安装到进阶的完整使用教程
- [❓ FAQ](FAQ.md) — 常见问题解答
- [🔒 安全文档](SECURITY.md) — 信任边界、SSRF 防护、漏洞报告
- [🤝 贡献指南](CONTRIBUTING.md) — 开发环境、测试与提交约定
- [📋 更新日志](CHANGELOG.md) — 版本变更记录
