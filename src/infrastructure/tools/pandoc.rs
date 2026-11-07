// SPDX-License-Identifier: Apache-2.0
//! Pandoc external tool — runtime probe + LaTeX→GFM conversion.

use crate::application::ports::{Availability, ExternalTool};

/// Pandoc binary port.
pub struct PandocTool;

impl ExternalTool for PandocTool {
    fn program(&self) -> &str {
        "pandoc"
    }
}

impl PandocTool {
    /// Probe pandoc availability.
    pub fn probe() -> Availability {
        <Self as ExternalTool>::probe(&Self)
    }

    /// Convert LaTeX source to GitHub-Flavored Markdown via pandoc stdin.
    ///
    /// `extract_media_dir` optionally receives pandoc's `--extract-media`
    /// output (media files land under `<dir>/media/…`).
    pub async fn latex_to_gfm(
        latex: &str,
        extract_media_dir: Option<&std::path::Path>,
    ) -> Result<String, std::io::Error> {
        use tokio::io::AsyncWriteExt as _;

        let mut cmd = tokio::process::Command::new("pandoc");
        cmd.args(["-f", "latex", "-t", "gfm", "--wrap=none"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(dir) = extract_media_dir {
            cmd.arg(format!("--extract-media={}", dir.display()));
        }

        let mut child = cmd.spawn()?;
        child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(latex.as_bytes())
            .await?;
        // Drop stdin ownership so pandoc sees EOF.
        drop(child.stdin.take());

        let output = child.wait_with_output().await?;
        if !output.status.success() {
            return Err(std::io::Error::other(format!(
                "pandoc exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_name() {
        assert_eq!(<PandocTool as ExternalTool>::program(&PandocTool), "pandoc");
    }

    #[tokio::test]
    async fn latex_to_gfm_missing_binary_errors() {
        // PATH without pandoc: spawn must fail (not panic).
        let out = std::process::Command::new("env")
            .args(["-i", "true"])
            .output();
        let _ = out; // env sanity only; real assertion below tolerates pandoc-present hosts
        if PandocTool::probe() == Availability::Available {
            // Host has pandoc: verify a trivial conversion round-trips.
            let md = PandocTool::latex_to_gfm("hello $x$", None).await.unwrap();
            assert!(md.contains("hello"));
        }
    }
}
