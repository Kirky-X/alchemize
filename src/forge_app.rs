// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! sdforge `#[forge]` 端点层 —— `alch`（CLI）与 `alch-mcp`（MCP stdio）
//! 共享的参数元数据单一来源。
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
/// html-deep feature 构建下生效（未编译时诚实告警，不静默吞选项）。
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
        #[cfg(feature = "html-deep")]
        {
            builder = builder.with_html_deep(alchemize::HtmlDeepEngineConfig {
                base_url,
                extract_main,
            });
        }
        #[cfg(not(feature = "html-deep"))]
        {
            let _ = (&base_url, &extract_main);
            eprintln!(
                "warning: --base-url/--extract-main require a build with --features html-deep; options ignored"
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

/// AlchemizeError → ApiError。映射即退出码：`NotFound("format")` → 2、
/// `ServiceUnavailable` → 3、其余 → 1（见 main.rs runner）。
fn map_error(err: AlchemizeError) -> ApiError {
    match err {
        AlchemizeError::UnsupportedFormat(ext) => {
            eprintln!("unsupported format: {ext}");
            ApiError::NotFound {
                resource: "format".into(),
                resource_id: Some(ext),
            }
        }
        AlchemizeError::AllBackendsFailed { ext, attempts } => {
            eprintln!("all backends failed for {ext}:");
            for (engine, reason) in &attempts {
                eprintln!("  - {engine}: {reason}");
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
            message: format!("input file not found: {}", input_path.display()),
            field: Some("input".into()),
            value: None,
        });
    }
    let asset_mode = match assets.as_deref() {
        None | Some("dir") => AssetMode::Dir,
        Some("inline") => AssetMode::Inline,
        Some(other) => {
            return Err(ApiError::InvalidInput {
                message: format!("unknown asset mode '{other}' (dir|inline)"),
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
                map_error(AlchemizeError::Other(format!("failed to read input: {e}")))
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
            std::fs::write(&path, &result.markdown)
                .map_err(|e| map_error(AlchemizeError::Other(format!("write failed: {e}"))))?;
            // Dir mode: write extracted assets at their converter-assigned
            // names next to the output. The write boundary validates the
            // converter-provided filename first: an illegal name is a hard
            // error, not a silent rewrite (rewriting would desynchronize
            // the markdown reference from the file on disk).
            for asset in &result.assets {
                if let Err(reason) =
                    alchemize::domain::asset::validate_asset_filename(&asset.filename)
                {
                    return Err(map_error(AlchemizeError::Other(format!(
                        "asset filename rejected by write boundary: {reason}"
                    ))));
                }
                let asset_path = path
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join(&asset.filename);
                if let Some(dir) = asset_path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::write(&asset_path, &asset.bytes).map_err(|e| {
                    map_error(AlchemizeError::Other(format!("asset write failed: {e}")))
                })?;
            }
            eprintln!(
                "converted {} -> {} [{quality}] (engine: {engine_used})",
                input_path.display(),
                path.display(),
            );
            // Null 哨兵：CLI runner 据此跳过 stdout 渲染（摘要已走 stderr）。
            Ok(serde_json::Value::Null)
        }
        None => Ok(serde_json::Value::String(result.markdown)),
    }
}

/// Report engine availability.
#[forge(
    name = "doctor",
    version = "1.0",
    description = "Report engine availability",
    cli = true,
    tool_name = "doctor"
)]
async fn doctor() -> Result<String, ApiError> {
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
    Ok(lines.join("\n"))
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
