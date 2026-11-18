// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! sdforge `#[forge]` 端点层 —— `alch` CLI 与其 `alch mcp` 子命令
//! （MCP stdio）共享的参数元数据单一来源。
//!
//! 契约与错误映射：`specmark/specs/cli-output/spec.md`。领域 stderr 行
//! （`unsupported format` / attempts 明细）由本层先行输出；`ApiError`
//! 变体只承载退出码语义与 JSON 面——runner 按变体映射退出码，禁止按
//! 消息文本匹配。

use std::path::PathBuf;

use sdforge::core::ApiError;
use sdforge::forge;

use alchemize::application::builder::{Alchemize, AlchemizeBuilder, AssetMode, MineruConfig};
use alchemize::application::ports::Availability;
use alchemize::domain::error::AlchemizeError;
use alchemize::domain::input::Input;

/// 平移自原 main.rs：按环境变量与参数组装门面；base_url/extract_main 仅在
/// html feature 构建下生效（未编译时诚实告警，不静默吞选项）。
fn build_alch(
    no_remote: bool,
    asset_mode: AssetMode,
    base_url: Option<String>,
    extract_main: bool,
) -> Alchemize {
    let mut builder = AlchemizeBuilder::new().with_asset_mode(asset_mode);
    if no_remote {
        builder = builder.with_no_remote();
    }
    if base_url.is_some() || extract_main {
        #[cfg(feature = "html")]
        {
            builder = builder.with_html_deep(alchemize::HtmlDeepEngineConfig {
                base_url,
                extract_main,
            });
        }
        #[cfg(not(feature = "html"))]
        {
            let _ = (&base_url, &extract_main);
            eprintln!(
                "{}",
                alchemize::tr("warning-base-url-extract-main-require-html")
            );
        }
    }
    // MinerU endpoints come from env when present; otherwise local-only chain.
    if let Ok(base) = std::env::var("ALCHEMIZE_MINERU_URL") {
        let cfg = MineruConfig {
            base_url: base,
            ..MineruConfig::default()
        };
        builder = builder.with_mineru(cfg);
    }
    builder.build()
}

/// Diagnostics for an unsupported extension: names the missing capability
/// features (and the rebuild flag) when the format merely was not compiled
/// in, and stays quiet when no feature can help.
///
/// The extension comes from a user-supplied filename, so it is sanitized
/// before printing: control characters and Unicode bidi controls could
/// otherwise drive terminal escapes or reorder a fake diagnostic line
/// (CWE-150). Character set: C0/C1 controls plus `Bidi_Control`
/// (U+061C, U+200E, U+200F, U+202A–U+202E, U+2066–U+2069).
fn unsupported_format_message(ext: &str) -> String {
    let safe = sanitize_ext(ext);
    match alchemize::domain::feature_map::missing_features(ext) {
        None => alchemize::tr_args("unsupported-format", &[("ext", safe)]),
        Some(missing) => {
            let names = missing
                .iter()
                .map(|f| format!("'{f}'"))
                .collect::<Vec<_>>()
                .join(" and ");
            let plural = if missing.len() > 1 { "s" } else { "" };
            let them = if missing.len() > 1 { "them" } else { "it" };
            alchemize::tr_args(
                "unsupported-format-requires-feature",
                &[
                    ("ext", safe),
                    ("names", names),
                    ("plural", plural.to_string()),
                    ("them", them.to_string()),
                    ("flags", missing.join(",")),
                ],
            )
        }
    }
}

fn sanitize_ext(ext: &str) -> String {
    ext.chars()
        .map(|c| {
            let bidi = matches!(
                c,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            );
            if c.is_control() || bidi { '?' } else { c }
        })
        .collect()
}

