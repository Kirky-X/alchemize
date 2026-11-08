// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Real-file benchmark harness (T030).
//!
//! Run with `cargo bench --bench bench_real` (no criterion dependency: a
//! plain harness measuring wall time over 5 repetitions, reporting the
//! median). The markdown table is printed to stdout and can be pasted into
//! `docs/benchmarks.md`.
//!
//! Red lines (spec R-self-003 constraints): docx < 1s, xlsx < 5s, pdf < 2s,
//! plain text < 50ms.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use alchemize::application::builder::AlchemizeBuilder;
use alchemize::domain::input::Input;

/// Fixtures measured per run, in report order.
const FIXTURES: &[(&str, &str)] = &[
    ("real.docx", "docx (923KB wordlist)"),
    ("real.xlsx", "xlsx (6.7MB lexicon)"),
    ("real.pdf", "pdf (100KB multi-page)"),
    ("real.odt", "odt (ODF text)"),
    ("real.odp", "odp (ODF presentation)"),
    ("real.pptx", "pptx (slides)"),
    ("real.html", "html"),
    ("real.rs", "rust source"),
    ("real.md", "markdown"),
    ("real.png", "png image"),
    ("real.wav", "wav audio"),
    ("real.heic", "heic image"),
];

/// Repetitions per fixture (median reported).
const RUNS: usize = 5;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/real")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let alch = AlchemizeBuilder::new().build();
    println!("| fixture | format | size | median | min | max | throughput | engine |");
    println!("|---|---|---|---|---|---|---|---|");

    let mut violations = 0usize;
    for (file, label) in FIXTURES {
        let path = fixture_dir().join(file);
        if !path.is_file() {
            println!("| {file} | {label} | — | *(missing fixture)* | | | | |");
            continue;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut times: Vec<Duration> = Vec::with_capacity(RUNS);
        let mut engine = String::from("?");
        for _ in 0..RUNS {
            let start = Instant::now();
            let result = alch.convert(&Input::Path(path.clone())).await;
            let elapsed = start.elapsed();
            match result {
                Ok(r) => {
                    engine = r
                        .metadata
                        .get("converter")
                        .or_else(|| r.metadata.get("parser"))
                        .or_else(|| r.metadata.get("engine"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(engine.as_str())
                        .to_string();
                    times.push(elapsed);
                }
                Err(e) => {
                    println!("| {file} | {label} | {size} | *(error: {e})* | | | | |");
                    break;
                }
            }
        }
        if times.is_empty() {
            continue;
        }
        times.sort();
        let median = times[times.len() / 2];
        let min = times[0];
        let max = times[times.len() - 1];
        let mb_per_s = if median.as_secs_f64() > 0.0 {
            (size as f64 / 1_048_576.0) / median.as_secs_f64()
        } else {
            0.0
        };
        let red_line = red_line_for(file);
        let verdict = match red_line {
            Some(limit) if median > limit => {
                violations += 1;
                " ⚠️"
            }
            _ => "",
        };
        println!(
            "| {file} | {label} | {} | {:.1}ms | {:.1}ms | {:.1}ms | {:.1} MB/s | {engine}{verdict} |",
            human_size(size),
            median.as_secs_f64() * 1000.0,
            min.as_secs_f64() * 1000.0,
            max.as_secs_f64() * 1000.0,
            mb_per_s
        );
    }

    println!();
    if violations == 0 {
        println!("All red lines met (docx<1s, xlsx<5s, pdf<2s, text<50ms).");
    } else {
        println!("{violations} red-line violation(s) — optimize before the release gate.");
        std::process::exit(1);
    }
}

/// Spec red lines per format family.
fn red_line_for(file: &str) -> Option<Duration> {
    match file.rsplit('.').next().unwrap_or("") {
        "docx" | "odt" | "odp" | "pptx" => Some(Duration::from_secs(1)),
        "xlsx" => Some(Duration::from_secs(5)),
        "pdf" => Some(Duration::from_secs(2)),
        "md" | "rs" => Some(Duration::from_millis(50)),
        _ => None,
    }
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0}KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}
