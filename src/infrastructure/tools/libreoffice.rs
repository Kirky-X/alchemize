// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! LibreOffice headless external tool — runtime probe + document conversion.
//!
//! Only used when the `legacy-office` feature is compiled in and `soffice`
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

        let profile_url = format!(
            "-env:UserInstallation=file://{}",
            self.profile.path().display()
        );
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

    #[tokio::test]
    async fn probe_reports_unavailable_with_empty_path() {
        let _serial = lock_path().await;
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

    #[tokio::test]
    async fn on_path_matches_which_program() {
        let _serial = lock_path().await;
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

    /// PATH is process-global: serialize with the other env-mutating tests.
    /// Async-aware: callers hold the guard across `await` points
    /// (clippy::await_holding_lock).
    async fn lock_path() -> tokio::sync::MutexGuard<'static, ()> {
        crate::test_support::env_lock().await
    }

    /// Install a fake `soffice` script on PATH; returns the guard that
    /// restores PATH on drop and the script path.
    struct PathGuard {
        saved: Option<String>,
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            match &self.saved {
                Some(p) => unsafe { std::env::set_var("PATH", p) },
                None => unsafe { std::env::remove_var("PATH") },
            }
        }
    }

    fn fake_soffice(dir: &std::path::Path, body: &str) -> PathGuard {
        let script = dir.join("soffice");
        std::fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        let saved = std::env::var("PATH").ok();
        let mut paths = vec![dir.to_path_buf()];
        if let Some(p) = &saved {
            paths.extend(std::env::split_paths(p));
        }
        unsafe {
            std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        }
        PathGuard { saved }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn convert_reports_stderr_when_soffice_fails() {
        let _serial = lock_path().await;
        let tmp = tempfile::tempdir().unwrap();
        let _guard = fake_soffice(tmp.path(), "#!/bin/sh\necho boom >&2\nexit 3\n");
        let runner = LibreOfficeRunner::new().unwrap();
        let input = tmp.path().join("in.rtf");
        std::fs::write(&input, b"x").unwrap();
        let err = runner
            .convert(
                &input,
                "docx",
                "Office Open XML Text",
                &tmp.path().join("out"),
            )
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("boom"), "stderr must surface: {msg}");
        assert!(msg.contains('3'), "exit code must surface: {msg}");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn convert_errors_when_output_is_missing() {
        let _serial = lock_path().await;
        let tmp = tempfile::tempdir().unwrap();
        // Exits 0 but writes nothing.
        let _guard = fake_soffice(tmp.path(), "#!/bin/sh\nexit 0\n");
        let runner = LibreOfficeRunner::new().unwrap();
        let input = tmp.path().join("in.doc");
        std::fs::write(&input, b"x").unwrap();
        let err = runner
            .convert(
                &input,
                "docx",
                "Office Open XML Text",
                &tmp.path().join("out"),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("is missing"),
            "missing output must be reported: {err}"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn convert_succeeds_and_names_output_after_input_stem() {
        let _serial = lock_path().await;
        let tmp = tempfile::tempdir().unwrap();
        // A fake soffice that copies a prepared payload to <outdir>/<stem>.docx.
        let payload = tmp.path().join("payload.docx");
        std::fs::write(&payload, b"PKfake-docx").unwrap();
        let script = format!(
            "#!/bin/sh\n\
             outdir=\"\"\n\
             input=\"\"\n\
             prev=\"\"\n\
             for arg in \"$@\"; do\n\
               if [ \"$prev\" = \"--outdir\" ]; then outdir=\"$arg\"; fi\n\
               case \"$arg\" in *.doc|*.rtf) input=\"$arg\" ;; esac\n\
               prev=\"$arg\"\n\
             done\n\
             stem=$(basename \"$input\")\n\
             stem=${{stem%.*}}\n\
             cp '{}' \"$outdir/$stem.docx\"\n",
            payload.display()
        );
        let _guard = fake_soffice(tmp.path(), &script);
        let runner = LibreOfficeRunner::new().unwrap();
        let input = tmp.path().join("report.rtf");
        std::fs::write(&input, b"x").unwrap();
        let out_dir = tmp.path().join("out");
        let produced = runner
            .convert(&input, "docx", "Office Open XML Text", &out_dir)
            .await
            .unwrap();
        assert_eq!(produced, out_dir.join("report.docx"));
        assert_eq!(std::fs::read(produced).unwrap(), b"PKfake-docx");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn probe_accepts_a_working_binary_and_rejects_a_broken_one() {
        let _serial = lock_path().await;
        let tmp = tempfile::tempdir().unwrap();
        {
            let _guard = fake_soffice(tmp.path(), "#!/bin/sh\necho 'LibreOffice 25.2'\n");
            assert_eq!(LibreOfficeTool::probe(), Availability::Available);
            assert!(LibreOfficeTool::on_path());
        }
        {
            let _guard = fake_soffice(tmp.path(), "#!/bin/sh\nexit 9\n");
            match LibreOfficeTool::probe() {
                Availability::Unavailable { reason } => {
                    assert!(reason.contains("exit status: 9"), "{reason}")
                }
                Availability::Available => panic!("non-zero --version must be unavailable"),
            }
        }
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