/// AlchemizeError → ApiError。映射即退出码：`NotFound("format")` → 2、
/// `ServiceUnavailable` → 3、其余 → 1（见 main.rs runner）。
fn map_error(err: AlchemizeError) -> ApiError {
    match err {
        AlchemizeError::UnsupportedFormat(ext) => {
            // The extension may simply be a capability this build did not
            // compile in — say which feature turns it on instead of leaving
            // the user guessing (exit code stays 2: unsupported format).
            eprintln!("{}", unsupported_format_message(&ext));
            ApiError::NotFound {
                resource: "format".into(),
                resource_id: Some(ext),
            }
        }
        AlchemizeError::AllBackendsFailed { ext, attempts } => {
            eprintln!(
                "{}",
                alchemize::tr_args("all-backends-failed-header", &[("ext", ext.clone())])
            );
            for step in &attempts {
                eprintln!("  - {step}");
            }
            ApiError::ServiceUnavailable {
                service: format!("convert:{ext}"),
                retry_after: None,
                source: None,
            }
        }
        other => ApiError::InvalidInput {
            message: other.to_string(),
            field: None,
            value: None,
        },
    }
}

/// Convert one input file to markdown.
#[forge(
    name = "convert",
    version = "1.0",
    description = "Convert one input file to markdown",
    cli = true,
    tool_name = "convert"
)]
// 参数面 = CLI/MCP 端点契约，不可收敛为结构体参数（宏按参数生成旗标与 schema）
#[allow(clippy::too_many_arguments)]
async fn convert(
    #[param(kind = "path")] input: String,
    // 注意三点：① 宏对 Option<T> 的默认推断是 Query（CLI 侧跳过），必须
    // 显式 kind = "body" 才生成 --option 参数；② 参数名不能叫 format——
    // sdforge 根级内置全局 --format text|json（输出格式、带默认值）会同 ID
    // 撞车并经全局传播污染取值，故这里用 input_format，CLI 契约由 runner
    // 的 argv 预处理（convert 语境下 --format → --input_format）保住；
    // ③ no_remote 用 Option<bool>（裸旗标语义由 runner 归一补 true）。
    #[param(kind = "body")] output: Option<String>,
    #[param(kind = "body")] engine: Option<String>,
    #[param(kind = "body")] assets: Option<String>,
    #[param(kind = "body")] no_remote: Option<bool>,
    #[param(kind = "body")] input_format: Option<String>,
    #[param(kind = "body")] base_url: Option<String>,
    #[param(kind = "body")] extract_main: Option<bool>,
) -> Result<serde_json::Value, ApiError> {
    let input_path = PathBuf::from(&input);
    if !input_path.is_file() {
        return Err(ApiError::InvalidInput {
            message: alchemize::tr_args(
                "input-file-not-found",
                &[("path", input_path.display().to_string())],
            ),
            field: Some("input".into()),
            value: None,
        });
    }
    let asset_mode = match assets.as_deref() {
        None | Some("dir") => AssetMode::Dir,
        Some("inline") => AssetMode::Inline,
        Some(other) => {
            return Err(ApiError::InvalidInput {
                message: alchemize::tr_args("unknown-asset-mode", &[("what", other.to_string())]),
                field: Some("assets".into()),
                value: None,
            });
        }
    };
    let alch = build_alch(
        no_remote.unwrap_or(false),
        asset_mode,
        base_url,
        extract_main.unwrap_or(false),
    );
    let converted_input = match input_format {
        Some(fmt) => {
            let data = std::fs::read(&input_path).map_err(|e| {
                map_error(AlchemizeError::Other(alchemize::tr_args(
                    "failed-to-read-input",
                    &[("source", e.to_string())],
                )))
            })?;
            Input::Bytes {
                data,
                filename: input_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "input".into()),
                mimetype: None,
                extension: Some(fmt.to_lowercase()),
            }
        }
        None => Input::Path(input_path.clone()),
    };

    let result = alch
        .convert_pref(&converted_input, engine.as_deref())
        .await
        .map_err(map_error)?;
    let engine_used = result
        .metadata
        .get("converter")
        .and_then(|c| c.as_str())
        .unwrap_or("unknown")
        .to_string();
    let quality = result.quality;
    match output {
        Some(path) => {
            let path = PathBuf::from(path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Dir mode: write extracted assets at their converter-assigned
            // names next to the output. Validate every converter-provided
            // name BEFORE any write: an illegal name is a hard error, not a
            // silent rewrite (rewriting would desynchronize the markdown
            // reference), and it must not leave a half-written output with
            // the markdown on disk but assets missing.
            let asset_dir = path.parent().unwrap_or(std::path::Path::new("."));
            let mut asset_paths = Vec::with_capacity(result.assets.len());
            for asset in &result.assets {
                if let Err(reason) =
                    alchemize::domain::asset::validate_asset_filename(&asset.filename)
                {
                    return Err(map_error(AlchemizeError::Other(alchemize::tr_args(
                        "asset-filename-rejected-by-write-boundary",
                        &[("reason", reason)],
                    ))));
                }
                asset_paths.push(asset_dir.join(&asset.filename));
            }
            std::fs::write(&path, &result.markdown).map_err(|e| {
                map_error(AlchemizeError::Other(alchemize::tr_args(
                    "write-failed",
                    &[("source", e.to_string())],
                )))
            })?;
            for (asset, asset_path) in result.assets.iter().zip(asset_paths) {
                if let Some(dir) = asset_path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::write(&asset_path, &asset.bytes).map_err(|e| {
                    map_error(AlchemizeError::Other(alchemize::tr_args(
                        "asset-write-failed",
                        &[("source", e.to_string())],
                    )))
                })?;
            }
            eprintln!(
                "{}",
                alchemize::tr_args(
                    "converted-to",
                    &[
                        ("input", input_path.display().to_string()),
                        ("output", path.display().to_string()),
                        ("quality", quality.to_string()),
                        ("engine", engine_used),
                    ],
                )
            );
            // Null 哨兵：CLI runner 据此跳过 stdout 渲染（摘要已走 stderr）。
            Ok(serde_json::Value::Null)
        }
        None => Ok(serde_json::Value::String(result.markdown)),
    }
}

