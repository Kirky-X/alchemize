# 📖 Alchemize User Guide

The complete tutorial from installation to advanced use. Commands and behaviors defer to the current sources; for API details see the [API Reference](API_REFERENCE.md), and for design background see the [Architecture doc](ARCHITECTURE.md).

## 📋 Table of Contents

- [Introduction](#-introduction)
- [Quick Start](#-quick-start)
- [Core Concepts](#-core-concepts)
- [Configuration](#️-configuration)
- [Advanced Usage](#-advanced-usage)
- [Best Practices](#-best-practices)
- [Troubleshooting](#-troubleshooting)
- [Further Reading](#-further-reading)

## 🎯 Introduction

Alchemize transmutes any document into Markdown: a PDF, a Word file, an audio clip, even a zip archive — one command yields Markdown text. It degrades automatically along an engine chain of "local first, cloud as fallback", and every output is labeled with its quality tier and the actual engine — no swallowed errors, no silence. The default-feature build has zero external-service dependencies and can run fully offline.

## 🚀 Quick Start

### 📌 Prerequisites

- **Rust 1.97.1+** (the `rust-version` in `Cargo.toml`, edition 2024)
- Optional: `ffmpeg` (video track extraction), `pandoc` (the LaTeX high-fidelity path) — both probed on the PATH; the matching capability degrades or errors when absent

```bash
# Check the Rust version
rustc --version
# Expected output: rustc 1.97.1 (or higher)
```

### 📦 Installation

```bash
git clone https://github.com/Kirky-X/alchemize.git
cd alchemize

# Default features (CLI + plain text conversion only, zero external-service dependencies; breaking change)
cargo build --release          # binary: target/release/alch

# Opt in per format (example: PDF + Word; non-enabled formats return unsupported, exit code 2)
cargo build --release --features pdf,docx

# Full build (MinerU cloud + local ASR)
cargo build --release --features full

# Install locally
cargo install --path . --features full
```

The default is `default = ["cli", "text"]`: `cargo build` compiles only the CLI skeleton and plain-text/code-fencing conversion. To convert PDF, Word, HTML and other formats you must enable the matching feature explicitly (see the feature column in [Engines and Extensions](#engines-and-extensions)).

### 💡 First Steps

```bash
# Convert to stdout (PDF requires building with --features pdf)
alch convert paper.pdf

# Write a file (parent dirs auto-created); stderr prints a summary line (Word requires --features docx)
alch convert report.docx -o report.md
# converted report.docx -> report.md [local_full] (engine: docx-deep)

# See which engines are available and which formats are supported
alch doctor
alch formats
```

## 🔑 Core Concepts

### Registry and Fallback Chain

The candidate engines for each extension form a chain in descending `priority()` order: when an earlier engine returns `Err`, the next-best is tried automatically; `Unavailable` engines are skipped outright with the reason recorded in attempts. `--engine NAME` moves the preferred engine to the front (exact name first, prefix matching when no exact hit, case-insensitive; on a miss it silently falls back without erroring).

### Quality Tiers

The `quality` field of every output (same source as the `[quality]` in the stderr summary line):

| Tier | Meaning | Typical source |
| --- | --- | --- |
| `api` | Remote high fidelity | MinerU cloud |
| `local_full` | Full local | Native parsers (docx-deep/odt/odp/pptx-deep etc.) |
| `local_fallback` | Degraded local | The pdf-extract text chain, LaTeX fencing without pandoc |

Degraded output still counts as success (exit code 0) — whether it is acceptable is judged by the quality label; the tool does not decide for you.

### Engines and Extensions

| Engine | Extensions | Feature | Notes |
| --- | --- | --- | --- |
| `text` | txt/text/log/md + ~45 code/config extensions | `text` (default member) | Fenced code blocks (language-tag mapping) |
| `csv` | csv/tsv/psv | `csv` | Delimiter auto-detection (samples the first 5 non-empty lines; tsv forces tabs, psv forces pipes) |
| `html` / `html-deep` | html/htm | `html` | `html` is the htmd base chain, `html-deep` the native deep chain (both chains in the `html` feature); the deep chain supports `--extract-main` main-content extraction and `--base-url` link absolutization, and auto-routes `<math` to htmd for fidelity |
| `epub` | epub | `epub` | Structured text parsing |
| `ipynb` | ipynb | `ipynb` | Notebook cells to markdown |
| `docx` / `docx-deep` | docx | `docx` | OOXML native XML (spreadsheets xlsx/xls/xlsb/ods unsupported) |
| `pptx` / `pptx-deep` | pptx | `pptx` | Same as above |
| `odt-deep` / `odp-deep` | odt / odp | `odt` / `odp` | OpenDocument deep parsing |
| `legacy-office` / `legacy-ppt` | doc, rtf / ppt,pps,pot | `legacy-office` | External soffice conversion, then the deep chain |
| `pdf` | pdf | `pdf` | lopdf text extraction + TOC removal/structure heuristics |
| `pdfium` | pdf | `pdf-local` | Local high fidelity (implies `pdf`); degrades when the dynamic library is missing |
| `mineru` | pdf | `pdf-cloud` + endpoint config | Cloud high fidelity, chain tail |
| `latex-source` | tex/latex | `latex` | pandoc probe, fenced fallback when missing |
| `pandoc-generic` | org/rst/adoc/asciidoc/opml/mediawiki etc. | `pandoc` | Generic markup languages |
| `image` / `audio` / `video` / `subtitle` | image/audio/video/subtitle families | `media` | See Advanced Usage below; the transcription backend additionally needs `audio` |
| `heic` | heic, heif | `heic` | sips or metadata fallback |
| `zip` (facade) | zip, cbz | `zip` | Recursive container; nested zip unsupported |

### Asset Modes

| Mode | Behavior |
| --- | --- |
| `dir` (default) | Embedded images extracted in document order, written to `images/` next to the output, duplicates get an index prefix, markdown references them with relative paths |
| `inline` | base64 data URL inline, single-file delivery |

## ⚙️ Configuration

### 🧱 Environment Variables

| Variable | Purpose |
| --- | --- |
| `ALCHEMIZE_MINERU_URL` | Registers the MinerU engine when set (local/gateway service base URL) |
| `MINERU_API_TOKEN` | Cloud Precision API bearer token |
| `ALCHEMIZE_TRANSCRIBE_MODELS` | Transcription model bundle directory (defaults to probing `~/.cache/alchemize/models/transcribe/`) |
| `ORT_DYLIB_PATH` | onnxruntime dynamic library path |
| `ALCHEMIZE_ASR_TEST_WAV` | Used only by integration tests |
| `ALCHEMIZE_LOG` | Log level `off`/`error`/`warn`/`info`/`debug`/`trace` (default `off`, silent); once a level is set, all levels write only to stderr |

### 📜 Logging

The CLI/MCP logging frontend is assembled by inklog; the only switch is `ALCHEMIZE_LOG`:

```bash
ALCHEMIZE_LOG=debug alch convert paper.pdf > paper.md   # logs all on stderr; the stdout pipe stays clean
```

- Default (unset or `off`): no subscriber is assembled; the output is byte-for-byte identical to the no-logging build.
- Once a level is set: all levels from `error` to `trace` (including `info`/`debug`) write only to stderr; stdout carries only the conversion product.
- Invalid values: one stderr notice, then continue as `off` — no silent swallowing.
- The same switch applies to the `alch mcp` subcommand; MCP stdout carries JSON-RPC, and logs likewise go only to stderr.

### 📥 MinerU Endpoint

```bash
export ALCHEMIZE_MINERU_URL="http://localhost:8888"   # local/gateway MinerU service
alch convert paper.pdf
```

Without it, the MinerU engine is never registered and the chain contains only local engines. Timeout defaults to 300 seconds; when the cloud Precision API requires a token, use `MINERU_API_TOKEN`.

### 🔢 ASR Model Bundle

Local transcription with feature `audio` (Qwen3-ASR + silero VAD) needs a model bundle and the onnxruntime dynamic library:

1. Directory layout: `qwen3-conv_frontend.onnx`, `qwen3-encoder.int8.onnx`, `qwen3-decoder.int8.onnx`, `silero_vad.onnx`, `tokenizer/`.
2. `ALCHEMIZE_TRANSCRIBE_MODELS` points to that directory (if unset, `~/.cache/alchemize/models/transcribe/` is probed).
3. `ORT_DYLIB_PATH` points to the onnxruntime dynamic library.

When the bundle is missing the engine honestly reports Unavailable with acquisition guidance — it will **never download automatically**.

### 💻 Command Line Tool

The CLI is wrapped by sdforge `#[forge]` endpoints (`src/forge_app.rs`): one endpoint definition produces both the CLI and the MCP tool surface.

```bash
alch convert <INPUT> [-o <PATH>] [--engine <NAME>] [--assets <dir|inline>] [--no-remote] [--format <EXT>]
alch doctor      # one line per registered engine: AVAILABLE or UNAVAILABLE + reason
alch formats     # all supported extensions, sorted, deduplicated, space-separated
```

`--format` has dual semantics: in the **convert context** it is the extension override (the old contract); at the **root level** (before the subcommand) it is the sdforge output format `text|json` (e.g. `alch --format json formats`).

Note: **engines not compiled into features, and MinerU without a configured endpoint, do not appear in the doctor list** (they are never registered); missing dependencies of media engines degrade at conversion time, and doctor still shows AVAILABLE. Exit codes: `0` success (including degraded), `1` other errors, `2` unsupported format, `3` entire chain failed (stderr lists each engine's failure reason line by line).

## 🚧 Advanced Usage

### ✅ Reading Quality and Degradation

`local_fallback` is not failure: the local PDF text chain is expected behavior. Reading order: check the stderr summary line `[quality]` first, then upgrade features as needed (configure MinerU). Library users read `metadata.converter` and the `quality` field to make the same call.

### ☁️ MinerU Cloud and Local Services

MinerU sits at the chain tail: the cloud is tried only after the local chain (pdf-extract) fails (producing `api` tier). For fully offline use, `--no-remote`. The agent/precision dual clients are selected automatically by file size (precision has its own 200 MB cap); response bodies are limited in tiers by purpose.

### 📝 Subtitles and Timecodes

srt/vtt/ass/ssa are parsed natively (aspasia) into timestamped segment lists, without ffmpeg.

### 👀 Video Processing Modes

Library API `with_video_mode` has three settings: `Gpu` / `AudioOnly` (default, ffmpeg extracts the track into the audio chain) / `Disabled`. When ffmpeg is not on the PATH, the video engine reports `VideoUnavailable`.

### 🔐 Injecting Custom Providers

Image vision and audio transcription are both ports: `with_vision_provider(Arc<dyn VisionProvider>)`, `with_transcribe_provider(Arc<dyn TranscribeProvider>)`. Without injection, the matching conversion degrades to metadata markdown (dimensions/duration/hash); there is no built-in cloud fallback.

### 🧲 Main-Content Extraction and Link Absolutization (feature `html`)

```bash
# Convert only the main content (readability extraction, removing nav/footer noise; honest fallback to the full page on failure)
alch convert saved-page.html --extract-main

# Rewrite relative links to absolute (#anchor, mailto: etc. preserved)
alch convert saved-page.html --base-url "https://origin.example/path/"
```

Internally the engine is a native state machine → htmd fallback chain: when the engine's main-path output is empty but visible text exists, it drops to htmd automatically; pages containing a `<math` tag are routed to the existing htmd engine to preserve LaTeX.

**Applicability guidance** (measured on real archives: Wikipedia 1MB page -71%, docs.rs index page -89%, marketing landing page -30%, legal text page -2%):

| Scenario | Recommendation | Reason |
| --- | --- | --- |
| Web archives → RAG corpus / read-only main text | On | Removes nav, footer, Edit-link noise (measured on a wiki page "Edit section" 56→0 with zero loss of body anchors) |
| Hand-written documents / structure is content (sidebar TOC semantics included) | Off | readability heuristics may cut structure you need; fidelity first |
| Legal/contract full-page archives | Off, or compare first | Extraction yields body markdown (paragraphs and images kept) but page-level layout flattens (the -2% license page kept its body intact) |

The criterion is simple: pages with clear "main content block" semantics (article, high body density) → on; otherwise off. When extraction fails or there is no main content to extract, it **automatically falls back to full-page conversion** (no error, no empty string), so leaving it on never loses content — the output shape just differs. The quality label distinguishes the two paths: the extraction path is `local_fallback`, the full-page path `local_full`, so downstream can sort by label.

### 🤖 MCP Service (feature `mcp`)

```bash
cargo build --release --features mcp   # produces alch (the mcp subcommand is compiled in with the feature)
# Register via stdio in your MCP client config:
# { "command": "/path/to/alch", "args": ["mcp"] }
```

The `alch mcp` subcommand goes through sdforge (rmcp, stdio transport — the transport surface has no network egress; the convert tool's own network capability follows the `pdf-cloud` feature) and exposes convert / doctor / formats as MCP tools with parameter schemas generated from endpoint signatures; it shares the same endpoint definition as the CLI (the former standalone `alch-mcp` binary was folded into this subcommand).

### 🔭 As a Library

```rust
use alchemize::{AlchemizeBuilder, Input};
use alchemize::application::builder::AssetMode;

let alch = AlchemizeBuilder::new()
    .with_asset_mode(AssetMode::Inline)
    .with_no_remote()               // offline environments
    .build();
let result = alch.convert_pref(&Input::Path("report.docx".into()), Some("docx-deep")).await?;
println!("{} [{}]", result.title.as_deref().unwrap_or("-"), result.quality);
```

For the full API see the [API Reference](API_REFERENCE.md#-usage-examples).

## 🌟 Best Practices

### ✅ Recommended Usage

- Start with default features and move to `full` only when truly needed — fewer features mean a smaller attack surface and fewer dependencies.
- Compose pipes and scripts for batch conversion (stdout mode is pipe-friendly).
- Use `--format <ext>` only when the extension is untrustworthy; otherwise trust auto-detection.

### 🔒 Security Practices

- Add `--no-remote` in untrusted environments — zero network egress for the process.
- Credentials come from environment variables or `MineruConfig::api_token`; do not put `MINERU_API_TOKEN` into shell history (use env files or a secret manager).
- Details in the [Security doc](SECURITY.md).

## 🔧 Troubleshooting

| Symptom | Handling |
| --- | --- |
| Exit code 2 `unsupported format: <ext>` | Confirm the extension with `alch formats`; or specify explicitly with `--format`; the needed engine may not be compiled into the features |
| Exit code 3, entire chain failed | stderr lists each engine's failure reason line by line; handle per reason (endpoint unreachable/file corrupted/dependency missing) |
| Output quality is only `local_fallback` | Expected degradation; configure MinerU for higher fidelity |
| Images are metadata-only | No VisionProvider injected; no built-in fallback is by design |
| Conversion reports `AudioUnavailable` | doctor does not show provider-level absence; verify the bundle layout and `ORT_DYLIB_PATH` |
| `zip` conversion reports nesting unsupported | zip recursion supports only plain files inside a single container; nested zips are explicitly unsupported |
| latex did not go through pandoc | pandoc is not on the PATH and the fenced fallback applied (quality `local_fallback`); installing pandoc upgrades automatically |

## 🎯 Further Reading

- [📘 API Reference](API_REFERENCE.md) — library integration and custom engines
- [🏗️ Architecture](ARCHITECTURE.md) — the design rationale for the fallback chain and module layout
- [❓ FAQ](FAQ.md) — quick answers to frequent questions
- [🔒 Security](SECURITY.md) — trust boundaries and hardening checklist
- [🤝 Contributing](CONTRIBUTING.md) — want to change code? Start here
