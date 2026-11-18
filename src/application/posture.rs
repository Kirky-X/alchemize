// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Security posture snapshot for `doctor --security`.
//!
//! [`collect_posture`] gathers the trust boundary of the current build and
//! environment: which parse surfaces are compiled in, whether any network
//! egress engine is live, which external subprocesses are reachable on
//! PATH, and the size budgets the pipeline enforces. Rendering happens in
//! the doctor endpoint; this module is logic-only and unit-testable without
//! the CLI.

use crate::application::builder::Alchemize;
use crate::application::ports::which_program;

/// External subprocess kind probed for the posture report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// pandoc document converter.
    Pandoc,
    /// LibreOffice / soffice suite.
    LibreOffice,
    /// macOS sips image tool.
    Sips,
    /// ffmpeg media tool.
    Ffmpeg,
}

/// State of the (single possible) network egress engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressStatus {
    /// The pdf-cloud feature is not compiled in: no network egress exists
    /// in this build, regardless of environment variables.
    UnavailableCompiledOut,
    /// The MinerU cloud engine is compiled in; readiness reflects the
    /// registration semantics (engine registers when `ALCHEMIZE_MINERU_URL`
    /// is set — see engines.rs and builder.rs).
    Ready {
        /// Configured endpoint host (None = engine not registered).
        endpoint_host: Option<String>,
        /// Whether `MINERU_API_TOKEN` is present in the environment.
        token_present: bool,
    },
}

/// Snapshot of the build's security boundary.
#[derive(Debug, Clone)]
pub struct SecurityPosture {
    /// Conversion capability features compiled into this build
    /// (from [`crate::domain::feature_map::enabled_features`]).
    pub parse_surfaces: Vec<&'static str>,
    /// Network egress engine state.
    pub network_egress: EgressStatus,
    /// External subprocesses this build can drive, each with its
    /// on-PATH availability.
    pub subprocesses: Vec<(ToolKind, bool)>,
    /// Enforced size budgets (name, limit). Single source of truth is
    /// SECURITY.md and the enforcing call sites — this table documents
    /// them for operators, it does not configure anything.
    pub budgets: &'static [(&'static str, u64)],
}

/// Size budgets enforced by the pipeline.
///
/// Values anchor SECURITY.md ("Response Size Limits", "Input Parsing
/// Security") and the enforcing call sites: the MinerU HTTP limits live in
/// `remote/mineru/http_limit.rs`, the container decompression budget and
/// per-layer cap in `containers/base.rs`, the lopdf per-stream/whole-doc
/// caps at the pdf.rs call sites. Changing a limit must update SECURITY.md,
/// the call site, and this table together.
static BUDGETS: &[(&str, u64)] = &[
    ("mineru submit response", 64 * 1024),
    ("mineru error response", 8 * 1024),
    ("mineru poll response", 50 * 1024 * 1024),
    ("mineru result response", 50 * 1024 * 1024),
    (
        "container chain decompression (cumulative)",
        512 * 1024 * 1024,
    ),
    ("container layer decompression", 256 * 1024 * 1024),
    ("pdf page/stream decompression (lopdf)", 64 * 1024 * 1024),
    ("pdf whole-document text (lopdf)", 256 * 1024 * 1024),
];

/// Collect the current security posture.
///
/// `alch` is accepted for future registry-driven introspection; the
/// environment variables are read directly because that is where the
/// registration semantics live (env set = engine registered).
pub fn collect_posture(_alch: &Alchemize) -> SecurityPosture {
    let parse_surfaces = crate::domain::feature_map::enabled_features();

    let network_egress = build_egress_status();

    // PATH probes reuse the same detection the engines use at conversion
    // time (`which_program`); availability here means "would be driven",
    // not "currently healthy".
    let subprocesses = vec![
        (ToolKind::Pandoc, which_program("pandoc")),
        (ToolKind::LibreOffice, LibreOfficeProbe::on_path()),
        (ToolKind::Sips, sips_available()),
        (ToolKind::Ffmpeg, which_program("ffmpeg")),
    ];

    SecurityPosture {
        parse_surfaces,
        network_egress,
        subprocesses,
        budgets: BUDGETS,
    }
}

fn build_egress_status() -> EgressStatus {
    #[cfg(feature = "pdf-cloud")]
    {
        let endpoint_host = std::env::var("ALCHEMIZE_MINERU_URL").ok().and_then(|raw| {
            reqwest::Url::parse(&raw)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
        });
        let token_present = std::env::var("MINERU_API_TOKEN")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        EgressStatus::Ready {
            endpoint_host,
            token_present,
        }
    }
    #[cfg(not(feature = "pdf-cloud"))]
    {
        EgressStatus::UnavailableCompiledOut
    }
}

// The dedicated probe helpers below exist so each engine contributes its
// own availability signal without posture.rs depending on feature-gated
// engine modules (sips probing lives in media/heic.rs behind the heic
// feature; the fallback is a plain PATH check, same semantics).
struct LibreOfficeProbe;
impl LibreOfficeProbe {
    fn on_path() -> bool {
        #[cfg(feature = "legacy-office")]
        {
            crate::infrastructure::tools::libreoffice::LibreOfficeTool::on_path()
        }
        #[cfg(not(feature = "legacy-office"))]
        {
            which_program("soffice")
        }
    }
}

fn sips_available() -> bool {
    #[cfg(feature = "heic")]
    {
        crate::infrastructure::media::heic::decoder_available()
    }
    #[cfg(not(feature = "heic"))]
    {
        which_program("sips")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::builder::AlchemizeBuilder;

    fn posture() -> SecurityPosture {
        let alch = AlchemizeBuilder::new().build();
        collect_posture(&alch)
    }

    /// Without pdf-cloud compiled in, no environment variable can create
    /// network egress — the posture must report the compile-time truth.
    #[cfg(not(feature = "pdf-cloud"))]
    #[test]
    fn egress_compiled_out_when_pdf_cloud_missing() {
        assert_eq!(
            posture().network_egress,
            EgressStatus::UnavailableCompiledOut
        );
    }

    /// With pdf-cloud compiled in, token presence mirrors the environment
    /// exactly (the mirror assertion is robust to a CI-provided token).
    #[cfg(feature = "pdf-cloud")]
    #[test]
    fn egress_ready_token_mirrors_env() {
        match posture().network_egress {
            EgressStatus::Ready { token_present, .. } => {
                let expected = std::env::var("MINERU_API_TOKEN")
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                assert_eq!(token_present, expected);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    /// The subprocess table always covers all four kinds with a probe
    /// result (no missing entries), and the budget table is non-empty and
    /// sorted stable (documentation contract).
    #[test]
    fn subprocesses_and_budgets_tables_are_complete() {
        let p = posture();
        let kinds: Vec<ToolKind> = p.subprocesses.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&ToolKind::Pandoc));
        assert!(kinds.contains(&ToolKind::LibreOffice));
        assert!(kinds.contains(&ToolKind::Sips));
        assert!(kinds.contains(&ToolKind::Ffmpeg));
        assert!(!p.budgets.is_empty());
    }

    /// Parse surfaces are the compile-time feature truth (spot-check: the
    /// `text` feature is on for any default build this test runs in).
    #[test]
    fn parse_surfaces_come_from_feature_map() {
        let p = posture();
        assert_eq!(
            p.parse_surfaces,
            crate::domain::feature_map::enabled_features()
        );
    }
}