/// Report engine availability; `security = true` appends the build's
/// security posture (parse surfaces / network egress / subprocesses /
/// budgets).
#[forge(
    name = "doctor",
    version = "1.1",
    description = "Report engine availability",
    cli = true,
    tool_name = "doctor"
)]
async fn doctor(security: bool) -> Result<String, ApiError> {
    let alch = build_alch(false, AssetMode::Dir, None, false);
    let mut lines: Vec<String> = Vec::new();
    for (name, exts, availability) in alch.availability() {
        match availability {
            Availability::Available => {
                lines.push(format!("{name:<16} AVAILABLE   {}", exts.join(",")));
            }
            Availability::Unavailable { reason } => {
                lines.push(format!("{name:<16} UNAVAILABLE {reason}"));
            }
        }
    }
    if security {
        lines.push(String::new());
        lines.extend(render_security_posture(&alch));
    }
    Ok(lines.join("\n"))
}

/// Render the four posture sections (one line each + detail lines), with
/// ✓/!/✗ prefixes matching SECURITY.md's visual language.
fn render_security_posture(alch: &alchemize::application::builder::Alchemize) -> Vec<String> {
    use alchemize::application::posture::{
        EgressStatus, SecurityPosture, ToolKind, collect_posture,
    };
    let p: SecurityPosture = collect_posture(alch);
    let mut lines = Vec::new();

    lines.push(format!(
        "Parse surfaces:    {}",
        if p.parse_surfaces.is_empty() {
            "NONE".to_string()
        } else {
            p.parse_surfaces.join(", ")
        }
    ));

    match &p.network_egress {
        EgressStatus::UnavailableCompiledOut => {
            lines.push("Network egress:    ✗ NONE (pdf-cloud not compiled)".to_string());
        }
        EgressStatus::Ready {
            endpoint_host,
            token_present,
        } => match endpoint_host {
            Some(host) => {
                lines.push(format!(
                    "Network egress:    ! MinerU cloud -> {host} (HTTPS-only, SSRF-validated, no redirects)"
                ));
                lines.push(format!(
                    "  token:           {}",
                    if *token_present {
                        "✓ present"
                    } else {
                        "! NOT SET (precision API unavailable)"
                    }
                ));
            }
            None => {
                lines.push(
                    "Network egress:    ✓ NONE (pdf-cloud compiled, ALCHEMIZE_MINERU_URL unset — engine not registered)"
                        .to_string(),
                );
                lines.push(format!(
                    "  token:           {}",
                    if *token_present {
                        "✓ present"
                    } else {
                        "! NOT SET"
                    }
                ));
            }
        },
    }

    lines.push("External subprocesses:".to_string());
    for (tool, available) in &p.subprocesses {
        let name = match tool {
            ToolKind::Pandoc => "pandoc",
            ToolKind::LibreOffice => "soffice",
            ToolKind::Sips => "sips",
            ToolKind::Ffmpeg => "ffmpeg",
        };
        lines.push(format!(
            "  {name:<15} {}",
            if *available {
                "✓ on PATH"
            } else {
                "✗ not found"
            }
        ));
    }

    lines.push("Budgets:".to_string());
    for (name, limit) in p.budgets {
        lines.push(format!("  {name:<44} {}", format_bytes(*limit)));
    }
    lines
}

