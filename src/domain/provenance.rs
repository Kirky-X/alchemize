// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Security provenance types — the trust origin of a conversion result.
//!
//! These types are core (not feature-gated): they are part of the public API
//! surface of [`crate::domain::result::ConvertResult`] and must compile in every
//! feature configuration (see the public API policy in Cargo.toml).

use serde::{Deserialize, Serialize};

/// The complete engine chain a conversion actually traversed, with its trust origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProvenanceEnvelope {
    /// Engine steps in attempt order, including skipped and failed steps.
    pub engine_path: Vec<EngineStep>,
    /// Name of the engine that produced the final result (same source as `metadata.converter`).
    pub final_engine: Option<String>,
    /// Network egress used by the engine that produced the result.
    pub network_egress: Option<NetworkEgress>,
    /// External child processes driven by used engines (deduplicated).
    pub external_tools: Vec<ExternalToolKind>,
    /// Local model bundle origins (ASR path).
    pub model_bundles: Vec<String>,
    /// Engine-declared output transforms. An empty set means "not declared",
    /// NOT a guarantee that no transform happened.
    pub sanitization: SanitizationSummary,
    /// Aggregated trust level for downstream sanitize decisions.
    pub trust_level: TrustLevel,
}

/// Aggregated trust level of a conversion result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Pure local Rust parsing chain: no external process, no network.
    #[default]
    SelfContained,
    /// Went through an external child process or a local model bundle, but no network.
    Mixed,
    /// Data left this process (MinerU cloud upload/download).
    ExternalService,
}

/// One step on the engine chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineStep {
    /// Stable engine name.
    pub engine: String,
    /// Outcome of this step.
    pub outcome: StepOutcome,
    /// Wall time this step took, when it executed (`None` for skipped
    /// steps and steps recorded before this field existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Outcome of one engine chain step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// Skipped because availability was `Unavailable`, with a reason summary.
    Skipped {
        /// Human-readable reason for the skip.
        reason: String,
    },
    /// Conversion failed and was downgraded, with the `AlchemizeError::kind()`
    /// label only (never the full message, which may embed remote response bodies).
    Failed {
        /// Error kind label.
        error_kind: String,
    },
    /// Produced the final result.
    Used,
}

/// A network destination data was sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkEgress {
    /// Target endpoint host only (no path/query, never credentials).
    pub endpoint_host: String,
    /// Whether the download URL is presigned.
    pub presigned: bool,
}

/// External child process an engine drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalToolKind {
    /// Pandoc document converter.
    Pandoc,
    /// LibreOffice / soffice suite.
    LibreOffice,
    /// macOS sips image tool.
    Sips,
    /// ffmpeg media tool.
    Ffmpeg,
}

/// Engine-declared output transforms.
///
/// An empty set means the engine did not declare the transform, NOT a guarantee
/// that no transform happened (see Non-Goals in the change proposal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SanitizationSummary {
    /// Fence-injection protection (`fence_for`) was applied to the output.
    pub fence_reinforced: bool,
    /// Links were absolutized with an explicit base URL.
    pub links_absolutized: bool,
}

impl std::fmt::Display for EngineStep {
    /// Render as `engine: outcome (detail)` — one attempt per line, matching
    /// the registry's existing attempts text so failure messages do not drift.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: ", self.engine)?;
        match &self.outcome {
            StepOutcome::Skipped { reason } => write!(f, "skipped ({reason})"),
            StepOutcome::Failed { error_kind } => write!(f, "failed ({error_kind})"),
            StepOutcome::Used => write!(f, "used"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_level_serde_uses_snake_case_values() {
        assert_eq!(
            serde_json::to_string(&TrustLevel::SelfContained).unwrap(),
            "\"self_contained\""
        );
        assert_eq!(
            serde_json::to_string(&TrustLevel::Mixed).unwrap(),
            "\"mixed\""
        );
        assert_eq!(
            serde_json::to_string(&TrustLevel::ExternalService).unwrap(),
            "\"external_service\""
        );
        for value in [
            TrustLevel::SelfContained,
            TrustLevel::Mixed,
            TrustLevel::ExternalService,
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let back: TrustLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, value);
        }
    }

    /// `EngineStep` Display renders the stable `engine: outcome (detail)`
    /// shape for every outcome variant. The registry's attempts reporting
    /// (failure chains) builds on this, so the formats are pinned here.
    #[test]
    fn engine_step_display_formats_all_outcomes() {
        let skipped = EngineStep {
            engine: "mineru".to_string(),
            outcome: StepOutcome::Skipped {
                reason: "no token".to_string(),
            },
            duration_ms: None,
        };
        assert_eq!(skipped.to_string(), "mineru: skipped (no token)");

        let failed = EngineStep {
            engine: "pdf".to_string(),
            outcome: StepOutcome::Failed {
                error_kind: "zip".to_string(),
            },
            duration_ms: Some(12),
        };
        assert_eq!(failed.to_string(), "pdf: failed (zip)");

        let used = EngineStep {
            engine: "lopdf".to_string(),
            outcome: StepOutcome::Used,
            duration_ms: Some(388),
        };
        assert_eq!(used.to_string(), "lopdf: used");
    }

    /// duration_ms is a backward-compatible extension: old JSON without
    /// the key decodes to None; with the key it round-trips.
    #[test]
    fn engine_step_duration_ms_serde_compat() {
        let old_json = r#"{"engine":"lopdf","outcome":"used"}"#;
        let step: EngineStep = serde_json::from_str(old_json).unwrap();
        assert_eq!(step.duration_ms, None);

        let with_duration = EngineStep {
            engine: "lopdf".to_string(),
            outcome: StepOutcome::Used,
            duration_ms: Some(388),
        };
        let json = serde_json::to_string(&with_duration).unwrap();
        assert!(json.contains("\"duration_ms\":388"), "{json}");
        let back: EngineStep = serde_json::from_str(&json).unwrap();
        assert_eq!(back, with_duration);

        // None omits the key entirely.
        let none_step = EngineStep {
            engine: "lopdf".to_string(),
            outcome: StepOutcome::Used,
            duration_ms: None,
        };
        let json = serde_json::to_string(&none_step).unwrap();
        assert!(!json.contains("duration_ms"), "{json}");
    }

    #[test]
    fn provenance_envelope_serde_roundtrip() {
        let envelope = ProvenanceEnvelope {
            engine_path: vec![EngineStep {
                engine: "lopdf".to_string(),
                outcome: StepOutcome::Used,
                duration_ms: None,
            }],
            final_engine: Some("lopdf".to_string()),
            network_egress: None,
            external_tools: vec![ExternalToolKind::Pandoc],
            model_bundles: vec![],
            sanitization: SanitizationSummary {
                fence_reinforced: true,
                links_absolutized: false,
            },
            trust_level: TrustLevel::Mixed,
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let back: ProvenanceEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, envelope);
    }
}
