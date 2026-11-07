// Copyright © 2026 Kirky.X. All rights reserved.
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
                if let Ok(desc) = provider.describe(&frame_data, crate::domain::result::DetailLevel::Low).await {
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
}
