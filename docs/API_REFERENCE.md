# 📘 Alchemize API 参考

本文档覆盖 alchemize 库公开 API 与 `alch` CLI 完整接口。所有签名与行为以 `src/` 源码为准（lib.rs 再导出清单、main.rs clap 定义、各实现文件）。

## 📋 目录

- [库 API](#库-api)
  - [Alchemize 门面](#alchemize-门面)
  - [AlchemizeBuilder](#alchemizebuilder)
  - [MineruConfig](#mineruconfig)
  - [Input 与格式检测](#input-与格式检测)
  - [ConvertResult 与质量分层](#convertresult-与质量分层)
  - [资产与 AssetSink](#资产与-assetsink)
  - [ConverterRegistry](#converterregistry)
  - [端口 trait](#端口-trait)
  - [错误类型 AlchemizeError](#错误类型-alchemizeerror)
- [CLI `alch`](#cli-alch)
- [环境变量](#环境变量)
- [特性开关](#特性开关)

## 库 API

`alchemize` crate 根再导出（`src/lib.rs`）：

```rust
pub use application::builder::{Alchemize, AlchemizeBuilder, MineruConfig};
pub use application::registry::ConverterRegistry;
pub use domain::error::AlchemizeError;
pub use domain::input::{Input, InputFormat, detect_format};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

### Alchemize 门面

构建产物 `Alchemize`（`application/builder.rs`）——转换入口与诊断聚合：

| 方法 | 签名 | 说明 |
| --- | --- | --- |
| `convert` | `async fn convert(&self, input: &Input) -> Result<ConvertResult, AlchemizeError>` | 按 fallback 链转换 |
| `convert_pref` | `async fn convert_pref(&self, input: &Input, prefer: Option<&str>) -> Result<ConvertResult, AlchemizeError>` | 指定偏好引擎（名称精确或前缀匹配，大小写不敏感；未命中静默回落 priority 路由） |
| `availability` | `fn availability(&self) -> Vec<(String, Vec<String>, Availability)>` | 每引擎 `(名称, 扩展名表, 可用性)`，`doctor` 数据源；zip 单列一行 |
| `engines` | `fn engines(&self) -> Vec<&'static str>` | 已注册引擎名（zip 不在内） |
| `engine_count` | `fn engine_count(&self) -> usize` | 注册表成员数（zip 不计） |
| `is_no_remote` | `fn is_no_remote(&self) -> bool` | 构建时是否排除了云端引擎 |
| `asset_sink_for` | `fn asset_sink_for(&self, output_root: &Path) -> Box<dyn AssetSink>` | 按资产模式构造 sink（Dir → `<root>/images/`） |

zip 特例：扩展名为 `zip` 的输入由门面层 `ZipConverter` 处理（共享主注册表递归转换）；`zip` 特性关闭时报 `UnsupportedFormat("zip")`。嵌套 zip 不支持。

### AlchemizeBuilder

```rust
let alch = AlchemizeBuilder::new()
    .with_asset_mode(AssetMode::Dir)          // 资产模式（默认 Dir）
    .with_no_remote()                          // 排除云端引擎
    .with_mineru(cfg)                          // MineruConfig（注册 MinerU 引擎）
    .with_vision_provider(Arc::new(my_vlm))    // 图片描述/OCR（None 则元数据降级）
    .with_transcribe_provider(Arc::new(my_asr))// 音视频转录（None 则元数据降级）
    .with_video_mode(VideoProcessingMode::AudioOnly) // Gpu | AudioOnly（默认） | Disabled
    .with_max_file_size(100 * 1024 * 1024)     // 见下方注意
    .build();
```

| 方法 | 默认 | 说明 |
| --- | --- | --- |
| `new()` | — | 100 MB 上限字段、无 MinerU、无视觉/转录提供方、`AudioOnly`、`Dir`、允许云端 |
| `with_mineru(MineruConfig)` | 无 | 配置后且 feature `mineru` 开启时注册 `mineru` 引擎（priority 5，链尾） |
| `with_vision_provider(Arc<dyn VisionProvider>)` | 无 | 图片引擎描述/OCR；不注入则图片转元数据 markdown，无内置兜底 |
| `with_transcribe_provider(Arc<dyn TranscribeProvider>)` | 无 | 音频引擎转录；feature `asr` 下默认用 vecboost 门面 |
| `with_video_mode(VideoProcessingMode)` | `AudioOnly` | `Gpu` / `AudioOnly`（抽轨转音频链）/ `Disabled` |
| `with_asset_mode(AssetMode)` | `Dir` | 资产落盘模式 |
| `with_no_remote()` | false | `is_remote()==true` 的引擎不进入候选链 |
| `with_max_file_size(usize)` | 104857600 | ⚠️ 当前引擎装配（`engines.rs`）未消费该值——限制未生效；MinerU precision 客户端自身有 200 MB 上限。预留接口，勿依赖 |
| `build()` | — | 组装默认引擎集并返回 `Alchemize` |

### MineruConfig

```rust
pub struct MineruConfig {
    pub base_url: String,            // 默认 "http://localhost:8888"（本地 MinerU 服务）
    pub agent_base_url: String,      // 默认 "https://mineru.net"（云端 Agent API）
    pub precision_base_url: String,  // 默认 "https://mineru.net"（云端 Precision API）
    pub api_token: Option<String>,   // 云端 bearer token；serde skip_serializing
    pub timeout_secs: u64,           // 默认 300
}
```

`api_token` 在 `Debug`、`Display`、serde 序列化三条通路全部脱敏（单测 `mineru_config_display_redacts_token` 钉住）。

### Input 与格式检测

```rust
pub enum Input {
    Path(PathBuf),
    Bytes { data: Vec<u8>, filename: String, mimetype: Option<String>, extension: Option<String> },
}
```

`Bytes` 载体中 `mimetype`/`extension` 显式覆盖优先于文件名推断。`detect_format(&Input) -> Option<InputFormat>`：先扩展名映射（`pdf`/`pptx`/`docx`/`xlsx`/`xls`/`odt|ods|odp`/`html|htm`/`epub`/`csv|tsv`/`zip`/`txt|text|log|md`/`ipynb`/图片/音频/视频族），另对 WEBP/WAVE/AVI 魔数嗅探兜底；无法识别返回 `None`。`InputFormat` 为 22 成员枚举（含 `Unknown`）。

### ConvertResult 与质量分层

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

`ConversionQuality`：`Api`（远端高保真，如 MinerU）｜ `LocalFull`（默认，本地全功能，如 anydoc）｜ `LocalFallback`（本地降级，如 pdf-extract 文本链）。serde 序列化为 `api`/`local_full`/`local_fallback`。质量由产出引擎如实标注，与尝试顺序无关。

### 资产与 AssetSink

`Asset { filename: String, bytes: Vec<u8>, role: AssetRole }`。`AssetSink` 两个实现：`DirSink`（写 `<输出同目录>/images/`，重名加序号前缀）与 `InlineSink`（base64 data URL 内联）。CLI 由 `--assets dir|inline` 选择。

### ConverterRegistry

也可绕过门面直接用注册表（`application::registry::ConverterRegistry`）：

| 方法 | 说明 |
| --- | --- |
| `new()` / `new_no_remote()` | 空注册表（后者排除云端引擎） |
| `register(Box<dyn Converter>)` | 按 priority 降序插入 |
| `convert(&Input)` / `convert_pref(&Input, Option<&str>)` | 候选链尝试，语义同门面 |
| `len()` / `is_empty()` | 成员数 |
| `availability()` | doctor 数据源 |
| `converters()` | `&dyn Converter` 迭代器 |

### 端口 trait

`application::ports`（实现自定义引擎的扩展点）：

- **`Converter`**（核心）：`convert(&self, Input) -> Result<ConvertResult, AlchemizeError>`、`supported_extensions() -> &[&str]`、`can_convert(&Input) -> bool`（默认扩展名匹配）、`priority() -> u8`（默认 10）、`name() -> &'static str`（默认 `"unknown"`）、`availability() -> Availability`（默认 `Available`）、`is_remote() -> bool`（默认 false）。
- **`VisionProvider`**：`describe(&[u8], DetailLevel) -> anyhow::Result<String>`、`ocr(&[u8]) -> anyhow::Result<String>`、`name()`。
- **`TranscribeProvider`**：`transcribe(&[u8] /*16kHz mono WAV*/) -> Result<Transcript, AlchemizeError>`、`availability()`、`name()`。`Transcript { language, duration, segments: Vec<TranscriptSegment{start_sec, end_sec, text}> }`，`full_text()` 拼接，`DISCLAIMER` 常量声明 VAD 非说话人识别。
- **`ExternalTool`**：`program()` + `probe()`（PATH 探测，无 shell 调用）；`which_program()` 公开辅助函数。
- **`Availability`**：`Available` ｜ `Unavailable { reason: String }`（doctor 与 attempts 展示原因）。

### 错误类型 AlchemizeError

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
| `SecurityViolation { kind, message }` | SSRF 校验拒绝（scheme/环回/私网/链路本地/域名白名单/签名缺失） |
| `ResponseTooLarge { actual, limit }` | HTTP 响应体超限 |
| `Other(String)` | 其他转换器错误 |

## CLI `alch`

```
alch convert <INPUT> [-o, --output <PATH>] [--engine <NAME>] [--assets <dir|inline>]
            [--no-remote] [--format <EXT>]
alch doctor
alch formats
```

| 子命令/参数 | 说明 |
| --- | --- |
| `convert <INPUT>` | 输入文件路径（必须是存在的文件） |
| `-o, --output <PATH>` | 输出文件；缺省写 stdout。dir 模式下资产写在输出同目录 `images/`，父目录自动创建 |
| `--engine <NAME>` | 偏好引擎名（见引擎名表）；未命中静默回落 |
| `--assets <dir\|inline>` | 资产模式，默认 `dir`；非法值报 `unknown asset mode '<v>' (dir\|inline)` |
| `--no-remote` | 排除云端引擎（MinerU） |
| `--format <EXT>` | 覆盖格式检测（无点扩展名，如 `pdf`），内部转 `Input::Bytes` 载体 |
| `doctor` | 每引擎一行：`<name> AVAILABLE <exts>` 或 `<name> UNAVAILABLE <reason>` |
| `formats` | 全部支持扩展名排序去重后空格分隔输出 |

**退出码**：`0` 成功（含降级产出）；`1` 其他错误（如输入不存在、写文件失败）；`2` 不支持格式；`3` 全链失败（stderr 逐行列出 `- <engine>: <reason>`）。

**引擎名**（`metadata.converter` 与 `--engine` 取值）：`text`、`csv`、`html`、`epub`、`docx`、`pptx`、`xlsx`、`ipynb`、`pdf`、`latex-source`、`image`、`audio`、`video`、`subtitle`、`anydoc`（feature）、`mineru`（feature + 配置）、`zip`（facade）。

## 环境变量

| 变量 | 作用域 | 说明 |
| --- | --- | --- |
| `ALCHEMIZE_MINERU_URL` | CLI/库 | 设置后注册 MinerU 引擎（本地/网关服务基址），未设置时链内无 MinerU |
| `MINERU_API_TOKEN` | precision 客户端 | 云端 Precision API bearer token（`MineruPrecisionClient::from_env`） |
| `ALCHEMIZE_TRANSCRIBE_MODELS` | ASR | 转录模型 bundle 目录；未设置时探测 `~/.cache/alchemize/models/transcribe/` |
| `ORT_DYLIB_PATH` | ASR | onnxruntime 动态库路径（非空才启用本地 ONNX 推理） |
| `ALCHEMIZE_ASR_TEST_WAV` | 仅测试 | ASR 集成测试的输入 wav 路径 |

## 特性开关

| feature | 引入 | 默认 |
| --- | --- | --- |
| `mineru` | MinerU 云端 PDF 引擎（reqwest/limiteron/futures） | 关 |
| `anydoc` | office 全格式后端（含旧版二进制 Office；priority 15 压过原生解析） | 关 |
| `asr` | vecboost 转录门面（Qwen3-ASR ONNX + silero VAD） | 关 |
| `full` | 以上全部 | 关 |

默认特性零外部服务依赖；引擎装配见 `src/infrastructure/engines.rs`（feature 关闭的引擎根本不出现在列表中）。
