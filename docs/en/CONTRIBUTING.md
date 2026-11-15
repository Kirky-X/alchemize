# 🤝 Alchemize Contributing Guide

Thank you for your interest in alchemize. This document covers the development environment, the TDD workflow, code standards, and the commit/PR process.

## 📋 Table of Contents

- [Welcome](#-welcome)
- [Environment Setup](#-environment-setup)
- [Development Workflow (TDD)](#-development-workflow-tdd)
- [Code Standards](#-code-standards)
- [Commit and PR Process](#-commit-and-pr-process)
- [Code of Conduct](#-code-of-conduct)

## 👋 Welcome

Contributions of any size are welcome: bug reports, documentation improvements, more tests, new engines. This project runs the specmark spec-driven process (proposal → apply → converge → archive); the capability-domain spec sources live in `specmark/specs/` — before changing behavior, check whether the matching capability-domain spec needs to be updated in the same batch.

## 🧰 Environment Setup

### Prerequisites

- **Rust 1.97.1+** (the `rust-version` in `Cargo.toml`, edition 2024; the floor is set by the sdforge MSRV)
- Components: `rustfmt` (formatting), `clippy` (static analysis), `cargo-llvm-cov` (coverage, optional)

```bash
rustup component add rustfmt clippy
cargo install cargo-llvm-cov   # optional: coverage measurement
```

### Fork and Clone

```bash
# After forking (GitHub UI):
git clone https://github.com/<you>/alchemize.git
cd alchemize
git remote add upstream https://github.com/Kirky-X/alchemize.git
```

### Finish Setting Up

```bash
# Install project dependencies and build
cargo build

# The full tier depends on vecboost (a registry dependency; no sibling repo needed)
cargo build --features full
```

Note: `audio` (included in `full`) works through the crates.io vecboost transcription facade; local ASR inference additionally needs the model bundle and `ORT_DYLIB_PATH` (see the `audio` row in the table above).

### Feature Flags

`default = ["cli", "text"]`: the default build has only the CLI skeleton and plain-text conversion; every other format is enabled on demand (full matrix in the README's "Feature Flags").

| feature | Pulls in | Notes |
| --- | --- | --- |
| `text` (default) | Plain-text/code-fencing converter | The only format enabled in the default build |
| Format family (15) | `csv`/`html`/`pdf`/`docx`/`pptx`/`odt`/`odp`/`epub`/`ipynb`/`zip`/`latex`/`pandoc`/`media`/`heic` | One feature per format; `html` contains both the htmd base chain and the native deep chain |
| `pdf-cloud` | MinerU cloud PDF engine | Registers only after the endpoint is configured via `ALCHEMIZE_MINERU_URL` |
| `pdf-local` | pdfium high-fidelity PDF (implies `pdf`) | Needs the pdfium dynamic library at runtime; degrades when missing |
| `audio` | vecboost transcription facade (implies `media`) | Needs the model bundle + `ORT_DYLIB_PATH` |
| `legacy-office` / `email` / `parquet` / `sqlite` / `bibtex` | Legacy Office / e-mail / structured data | Each independent |
| `tar`/`gzip`/`xz`/`zstd`/`sevenz` | Archive container family | `.tgz` needs `tar`+`gzip` |
| `mcp` | MCP tool surface (the `alch mcp` subcommand, rmcp stdio; implies `cli`) | Not in default |
| `cli` (default) | CLI skeleton (sdforge + inklog) | The required-features of `[[bin]] alch` |
| `full` | All 28 conversion features (excludes `cli`/`mcp`) | |

### Build and Test

```bash
cargo build                     # default tier: CLI + text conversion
cargo test                      # default tier (168 cases, core + text)
cargo test --features full      # full tier (912 passed, plus 5 ignored)
cargo test --all-features       # all-features tier (917 passed, plus 5 ignored)
```

Quality gates (enforced at two layers — the local pre-commit hook and GitHub CI; rules in `.pre-commit-config.yaml` and `.github/workflows/ci.yml`):

- **feature-isolation**: CI runs, per feature configuration, a three-step narrow build — downstream `cargo check --lib --no-default-features --features "<F>"` (no cli, exposing dependency edges) → all lib tests for that feature (including registered-present/absent assertions) → that feature's golden behavior subset (cases gated by format feature); plus a `cli,text` bin baseline and a feature_hint non-empty regression entry. Covers 29 configurations (28 single features + the `tar,gzip` combination for `.tgz`), one job looping sequentially over a shared target dir. Residual bin risk: only the `cli,text` baseline covers it; other cli combinations are carried by the full/all-features union.
- **Three-tier tests**: default / `--features full` / `--all-features` all green; fmt and `clippy -D warnings` with zero warnings.

```bash
pre-commit run --all-files  # local full run: fmt/clippy(-D warnings)/deny/typos/private-key detection
```

## 🔄 Development Workflow (TDD)

### Detailed Steps

```bash
# Sync upstream's main branch
git fetch upstream && git checkout main && git merge upstream/main

# Create a feature branch
git checkout -b feat/my-engine
# or a bugfix branch
git checkout -b fix/my-bugfix

# 1. Red: first write the failing test that exposes the defect / pins the new behavior
# 2. Green: write the minimum code to make the test pass
# 3. Refactor: improve the structure without changing behavior
# 4. Quality gates before committing
cargo fmt
cargo clippy --all-features
cargo test --features full

# When done, merge back to main (this repo prefers committing straight to main; a single-maintainer repo may push main directly)
git checkout main
git merge feat/my-engine
# Re-run the quality checks after resolving conflicts, then push the merge
git push upstream main
git branch -d feat/my-engine
```

### Test Requirements

- New behavior must come with a test that pins it; for bug fixes, write the failing test first, then fix.
- **Coverage gate ≥90%**: measured by `cargo llvm-cov --features full` (family-wide minimum 90 per 2026-11-15 directive). Currently measured 89.61% (release round); raise it with real tests — the CI coverage job fails until then. The diff-coverage gate stays ≥90% — no lowering.
- Golden-output changes (`tests/golden.rs`) must state the reason in the commit message.
- All three test tiers must pass: `cargo test` (default 168, the slimmed core + text tier), `cargo test --features full` (912 passed, plus 5 ignored), and `cargo test --all-features` (917 passed, plus 5 ignored, including cli/mcp).

## 📐 Code Standards

### Rust Best Practices

- **DDD three layers**: domain types and pure rules go in `domain/` (no network/process IO; file IO has only the two exceptions `Input::to_bytes` disk read and `DirSink` disk write); ports and the registry go in `application/`; concrete engines go in `infrastructure/`. New engines implement the `Converter` trait and are assembled in `infrastructure/engines.rs`.
- **Degradation is explicit**: an unavailable engine returns `Availability::Unavailable{reason}`; full-chain failure returns `AllBackendsFailed{ext, attempts}`; swallowing errors, masking failures with defaults, and labeling degraded results as high quality are all forbidden.
- **availability semantics**: engines that can degrade and still produce output always return `Available` (separating "can it produce" from "how good is the output"), otherwise the registry skips them as unavailable.

### Naming Conventions

- Engine `name()` uses lowercase with hyphens (`latex-source`, `docx-deep`, `mineru`), consistent with `--engine` and the `metadata.converter` values.
- Environment variables uniformly use the `ALCHEMIZE_` prefix (except test-only variables).

### Code Quality Requirements

- **Secret redaction**: any config type with sensitive fields must be redacted on the `Debug`/`Display`/serde paths with a unit test (see `MineruConfig::api_token` — note derived Debug does not redact; a hand-written impl is required).
- Every commit is fully working (straight-to-main preference): zero format diffs, zero new clippy warnings, both test tiers green.

### Documentation Standards

- Public items must have rustdoc (`//!`/`///`).
- **Copyright header**: every `.rs` file (src/ and tests/) must carry the two-line copyright header at lines 1-2 (before `//!`), with line 3 blank:

```rust
// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

```

- The license expression matches `Cargo.toml`'s `license` (currently Apache-2.0); files migrated from other repositories keep their original copyright year ranges (reflecting provenance honestly, e.g. `2025-2026`).
- Document layout follows the confers skeleton (emoji headings + 📋 table of contents + related-documents close-out); new documents follow this template.

## 📤 Commit and PR Process

### Commit Conventions

Conventional Commits (consistent with the existing history):

```
feat(infra): T014 MinerU client family migrated (9 files, 41 tests)
test(real): real-world file parsing suite (11 fixtures)
docs: add 7-doc suite (architecture/API/user/FAQ/changelog/contributing/security)
```

- Types: `feat` / `fix` / `refactor` / `test` / `docs` / `chore` / `perf` / `ci`.
- One focused change per commit; a specmark task is committed as soon as it completes (messages may carry the task number).
- Batched-commit convention: deps → config → src → tests → docs → chore, running the matching tests before each batch.

### PR Submission Standards

- Commit messages follow Conventional Commits.
- CI (if enabled) all green; local gates (fmt/clippy/both test tiers) all green.
- Spec changes (`specmark/specs/`) are committed in the same batch as behavior changes.

### Review Standards

- Are there tests pinning the behavior; are degradation paths explicit (attempts/quality/metadata.converter).
- Does it break a capability-domain spec (`specmark/specs/`) — a break is a block.

## 🤗 Code of Conduct

By participating in this project you agree to stay professional and respectful: address the matter, not the person; keep feedback focused on the code and documentation itself; harassment and demeaning speech are unacceptable. Detailed terms follow the spirit of the Contributor Covenant (this repo ships no separate file).

### 💝 Thank you for contributing to Alchemize!
