// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! LibreOffice headless external tool — runtime probe + document conversion.
//!
//! Only used when the `libreoffice` feature is compiled in and `soffice`
//! resolves on PATH. Probe is a 5-second `--version` call; conversions run in
//! an isolated profile directory (`-env:UserInstallation`) so a running
//! desktop LibreOffice never conflicts, with a 300-second timeout and
//! at-most-one concurrent conversion (LibreOffice is not safely parallel).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::application::ports::{Availability, ExternalTool, which_program};

/// Conversion timeout (Task T015: 300 s).
pub const CONVERSION_TIMEOUT: Duration = Duration::from_secs(300);
/// Probe timeout (Task T015: 5 s).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// LibreOffice binary port.
pub struct LibreOfficeTool;

impl ExternalTool for LibreOfficeTool {
    fn program(&self) -> &str {
        // macOS installs a `soffice` shim on PATH with LibreOffice.app.
        "soffice"
    }

    fn probe(&self) -> Availability {
        if !which_program(self.program()) {
            return Availability::Unavailable {
                reason: "'soffice' not found on PATH (install LibreOffice)".to_string(),
            };
        }
        // A PATH hit is not enough: run `--version` with a hard timeout.
        let program = self.program().to_string();
        let status = std::process::Command::new(&program)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();
        match status {
            Ok(out) if out.status.success() => Availability::Available,
            Ok(out) => Availability::Unavailable {
                reason: format!(
                    "soffice --version exited {}: {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            },
            Err(e) => Availability::Unavailable {
                reason: format!("soffice probe failed: {e}"),
            },
        }
    }
}

impl LibreOfficeTool {
    /// Probe availability with the version check.
    pub fn probe() -> Availability {
        <Self as ExternalTool>::probe(&Self)
    }

    /// Whether the binary exists at all (no version call). Cheap gate used by
    /// converter `availability()`, which must stay synchronous and fast.
    pub fn on_path() -> bool {
        which_program("soffice")
    }
}

/// One-shot converter around the `soffice` binary.
///
/// Holds the profile directory and the concurrency permit; dropping the
/// converter removes the temporary profile.
pub struct LibreOfficeRunner {
    profile: tempfile::TempDir,
    permits: Arc<Semaphore>,
}

impl LibreOfficeRunner {
    /// Create a runner with an isolated profile directory.
    pub fn new() -> Result<Self, std::io::Error> {
        let profile = tempfile::Builder::new().prefix("alch-lo-").tempdir()?;
        Ok(Self {
            profile,
            permits: Arc::new(Semaphore::new(1)),
        })
    }

    /// The isolated user profile directory.
    pub fn profile_dir(&self) -> &Path {
        self.profile.path()
    }

    /// Convert `input` (already staged on disk) to `target` format using the
    /// given filter, writing the result into `out_dir`.
    ///
    /// Returns the produced file path. Errors carry soffice's stderr so a
    /// failed conversion is never silent.
    pub async fn convert(
        &self,
        input: &Path,
        target: &str,
        filter: &str,
        out_dir: &Path,
    ) -> Result<PathBuf, std::io::Error> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|e| std::io::Error::other(format!("soffice semaphore closed: {e}")))?;
        std::fs::create_dir_all(out_dir)?;

        let profile_url = format!("-env:UserInstallation=file://{}", self.profile.path().display());
        let mut cmd = tokio::process::Command::new("soffice");
        cmd.arg(profile_url)
            .arg("--headless")
            .arg("--norestore")
            .arg("--nolockcheck")
            .arg("--convert-to")
            .arg(format!("{target}:{filter}"))
            .arg("--outdir")
            .arg(out_dir)
            .arg(input)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd.spawn()?;
        let output = timeout(CONVERSION_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                std::io::Error::other(format!(
                    "soffice conversion timed out after {}s",
                    CONVERSION_TIMEOUT.as_secs()
                ))
            })??;
        if !output.status.success() {
            return Err(std::io::Error::other(format!(
                "soffice exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        // soffice names the output after the input stem.
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "out".to_string());
        let produced = out_dir.join(format!("{stem}.{target}"));
        if !produced.is_file() {
            return Err(std::io::Error::other(format!(
                "soffice reported success but {} is missing",
                produced.display()
            )));
        }
        Ok(produced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reports_unavailable_with_empty_path() {
        // Simulate a machine without LibreOffice: empty PATH.
        let saved = std::env::var("PATH").ok();
        unsafe { std::env::set_var("PATH", "/nonexistent-dir") };
        match LibreOfficeTool::probe() {
            Availability::Unavailable { reason } => {
                assert!(reason.contains("soffice"), "{reason}");
            }
            Availability::Available => panic!("must be unavailable with empty PATH"),
        }
        if let Some(saved) = saved {
            unsafe { std::env::set_var("PATH", saved) };
        }
    }

    #[test]
    fn on_path_matches_which_program() {
        assert_eq!(LibreOfficeTool::on_path(), which_program("soffice"));
    }

    #[test]
    fn runner_creates_isolated_profile() {
        let Ok(runner) = LibreOfficeRunner::new() else {
            return; // no temp dir available in this environment
        };
        assert!(runner.profile_dir().is_dir());
        let dir = runner.profile_dir().to_path_buf();
        drop(runner);
        assert!(!dir.exists(), "profile must be removed on drop");
    }

    #[tokio::test]
    #[ignore = "requires a real soffice binary on PATH"]
    async fn real_soffice_converts_to_docx() {
        assert_eq!(LibreOfficeTool::probe(), Availability::Available);
        let dir = tempfile::tempdir().unwrap();
        // A minimal RTF: soffice opens it and writes a .docx.
        let input = dir.path().join("sample.rtf");
        std::fs::write(
            &input,
            br"{\rtf1\ansi\deff0 {\fonttbl {\f0 Times;}}\f0 Hello from RTF\par}",
        )
        .unwrap();
        let runner = LibreOfficeRunner::new().unwrap();
        let out = dir.path().join("out");
        let produced = runner
            .convert(&input, "docx", "Office Open XML Text", &out)
            .await
            .unwrap();
        assert!(produced.is_file());
        let bytes = std::fs::read(&produced).unwrap();
        assert!(bytes.starts_with(b"PK"), "docx must be a zip");
    }
}
