# 📘 Alchemize API Reference

This document covers the public API of the alchemize library and the full `alch` CLI interface. All signatures and behaviors defer to the `src/` sources (the lib.rs re-export list, the main.rs clap definitions, and the individual implementation files).

## 📋 Table of Contents

- [Overview](#-overview)
- [Core API](#-core-api)
- [Configuration/Extension API](#-configurationextension-api)
- [Error Types](#-error-types)
- [Feature-Gated API](#-feature-gated-api)
- [Usage Examples](#-usage-examples)
- [Best Practices](#-best-practices)
- [Related Documents](#-related-documents)

## 🎯 Overview

### API Design Principles

- **Facade first**: `AlchemizeBuilder` → `Alchemize` is the only recommended entry point; `ConverterRegistry` is public but aimed at custom assembly scenarios.
- **Explicit degradation**: a missing dependency is not an `Err` but a self-describing `Unavailable{reason}` or a degraded output in metadata; only a full-chain failure returns an error carrying per-engine attempts.
- **Observable results**: `ConvertResult` carries `metadata.converter` (the actual engine name) and `quality` (the source tier), so callers never guess which chain produced the output.
- **Trait-driven extension**: custom engines/vision/transcription capabilities are all injected through traits in `application::ports` — no global registry, no reflection.

### 📦 Feature Notes

Root re-exports (`src/lib.rs`):

```rust
pub use application::builder::{Alchemize, AlchemizeBuilder, MineruConfig};
pub use application::registry::ConverterRegistry;
pub use domain::error::AlchemizeError;
pub use domain::feature_map::{enabled_features, features_for_extension, missing_features};
pub use domain::input::{Input, InputFormat, detect_format};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

| feature | Gated API | Default |
| --- | --- | --- |
| `text` | TextConverter (plain text/code fencing) | **on** |
| `csv` | CsvConverter | off |
| `html` | HtmlConverter (htmd base chain) + HtmlDeepConverter (`HtmlDeepEngineConfig`/`with_html_deep`) | off |
| `pdf` | PdfConverter (lopdf fallback tier) | off |
| `docx` / `pptx` / `odt` / `odp` | The corresponding OOXML/ODF deep and basic converters | off |
| `epub` | EpubConverter | off |
| `ipynb` | IpynbConverter | off |
| `zip` | ZipConverter (wired at the facade layer, not in the registry) | off |
| `latex` / `pandoc` | LatexSourceConverter / PandocGenericConverter | off |
| `media` | The four converters: image / audio / video / subtitle | off |
| `heic` | HeicConverter | off |
| `pdf-local` | PdfiumConverter (implies `pdf`) | off |
| `pdf-cloud` | MinerU engine registration + `MineruConfig` assembly | off |
| `audio` | vecboost transcription facade + Whisper-compatible backend (implies `media`) | off |
| `legacy-office` | LegacyOffice / LegacyPpt engines | off |
| `email` / `parquet` / `sqlite` / `bibtex` | The corresponding format engines | off |
| `tar` / `gzip` / `xz` / `zstd` / `sevenz` | The archive container family | off |
| `cli` | CLI skeleton (required-features of `[[bin]] alch`) | **on** |
| `mcp` | The `alch mcp` subcommand tool surface (implies `cli`) | off |
| `full` | The set of all 30 conversion features (format family + backends/data/containers; excludes `cli`/`mcp`) | off |

Additional (non-feature) exports: `domain::feature_map` provides the `EXTENSION_FEATURES` mapping table and the data surface `features_for_extension` / `missing_features` / `enabled_features`, used to answer "which capability features does this extension need" and "which ones does this build enable"; the CLI's `unsupported format: … requires the 'pdf' feature` hint is assembled from it.

`default = ["cli", "text"]`: the default build has only the text converter (+ CLI skeleton). `Alchemize::convert` returns `UnsupportedFormat` (an `Err`, CLI exit code 2) when the input format has no candidate engine — at the facade layer, a disabled format feature and a genuinely unsupported extension are the same terminal state.

## 🧱 Core API

### Conversion Facade and Builder

`AlchemizeBuilder::new()` → chained configuration → `build()` yields `Alchemize`:

| Builder method | Default | Description |
| --- | --- | --- |
| `with_mineru(MineruConfig)` | none | When configured and the `pdf-cloud` feature is on, registers the `mineru` engine (priority 5, chain tail) |
| `with_vision_provider(Arc<dyn VisionProvider>)` | none | Image description/OCR; without injection, images degrade to metadata markdown with no built-in fallback |
| `with_transcribe_provider(Arc<dyn TranscribeProvider>)` | none | Audio engine transcription; with feature `audio` the default is the vecboost facade |
| `with_video_mode(VideoProcessingMode)` | `AudioOnly` | `Gpu` / `AudioOnly` (extract track into the audio chain) / `Disabled` |
| `with_asset_mode(AssetMode)` | `Dir` | Asset writing mode |
| `with_no_remote()` | false | Engines with `is_remote()==true` are excluded from the candidate chain |
| `with_max_file_size(usize)` | 104857600 | Input size cap (bytes): the facade rejects oversized inputs before any dispatch; the ZIP converter uses the same cap as its nested-entry cap. Inner entries of tar/tgz/7z are not bound by this value (64 MiB per entry, 256 MiB per archive); gz/xz/zst single-stream decompression is fixed at 64 MiB. The MinerU precision client has its own 200 MB cap |
| `build()` | — | Assembles the default engine set and returns `Alchemize` |

`Alchemize` methods:

| Method | Signature | Description |
| --- | --- | --- |
| `convert` | `async fn convert(&self, input: &Input) -> Result<ConvertResult, AlchemizeError>` | Converts via the fallback chain; when the format feature is not enabled there is no candidate engine and it returns `UnsupportedFormat` |
| `convert_pref` | `async fn convert_pref(&self, input: &Input, prefer: Option<&str>) -> Result<ConvertResult, AlchemizeError>` | Prefers an engine (exact name or prefix match, case-insensitive; on miss it silently falls back to priority routing) |
| `availability` | `fn availability(&self) -> Vec<(String, Vec<String>, Availability)>` | Per engine `(name, extension table, availability)`; the data source for `doctor`; zip gets its own row |
| `engines` | `fn engines(&self) -> Vec<&'static str>` | Registered engine names (zip not included) |
| `engine_count` | `fn engine_count(&self) -> usize` | Registry member count (zip not counted) |
| `is_no_remote` | `fn is_no_remote(&self) -> bool` | Whether the build excluded cloud engines |
| `asset_sink_for` | `fn asset_sink_for(&self, output_root: &Path) -> Box<dyn AssetSink>` | Builds the sink per asset mode (Dir → `<root>/images/`) |

`MineruConfig` (feature `pdf-cloud`):

```rust
pub struct MineruConfig {
    pub base_url: String,            // default "http://localhost:8888" (local MinerU service)
    pub agent_base_url: String,      // default "https://mineru.net" (cloud Agent API)
    pub precision_base_url: String,  // default "https://mineru.net" (cloud Precision API)
    pub api_token: Option<String>,   // cloud bearer token; serde skip_serializing
    pub timeout_secs: u64,           // default 300
}
```

`api_token` is redacted on all three paths — `Debug` (hand-written impl), `Display`, and serde serialization (pinned by the `mineru_config_display_redacts_token` unit test).

### Registry

`ConverterRegistry` (for custom assembly; also used inside the facade):

| Method | Description |
| --- | --- |
| `new()` / `new_no_remote()` | Empty registry (the latter excludes cloud engines) |
| `register(Box<dyn Converter>)` | Inserts in descending priority order |
| `convert(&Input)` / `convert_pref(&Input, Option<&str>)` | Candidate-chain attempts, same semantics as the facade |
| `len()` / `is_empty()` | Member count |
| `availability()` | Data source for doctor |
| `converters()` | Iterator over `&dyn Converter` |

`infrastructure::engines::default_converters(...)` (for custom assembly; signature varies with features):

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

After assembly you must run `container_registry_slot.set(Arc::downgrade(&registry))`; otherwise the inner entries of tar/tgz/gz/xz/zst/7z degrade to attachments (`metadata.registry_wired=false`). `AlchemizeBuilder::build()` already does this step. The returned list contains no spreadsheet engines (xlsx/xls/xlsb/ods are unsupported).

### Input and Format Detection

```rust
pub enum Input {
    Path(PathBuf),
    Bytes { data: Vec<u8>, filename: String, mimetype: Option<String>, extension: Option<String> },
}
```

In the `Bytes` carrier, an explicit `mimetype`/`extension` override takes precedence over inference from the filename. `detect_format(&Input) -> Option<InputFormat>`: **magic-byte sniffing first** (only for the `Input::Bytes` carrier; covers `%PDF`, the ZIP-family ODF/docx/pptx/xlsx/epub subdivisions, GIF/PNG/JPEG, RIFF (WEBP/WAV/AVI), Ogg/FLAC, etc.; see `detect_magic_bytes` in `domain/input.rs`), falling back to the extension mapping when sniffing fails; `Input::Path` has no bytes to sniff and goes straight to the extension. Unrecognized input returns `None`. `InputFormat` is a 22-member enum (including `Unknown`).

### Results and Assets

```rust
pub struct ConvertResult {
    pub markdown: String,
    pub metadata: serde_json::Value,   // the registry injects metadata.converter = actual engine name
    pub content_type: ContentType,     // Document | Image | Audio | Video
    pub quality: ConversionQuality,
    pub title: Option<String>,         // ≤20 tokens, from the content prefix or "Untitled"
    pub assets: Vec<Asset>,            // binary assets extracted in document order
}
```

`ConversionQuality`: `Api` (remote high fidelity) | `LocalFull` (default, full local) | `LocalFallback` (degraded local). Serialized as `api`/`local_full`/`local_fallback`; quality is labeled honestly by the producing engine and is decoupled from attempt order.

`Asset { filename: String, bytes: Vec<u8>, role: AssetRole }`; the two `AssetSink` implementations: `DirSink` (writes `<output directory>/images/`, duplicate names get an index prefix) and `InlineSink` (base64 data URL inline).

## 🔌 Configuration/Extension API

`application::ports` is the extension point for implementing custom engines.

### Converter Trait

```rust
#[async_trait]
pub trait Converter: Send + Sync {
    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError>;
    fn supported_extensions(&self) -> &[&str];
    fn can_convert(&self, input: &Input) -> bool { /* default: extension match */ }
    fn priority(&self) -> u8 { 10 }
    fn name(&self) -> &'static str { "unknown" }
    fn availability(&self) -> Availability { Availability::Available }
    fn is_remote(&self) -> bool { false }
}
```

| Method | Default | Purpose |
| --- | --- | --- |
| `supported_extensions()` | must implement | Lowercase, dot-free extension table |
| `can_convert(&Input)` | extension match | Whether it enters the candidate chain |
| `priority()` | 10 | Higher values are tried first |
| `name()` | `"unknown"` | The name used by `--engine` selection and recorded in `metadata.converter` |
| `availability()` | `Available` | Runtime self-description (`Unavailable{reason}` when a dependency is missing; doctor and attempts show the reason) |
| `is_remote()` | `false` | Removed from the candidate chain under `--no-remote` |

Note the availability semantics: an engine that can degrade and still produce output should always return `Available` (e.g. latex-source without pandoc yields a fenced fallback), separating "can it produce" from "how good is the output" — otherwise the registry skips it as unavailable.

### VisionProvider Trait

```rust
#[async_trait]
pub trait VisionProvider: Send + Sync {
    async fn describe(&self, image_data: &[u8], detail: DetailLevel) -> anyhow::Result<String>;
    async fn ocr(&self, image_data: &[u8]) -> anyhow::Result<String>;
    fn name(&self) -> &'static str;
}
```

The source of description and OCR for the image engine; without injection, image conversion degrades to metadata markdown (dimensions/hash), and alchemize ships no built-in vision backend.

### TranscribeProvider Trait

```rust
#[async_trait]
pub trait TranscribeProvider: Send + Sync {
    async fn transcribe(&self, audio_wav16k: &[u8]) -> Result<Transcript, AlchemizeError>;
    fn availability(&self) -> Availability;
    fn name(&self) -> &'static str;
}
```

The input is 16kHz mono WAV bytes. `Transcript { language, duration, segments: Vec<TranscriptSegment{start_sec, end_sec, text}> }`; `full_text()` concatenates segment texts; the `DISCLAIMER` constant states "segments come from VAD, not speaker identification".

### ExternalTool Trait

```rust
pub trait ExternalTool: Send + Sync {
    fn program(&self) -> &str;
    fn probe(&self) -> Availability { /* PATH probe, no shell invocation */ }
}
pub fn which_program(program: &str) -> bool;
```

External binaries (pandoc/ffmpeg) all go through PATH probing: absent at build time, discovered at runtime.

## 🚨 Error Types

### `AlchemizeError`

Main variants of `domain::error::AlchemizeError` (thiserror):

| Variant | Trigger |
| --- | --- |
| `UnsupportedFormat(String)` | No candidate engine (CLI exit code 2) |
| `NoConverterAccepted` | No converter accepted the input |
| `AllBackendsFailed { ext, attempts }` | Entire chain failed, `attempts: Vec<(engine name, failure reason)>` (CLI exit code 3) |
| `FileRead { path, source }` | Input read failure |
| `Pdf(String)` / `Zip(String)` / `Xml(String)` | Local parsing failure |
| `VideoUnavailable` / `AudioUnavailable` / `VisionUnavailable` | Missing dependency (ffmpeg/transcription/vision) |
| `MineruUnavailable` / `MineruConversion` | Cloud service unreachable/conversion failed |
| `SecurityViolation { kind, message }` | SSRF validation rejected |
| `ResponseTooLarge { actual, limit }` | HTTP response body over the limit |
| `Other(String)` | Other converter errors |

### `SecurityViolationType`

The six rejection reasons for URL validation: `InvalidScheme` (not HTTPS), `LoopbackAddress`, `PrivateAddress` (RFC1918), `LinkLocalAddress`, `HostnameNotAllowed` (outside the allowlist), `MissingSignature` (missing presignature).

### Related Type Definitions

`Availability::Available` | `Availability::Unavailable { reason: String }` (comparable equality; doctor and attempts display the reason); `VideoProcessingMode::{Gpu, AudioOnly, Disabled}` (default `AudioOnly`); `AssetMode::{Dir, Inline}` (default `Dir`).

## 🚪 Feature-Gated API

### Format features (15)

One feature per format: `text` (default member), `csv`, `html`, `pdf`, `docx`, `pptx`, `odt`, `odp`, `epub`, `ipynb`, `zip`, `latex`, `pandoc`, `media`, `heic`. Disabled features do not compile their converters; the engine list returned by `default_converters` changes with the enabled features (the function signature does not); `AlchemizeBuilder::build()` assembles only the compiled engines. Therefore calling `convert` for a non-enabled format yields `UnsupportedFormat` — the same terminal state as "the extension is genuinely unsupported"; callers use the `formats`/feature surface to tell which one it is.

### MinerU (the `pdf-cloud` feature)

- Registration conditions: `with_mineru(MineruConfig)` and the feature compiled in; on the CLI side it is triggered by `ALCHEMIZE_MINERU_URL`.
- Assembly result: the `mineru` engine (only the `pdf` extension), priority 5 chain tail, producing `quality=api`.
- Internal contract: agent/precision dual clients selected by file size (factory); polling backoff via limiteron; tiered response size limits; download URLs pass `ValidatedFileUrl` SSRF validation.
- Environment variables: `ALCHEMIZE_MINERU_URL` (endpoint), `MINERU_API_TOKEN` (precision bearer token).

### ASR (the `audio` feature)

- Assembly result: the transcription provider is wired into the audio/video engines. The default provider is the vecboost facade (the Qwen3-ASR ONNX trio + silero VAD, local inference); the Whisper-compatible cloud backend is activated only via explicit injection of configuration.
- Model probing: `ALCHEMIZE_TRANSCRIBE_MODELS` → default `~/.cache/alchemize/models/transcribe/` (layout `qwen3-conv_frontend.onnx`, `qwen3-encoder.int8.onnx`, `qwen3-decoder.int8.onnx`, `silero_vad.onnx`, `tokenizer/`); `ORT_DYLIB_PATH` points at the onnxruntime dynamic library. When missing, the provider self-reports `Unavailable` with acquisition guidance and never downloads automatically.
- Test-only: `ALCHEMIZE_ASR_TEST_WAV` (integration test input path).

### HtmlDeep (the `html` feature)

- Assembly result: the `html-deep` engine (exts html/htm, priority 15 outranking html/htmd) — this repo's own iterative state machine (`src/infrastructure/document/html_deep/engine/`, Apache-2.0) → htmd fallback chain.
- Chain semantics: the native engine's main path → (main output empty with visible text present) htmd fallback (reusing the existing html engine, math fidelity included) → on double failure `Err(Other)` reports honestly; the empty output for pure-script/empty-shell pages is correct suppression.
- Math routing: input containing a `<math` tag bypasses entirely to the existing html engine (the engine sanitizes MathML into inline HTML rather than LaTeX; the detection condition matches the html.rs preprocess_math trigger).
- Optional semantics: `base_url` (absolutize links before conversion, `#`/`mailto:`/`data:` prefixes preserved) and `extract_main` (dom_smoothie main-content extraction, with the degraded plain-text branch escaped at line starts; failure/empty honestly falls back to the full page). Both off by default — fidelity first.
- Options surface: builder `with_html_deep(HtmlDeepEngineConfig{base_url, extract_main})`; CLI/MCP convert parameters `--base-url` / `--extract-main`.

### MCP (the `mcp` feature)

- Assembly shape: the `alch mcp` subcommand (the former standalone `alch-mcp` binary was folded in; feature `mcp` implies `cli`) — `sdforge::mcp::serve_stdio(build())` carries the three MCP tools convert / doctor / formats (registered via `#[forge(tool_name)]`, parameter schemas generated from endpoint signatures), stdio transport — the transport surface has no network egress (the convert tool's own network capability follows the `pdf-cloud` feature).
- CLI shared source: the three endpoints are also registered into `alch` with `cli = true` (CliBuilder + custom runner; the exit-code 0/1/2/3 contract lives in the main cli-output spec).
- Note: the root level has a built-in global `--format text|json` (the sdforge output format); convert's extension-override parameter is named `input_format`, and the CLI-side runner normalizes `--format` into `--input_format` in the convert context.

### Internationalization (i18n, unconditional dependency)

- Coverage: the Display of all `AlchemizeError` variants and `SecurityViolationType`, CLI/MCP diagnostics (unsupported-format/write-failure/success summary), runner messages, and tracing log bodies (registry fallback chain, html_deep degradation, subtitle/video parsing, MinerU task lifecycle).
- Message catalog: `locales/{en,zh}/messages.ftl` embedded at compile time (`include_str!`), a concurrency-safe FluentBundle cached in a `OnceLock`; lookup chain current language → `en` → key as-is. en/zh key-set consistency is anchored by a guard test.
- Language detection: `ALCHEMIZE_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `sys-locale` → `en`; POSIX values normalized (`zh_CN.UTF-8` → `zh-CN`), `C`/`POSIX` treated as English. Any language whose primary subtag is `zh` maps to the Chinese bundle.
- Public API: `alchemize::tr(key)`, `alchemize::tr_args(key, args)`, `alchemize::i18n::translate(key, args)` (current locale), `alchemize::i18n::translate_for(lang, key, args)` (explicit language), `alchemize::i18n::locale_of_current_process()`.
- Environment variable: `ALCHEMIZE_LANG` (highest-priority language override; BCP-47 or POSIX forms both accepted).
- Implementation note: the bundle sets `set_use_isolating(false)` — Unicode isolating characters would change the English output bytes and break the CLI output contract; CWE-150 protection against user-supplied extensions lives in the diagnostic layer's `sanitize_ext` (non-overlapping responsibilities).

## 💡 Usage Examples

### Basic Conversion

```rust
use alchemize::{AlchemizeBuilder, Input};

#[tokio::main]
async fn main() -> Result<(), alchemize::AlchemizeError> {
    // Enable the matching format feature in Cargo.toml (e.g. features = ["pdf"]); otherwise convert returns UnsupportedFormat
    let alch = AlchemizeBuilder::new().build();
    let result = alch.convert(&Input::Path("paper.pdf".into())).await?;
    println!("engine: {}", result.metadata["converter"]);
    println!("quality: {}", result.quality);
    println!("{}", result.markdown);
    Ok(())
}
```

### Engine Preference

```rust
// Move the pdf engine to the front of the queue; on a name miss it silently falls back to the default chain
let result = alch.convert_pref(&input, Some("pdf")).await?;
```

### Asset Modes

```rust
use alchemize::application::builder::AssetMode;

let alch = AlchemizeBuilder::new()
    .with_asset_mode(AssetMode::Inline) // base64 data URL inline
    .build();
// Dir mode (default) writes <output>/images/ via asset_sink_for
```

### Custom Vision Provider

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

### Custom Transcription Provider

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

## 🌟 Best Practices

### Error Handling

- Match the three terminal states and handle each: `UnsupportedFormat` (point the user at `formats`), `AllBackendsFailed{attempts}` (present the attempts line by line to the user), everything else (report by message).
- Do not swallow `Err` and do not substitute defaults for failed output; degraded output (`quality=local_fallback`) is an `Ok` — the caller decides acceptability by quality.

### Engine Selection

- Trust the default chain first (local first, cloud fallback is the settled policy); when you truly must force, use `convert_pref`/`--engine` (prefix matching suffices) and accept silent fallback on a miss.
- `priority()` for custom engines: use a value below 15 to join an existing chain; `availability()` separates "can it produce" from "how good is the output" — engines that can degrade stay `Available`.

### Security Notes

- Credentials come from environment variables (`MINERU_API_TOKEN`) or `MineruConfig::api_token`; redacted on all three paths, never written to logs.
- Printing/serializing `MineruConfig` is safe (three-path redaction), but custom types holding sensitive fields should imitate the hand-written `Debug` + unit-test pinning practice.
- Enable `with_no_remote()` in untrusted environments — the process then has zero network egress.

### Common Troubleshooting

- `metadata` is `Null`: only outputs produced directly through a `Converter` impl (not via the registry) look like this; the facade path always injects `converter`.
- An engine is missing from `availability()`: its feature is not compiled or (for MinerU) the endpoint is unconfigured — it was never registered.
- A zip input reports `UnsupportedFormat("zip")`: happens only when calling `default_converters` directly to build your own registry (not via `AlchemizeBuilder`) — the zip converter is wired at the facade layer, not in the registry (to avoid nested-zip self-recursion).
- All inner files of a container (tar/tgz/gz/xz/zst/7z) degrade to attachments and `metadata.registry_wired` is `false`: you called `default_converters` directly and never backfilled the container registry slot. After assembly you must run `slot.set(Arc::downgrade(&registry))`; `AlchemizeBuilder::build()` does it automatically.

## 📚 Related Documents

- [🏗️ Architecture](ARCHITECTURE.md) — design principles, module layout, and data flows
- [📖 User Guide](USER_GUIDE.md) — the complete tutorial from installation to advanced use
- [❓ FAQ](FAQ.md) — frequently asked questions
- [🔒 Security](SECURITY.md) — trust boundaries, SSRF guards, vulnerability reporting
- [🤝 Contributing](CONTRIBUTING.md) — dev environment, testing, and commit conventions
- [📋 Changelog](CHANGELOG.md) — release history
