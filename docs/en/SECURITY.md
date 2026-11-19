# 🔒 Alchemize Security

Alchemize's security model: **untrusted input, restricted egress, minimal secrets**. This document covers the support policy, trust boundaries, attack surfaces and countermeasures, the vulnerability reporting process, and best practices — every item corresponds to the source implementation.

## 📋 Table of Contents

- [Supported Versions and Maintenance Policy](#-supported-versions-and-maintenance-policy)
- [Vulnerability Reporting](#-vulnerability-reporting)
- [Security Design Overview](#️-security-design-overview)
- [Security Best Practices](#-security-best-practices)
- [Security Contact](#-security-contact)
- [Related Documents](#-related-documents)

## 📌 Supported Versions and Maintenance Policy

| Version | Status | Supported until |
| --- | --- | --- |
| 0.1.x | ✅ Maintained | Before 0.2.0 ships |
| < 0.1.0 | ❌ Unsupported | — |

### Minimum Supported Rust Version (MSRV)

Rust **1.97.1** (the `rust-version` in `Cargo.toml`, edition 2024; the floor is set by the sdforge MSRV). Security fixes do not raise the MSRV.

### Dependency Security

```bash
cargo audit              # run the security audit
cargo update -d          # update the advisory database
```

Key dependency policy: ZIP container parsing uses `zip` 9.x (the extract-family paths affected by RUSTSEC-2026-0367/0368/0369/0429 are outside the used surface; pinned to 9.5.1); TLS goes through rustls; `vecboost` comes in as a crates.io registry dependency (0.3).

### Dependency Approval Process

A new dependency must satisfy: default features off (`default-features = false` preferred), no transitive network stack (unless the dependency itself is the network capability), zero new `cargo audit` advisories, and reasonable maintenance activity. Dependency changes are committed in the same batch as Cargo.lock.

### Known and Accepted Risks

| Risk | Status | Mitigation |
| --- | --- | --- |
| `with_max_file_size` consumption point | Oversized inputs are already rejected at the facade before dispatch, and it doubles as the ZIP nested-entry cap | Covers registry dispatch and the zip branch; tar/tgz/7z nesting is fixed at 64 MiB/entry + 256 MiB/archive, gz/xz/zst single-stream decompression at 64 MiB — none bound by this value; MinerU precision has its own 200MB cap |
| libsqlite3-sys 0.37 standing advisory RUSTSEC-2026-0459 (C SQLite CVE-2026-119375, medium) | A transitive dependency of the `sqlite` feature; standing, not new | Cleared by cargo-audit; the rusqlite upgrade is an independent dependency change, handled with the next dependency batch |

### Supply Chain and Gates

- CI gates: fmt/clippy/test/coverage/audit/deny on push and PR (`.github/workflows/ci.yml`).
- `Cargo.lock` is committed to lock the dependency graph; dependency upgrades must pass `cargo audit`.
- Every source file carries the two-line `SPDX-License-Identifier: Apache-2.0` header (59/59), so license scanners can identify file-level ownership mechanically.
- The specmark spec directory is not committed (gitignored); the main specs serve as the local source of truth.
- The CLI/MCP wrapping frameworks sdforge (MIT, crates.io version pinned 0.5.1) and rmcp (exclusive to the `mcp` feature) are covered by `cargo audit` as well.
- The html-deep engine (feature `html`) is this repo's own iterative state machine (Apache-2.0; differential verification alignment records in repo history), depending on scraper 0.29, dom_smoothie 0.19, icu_properties 2.4 (all in the dependency tree); `--base-url` rewrites links only when explicitly provided.
- lopdf 0.47 (crates.io direct, MIT): decompression-bomb protection sets explicit limits at the call sites — document parsing and per-page text extraction are capped at 64 MiB per page/stream (`LoadOptions::with_max_decompressed_size` + `extract_text_chunks_with_limit`), image streams use `decompressed_content_with_limit` at 64 MiB, plus a 256 MiB whole-document cumulative text budget (rejection peaks at roughly the budget + one page's 64 MiB); over-limit is rejected explicitly as `AlchemizeError::Pdf`. Bomb-class errors (`MemoryLimitExceeded`) propagate natively from lopdf with no swallowed path; corrupted streams (non-bomb decoding errors) fall back to producing garbled text from the original compressed bytes — data integrity is backed by the output quality tier and poses no memory risk.

## 🐛 Vulnerability Reporting

### How to Report

**Do not** report security vulnerabilities through public GitHub issues. Use a GitHub private security advisory: the repo's **Security tab → Report a vulnerability**.

### What to Include

- A crafted input file (sanitized) or command-line reproduction steps
- `alch --version` / `alchemize::VERSION` and the compiled features
- Impact assessment: disclosure/denial of service/bypass surface

### ⏱️ Response Commitments

- **Initial assessment**: acknowledge receipt and give a first assessment within 14 days
- **Fix**: scheduled by severity (CRITICAL hotfixed immediately, HIGH with the next patch release, MEDIUM/LOW with regular releases)
- **Release**: fixes ship with a new version plus an advisory, crediting the reporter (unless anonymity is requested)

### Severity Levels

| Level | Criteria |
| --- | --- |
| CRITICAL | Remote code execution, SSRF piercing the allowlist, credential leakage into logs/serialization |
| HIGH | Local parser memory corruption, silent swallowing of degradation errors, privilege boundary bypass |
| MEDIUM | Denial of service (resource exhaustion), error messages leaking sensitive paths |
| LOW | Documentation inaccuracies, non-sensitive information disclosure |

### Disclosure Timeline

Details are disclosed only after the fix ships; the advisory also updates this page's "Known and Accepted Risks" table.

## 🛡️ Security Design Overview

### Input Parsing Security

Document input is **fully untrusted** (email attachments/download pages/crawler sources). Parsing surfaces and countermeasures:

| Attack surface | Parser | Countermeasure |
| --- | --- | --- |
| zip / docx / pptx / epub (ZIP containers) | the `zip` crate (deflate) | Declarative dependencies + Cargo.lock pinning; parse failures return structured `Zip`/`Xml` errors instead of panicking |
| pdf | `lopdf` 0.47 (crates.io direct; pdfium tier in the row below) | Parse failures return `Pdf` errors; TOC removal and structure heuristics affect only output quality; 64 MiB per-page/per-stream decompression caps + a 256 MiB whole-document cumulative text budget, over-limit rejected explicitly, blocking decompression bombs; lopdf propagates decompression errors natively (no swallowed path) |
| xml | `quick-xml` | No external entity parsing path |
| tex/latex | pandoc subprocess | PATH probing + parameterized invocation only, no shell interpolation; the fenced fallback path spawns no subprocess |
| Resource consumption of crafted inputs | every parser | The zip facade rejects nested recursion (unbounded depth); the HTML engine iterates throughout (DOM depth never enters the call stack); response reading is memory-bounded |

### SSRF Protection

All network egress is concentrated in the MinerU backend (feature `pdf-cloud`); download URLs must pass `ValidatedFileUrl` validation (`validated_url.rs`). The MCP surface (the `alch mcp` subcommand) uses stdio transport — **the transport surface has no network egress** (the convert tool can reach the network when the `pdf-cloud` feature and endpoint env var are both present — that is business capability, not transport); HTTP/gRPC/WS protocols are not enabled:

- Only the HTTPS scheme is allowed;
- Loopback (`127.0.0.0/8`, `::1`, `localhost`), `0.0.0.0/8`, and broadcast addresses are denied;
- RFC1918 private ranges (`10/8`, `172.16/12`, `192.168/16`), CGNAT (`100.64/10`, including the cloud metadata endpoint `100.100.100.200`), and link-local (`169.254/16`) are denied;
- IPv4-mapped/compatible IPv6 (`::ffff:a.b.c.d` etc.) is classified by its embedded IPv4, preventing bypass;
- The hostname allowlist defaults to `mineru.net` / `cdn.mineru.net` / `api.mineru.net` (`DEFAULT_MINERU_ALLOWLIST`);
- Violations return `AlchemizeError::SecurityViolation { kind, message }`; kinds in `SecurityViolationType` (including `MissingSignature` for a missing presignature).
- **Automatic redirects disabled**: all three MinerU clients set `reqwest::redirect::Policy::none()`, so a 3xx to an internal address is never followed.
- **Connect-time per-IP revalidation**: `SsrfSafeResolver` revalidates every resolved address against the same policy as parse-time validation just before connecting; a denied address is dropped and an empty result set rejects the connection (deny-by-default).
- **DNS rebinding defense**: address policy is checked both at URL validation time and at connection time; the connect-time recheck closes the rebind window between resolution and TCP connect.

### Response Size Limits

All remote response reading goes through the limited reader (`http_limit.rs`), tiered by purpose:

| Response type | Cap |
| --- | --- |
| Submission response | 64 KB |
| Error response | 8 KB |
| Polling response | 50 MB |
| Result response | 50 MB |

Preallocation is capped at 8 MB (no matter how large the declared Content-Length, only 8MB is preallocated; over-limit is rejected in a streaming fashion); over-limit returns `AlchemizeError::ResponseTooLarge { actual, limit }`.

### Credentials and Sensitive Information

- **Credential sources and delivery**: `MINERU_API_TOKEN` or `MineruConfig::api_token` (both become Bearer tokens, sent only to endpoints passing `validate_base_url`); no built-in secrets, no encrypted-storage requirement. The token is sent with requests to the configured base_url: `validate_base_url` constrains the host and scheme allowlist, but when a local deployment uses a plaintext http endpoint, the deployer bears the link risk. Container nesting has a separate chain-level 512 MiB cumulative decompression budget (counted per conversion chain); each layer is billed only after its decompression is materialized, so transient peaks can exceed it by one layer's 256 MiB.
- **Three-path redaction**: `MineruConfig::api_token` never appears in plaintext in `Debug` (hand-written impl), `Display`, or serde serialization (`skip_serializing`) — pinned by the `mineru_config_display_redacts_token` unit test across all three paths.
- **Error-path redaction**: on error paths the Authorization token and presigned URL parameters never enter `AlchemizeError`, tracing, or Display/Debug output — upstream error bodies and reqwest error text are scrubbed at every construction point (pinned by mock echo tests in `mineru/tests.rs`).
- **No secrets in logs**: tracing output contains only diagnostic fields such as engine name, priority, and failure reasons.

### No Automatic Networking Principle

- Missing ASR model bundle: reports `Unavailable` + acquisition guidance, **never auto-downloads** — model acquisition is the user's integrity and supply-chain decision.
- Whisper-compatible cloud backend: activated only via explicit injected configuration, never by default.
- Vision models: no built-in provider; library users must inject explicitly (no implicit cloud fallback).

### Output Trust Boundary

- **`ConvertResult.provenance`**: every successful conversion carries structured security provenance (`ProvenanceEnvelope`) — the full engine chain `engine_path` (skipped and failed steps included), the final engine `final_engine`, network egress `network_egress` (host only, never path/query/credentials), external child processes `external_tools`, local model bundles `model_bundles`, output-transform declarations `sanitization`, and the aggregated trust level `trust_level`.
- **TrustLevel semantics**: `self_contained` (pure local Rust parsing chain: no external process, no network); `mixed` (went through an external child process (pandoc/LibreOffice/sips/ffmpeg) or a local model bundle, but no network egress); `external_service` (data left this process, e.g. the MinerU cloud upload/download path).
- **Sanitization is the downstream consumer's responsibility**: alchemize does not sanitize the Markdown content itself — downstream pipelines (RAG, agent workflows) decide their own sanitize policy based on `trust_level` (e.g. disinfect `external_service` output before rendering, trust `self_contained` output).
- **Empty `SanitizationSummary` semantics**: an empty set means the engine did not declare the transform, NOT a guarantee that no transform happened. Only paths where the engine natively holds the information (pandoc/LaTeX fence fallbacks, html-deep explicit base_url link absolutization) populate the declaration.

## ✅ Security Best Practices

### For Library Users

- In untrusted environments use `AlchemizeBuilder::new().with_no_remote().build()` — zero network egress for the process.
- Prefer an isolated environment (container/sandbox) when processing untrusted files; this tool's parsing surface contains no network execution path, but deep defense against parser flaws still relies on isolation.
- Before printing configuration containing sensitive fields, confirm its Debug/Display are redacted; for custom types with secret fields, imitate `MineruConfig`'s hand-written implementation.

### Production Configuration Example

```bash
# Enable everything (cloud PDF + full office formats + local ASR)
cargo build --release --features full

# Required in production (precision cloud token)
export MINERU_API_TOKEN="<token>"

# Offline/compliance environments: leave ALCHEMIZE_MINERU_URL unset; the MinerU engine is never registered
alch convert --no-remote report.docx -o report.md
```

### Environment Variables

| Variable | Description |
| --- | --- |
| `MINERU_API_TOKEN` | Required in production when using the cloud Precision API (bearer token) |
| `ALCHEMIZE_MINERU_URL` | Registers the MinerU engine when set; unset means no cloud egress |
| `ALCHEMIZE_TRANSCRIBE_MODELS` / `ORT_DYLIB_PATH` | Local ASR paths; no network meaning |

### Hardening Checklist

- [ ] Untrusted environments: `--no-remote` / leave `ALCHEMIZE_MINERU_URL` unset
- [ ] `cargo audit` part of the upgrade flow
- [ ] Credentials injected into environment variables via a secret manager, never into shell history
- [ ] Re-review this page's "Known and Accepted Risks" table on upgrades
- [ ] Custom engines introduce no new network egress; when truly needed, reuse `ValidatedFileUrl` validation

## 📎 Security Contact

- Vulnerability reports: GitHub private security advisories (**Security → Report a vulnerability**), process above
- General questions: GitHub Issues (non-security defects)

## 📚 Related Documents

- [🏗️ Architecture](ARCHITECTURE.md) — where security design sits in the architecture
- [📖 User Guide](USER_GUIDE.md) — offline and privacy practices
- [🤝 Contributing](CONTRIBUTING.md) — contribution and submission process
- [❓ FAQ](FAQ.md) — the quick answer to "Is Alchemize secure?"
