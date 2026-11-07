// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `alch` — transmute documents into Markdown from the command line.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use alchemize::application::builder::{Alchemize, AlchemizeBuilder, AssetMode, MineruConfig};
use alchemize::application::ports::Availability;
use alchemize::domain::input::Input;

#[derive(Parser)]
#[command(name = "alch", version, about = "Transmute any document into Markdown")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert one input file to markdown.
    Convert {
        /// Input file path.
        input: PathBuf,
        /// Output file path (default: stdout).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Prefer a specific engine by name (falls back to the chain).
        #[arg(long)]
        engine: Option<String>,
        /// Asset mode: dir (images/ next to output) or inline (base64).
        #[arg(long, default_value = "dir")]
        assets: AssetModeArg,
        /// Exclude remote (cloud API) engines entirely.
        #[arg(long)]
        no_remote: bool,
        /// Override format detection (extension without dot).
        #[arg(long)]
        format: Option<String>,
    },
    /// Report engine availability.
    Doctor,
    /// List supported extensions.
    Formats,
}

/// Clap-friendly mirror of [`AssetMode`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum AssetModeArg {
    Dir,
    Inline,
}

impl std::str::FromStr for AssetModeArg {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dir" => Ok(Self::Dir),
            "inline" => Ok(Self::Inline),
            other => Err(format!("unknown asset mode '{other}' (dir|inline)")),
        }
    }
}

impl From<AssetModeArg> for AssetMode {
    fn from(arg: AssetModeArg) -> Self {
        match arg {
            AssetModeArg::Dir => AssetMode::Dir,
            AssetModeArg::Inline => AssetMode::Inline,
        }
    }
}

fn build_alch(no_remote: bool, asset_mode: AssetMode) -> Alchemize {
    let mut builder = AlchemizeBuilder::new().with_asset_mode(asset_mode);
    if no_remote {
        builder = builder.with_no_remote();
    }
    // MinerU endpoints come from env when present; otherwise local-only chain.
    if let Ok(base) = std::env::var("ALCHEMIZE_MINERU_URL") {
        let mut cfg = MineruConfig::default();
        cfg.base_url = base;
        builder = builder.with_mineru(cfg);
    }
    builder.build()
}

fn run(runtime: &tokio::runtime::Runtime) -> Result<ExitCode, String> {
    let cli = Cli::parse();
    match cli.command {
        Command::Formats => {
            let alch = build_alch(false, AssetMode::Dir);
            let mut exts: Vec<String> = alch
                .availability()
                .into_iter()
                .flat_map(|(_, exts, _)| exts)
                .collect();
            exts.sort();
            exts.dedup();
            println!("{}", exts.join(" "));
            Ok(ExitCode::SUCCESS)
        }
        Command::Doctor => {
            let alch = build_alch(false, AssetMode::Dir);
            for (name, exts, availability) in alch.availability() {
                match availability {
                    Availability::Available => {
                        println!("{name:<16} AVAILABLE   {}", exts.join(","));
                    }
                    Availability::Unavailable { reason } => {
                        println!("{name:<16} UNAVAILABLE {reason}");
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Convert {
            input,
            output,
            engine,
            assets,
            no_remote,
            format,
        } => {
            if !input.is_file() {
                return Err(format!("input file not found: {}", input.display()));
            }
            let alch = build_alch(no_remote, assets.into());
            let converted_input = match format {
                Some(fmt) => {
                    let data =
                        std::fs::read(&input).map_err(|e| format!("failed to read input: {e}"))?;
                    Input::Bytes {
                        data,
                        filename: input
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "input".into()),
                        mimetype: None,
                        extension: Some(fmt.to_lowercase()),
                    }
                }
                None => Input::Path(input.clone()),
            };

            let result = runtime.block_on(alch.convert_pref(&converted_input, engine.as_deref()));

            match result {
                Ok(convert_result) => {
                    let engine_used = convert_result
                        .metadata
                        .get("converter")
                        .and_then(|c| c.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let quality = convert_result.quality;
                    match output {
                        Some(path) => {
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            std::fs::write(&path, &convert_result.markdown)
                                .map_err(|e| format!("write failed: {e}"))?;
                            // Dir mode: write extracted assets at their
                            // converter-assigned names next to the output.
                            for asset in &convert_result.assets {
                                let asset_path = path
                                    .parent()
                                    .unwrap_or(std::path::Path::new("."))
                                    .join(&asset.filename);
                                if let Some(dir) = asset_path.parent() {
                                    let _ = std::fs::create_dir_all(dir);
                                }
                                std::fs::write(&asset_path, &asset.bytes)
                                    .map_err(|e| format!("asset write failed: {e}"))?;
                            }
                            eprintln!(
                                "converted {} -> {} [{quality}] (engine: {engine_used})",
                                input.display(),
                                path.display(),
                            );
                        }
                        None => {
                            let mut stdout = std::io::stdout().lock();
                            let _ = writeln!(stdout, "{}", convert_result.markdown);
                        }
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Err(alchemize::AlchemizeError::UnsupportedFormat(ext)) => {
                    eprintln!("unsupported format: {ext}");
                    Ok(ExitCode::from(2))
                }
                Err(alchemize::AlchemizeError::AllBackendsFailed { ext, attempts }) => {
                    eprintln!("all backends failed for {ext}:");
                    for (engine, reason) in &attempts {
                        eprintln!("  - {engine}: {reason}");
                    }
                    Ok(ExitCode::from(3))
                }
                Err(other) => Err(other.to_string()),
            }
        }
    }
}

fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match run(&runtime) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("alch: {message}");
            ExitCode::FAILURE
        }
    }
}
