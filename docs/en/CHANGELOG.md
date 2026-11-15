# Changelog

All notable changes to alchemize are recorded in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versioning follows [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-11-15

### Added
- **Conversion engine registry**: the `Converter` port + priority fallback chain; `metadata.converter` records the actual engine, `quality` labels `api`/`local_full`/`local_fallback` honestly across three tiers; full-chain failure returns per-engine attempts details.
- **Document formats**: pdf (pdf-extract local text + TOC removal/structure heuristics), docx/pptx/xlsx/xls/ods (native XML + calamine), odt/odp/rtf, html/htm, epub, csv/tsv (delimiter auto-detection), txt/text/log/md and ~45-extension code fencing, ipynb, tex/latex (pandoc probe + fenced fallback), recursive zip container (nested zip explicitly unsupported).
- **anydoc backend** (feature `anydoc`): full office formats including legacy binary Office (doc/ppt/…), priority 15 outranking native parsing; images exported in document order through the in-house GFM renderer.
- **MinerU cloud PDF** (feature `pdf-cloud`): Agent/Precision dual clients selected by file size, limiteron polling backoff, tiered response size caps, SSRF validation (scheme/loopback/private range/CGNAT/link-local/hostname allowlist/signature); endpoint configured via `ALCHEMIZE_MINERU_URL`, local first with cloud fallback (priority 5 chain tail).
- **Media conversion**: images (VisionProvider injection or metadata degradation), audio/video (ffmpeg track extraction → transcription chain), subtitles (native srt/vtt/ass/ssa parsing), video frame dedup, video processing modes (Gpu/AudioOnly/Disabled).
- **Local ASR transcription** (feature `audio`): the vecboost facade (Qwen3-ASR ONNX trio + silero VAD); a missing model bundle reports Unavailable honestly without auto-downloading; output is VAD-segmented text + timestamps, explicitly declared as not speaker identification; a Whisper-compatible API backend is also available (activated via explicit injection).
- **CLI `alch`**: `convert` (-o/--engine/--assets dir|inline/--no-remote/--format), `doctor` (per-engine availability diagnostics), `formats` (supported-extension list); semantic exit codes 0/1/2/3.
- **Multi-encoding support**: UTF-8 direct read → GBK/Big5/Shift-JIS/EUC-JP/ISO-2022-JP/EUC-KR/windows-1252 probing (CJK priority) → lossy UTF-8 fallback.
- **Secret leak prevention**: `MineruConfig::api_token` redaction (unit-test pinned).


- **Diagnostics for non-enabled formats (feature hint)**: under default/trimmed builds, `alch convert x.pdf` now prints on stderr `unsupported format: pdf: requires the 'pdf' feature; this build has it disabled, rebuild with --features pdf` (exit code stays 2; stdout and the error-enum shape unchanged; the extension is first sanitized of control characters and bidi controls). The library side adds the additive export `alchemize::domain::feature_map`: the `EXTENSION_FEATURES` mapping table (single source of truth, covering only extensions actually declared by engines), `features_for_extension` / `missing_features` (data surface) and `enabled_features` (compile-time list); mapping reliability is anchored by triple machine guards (single-feature binding, full reverse coverage, Cargo `full` set equality).
- **Isolation-matrix behavior verification**: each cell of the CI feature-isolation job now runs a three-step narrow build — downstream dependency-edge check (`cargo check --lib`, no cli) → all lib tests for that feature → that feature's golden behavior subset (golden cases are now gated by format feature; proptest still runs only under full); plus a `cli,text` bin baseline and a `feature_hint` non-empty regression entry. Coverage grows substantially while dropping the whole-tree `--all-targets` compile — cost down, not up.
- **PDF decompression-bomb protection**: vendored `lopdf 0.42.0` fork (`third_party/lopdf`, wired via `[patch.crates-io]`, MIT shipped along) adds a 64 MiB per-stream output cap to FlateDecode/LZWDecode stream decompression (aligned with the container family's `MAX_ENTRY_BYTES`); a single choke point covers all parse-phase ObjStm/XRefStm and pdf-extract content-stream/ICC-stream paths; `decompress_predictor` gains DecodeParms geometry guards; the fork's thread-local violation flag lets the host reject explicitly after pdf-extract swallows the error (`AlchemizeError::Pdf`); the converter and CLI verified end to end that a 100 MB inflation bomb is rejected (peak RSS 79 MB).
- **PDF fallback tier switches to lopdf 0.47 direct and retires the fork**: the `pdf-extract` dependency and the `third_party/lopdf` vendor fork are removed; the supply chain returns to the single crates.io upstream `lopdf 0.47`; fallback-tier text extraction moves to lopdf's own `extract_text_chunks_with_limit` (64 MiB per page) plus a new 256 MiB whole-document cumulative text budget (a page-farm DoS guard); the image path uses `decompressed_content_with_limit` (64 MiB per stream); decompression errors propagate natively from lopdf, mapped at the type level to `AlchemizeError::Pdf` (replacing the thread-local flag), and extraction segments wrap `spawn_blocking` to match the crate's convention. A differential benchmark (Apple EULA / arxiv paper / fork fixture corpora) judged the fallback tier's quality acceptable.
- **MCP tool surface** (feature `mcp`): the `alch-mcp` binary exposes convert/doctor/formats as MCP tools via sdforge (rmcp, stdio).
- **html-deep native engine** (feature `html`): a read-only DOM → Markdown iterative state machine — empty-output guard, iterative DOM against stack blowup, honest degradation; optional main-content extraction (dom_smoothie, `--extract-main`) and link absolutization (`--base-url`); pages containing `<math` tags auto-route to htmd for fidelity, and `$..$` inline TeX spans are preserved as-is. The convert endpoint gains `--base-url`/`--extract-main`.
- Documentation suite (seven topical documents) and copyright headers for all source files (Apache-2.0 SPDX).
- Document skeleton aligned with confers: all seven documents map one-to-one onto the confers section sequence (overview → design principles → system architecture → module layout → data flow → security design → performance design → related documents etc., per document in its "📋 目录" / Table of Contents).
- Documentation navigation section: the README.md "📚 文档" table and the README_EN.md "Docs" table.

### Security
- All MinerU download URLs pass `ValidatedFileUrl` SSRF validation (HTTPS-only, loopback/RFC1918/CGNAT/link-local/IPv4-mapped-IPv6 denial, hostname allowlist, presignature requirement).
- Tiered remote response size limits (64KB/8KB/50MB/50MB, 8MB preallocation cap).
- Credentials pass only through environment variables; no built-in secrets, no encrypted-storage requirement.

### Quality
- Tests: default tier 317 cases, `--features full` tier 396 cases, all passing.
- Coverage: 90.49% (gate ≥90%).
- Real-file suite: 12 tests over 9 in-repo fixtures and 1 external video source; the verification surfaced and fixed a CSS-leak-in-HTML-output defect.
- Spec-driven delivery: the whole migration ran on specmark specs (five capability-domain delta specs + 24/24 tasks).

### Acknowledgments
- [firecrawl/anydoc](https://github.com/firecrawl/anydoc) (MIT) — the office full-format backend dependency.
- [PDFMathTranslate](https://github.com/PDFMathTranslate/PDFMathTranslate) (AGPL-3.0) — design ideas absorbed only (layout region classification, image-page OCR fallback, paragraph reassembly); zero code copied.
- mnemis (own project, not public) — source of the migrated converter skeleton.

### Breaking Changes
- **The `alch-mcp` binary folds into the `alch mcp` subcommand**: `[[bin]] alch-mcp` and `src/bin/alch_mcp.rs` are deleted; `sdforge::mcp::serve_stdio` logic moves into `alch mcp` (feature `mcp`, now implying `cli`). The tool surface (convert/doctor/formats), stdio transport, and logging contract (stdout carries JSON-RPC, all logs go to stderr) are unchanged; MCP client configs change from `"command": "alch-mcp"` to `"command": "alch", "args": ["mcp"]`.
- **default narrowed to `["cli", "text"]`**: the default build compiles only the CLI skeleton and plain-text/code-fencing conversion; converters and dependencies for PDF/docx/html etc. are all feature-gated. Without the matching feature, `alch convert x.pdf` has no candidate engine and returns `unsupported format` (exit code 2). Either pass explicit `--features pdf,docx,...`, or `--features full` to restore full capability. `[[bin]] alch` gains `required-features = ["cli"]` accordingly.
- **15 new format features** (previously these converters compiled unconditionally, or `html` contained only the deep chain): `text`/`csv`/`html`/`pdf`/`docx`/`pptx`/`odt`/`odp`/`epub`/`ipynb`/`zip`/`latex`/`pandoc`/`media`/`heic`. `html` now contains both the htmd base chain and the native deep chain; without the `html` feature, html conversion is entirely unavailable. Existing feature adjustments: `pdf-local = ["pdf", ...]`, `audio = ["media", ...]`, `legacy-office` gains quick-xml/zip/tempfile, the media family unifies under `media`, `heic` becomes standalone.
- **All dependencies made optional**: apart from the 9 core crates (`serde`/`serde_json`/`thiserror`/`tracing`/`tokio`/`async-trait`/`anyhow`/`base64`/`sha2`), every dependency is `optional = true` and activated only through a feature's `dep:` reference; declarations unified as `default-features = false` + explicit features.
- **Version upgrades**: `base64 0.23`, `sha2 0.11`, `image 0.25`, `pdfium-render 0.9（`scaled_font_size` replaces `font_size`; extraction of rotated/scaled text may differ slightly from 0.5）`（bindings pick `pdfium_6295`; runtime e2e awaits a newer pdfium binary）, `zstd 0.14`, `icu_properties 2.4`; `rusqlite` stays at `0.39` (`0.41` hard-conflicts with the `libsqlite3-sys <0.38` requirement of limiteron→dbnexus→sqlx); everything else `cargo update`d to latest.
- **Cargo features renamed to capability names and split per format**: 8 feature adjustments — 6 renames + 2 splits; the `full` umbrella membership synced to the new 14-name set; no old-name aliases kept, old `--features` combinations must be updated to the new names.
| Old feature | New feature |
| --- | --- |
| `html-deep` | `html` |
| `pdfium` | `pdf-local` |
| `mineru` | `pdf-cloud` |
| `asr` | `audio` |
| `libreoffice` | `legacy-office` |
| `eml` | `email` |
| `containers` (split) | `tar` / `gzip` / `xz` / `zstd` / `sevenz` (`.tgz` needs `tar`+`gzip`) |
| `data` (split) | `parquet` / `sqlite` / `bibtex` |
- The `full` umbrella updates to: `pdf-cloud`+`pdf-local`+`audio`+`email`+`legacy-office`+`html`+`tar`+`gzip`+`xz`+`zstd`+`sevenz`+`parquet`+`sqlite`+`bibtex` (14; later gating changes expand it to the 28-conversion-feature full set).
- Split benefit accounting: single-format builds drop 6–9 dependency nodes (container family) and 10–31 (data family) versus the old bundles, with cold-build time down 2%–17%; `sqlite`/`bibtex` benefit clearly (15%–17%), `parquet` about 3%, and `gzip` gains zero on the dependency tree (flate2 was already in it — the benefit is in compile surface/binary size).
- Degradation semantics for the `.tar.gz` combined spelling: with only `gzip` enabled, `.tar.gz` goes through gz single-stream decompression and the inner tar bytes degrade to an attachment (content preserved); `.tgz` converts only when both `tar` and `gzip` are enabled, otherwise unsupported.
- **SQLite Path input switched to a private temporary copy**: all inputs are staged to a `tempfile` private copy (0600, random name, exclusive creation) before opening, cutting off the attacker same-directory `-wal`/`-shm` injection vector (RUSTSEC-2026-0459). Semantic impact: Path inputs no longer read un-checkpointed transactions from an accompanying `-wal` (previously read when opened in place); staging requires TMPDIR to fit a temporary copy as large as the database.

### Fixed
- `MineruConfig` hand-written `Debug`: fixes the derived Debug printing `api_token` in plaintext (Display/serde were already redacted); a unit test pins the Debug path.
- Factual corrections to README.md: the PDF fallback chain order (previously written with MinerU at the head; in reality the local chain is first and MinerU at the tail); the exit-code list gains `1` (other errors).
- gz/xz/zst wrappers now preserve the inner conversion's `assets` and `metadata`: the original implementation took only the markdown, discarding images extracted from inner docx/odt and leaving dangling `images/...` references.
- Ragged CSV rows now widen the table (synthesized `column_N` headers) instead of truncating; containers and zip uniformly filter `._*`/`__MACOSX` metadata entries; `MinerUHttpConfig` gets a hand-written redacting `Debug`.
- Unified fence-injection protection: text/code/ipynb/eml/pandoc/LaTeX/GFM-render and container/zip paths all switch to `fence_for` (lengthens by the longest backtick run in the body, tolerates lone CR/CRLF, tolerates ≤3-space indentation), preventing untrusted content from closing code blocks early.

### Changed
- **In-house dependency upgrades to stable and loosened to minor precision**: `confers 0.6.0` (transitive), `trait-kit 0.5.0`, `sdforge 0.5.0`, `inklog 0.3.0`, `limiteron 0.3.1`, `vecboost 0.3.0` — declarations change from rc full-version pins (e.g. `0.5.0-rc.14`) to minor precision (`"0.5"`/`"0.3"`), with no public-API breakage.
- **`vecboost` becomes a crates.io registry dependency**: the path dependency `../vecboost` is removed (path dependencies are rejected by `cargo publish`); compiling the `audio` feature no longer requires a sibling repo directory.
- **Spreadsheet parser removed**: xlsx/ods support and the `calamine` dependency are removed wholesale — its materialization surface (per-cell shared-string cloning, styles/theme, ODS repeats and `text:c` expansion) cannot be bounded by pre-validation on untrusted input. xlsx/xls/xlsb/ods now always return `UnsupportedFormat`.
- **Container converter construction surface narrowed**: tar/tgz/gz/zst/7z keep only `with_registry_slot(slot)`; nesting depth switches to per-conversion-chain task-local counting (concurrent conversions do not interfere); with the slot unbackfilled, entries degrade to attachments and `metadata.registry_wired=false` is recorded (previously a hard error).
- `default_converters` gains a container registry slot parameter; `AlchemizeBuilder::build()` backfills automatically.
- **CLI rewritten on sdforge**: convert/doctor/formats become `#[forge]` endpoints (`src/forge_app.rs`), with `alch` driven via CliBuilder + a custom runner; exit codes 0/1/2/3, positional arguments, stdout/stderr shapes, and the `-o`/`--no-remote`/`--format` contracts are all preserved (tests/cli.rs green with zero changes). MSRV 1.91 → 1.97.1 (required by sdforge); the direct clap dependency is removed (sdforge brings its own).
- Documentation wording aligned to code facts: the domain-layer IO exceptions (`Input::to_bytes` disk read, `DirSink` disk write), the doctor/availability semantics (unregistered engines do not appear; media dependency absence degrades at conversion time), and the module tree gains engines/shared/subtitle_converter.

### Removed
- The spreadsheet parsers `xlsx`/`ods` (the `calamine` dependency), `XlsxConverter`, and their fixtures/benchmark rows.
- The container converters' `new(registry)` / `with_arc(registry)` public constructors (the test paths remain as `#[cfg(test)]`).


[Unreleased]:
[Unreleased]: https://github.com/Kirky-X/alchemize/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Kirky-X/alchemize/releases/tag/v0.1.0

## Roadmap

<table style="width:100%; border-collapse: collapse">
<tr><th style="text-align:center">Status</th><th style="text-align:left">Area</th><th style="text-align:left">Items</th></tr>
<tr><td align="center">✅</td><td>Conversion engines</td><td>registry fallback chain, full document formats, code fencing, recursive zip, multi-encoding detection</td></tr>
<tr><td align="center">✅</td><td>Optional backends</td><td>self-hosted office deep parsers (docx/odt/odp/pptx), MinerU cloud PDF (SSRF guards + limits), vecboost local ASR</td></tr>
<tr><td align="center">✅</td><td>Media pipelines</td><td>vision port, ffmpeg track extraction, native subtitle parsing, frame dedup, VAD-segmented transcription</td></tr>
<tr><td align="center">✅</td><td>CLI & diagnostics</td><td>sdforge-wrapped (CLI+MCP dual surface), convert/doctor/formats, semantic exit codes, engine preference & asset modes</td></tr>
<tr><td align="center">✅</td><td>Docs & compliance</td><td>seven-doc suite (structure aligned with confers), bilingual README, SPDX headers 59/59, coverage 90.49%</td></tr>
<tr><td align="center">✅</td><td>Supply chain</td><td>anydoc dependency removed, zip is 9.4.1 only; cargo audit vulnerabilities at zero (unmaintained notices remain)</td></tr>
<tr><td align="center">🔲</td><td>Release</td><td>first crates.io publish + crates.io/docs.rs badges</td></tr>
<tr><td align="center">🔲</td><td>CI & quality</td><td>GitHub Actions three-tier matrix + codecov coverage badge</td></tr>
<tr><td align="center">🔲</td><td>Performance</td><td>criterion benchmarks → real data for a PERFORMANCE doc</td></tr>
<tr><td align="center">🔲</td><td>ASR real pipeline</td><td>run the real-model e2e once the model bundle is available (currently #[ignore])</td></tr>
</table>

## Acknowledgments

### 🌟 Core Dependencies

- [pdf-extract](https://github.com/jrmuizel/pdf-extract) (MIT) — PDF text-layer extraction.
- [quick-xml](https://github.com/tafia/quick-xml) (MIT/Apache-2.0) — OOXML/ODF/OMML event streams.
- [sevenz-rust2](https://github.com/hasenbanck/sevenz-rust2) (Apache-2.0) plus [tar](https://github.com/alexcrichton/tar-rs)/[flate2](https://github.com/rust-lang/flate2)/[xz2](https://github.com/alexcrichton/xz2-rs)/[zstd](https://github.com/gyscos/zstd-rs) (MIT/Apache-2.0) — archive containers.
- [libchm](https://github.com/trypsynth/libchm) (MIT) — CHM compiled-help parsing (pure Rust, with LZX decompression).
- [cfb](https://github.com/mdsteele/rust-cfb) (MIT) — OLE Compound File reading (the Outlook .msg container layer).
- [parquet](https://github.com/apache/arrow-rs) (Apache-2.0), [rusqlite](https://github.com/rusqlite/rusqlite) (MIT), [biblatex](https://github.com/typst/biblatex) (MIT) — structured data engines.
- [mail-parser](https://github.com/stalwartlabs/mail-parser) (Apache-2.0/MIT) — EML parsing.
- [LibreOffice](https://www.libreoffice.org/) (MPL-2.0) — legacy binary Office conversion (optional external tool).
- [pandoc](https://pandoc.org/) (GPL-2.0+) — LaTeX/org/rst/asciidoc markup conversion (optional external tool).
- [vecboost](https://github.com/Kirky-X/vecboost) (own project) — the Qwen3-ASR ONNX + silero VAD local transcription facade.
- Excellent open-source crates: [pdf-extract](https://crates.io/crates/pdf-extract), [quick-xml](https://crates.io/crates/quick-xml), [aspasia](https://crates.io/crates/aspasia), [rbook](https://crates.io/crates/rbook), [htmd](https://crates.io/crates/htmd), and more.

### 💝 Special Thanks

- [PDFMathTranslate](https://github.com/PDFMathTranslate/PDFMathTranslate) (AGPL-3.0) — design ideas absorbed only (layout region classification, image-page OCR fallback, paragraph reassembly); zero code copied; alchemize remains Apache-2.0.
- mnemis (own project, not public) — source of the migrated converter skeleton.
- [confers](https://github.com/Kirky-X/confers) — the reference baseline for documentation style and engineering conventions.
