// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Video to Markdown converter with degradation strategy.
//!
//! Processing depends on `VideoProcessingMode`:
//! - `Gpu`: audio extraction + transcription + frame analysis via VisionProvider (requires ffmpeg)
//! - `AudioOnly`: audio extraction only (requires ffmpeg)
//! - `Disabled`: metadata only

use std::sync::Arc;

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Supported video file extensions.
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "avi", "mkv", "mov", "wmv", "flv", "webm", "m4v"];

use crate::application::ports::{TranscribeProvider, VisionProvider};
use crate::domain::result::VideoProcessingMode;

/// Converts video files to markdown with a graceful degradation strategy.
///
/// The converter checks `VideoProcessingMode` at construction time:
/// - `Gpu`: extracts audio via ffmpeg, transcribes it, then extracts key
///   frames for visual analysis via the VisionProvider.
/// - `AudioOnly`: extracts audio via ffmpeg and transcribes it.
/// - `Disabled`: returns file metadata as markdown.
///
/// Audio transcription is injected as a callback to avoid coupling to
/// mnemis-model. Frame analysis uses an optional VisionProvider.
pub struct VideoConverter {
    mode: VideoProcessingMode,
    ffmpeg_available: bool,
    transcribe: Option<Arc<dyn TranscribeProvider>>,
    vision_provider: Option<Arc<dyn VisionProvider>>,
}

impl VideoConverter {
    /// Create a new video converter with the given processing mode.
    pub fn new(mode: VideoProcessingMode) -> Self {
        let ffmpeg_available = Self::detect_ffmpeg();
        Self {
            mode,
            ffmpeg_available,
            transcribe: None,
            vision_provider: None,
        }
    }

    /// Attach a transcription function for audio extraction.
    pub fn with_transcriber(mut self, provider: Arc<dyn TranscribeProvider>) -> Self {
        self.transcribe = Some(provider);
        self
    }

    /// Attach a vision provider for frame analysis (Gpu mode).
    pub fn with_vision_provider(mut self, provider: Arc<dyn VisionProvider>) -> Self {
        self.vision_provider = Some(provider);
        self
    }

    /// Check if ffmpeg is available on the system.
    fn detect_ffmpeg() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

impl Default for VideoConverter {
    fn default() -> Self {
        Self::new(VideoProcessingMode::default())
    }
}

#[async_trait]
impl Converter for VideoConverter {
    fn name(&self) -> &'static str {
        "video"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read video input: {e}")))?;

        let filename = input.filename().unwrap_or("unknown").to_string();
        let byte_count = bytes.len();

        // If video processing is disabled, return metadata only.
        if matches!(self.mode, VideoProcessingMode::Disabled) {
            return self.metadata_only_result(&filename, byte_count);
        }

        // If ffmpeg is not available, we cannot extract audio or frames.
        if !self.ffmpeg_available {
            tracing::warn!("ffmpeg not found — video processing degraded to metadata only");
            return self.metadata_only_result(&filename, byte_count);
        }

        // Attempt audio extraction via ffmpeg.
        let audio_data = self.extract_audio(&bytes, &filename).await;

