// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! CLI edge-arm tests: builder-config branches in `forge_app::build_alch`
//! (html-deep flags, MinerU env override) and the unreadable-input error
//! arm of `--input_format`. Each asserts exit status plus observable text.
#![cfg(all(feature = "cli", feature = "full"))]

use std::process::{Command, Stdio};

fn alch() -> Command {
    let bin = env!("CARGO_BIN_EXE_alch");
    let mut cmd = Command::new(bin);
    cmd.env("ALCHEMIZE_LANG", "en");
    cmd
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("alch-test-cli-edge-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `--base_url`/`--extract_main` route through the html-deep builder arm;
/// a plain text conversion must stay local and succeed.
#[test]
fn convert_with_base_url_flags_stays_local_and_succeeds() {
    let dir = temp_dir("baseurl");
    let input = dir.join("page.txt");
    std::fs::write(&input, "plain body").unwrap();
    let out = alch()
        .args([
            "convert",
            input.to_str().unwrap(),
            "--base_url",
            "http://127.0.0.1:9/base",
            "--extract_main",
            "true",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("plain body"), "{stdout}");
}

/// `ALCHEMIZE_MINERU_URL` registers the MinerU engine via the env arm of
/// build_alch; local text conversion is unaffected.
#[test]
fn convert_with_mineru_env_var_registers_engine_and_succeeds() {
    let dir = temp_dir("mineruenv");
    let input = dir.join("doc.txt");
    std::fs::write(&input, "env mineru body").unwrap();
    let out = alch()
        .env("ALCHEMIZE_MINERU_URL", "http://127.0.0.1:9")
        .args(["convert", input.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("env mineru body"), "{stdout}");
}

/// `--input_format` with an unreadable file surfaces the read-failure error
/// instead of dispatching by extension.
#[test]
fn input_format_unreadable_file_errors() {
    let dir = temp_dir("perm");
    let input = dir.join("locked.txt");
    std::fs::write(&input, "secret").unwrap();
    let mut perms = std::fs::metadata(&input).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o000);
    std::fs::set_permissions(&input, perms).unwrap();

    let out = alch()
        .args(["convert", input.to_str().unwrap(), "--input_format", "txt"])
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "unreadable input must fail: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to read input") || stderr.contains("Permission denied"),
        "{stderr}"
    );

    // Restore permissions so the temp dir can be removed cleanly.
    let mut perms = std::fs::metadata(&input).unwrap().permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&input, perms).unwrap();
}