/// Format a byte count for the posture report (KiB/MiB units).
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else {
        format!("{} KiB", bytes / 1024)
    }
}

/// List supported extensions.
#[forge(
    name = "formats",
    version = "1.0",
    description = "List supported extensions",
    cli = true,
    tool_name = "formats"
)]
async fn formats() -> Result<String, ApiError> {
    let alch = build_alch(false, AssetMode::Dir, None, false);
    let mut exts: Vec<String> = alch
        .availability()
        .into_iter()
        .flat_map(|(_, exts, _)| exts)
        .collect();
    exts.sort();
    exts.dedup();
    Ok(exts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `security = false` keeps the availability-only output byte-identical
    /// (the posture block is strictly additive).
    #[tokio::test]
    async fn doctor_without_security_matches_availability_lines() {
        let out = doctor(false).await.unwrap();
        let alch = build_alch(false, AssetMode::Dir, None, false);
        let mut expected: Vec<String> = Vec::new();
        for (name, exts, availability) in alch.availability() {
            match availability {
                Availability::Available => {
                    expected.push(format!("{name:<16} AVAILABLE   {}", exts.join(",")));
                }
                Availability::Unavailable { reason } => {
                    expected.push(format!("{name:<16} UNAVAILABLE {reason}"));
                }
            }
        }
        assert_eq!(out, expected.join("\n"));
        assert!(!out.contains("Network egress"));
    }

    /// `security = true` appends all four posture sections.
    #[tokio::test]
    async fn doctor_with_security_appends_posture_sections() {
        let out = doctor(true).await.unwrap();
        assert!(out.contains("Network egress"), "{out}");
        assert!(out.contains("Parse surfaces:"), "{out}");
        assert!(out.contains("External subprocesses:"), "{out}");
        assert!(out.contains("Budgets:"), "{out}");
        // The token status line only exists when the egress engine is
        // compiled in (pdf-cloud); guard the assertion accordingly.
        #[cfg(feature = "pdf-cloud")]
        assert!(out.contains("token:"), "{out}");
    }

    #[test]
    fn sanitize_ext_replaces_controls_and_bidi() {
        // C0 controls -> '?'
        assert_eq!(sanitize_ext("pd\u{0001}f"), "pd?f");
        // DEL (C1 boundary) and C1 CSI
        assert_eq!(sanitize_ext("t\u{007f}x\u{009b}t"), "t?x?t");
        // Unicode bidi controls: RLM/LRM, embedding/overrides, isolates
        assert_eq!(
            sanitize_ext("\u{061c}\u{200e}a\u{202e}b\u{2066}c"),
            "??a?b?c"
        );
        // Normal extensions pass through untouched
        assert_eq!(sanitize_ext("pdf"), "pdf");
        assert_eq!(sanitize_ext("tar.gz"), "tar.gz");
        // Empty stays empty
        assert_eq!(sanitize_ext(""), "");
    }

    #[test]
    fn unsupported_format_message_names_clean_extension() {
        // Unknown extension (no feature can help): message embeds the
        // sanitized extension text.
        let msg = unsupported_format_message("definitely_not_a_format");
        assert!(msg.contains("definitely_not_a_format"), "{msg}");
        // Control characters are sanitized before printing (CWE-150)
        let msg = unsupported_format_message("x\u{001b}[31mred");
        assert!(!msg.contains('\u{001b}'), "escape survived: {msg:?}");
        assert!(msg.contains("?"), "{msg}");
    }

    #[test]
    fn map_error_maps_unsupported_format_to_not_found() {
        let err = map_error(AlchemizeError::UnsupportedFormat("zzz".into()));
        match err {
            ApiError::NotFound {
                resource,
                resource_id,
            } => {
                assert_eq!(resource, "format");
                assert_eq!(resource_id.as_deref(), Some("zzz"));
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn map_error_maps_all_backends_failed_to_service_unavailable() {
        use alchemize::domain::provenance::{EngineStep, StepOutcome};
        let err = map_error(AlchemizeError::AllBackendsFailed {
            ext: "docx".into(),
            attempts: vec![
                EngineStep {
                    engine: "anydoc".to_string(),
                    outcome: StepOutcome::Failed {
                        error_kind: "zip".to_string(),
                    },
                },
                EngineStep {
                    engine: "legacy".to_string(),
                    outcome: StepOutcome::Skipped {
                        reason: "soffice missing".to_string(),
                    },
                },
            ],
        });
        match err {
            ApiError::ServiceUnavailable { service, .. } => {
                assert_eq!(service, "convert:docx");
            }
            other => panic!("expected ServiceUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn map_error_maps_other_errors_to_invalid_input() {
        let err = map_error(AlchemizeError::DepthLimitExceeded { limit: 8 });
        match err {
            ApiError::InvalidInput { message, .. } => {
                assert!(!message.is_empty());
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    // -- 95 攻坚批四：build_alch 配置臂 + convert 入口契约 --------------------

    #[tokio::test]
    async fn convert_rejects_missing_input_file() {
        let err = convert(
            "definitely-missing-input.xyz".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        match err {
            ApiError::InvalidInput { field, .. } => assert_eq!(field.as_deref(), Some("input")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn convert_rejects_unknown_asset_mode() {
        let dir = std::env::temp_dir().join("alch-conv-fixture-xyz.txt");
        std::fs::write(&dir, b"hello").unwrap();
        let err = convert(
            dir.display().to_string(),
            None,
            None,
            Some("bogus".to_string()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        std::fs::remove_file(&dir).ok();
        match err {
            ApiError::InvalidInput { field, value, .. } => {
                assert_eq!(field.as_deref(), Some("assets"));
                let _ = value;
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn convert_with_output_writes_markdown_file() {
        let dir = std::env::temp_dir().join("alch-out-fixture");
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("note.txt");
        std::fs::write(&input, b"plain body").unwrap();
        let output = dir.join("note-out.md");
        let result = convert(
            input.display().to_string(),
            Some(output.display().to_string()),
            None,
            None,
            Some(true),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        std::fs::remove_file(&output).ok();
        std::fs::remove_file(&input).ok();
        let _ = result;
    }
}
