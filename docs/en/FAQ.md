# ❓ Alchemize FAQ

Frequently asked questions. All answers are based on the real behavior of the current sources, with nothing planned or speculative.

## 📋 Table of Contents

- [General Questions](#-general-questions)
- [Installation and Configuration](#-installation-and-configuration)
- [Usage and Features](#️-usage-and-features)
- [Performance](#-performance)
- [Troubleshooting](#-troubleshooting)
- [Other Questions?](#-other-questions)

## 🧭 General Questions

### ❓ What is Alchemize?

A Rust document-to-Markdown tool (the `alch` CLI plus the library of the same name): it accepts PDF, the full Office family, HTML/EPUB, CSV, code/config text, images, audio/video, subtitles, recursive zip containers and more as input, picks engines automatically via a priority fallback chain, and produces Markdown with quality labels.

### ❓ Why use Alchemize?

Three reasons: ① one command unifies any format to Markdown — no per-format tool hunting; ② degradation is explicit — every output is labeled with its `api`/`local_full`/`local_fallback` quality tier and the actual engine name, no swallowed errors, no silence; ③ the default build has zero external-service dependencies, and `--no-remote` runs fully offline.

### ❓ Is Alchemize production-ready?

Yes. Test scale: 168 cases in the default tier (core + text only), 912 passed in the full tier (plus 5 ignored), 917 passed in the all-features tier (plus 5 ignored); line coverage 90.49% (gate ≥90%); the real-file suite's 12 tests (9 in-repo fixtures and 1 external video source) cover real documents such as PDF/DOCX/XLSX/RTF. The security model is in the [Security doc](SECURITY.md).

### ❓ Which platforms are supported?

Any platform supported by Rust 1.97.1+ (macOS/Linux/Windows). External binary dependencies (ffmpeg/pandoc) are probed on the PATH; the matching capability degrades when absent. Local ASR inference additionally needs the onnxruntime dynamic library.

### ❓ How can I contribute?

See the [Contributing guide](CONTRIBUTING.md): environment setup, the TDD workflow, code standards, and the commit/PR process. The repo runs the specmark spec-driven process; capability-domain spec sources live in `specmark/specs/`.

### ❓ Where can I get help?

Start with this FAQ and the troubleshooting section of the [User Guide](USER_GUIDE.md); then the [Architecture doc](ARCHITECTURE.md) and the [API Reference](API_REFERENCE.md); if it is still unresolved, open a GitHub Issue (with a minimal reproduction and the `alch doctor` output).

### ❓ What license does the project use?

Apache-2.0 (`LICENSE`, consistent with Cargo.toml; every source file carries an SPDX header). Acknowledgments and boundaries: PDFMathTranslate (AGPL-3.0) contributed design ideas only, with zero code copied.

## 📦 Installation and Configuration

### ❓ How do I install it?

```bash
cargo install --path . --features full   # or cargo build --release (default: CLI + text conversion only)
```

Details in the [User Guide](USER_GUIDE.md#-installation).

### ❓ How do I pick the right feature combination?

```bash
cargo build                        # minimal build: text/code fencing only (default = cli + text)
cargo build --features pdf,docx    # enable exactly the format features you need
cargo build --features pdf,pdf-cloud # PDF local chain + MinerU cloud fallback (endpoint config also required)
cargo build --features audio       # local transcription (implies media; model bundle + ORT library also required)
cargo build --features full        # everything (30 conversion features)
```

### ❓ Why can't the default build convert pdf/docx?

This is the breaking change introduced by `dependency-feature-gating`: `default = ["cli", "text"]`, so the default build compiles only the CLI skeleton and plain-text conversion. The converters and dependencies for PDF/Word etc. are each feature-gated; when not enabled they are neither compiled nor registered — `alch convert x.pdf` therefore has no candidate engine and returns `unsupported format` (exit code 2). Enable what you need explicitly (e.g. `--features pdf,docx`); use `--features full` to restore the old full-default behavior.

### ❓ How do I build minimal?

```bash
cargo build --release                              # CLI + text/code fencing only
cargo build --release --no-default-features --features cli,text  # equivalent explicit spelling
```

The minimal build pulls in no network/cloud/Office/container dependencies; apart from the 9 core dependencies (`serde`/`serde_json`/`thiserror`/`tracing`/`tokio`/`async-trait`/`anyhow`/`base64`/`sha2`), all other dependencies are `optional` and activated only via `dep:` with their feature.

### ❓ How large are the dependency differences between feature combinations?

The default tier contains only the 9 core dependencies + `encoding_rs` (text) and no network/cloud/Office/container dependencies at all (reqwest/limiteron/futures/lopdf/quick-xml/zip/htmd are all optional); `pdf` brings lopdf, `pdf-cloud` brings reqwest+limiteron+futures, `audio` brings vecboost (the ort/ONNX stack, a registry dependency). Engines whose features are off do not participate in compilation — smaller binaries, smaller audit surface.

### ❓ What are the system requirements?

Rust 1.97.1+ (the `rust-version` gate). Optional external tools: ffmpeg (video), pandoc (LaTeX high fidelity). ASR additionally needs a model bundle and the onnxruntime dynamic library (paths in the [User Guide](USER_GUIDE.md#-asr-model-bundle)).

## 🛠️ Usage and Features

### ❓ How do I get started with the basic usage?

```bash
alch convert file.pdf > out.md      # stdout
alch convert file.pdf -o out.md     # write file + stderr summary line
alch doctor && alch formats         # engines and formats first
```

### ❓ Which formats and sources are supported?

The document family (pdf/docx/pptx/odt/odp/rtf/legacy binary Office/html/epub/csv/tsv/txt/md/recursive zip/ipynb/tex; spreadsheets (xlsx/xls/xlsb/ods) are unsupported), the code-config family (~45 extensions fenced), images (png/jpg/gif/bmp/webp/tiff/svg), audio (wav/mp3/ogg/opus/flac/aac/m4a/wma), video (mp4/avi/mkv/mov/wmv/flv/webm/m4v, ffmpeg track extraction), subtitles (srt/vtt/ass/ssa). All of these require the matching feature at build time (default has only `text`); the live `alch formats` output is authoritative.

### ❓ How do I read conversion quality? Is there a validation feature?

There is no standalone "validation" feature; quality reading relies on the output labels: the stderr summary line and the `metadata.converter`/`quality` fields. Degradation (`local_fallback`) is expected behavior, not an error; boundaries such as nested zips and unsupported formats are explicit errors.

### ❓ When should `--extract-main` be on?

| Scenario | Recommendation |
| --- | --- |
| Web archive → RAG corpus / main text only | On (nav/footer/Edit noise removed; measured on a real 1MB wiki page -71%, "Edit section" 56→0, zero loss of body anchors) |
| Hand-written documents / structure is content | Off (readability heuristics may cut structure you need) |
| Legal/contract full-page archives | Off, or compare first (extraction yields body markdown: paragraphs/images kept, page-level layout flattened) |

Safety net: when extraction fails or there is no main content to extract, it **automatically falls back to full-page conversion** — leaving it on never loses content. The two paths are distinguishable by quality label — extraction path `local_fallback`, full-page path `local_full` (stderr summary and the `quality` field). Measurement details in the [User Guide · Main-Content Extraction](USER_GUIDE.md#-main-content-extraction-and-link-absolutization-feature-html).

### ❓ Is Alchemize secure?

The security model has four defense lines: structured error handling for untrusted input (no panics), SSRF validation of MinerU download URLs (HTTPS-only/private-range denial/hostname allowlist/signatures), tiered remote response limits, and three-path credential redaction. The default build has zero network egress. Full details in the [Security doc](SECURITY.md).

### ❓ How do I report a security vulnerability?

Do not open a public issue. Use a GitHub private security advisory (the repo's Security tab → Report a vulnerability); process and response commitments in the [Security doc](SECURITY.md#-vulnerability-reporting).

## ⚡ Performance

### ❓ How fast is Alchemize?

Measured criterion benchmark data is in the [Benchmarks](benchmarks.md) doc (real files, median-based). Design-level trade-offs in [Architecture · Performance Design](ARCHITECTURE.md#-performance-design): multi-threaded runtime, async remote paths, no global health cache, bounded response memory.

### ❓ What about memory usage?

Conversion streams at single-file granularity; remote response reading is limited in tiers by purpose (64KB submissions/8KB errors/50MB polling and results, 8MB preallocation cap); zip rejects nested recursion, making unbounded depth structurally impossible. The builder's `with_max_file_size` is currently not consumed by engine assembly (known limitation, see the [API Reference](API_REFERENCE.md#-core-api)).

## 🔧 Troubleshooting

### ❓ Getting an "unsupported format" error

Exit code 2. Three possibilities: the extension is genuinely unsupported (confirm with `alch formats`), the needed engine is not compiled into the features, or the extension was misdetected (specify explicitly with `--format <ext>`).

### ❓ Getting an "all backends failed" error

Exit code 3. stderr lists `- <engine>: <reason>` line by line: unreachable endpoint → check the service; corrupted file → change the source; missing dependency → install it. A full-chain failure never produces a half-written file.

## 🎯 Other Questions?

- First flip through the [User Guide](USER_GUIDE.md) and the [API Reference](API_REFERENCE.md)
- For behavior questions, open a GitHub Issue with `alch doctor` output and a minimal reproduction file
- Want to change code → the [Contributing guide](CONTRIBUTING.md)