        match audio_data {
            Ok(audio_bytes) => {
                // Transcribe the extracted audio.
                if let Some(ref provider) = self.transcribe {
                    let transcript = provider.transcribe(&audio_bytes).await.map_err(|e| {
                        AlchemizeError::AudioUnavailable(format!("transcription failed: {e}"))
                    })?;
                    let transcript = transcript.full_text();

                    // Frame analysis for Gpu mode with VisionProvider.
                    let frame_descriptions =
                        if self.mode.supports_frame_analysis() && self.vision_provider.is_some() {
                            // Estimate duration from byte count (rough heuristic).
                            let estimated_duration = byte_count as f64 / 1_000_000.0 * 10.0;
                            self.analyze_frames(&bytes, estimated_duration)
                                .await
                                .ok()
                                .unwrap_or_default()
                        } else {
                            vec![]
                        };

                    let markdown = if transcript.trim().is_empty() && frame_descriptions.is_empty()
                    {
                        format!(
                            "*Video '{filename}' — transcription produced no \
                             text.*\n"
                        )
                    } else {
                        let mut md = String::new();
                        if !transcript.trim().is_empty() {
                            md.push_str(&format!("## Video Transcript\n\n{transcript}\n"));
                        }
                        if !frame_descriptions.is_empty() {
                            md.push_str("\n## Key Frames\n\n");
                            for desc in &frame_descriptions {
                                md.push_str(&format!("{desc}\n\n"));
                            }
                        }
                        md
                    };

                    let metadata = serde_json::json!({
                        "filename": filename,
                        "byte_count": byte_count,
                        "processing_mode": format!("{}", self.mode),
                        "audio_extracted": true,
                        "transcribed": true,
                        "frame_analysis": frame_descriptions.len(),
                        "source_format": "video",
                    });

                    Ok(
                        ConvertResult::with_metadata(markdown, ContentType::Video, metadata)
                            .with_quality(ConversionQuality::LocalFull),
                    )
                } else {
                    // Audio extracted but no transcriber available.
                    let markdown = format!(
                        "*Video '{filename}' — audio extracted but no ASR \
                         provider configured.*\n"
                    );

                    let metadata = serde_json::json!({
                        "filename": filename,
                        "byte_count": byte_count,
                        "processing_mode": format!("{}", self.mode),
                        "audio_extracted": true,
                        "transcribed": false,
                        "source_format": "video",
                    });

                    Ok(
                        ConvertResult::with_metadata(markdown, ContentType::Video, metadata)
                            .with_quality(ConversionQuality::LocalFallback),
                    )
                }
            }
            Err(e) => {
                tracing::warn!("audio extraction failed for '{filename}': {e}");
                self.metadata_only_result(&filename, byte_count)
            }
        }
    }

    fn supported_extensions(&self) -> &[&str] {
        VIDEO_EXTENSIONS
    }
}

impl VideoConverter {
    /// Return a metadata-only result when video processing is not possible.
    fn metadata_only_result(
        &self,
        filename: &str,
        byte_count: usize,
    ) -> Result<ConvertResult, AlchemizeError> {
        let markdown = format!(
            "*Video '{filename}' ({byte_count} bytes) — video processing \
             unavailable (mode: {}).*\n",
            self.mode
        );

        let metadata = serde_json::json!({
            "filename": filename,
            "byte_count": byte_count,
            "processing_mode": format!("{}", self.mode),
            "audio_extracted": false,
            "transcribed": false,
            "source_format": "video",
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Video, metadata)
                .with_quality(ConversionQuality::LocalFallback),
        )
    }

    /// Extract audio from video bytes using ffmpeg.
    ///
    /// Converts to 16kHz mono WAV for ASR compatibility.
    async fn extract_audio(&self, video_bytes: &[u8], _filename: &str) -> anyhow::Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;

        let mut child = tokio::process::Command::new("ffmpeg")
            .args([
                "-i",
                "pipe:0",
                "-vn", // No video.
                "-acodec",
                "pcm_s16le", // 16-bit PCM.
                "-ar",
                "16000", // 16kHz sample rate.
                "-ac",
                "1", // Mono.
                "-f",
                "wav",
                "pipe:1", // Output to stdout.
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn ffmpeg: {e}"))?;

        // Write video bytes to ffmpeg stdin.
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(video_bytes)
                .await
                .map_err(|e| anyhow::anyhow!("failed to write to ffmpeg stdin: {e}"))?;
            drop(stdin); // Close stdin to signal EOF.
        }

        // Read audio output from ffmpeg stdout.
        let mut audio_output = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout
                .read_to_end(&mut audio_output)
                .await
                .map_err(|e| anyhow::anyhow!("failed to read ffmpeg stdout: {e}"))?;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| anyhow::anyhow!("failed to wait for ffmpeg: {e}"))?;

