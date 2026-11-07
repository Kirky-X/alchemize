# Alchemize

Transmute any document into Markdown (Rust, Apache-2.0).

中文版本: [README.md](README.md)

## Supported formats

| Family | Formats | Engine |
| --- | --- | --- |
| Documents | pdf, docx/pptx/xlsx (+xls/ods), odt/odp/rtf, legacy binary Office (doc/ppt/…, feature `anydoc`), html/htm, epub, csv/tsv, txt/text/log/md, zip (recursive), ipynb, tex/latex (pandoc) | registry fallback chain |
| Code/config | py/rs/js/ts/go/java/c/cpp/json/yaml/toml/xml/sh and ~45 extensions total | fenced code blocks (language-tag mapping) |
| Images | png/jpg/jpeg/gif/bmp/webp/tiff/svg | VisionProvider or metadata degradation |
| Audio | wav/mp3/ogg/flac/aac/m4a/wma | vecboost transcription (feature `asr`) or metadata degradation |
| Video | mp4/avi/mkv/mov/wmv/flv/webm/m4v | ffmpeg track extraction → audio chain |
| Subtitles | srt/vtt/ass/ssa | native parsing (timestamped segments) |

## Usage

```bash
# Convert (stdout)
alch convert paper.pdf

# Write file + engine preference + asset mode
alch convert report.docx -o report.md --engine anydoc --assets dir|inline

# Engine diagnostics / supported formats
alch doctor
alch formats
```

Exit codes: `0` success (including degraded output); `1` other errors; `2` unsupported format; `3` entire chain failed (stderr lists each engine's failure reason).

## Architecture

Single-crate DDD three layers: `domain` (types and rules, no network/process IO) → `application` (converter registry with a priority fallback chain + ports) → `infrastructure` (per-format converters + MinerU/ASR backends).

The registry tries engines for an extension in descending priority order. PDF example: anydoc (`local_full`, priority 15) → pdf-extract local text + TOC pruning/structure heuristics (`local_fallback`, priority 10) → MinerU cloud (`api`, priority 5, registered only when configured, tried after the local chain fails). The quality tier and the engine that actually produced the result are recorded in `metadata` and the stderr summary line.

## Docs

| Doc | Description |
|------|------|
| [🏗️ Architecture](docs/ARCHITECTURE.md) | Design principles, module layout, fallback-chain mechanics |
| [📘 API Reference](docs/API_REFERENCE.md) | Library API, CLI surface, environment variables |
| [📖 User Guide](docs/USER_GUIDE.md) | Complete tutorial from install to advanced use |
| [❓ FAQ](docs/FAQ.md) | Frequently asked questions |
| [📋 Changelog](docs/CHANGELOG.md) | Release history |
| [🤝 Contributing](docs/CONTRIBUTING.md) | Dev environment, testing, commit conventions |
| [🔒 Security](docs/SECURITY.md) | Trust boundaries, SSRF guards, vulnerability reporting |

## Features

| feature | Pulls in | Notes |
| --- | --- | --- |
| `mineru` | MinerU cloud PDF engine | endpoint via `ALCHEMIZE_MINERU_URL` |
| `anydoc` | office full-format backend (firecrawl/anydoc, MIT) | includes legacy binary Office; image bytes exported in document order |
| `asr` | vecboost transcription + Whisper-compatible API | in-house [vecboost](https://github.com/Kirky-X/vecboost) (Qwen3-ASR ONNX + silero VAD) |

`full` = all of the above. Default features have zero external-service dependencies.

## Transcription (feature `asr`)

Audio/video transcription calls the in-house vecboost `transcribe` facade (fully local ONNX inference):

- Model bundle: point `ALCHEMIZE_TRANSCRIBE_MODELS` at a directory; defaults to probing `~/.cache/alchemize/models/transcribe/`. Layout: `qwen3-conv_frontend.onnx`, `qwen3-encoder.int8.onnx`, `qwen3-decoder.int8.onnx`, `silero_vad.onnx`, `tokenizer/` (same contract as vecboost). When missing, the engine reports Unavailable with retrieval guidance — no silent behavior, no automatic network fetches.
- Runtime dynamic library: `ORT_DYLIB_PATH` points at the onnxruntime library.
- Output is VAD-segment text + timestamps; segmentation comes from silero VAD (voice activity detection) and is **not speaker diarization** — no SpeakerN attribution is provided.
- The Whisper-compatible cloud backend activates only with an explicitly injected configuration.

Release ordering constraint: vecboost is published to crates.io before alchemize (currently a path dependency).

## License

Apache-2.0. Acknowledgements and boundaries:

- [firecrawl/anydoc](https://github.com/firecrawl/anydoc) (MIT) — office format backend dependency.
- [PDFMathTranslate](https://github.com/PDFMathTranslate/PDFMathTranslate) (AGPL-3.0) — **design ideas absorbed only (layout region classification, image-page OCR fallback, paragraph reassembly); zero code copied**; alchemize remains Apache-2.0.
- [mnemis](https://github.com/Kirky-X/mnemis) (own project) — source of the migrated converter skeleton.
