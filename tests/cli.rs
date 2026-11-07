// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! CLI end-to-end tests: exercise the `alch` binary (formats/doctor/convert,
//! exit codes, asset writing).

use std::process::{Command, Stdio};

fn alch() -> Command {
    let bin = env!("CARGO_BIN_EXE_alch");
    Command::new(bin)
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("alch-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn formats_lists_core_extensions() {
    let out = alch().arg("formats").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for ext in [
        "pdf", "docx", "csv", "srt", "png", "wav", "mp4", "tex", "py",
    ] {
        assert!(stdout.contains(ext), "formats missing {ext}: {stdout}");
    }
}

#[test]
fn doctor_reports_engines_and_zip_row() {
    let out = alch().arg("doctor").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("AVAILABLE"));
    assert!(stdout.contains("zip"));
}

#[test]
fn convert_txt_to_stdout_exit_zero() {
    let dir = temp_dir("stdout");
    let file = dir.join("hello.txt");
    std::fs::write(&file, "cli body").unwrap();
    let out = alch()
        .args(["convert", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cli body"));
}

#[test]
fn convert_md_to_file_writes_output() {
    let dir = temp_dir("outfile");
    let input = dir.join("in.md");
    let output = dir.join("out.md");
    std::fs::write(&input, "# T\n\nB").unwrap();
    let out = alch()
        .args([
            "convert",
            input.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = std::fs::read_to_string(&output).unwrap();
    assert!(written.contains("# T"));
}

#[test]
fn convert_py_gets_fenced_python() {
    let dir = temp_dir("py");
    let file = dir.join("m.py");
    std::fs::write(&file, "x = 1\n").unwrap();
    let out = alch()
        .args(["convert", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("```python"));
}

#[test]
fn unsupported_format_exits_two() {
    let dir = temp_dir("xyz");
    let file = dir.join("f.xyz");
    std::fs::write(&file, b"x").unwrap();
    let out = alch()
        .args(["convert", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unsupported format"));
}

#[test]
fn all_backends_failed_exits_three_with_attempts() {
    let dir = temp_dir("broken-docx");
    let file = dir.join("broken.docx");
    // Not a real docx: the only docx engine (native, default build) fails.
    std::fs::write(&file, b"garbage-not-a-zip").unwrap();
    let out = alch()
        .args(["convert", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("all backends failed"), "stderr: {stderr}");
    assert!(stderr.contains("docx:"));
}

#[test]
fn format_override_reroutes_extension() {
    let dir = temp_dir("override");
    let file = dir.join("photo.bin");
    std::fs::write(&file, "plain words").unwrap();
    // .bin is unsupported; --format txt routes it through the text engine.
    let out = alch()
        .args(["convert", file.to_str().unwrap(), "--format", "txt"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("plain words"));
}

#[test]
fn no_remote_flag_still_converts_local_formats() {
    let dir = temp_dir("noremote");
    let file = dir.join("t.csv");
    std::fs::write(&file, "a,b\n1,2\n").unwrap();
    let out = alch()
        .args(["convert", file.to_str().unwrap(), "--no-remote"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("| a | b |"));
}

#[test]
fn missing_input_reports_error_nonzero() {
    let out = alch()
        .args(["convert", "/nonexistent/alch-probe.txt"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found"));
}

#[test]
fn engine_pref_unknown_falls_back_to_chain() {
    let dir = temp_dir("pref");
    let file = dir.join("t.txt");
    std::fs::write(&file, "engine body").unwrap();
    let out = alch()
        .args(["convert", file.to_str().unwrap(), "--engine", "nonexistent"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("engine body"));
}

#[test]
fn inline_asset_mode_accepted() {
    let dir = temp_dir("inline");
    let file = dir.join("t.txt");
    std::fs::write(&file, "body").unwrap();
    let out = alch()
        .args([
            "convert",
            file.to_str().unwrap(),
            "-o",
            dir.join("o.md").to_str().unwrap(),
            "--assets",
            "inline",
        ])
        .stdout(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn version_flag_works() {
    let out = alch().arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("alch"));
}

/// Fake ffmpeg: consumes stdin, emits a minimal 16kHz mono WAV on stdout.
/// Injected via PATH so the video converter's extraction path is exercised
/// without a real ffmpeg install.
#[test]
fn convert_video_with_fake_ffmpeg_extracts_audio() {
    let dir = temp_dir("fake-ffmpeg");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    let wav: Vec<u8> = {
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&36u32.to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&16000u32.to_le_bytes());
        w.extend_from_slice(&32000u32.to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&0u32.to_le_bytes());
        w
    };
    std::fs::write(bin.join("wav.payload"), &wav).unwrap();

    let script = format!(
        "#!/bin/sh\ncat > /dev/null\ncat \"{}\"\nexit 0\n",
        bin.join("wav.payload").display()
    );
    let ffmpeg = bin.join("ffmpeg");
    std::fs::write(&ffmpeg, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let video = dir.join("clip.mp4");
    std::fs::write(&video, vec![0, 0, 0, 24, b'f', b't', b'y', b'p']).unwrap();

    let old_path = std::env::var("PATH").unwrap_or_default();
    let out = {
        let mut cmd = alch();
        cmd.args(["convert", video.to_str().unwrap()])
            .env("PATH", format!("{}:{}", bin.display(), old_path));
        cmd.output().unwrap()
    };
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("audio extracted but no ASR provider"),
        "stdout: {stdout}"
    );
}