        if !status.success() {
            return Err(anyhow::anyhow!("ffmpeg exited with status: {status}"));
        }

        Ok(audio_output)
    }

    /// Extract a single frame from video bytes using ffmpeg.
    ///
    /// Extracts frame at specified timestamp (seconds) as JPEG.
    async fn extract_frame(
        &self,
        video_bytes: &[u8],
        timestamp_secs: f64,
    ) -> anyhow::Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;

        let mut child = tokio::process::Command::new("ffmpeg")
            .args([
                "-i",
                "pipe:0",
                "-ss",
                &format!("{:.2}", timestamp_secs),
                "-vframes",
                "1",
                "-f",
                "image2",
                "-vcodec",
                "mjpeg",
                "pipe:1",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn ffmpeg for frame extraction: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(video_bytes)
                .await
                .map_err(|e| anyhow::anyhow!("failed to write video to ffmpeg stdin: {e}"))?;
            drop(stdin);
        }

        let mut frame_output = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout
                .read_to_end(&mut frame_output)
                .await
                .map_err(|e| anyhow::anyhow!("failed to read frame from ffmpeg stdout: {e}"))?;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| anyhow::anyhow!("failed to wait for ffmpeg: {e}"))?;
        if !status.success() {
            return Err(anyhow::anyhow!(
                "ffmpeg frame extraction failed with status: {status}"
            ));
        }

        Ok(frame_output)
    }

    /// Extract key frames at regular intervals and describe them via VisionProvider.
    async fn analyze_frames(
        &self,
        video_bytes: &[u8],
        duration_secs: f64,
    ) -> anyhow::Result<Vec<String>> {
        if self.vision_provider.is_none() {
            return Ok(vec![]);
        }

        let provider = self
            .vision_provider
            .as_ref()
            .expect("vision_provider checked above");
        let frame_count = ((duration_secs / 30.0).ceil() as usize).clamp(1, 5);
        let interval = duration_secs / frame_count as f64;

        let mut descriptions = Vec::new();
        for i in 0..frame_count {
            let timestamp = i as f64 * interval;
            if let Ok(frame_data) = self.extract_frame(video_bytes, timestamp).await {
                if let Ok(desc) = provider
                    .describe(&frame_data, crate::domain::result::DetailLevel::Low)
                    .await
                {
                    if !desc.trim().is_empty() {
                        descriptions.push(format!("Frame @ {:.1}s: {}", timestamp, desc));
                    }
                }
            }
        }

        Ok(descriptions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_video_extensions() {
        let converter = VideoConverter::new(VideoProcessingMode::AudioOnly);
        let exts = converter.supported_extensions();
        assert!(exts.contains(&"mp4"));
        assert!(exts.contains(&"avi"));
        assert!(exts.contains(&"mkv"));
        assert!(exts.contains(&"webm"));
    }

    #[test]
    fn can_convert_mp4() {
        let converter = VideoConverter::new(VideoProcessingMode::AudioOnly);
        let input = Input::Bytes {
            data: vec![],
            filename: "clip.mp4".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(converter.can_convert(&input));
    }

    #[test]
    fn cannot_convert_pdf() {
        let converter = VideoConverter::new(VideoProcessingMode::AudioOnly);
        let input = Input::Bytes {
            data: vec![],
            filename: "doc.pdf".to_string(),
            mimetype: None,
            extension: None,
        };
        assert!(!converter.can_convert(&input));
    }

    #[tokio::test]
    async fn disabled_mode_returns_metadata() {
        let converter = VideoConverter::new(VideoProcessingMode::Disabled);
        let input = Input::Bytes {
            data: vec![0x00, 0x00, 0x00, 0x20],
            filename: "video.mp4".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await.unwrap();
        assert_eq!(result.content_type, ContentType::Video);
        assert!(result.markdown.contains("video processing unavailable"));
        assert!(result.markdown.contains("disabled"));
    }

    #[test]
    fn default_uses_audio_only_mode() {
        let converter = VideoConverter::default();
        assert_eq!(converter.mode, VideoProcessingMode::AudioOnly);
    }

    #[test]
    fn video_processing_mode_display() {
        assert_eq!(format!("{}", VideoProcessingMode::Gpu), "gpu");
        assert_eq!(format!("{}", VideoProcessingMode::AudioOnly), "audio_only");
        assert_eq!(format!("{}", VideoProcessingMode::Disabled), "disabled");
    }

    #[test]
    fn video_processing_mode_capabilities() {
        assert!(VideoProcessingMode::Gpu.supports_frame_analysis());
        assert!(VideoProcessingMode::Gpu.supports_audio());
        assert!(!VideoProcessingMode::AudioOnly.supports_frame_analysis());
        assert!(VideoProcessingMode::AudioOnly.supports_audio());
        assert!(!VideoProcessingMode::Disabled.supports_frame_analysis());
        assert!(!VideoProcessingMode::Disabled.supports_audio());
    }

    fn converter_with_ffmpeg_flag(mode: VideoProcessingMode, ffmpeg: bool) -> VideoConverter {
        VideoConverter {
            mode,
            transcribe: None,
            vision_provider: None,
            ffmpeg_available: ffmpeg,
        }
    }

    struct StubTranscriber(&'static str);

    #[async_trait::async_trait]
    impl crate::application::ports::TranscribeProvider for StubTranscriber {
        async fn transcribe(
            &self,
            _audio: &[u8],
        ) -> Result<crate::application::ports::Transcript, AlchemizeError> {
            Ok(crate::application::ports::Transcript {
                language: None,
                duration: Some(1.0),
                segments: vec![crate::application::ports::TranscriptSegment {
                    start_sec: 0.0,
                    end_sec: 1.0,
                    text: self.0.to_string(),
                }],
            })
        }
        fn availability(&self) -> crate::application::ports::Availability {
            crate::application::ports::Availability::Available
        }
        fn name(&self) -> &'static str {
            "stub-video"
        }
    }

    struct StubVision(Option<&'static str>);

    #[async_trait::async_trait]
    impl crate::application::ports::VisionProvider for StubVision {
        async fn describe(
            &self,
            _image_data: &[u8],
            _detail: crate::domain::result::DetailLevel,
        ) -> anyhow::Result<String> {
            match self.0 {
                Some(text) => Ok(text.to_string()),
                None => Err(anyhow::anyhow!("vision provider down")),
            }
        }
        async fn ocr(&self, _image_data: &[u8]) -> anyhow::Result<String> {
            Ok(String::new())
        }
        fn name(&self) -> &'static str {
            "stub-vision"
        }
    }

    /// Restores `PATH` on drop so a panicking assertion cannot poison it.
    struct PathRestore(Option<String>);

    impl Drop for PathRestore {
        fn drop(&mut self) {
            match &self.0 {
                Some(p) => unsafe { std::env::set_var("PATH", p) },
                None => unsafe { std::env::remove_var("PATH") },
            }
        }
    }

    /// A fake `ffmpeg` that answers audio calls with a WAV payload and frame
    /// calls (`-vframes`) with a JPEG payload. `frame_exit` overrides the exit
    /// status for frame calls.
    fn fake_ffmpeg(dir: &std::path::Path, frame_exit: Option<i32>) -> PathRestore {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&36u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&[1, 0, 1, 0]);
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&32000u32.to_le_bytes());
        wav.extend_from_slice(&[2, 0, 16, 0]);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&0u32.to_le_bytes());
        std::fs::write(dir.join("wav.payload"), &wav).unwrap();
        std::fs::write(dir.join("jpeg.payload"), b"\xff\xd8\xff\xe0fake-jpeg").unwrap();
        let frame_branch = match frame_exit {
            Some(code) => format!("exit {code}"),
            None => format!("cat '{}'", dir.join("jpeg.payload").display()),
        };
        let script = format!(
            "#!/bin/sh\n\
             cat > /dev/null\n\
             case \"$*\" in\n\
               *-vframes*) {frame_branch} ;;\n\
               *) cat '{}' ;;\n\
             esac\n\
             exit 0\n",
            dir.join("wav.payload").display()
        );
        let fake = dir.join("ffmpeg");
        std::fs::write(&fake, script).unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let saved = std::env::var("PATH").ok();
        let mut paths = vec![dir.to_path_buf()];
        if let Some(p) = &saved {
            paths.extend(std::env::split_paths(p));
        }
        unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };
        PathRestore(saved)
    }

    #[test]
    fn constructor_builders_keep_mode_and_providers() {
        let converter = VideoConverter::new(VideoProcessingMode::Gpu)
            .with_transcriber(std::sync::Arc::new(StubTranscriber("t")))
            .with_vision_provider(std::sync::Arc::new(StubVision(Some("v"))));
        assert_eq!(converter.mode, VideoProcessingMode::Gpu);
        assert!(converter.transcribe.is_some());
        assert!(converter.vision_provider.is_some());
        assert_eq!(converter.name(), "video");
    }

    #[tokio::test]
    async fn gpu_mode_describes_key_frames() {
        let _serial = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let _path = fake_ffmpeg(dir.path(), None);
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::Gpu, true);
        converter.transcribe = Some(std::sync::Arc::new(StubTranscriber("hello world")));
        converter.vision_provider =
            Some(std::sync::Arc::new(StubVision(Some("a person speaking"))));
        // 3MB → estimated duration 30s → exactly one 30s frame interval.
        let input = Input::bytes(vec![0u8; 3_000_000], "talk.mp4");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("## Video Transcript"));
        assert!(result.markdown.contains("hello world"));
        assert!(
            result.markdown.contains("## Key Frames"),
            "{}",
            result.markdown
        );
        assert!(
            result.markdown.contains("Frame @ 0.0s: a person speaking"),
            "{}",
            result.markdown
        );
        assert_eq!(result.metadata["frame_analysis"], 1);
        assert_eq!(result.metadata["transcribed"], true);
        assert_eq!(result.quality, ConversionQuality::LocalFull);
    }

    #[tokio::test]
    async fn frame_provider_failure_is_skipped_without_dropping_audio() {
        let _serial = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let _path = fake_ffmpeg(dir.path(), None);
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::Gpu, true);
        converter.transcribe = Some(std::sync::Arc::new(StubTranscriber("narration")));
        // Vision provider errors: frame descriptions stay empty.
        converter.vision_provider = Some(std::sync::Arc::new(StubVision(None)));
        let input = Input::bytes(vec![0u8; 3_000_000], "talk.mp4");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("narration"));
        assert!(
            !result.markdown.contains("## Key Frames"),
            "{}",
            result.markdown
        );
        assert_eq!(result.metadata["frame_analysis"], 0);
    }

    #[tokio::test]
    async fn frame_extraction_failure_is_skipped() {
        let _serial = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let _path = fake_ffmpeg(dir.path(), Some(7));
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::Gpu, true);
        converter.transcribe = Some(std::sync::Arc::new(StubTranscriber("speech")));
        converter.vision_provider = Some(std::sync::Arc::new(StubVision(Some("unused"))));
        let input = Input::bytes(vec![0u8; 3_000_000], "talk.mp4");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("speech"));
        assert!(!result.markdown.contains("Key Frames"));
        assert_eq!(result.metadata["frame_analysis"], 0);
    }

    #[tokio::test]
    async fn empty_transcript_without_frames_reports_no_text() {
        let _serial = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let _path = fake_ffmpeg(dir.path(), None);
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::Gpu, true);
        converter.transcribe = Some(std::sync::Arc::new(StubTranscriber("   ")));
        let input = Input::bytes(vec![0u8; 10_000], "silent.mp4");
        let result = converter.convert(input).await.unwrap();
        assert!(
            result.markdown.contains("transcription produced no text"),
            "{}",
            result.markdown
        );
        // analyze_frames early-returns without a vision provider.
        let frames = converter.analyze_frames(b"x", 120.0).await.unwrap();
        assert!(frames.is_empty());
    }

    #[tokio::test]
    async fn audio_extracted_without_transcriber_is_reported() {
        let _serial = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let _path = fake_ffmpeg(dir.path(), None);
        let converter = converter_with_ffmpeg_flag(VideoProcessingMode::AudioOnly, true);
        let input = Input::bytes(vec![0u8; 1024], "clip.mp4");
        let result = converter.convert(input).await.unwrap();
        assert!(
            result.markdown.contains("no ASR provider configured"),
            "{}",
            result.markdown
        );
        assert_eq!(result.metadata["audio_extracted"], true);
        assert_eq!(result.metadata["transcribed"], false);
    }

    #[tokio::test]
    async fn ffmpeg_extraction_failure_degrades_to_metadata() {
        let _serial = crate::test_support::env_lock();
        // PATH stripped to an empty dir: the ffmpeg spawn fails inside
        // extract_audio and the converter degrades — deterministic on any host.
        let empty_dir = tempfile::tempdir().unwrap();
        let restore = PathRestore(Some(std::env::var("PATH").unwrap_or_default()));
        unsafe { std::env::set_var("PATH", empty_dir.path()) };
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::AudioOnly, true);
        converter.transcribe = Some(std::sync::Arc::new(StubTranscriber("should not run")));
        let input = Input::bytes(vec![0, 0, 0, 24, b'f', b't', b'y', b'p'], "v.mp4");
        let result = converter.convert(input).await.unwrap();
        assert!(!result.markdown.is_empty());
        assert!(!result.markdown.contains("should not run"));
        drop(restore);
    }

    #[tokio::test]
    async fn gpu_mode_without_ffmpeg_still_degrades() {
        let _serial = crate::test_support::env_lock();
        // PATH stripped to an empty dir: the ffmpeg spawn fails
        // deterministically on any host.
        let empty_dir = tempfile::tempdir().unwrap();
        let restore = PathRestore(Some(std::env::var("PATH").unwrap_or_default()));
        unsafe { std::env::set_var("PATH", empty_dir.path()) };
        let converter = converter_with_ffmpeg_flag(VideoProcessingMode::Gpu, true);
        let input = Input::bytes(vec![0, 0, 0, 24, b'f', b't', b'y', b'p'], "v.mp4");
        let result = converter.convert(input).await.unwrap();
        assert!(!result.markdown.is_empty());
        drop(restore);
    }

    #[tokio::test]
    async fn ffmpeg_extraction_success_with_transcriber() {
        let _serial = crate::test_support::env_lock();
        // Fake ffmpeg on PATH: emits a canned WAV; stub transcriber runs.
        let dir = tempfile::tempdir().unwrap();
        let wav: Vec<u8> = {
            let mut w = Vec::new();
            w.extend_from_slice(b"RIFF");
            w.extend_from_slice(&36u32.to_le_bytes());
            w.extend_from_slice(b"WAVEfmt ");
            w.extend_from_slice(&16u32.to_le_bytes());
            w.extend_from_slice(&[1, 0, 1, 0]);
            w.extend_from_slice(&16000u32.to_le_bytes());
            w.extend_from_slice(&32000u32.to_le_bytes());
            w.extend_from_slice(&[2, 0, 16, 0]);
            w.extend_from_slice(b"data");
            w.extend_from_slice(&0u32.to_le_bytes());
            w
        };
        std::fs::write(dir.path().join("wav.payload"), &wav).unwrap();
        let script = format!(
            "#!/bin/sh\ncat > /dev/null\ncat '{}'\nexit 0\n",
            dir.path().join("wav.payload").display()
        );
        let fake = dir.path().join("ffmpeg");
        std::fs::write(&fake, script).unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            use std::sync::OnceLock;
            static CHMOD: OnceLock<()> = OnceLock::new();
            let _ = fake.as_os_str().to_string_lossy();
            let _ = CHMOD.set(());
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let old_path = std::env::var("PATH").unwrap_or_default();
        // Tests run multi-threaded: mutate env inside the unsafe block (2024
        // edition) and restore immediately after the conversion completes.
        unsafe { std::env::set_var("PATH", format!("{}:{}", dir.path().display(), old_path)) };
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::AudioOnly, true);
        converter.transcribe = Some(std::sync::Arc::new(StubTranscriber("stub transcript")));
        let input = Input::bytes(vec![0, 0, 0, 24, b'f', b't', b'y', b'p'], "v.mp4");
        let result = converter.convert(input).await.unwrap();
        unsafe { std::env::set_var("PATH", old_path) };

        assert!(
            result.markdown.contains("stub transcript"),
            "md: {}",
            result.markdown
        );
        assert!(result.markdown.contains("## Video Transcript"));
        assert_eq!(result.quality, ConversionQuality::LocalFull);
    }

    #[tokio::test]
    async fn audioonly_with_provider_and_ffmpeg_missing_stubs_not_called() {
        let _serial = crate::test_support::env_lock();
        let empty_dir = tempfile::tempdir().unwrap();
        let restore = PathRestore(Some(std::env::var("PATH").unwrap_or_default()));
        unsafe { std::env::set_var("PATH", empty_dir.path()) };
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::AudioOnly, true);
        converter.transcribe = Some(std::sync::Arc::new(StubTranscriber("stub text")));
        let input = Input::bytes(b"RIFF0000WAVE".to_vec(), "a.mp4");
        let result = converter.convert(input).await.unwrap();
        // extraction failed -> degraded metadata path, stub never invoked
        assert!(!result.markdown.contains("stub text"));
        drop(restore);
    }

    struct FailingTranscriber;

    #[async_trait::async_trait]
    impl crate::application::ports::TranscribeProvider for FailingTranscriber {
        async fn transcribe(
            &self,
            _audio: &[u8],
        ) -> Result<crate::application::ports::Transcript, AlchemizeError> {
            Err(AlchemizeError::AudioUnavailable("asr down".into()))
        }
        fn availability(&self) -> crate::application::ports::Availability {
            crate::application::ports::Availability::Available
        }
        fn name(&self) -> &'static str {
            "failing"
        }
    }

    #[tokio::test]
    async fn transcription_failure_surfaces_audio_unavailable() {
        let _serial = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let _path = fake_ffmpeg(dir.path(), None);
        let mut converter = converter_with_ffmpeg_flag(VideoProcessingMode::AudioOnly, true);
        converter.transcribe = Some(std::sync::Arc::new(FailingTranscriber));
        let input = Input::bytes(vec![0u8; 1024], "v.mp4");
        let err = converter.convert(input).await.unwrap_err();
        assert!(
            matches!(err, AlchemizeError::AudioUnavailable(_)),
            "{err:?}"
        );
    }

    #[test]
    fn stub_vision_trait_methods_run() {
        // Direct invocation covers the stub impl arms the converter never hits.
        let vision = StubVision(Some("desc"));
        assert_eq!(vision.name(), "stub-vision");
        let described =
            futures_block(vision.describe(b"x", crate::domain::result::DetailLevel::Low))
                .expect("stub describe");
        assert_eq!(described, "desc");
        assert_eq!(futures_block(vision.ocr(b"x")).expect("stub ocr"), "");
    }

    fn futures_block<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    #[test]
    fn stub_trait_metadata_methods_run() {
        // The stub impls. availability/name arms are never hit through the
        // converter; call them directly so the impls are fully covered.
        let transcribe = StubTranscriber("t");
        assert!(matches!(
            crate::application::ports::TranscribeProvider::availability(&transcribe),
            crate::application::ports::Availability::Available
        ));
        assert_eq!(
            crate::application::ports::TranscribeProvider::name(&transcribe),
            "stub-video"
        );
        let vision = StubVision(None);
        assert_eq!(
            crate::application::ports::VisionProvider::name(&vision),
            "stub-vision"
        );
    }
}
