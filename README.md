English | [中文](README_CN.md)

<div align="center">

<img src="docs/assets/alchemize.png" alt="Alchemize Logo" width="240">

[![Rust 2024](https://img.shields.io/badge/Rust-2024-edded?logo=rust&style=for-the-badge)](https://www.rust-lang.org/) [![Rustc 1.97.1+](https://img.shields.io/badge/Rustc-1.97.1+-orange.svg?style=for-the-badge)](https://www.rust-lang.org/) [![license](https://img.shields.io/badge/license-Apache--2.0-blue?style=for-the-badge)](LICENSE)

**Transmute any document into Markdown**

[✨ Features](#-features) • [🚀 Quick Start](#-quick-start) • [📚 Documentation](#-documentation) • [🔒 Security](#-security) • [🤝 Contributing](#-contributing)

</div>

---

<div align="center">

### 🎯 One Input, Any Format, Markdown Out

Hand `alch` a file — PDF, Office, audio/video, a zip archive — and it picks the engine via a priority fallback chain, producing Markdown with an honest quality label:

<table style="width:100%; border-collapse: collapse">
<tr>
<td align="center" width="25%">🧪<br><b>Any format</b><br><span style="color:#64748B">documents · code · images · audio/video · subtitles · zip</span></td>
<td align="center" width="25%">🔗<br><b>Fallback chain</b><br><span style="color:#64748B">priority-descending · automatic degradation · degradation is explicit</span></td>
<td align="center" width="25%">🏷️<br><b>Quality tiers</b><br><span style="color:#64748B">api / local_full / local_fallback · engine traceable</span></td>
<td align="center" width="25%">🏠<br><b>Local first</b><br><span style="color:#64748B">zero external services by default · fully offline with --no-remote</span></td>
</tr>
</table>

</div>

---

## 📋 Table of Contents

<details open>
<summary>📑 Table of Contents</summary>

- [✨ Features](#-features)
- [🚀 Quick Start](#-quick-start)
- [📚 Documentation](#-documentation)
- [🔒 Security](#-security)
- [🤝 Contributing](#-contributing)
- [📋 Changelog](#-changelog)
- [📄 License](#-license)
- [🙏 Acknowledgments](#-acknowledgments)
- [📞 Contact & Support](#-contact--support)
- [⭐ Star History](#-star-history)

</details>

---

## ✨ Features

<table style="width:100%; border-collapse: collapse">
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">📄 <b>Full document formats</b><br><span style="color:#64748B">PDF (pdfium/pure-Rust/MinerU), docx/pptx, odt/odp, legacy binary Office (doc/ppt/rtf via LibreOffice), html/epub, csv/tsv/psv, LaTeX/org/rst/asciidoc (pandoc), ipynb</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧬 <b>Code fencing</b><br><span style="color:#64748B">py/rs/js/ts/go/java/c/cpp/json/yaml/toml/xml/sh and ~45 extensions total, as language-tagged fenced code blocks</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🖼️ <b>Image handling</b><br><span style="color:#64748B">Inject a VisionProvider for description and OCR; without one, degrades to metadata-only markdown (dimensions/hash)</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🎙️ <b>Audio/video transcription</b><br><span style="color:#64748B">In-house vecboost facade (Qwen3-ASR ONNX + silero VAD), fully local inference; VAD-segmented text with timestamps</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">⏱️ <b>Subtitle parsing</b><br><span style="color:#64748B">srt/vtt/ass/ssa parsed natively into timestamped segments, no ffmpeg involved</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">📦 <b>Recursive zip</b><br><span style="color:#64748B">Archive members routed to their own engines by extension; nested zips are explicitly unsupported — unbounded self-recursion is structurally impossible</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🌐 <b>Multi-encoding detection</b><br><span style="color:#64748B">UTF-8 first → CJK-priority probing (GBK/Big5/Shift-JIS/…) → lossy fallback; CSV delimiter auto-detection</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧭 <b>Engine preference & diagnostics</b><br><span style="color:#64748B"><code>--engine</code> exact-name first with prefix fallback (silent fallback on miss); <code>doctor</code> per-engine availability, <code>formats</code> full format list</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🖼 <b>Dual asset modes</b><br><span style="color:#64748B"><code>dir</code> (images written to <code>images/</code> with relative references) and <code>inline</code> (base64, single-file delivery)</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🛡️ <b>Security defenses</b><br><span style="color:#64748B">MinerU URL SSRF validation (HTTPS-only / private-range denial / hostname allowlist), tiered response limits, credentials redacted on all three output paths</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">☁️ <b>MinerU cloud fidelity</b><br><span style="color:#64748B">Agent/Precision dual clients selected by file size, polling with backoff; local first, cloud as last resort (chain tail)</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧰 <b>CLI diagnostics trio</b><br><span style="color:#64748B"><code>convert</code> (-o/--engine/--assets/--no-remote/--format), <code>doctor</code>, <code>formats</code>; semantic exit codes 0/1/2/3</span></td>
</tr>
</table>

Beyond these core capabilities, video track extraction and frame dedup, video processing modes (Gpu/AudioOnly/Disabled), a Whisper-compatible cloud backend (activated only via explicit injection), LaTeX math-delimiter validation, and an opt-in conversion-result dedup cache (`cache`, TTL + single-flight for pdf-cloud/audio) are all built in or provided behind independent feature flags; see [User Guide · Feature Flags](docs/en/USER_GUIDE.md#-feature-flags) for the full matrix.

---

## 🚀 Quick Start

### 📦 Installation

```bash
# Check the Rust version (1.97.1+ required)
rustc --version

# Default features (CLI + plain text conversion, ~220 crates of which 157 come from the CLI skeleton, zero external-service dependencies; breaking change — see "Feature Flags")
cargo build --release          # binary: target/release/alch

# Opt in per format (example: PDF + Word + HTML; enable only the features you need)
cargo build --release --features pdf,docx,html

# Full build (all 30 conversion features: MinerU cloud + self-hosted office deep parsers + containers/data/e-mail + LibreOffice + local ASR)
cargo build --release --features full

# Install locally
cargo install --path . --features full
# Note: with --no-default-features you must include `cli`, otherwise no `alch` binary is produced
```

### 💡 Minimal Example

```bash
# Convert to stdout (PDF requires building with --features pdf)
alch convert paper.pdf

# Write file + engine preference + asset mode (parent dirs auto-created; Word requires --features docx)
alch convert report.docx -o report.md --engine docx-deep --assets dir|inline

# stderr summary line: quality tier and actual engine at a glance
# converted report.docx -> report.md [local_full] (engine: docx-deep)
```

```bash
# Engine diagnostics / supported formats
alch doctor
alch formats
```

Exit codes: `0` success (including degraded output); `1` other errors; `2` unsupported format; `3` entire chain failed (stderr lists each engine's failure reason).

More examples (batch conversion, offline mode, custom vision/transcription providers, registry assembly) in [User Guide · CLI Examples](docs/en/USER_GUIDE.md#-cli-examples) and [API Reference · Usage Examples](docs/en/API_REFERENCE.md#-usage-examples).

### 📊 Format Matrix and Core Concepts

The full format matrix is in [User Guide · Supported-Format Matrix](docs/en/USER_GUIDE.md#-supported-format-matrix); core concepts such as the registry and fallback chain, quality tiers, and local-first are in [User Guide · Core Concepts](docs/en/USER_GUIDE.md#-core-concepts).

---

## 📚 Documentation

| Doc | Description |
|------|------|
| [📖 User Guide](docs/en/USER_GUIDE.md) | Complete tutorial from install to advanced use |
| [📘 API Reference](docs/en/API_REFERENCE.md) | All public APIs, CLI surface, environment variables |
| [🏗️ Architecture](docs/en/ARCHITECTURE.md) | Design principles, module layout, fallback-chain mechanics |
| [❓ FAQ](docs/en/FAQ.md) | Frequently asked questions |
| [🔒 Security](docs/en/SECURITY.md) | Trust boundaries, SSRF guards, vulnerability reporting |
| [🤝 Contributing](docs/en/CONTRIBUTING.md) | Dev environment, TDD workflow, commit conventions |
| [📊 Benchmarks](docs/en/benchmarks.md) | Real-file performance benchmarks (criterion) |
| [📋 Changelog](docs/en/CHANGELOG.md) | Release history, roadmap, and acknowledgments |

Capability-domain spec sources (verifiable behaviors for conversion / fallback chains / media / CLI output) live in `specmark/specs/`.

---

## 🔒 Security

### 🛡️ Security Design

Four defense lines: structured error handling for untrusted input (no panics); SSRF validation of MinerU download URLs (HTTPS-only, private-range denial, hostname allowlist); tiered response limits; credentials redacted on all three output paths. Details in the [🔒 Security doc](docs/en/SECURITY.md).

### 🚨 Reporting Security Issues

**Do not** open a public issue. Use GitHub private security advisories (**Security → Report a vulnerability**); process and response commitments in the [Security doc · Reporting](docs/en/SECURITY.md#-vulnerability-reporting).

---

## 🤝 Contributing

### 🛠️ Development Environment

Rust 1.97.1+, one command to get started: `cargo build --features full`. See [🤝 Contributing](docs/en/CONTRIBUTING.md#-environment-setup).

### 💖 Ways to Contribute

- New format engine: implement the `Converter` trait + wire it in `engines.rs` + behavior-pinning tests
- Bug fixes: write the failing test first, then fix (TDD)
- Docs & specs: docs follow the confers skeleton; behavior changes update `specmark/specs/` in the same batch

### 🐛 Bug Reports and PRs

- Bug reports: open a [GitHub Issue](https://github.com/Kirky-X/alchemize/issues) with a minimal reproduction file (sanitized), `alch --version`, and expected vs. actual output.
- Feature suggestions: open an issue labeled `enhancement`; proposals touching network egress or credential handling should read the [Security doc](docs/en/SECURITY.md) first.
- PRs: Conventional Commits (feat/fix/test/docs/chore); local gates before submitting: `cargo fmt --check` + `cargo clippy --all-features` with no new warnings + all three test tiers green.

---

## 📋 Changelog

Release history and the [roadmap](docs/en/CHANGELOG.md#roadmap) live in [docs/en/CHANGELOG.md](docs/en/CHANGELOG.md).

---

## 📄 License

Apache-2.0 ([LICENSE](LICENSE)). Every source file carries an SPDX header; files migrated from other repositories keep their original copyright year ranges.

---

## 🙏 Acknowledgments

The core-dependency and special-thanks lists live in [Changelog · Acknowledgments](docs/en/CHANGELOG.md#acknowledgments).

---

## 📞 Contact & Support

<table style="width:100%; max-width: 600px">
<tr>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/alchemize/issues"><b style="color:#991B1B">Issues</b></a><br>
<span style="color:#64748B">Report bugs and problems</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/alchemize/security/advisories"><b style="color:#1E40AF">Security Advisories</b></a><br>
<span style="color:#64748B">Private vulnerability reports</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/alchemize"><b style="color:#1E293B">GitHub</b></a><br>
<span style="color:#64748B">Browse the source</span>
</td>
</tr>
</table>

---

## ⭐ Star History

[![Star History Chart](https://api.star-history.com/svg?repos=Kirky-X/alchemize&type=Date)](https://star-history.com/#Kirky-X/alchemize&Date)

If this project helps you, please consider giving it a ⭐️!
