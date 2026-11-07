// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Subtitle file converter (srt/vtt/ass/ssa) — native aspasia-backed
//! parsing (ASS parses directly; no ffmpeg detour needed).

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

use super::subtitle::{SubtitleFormat, SubtitleParser, subtitles_to_simple_markdown};
use crate::infrastructure::document::detect_encoding as shared_text_decode;

/// Converts standalone subtitle files to a timestamped markdown list.
pub struct SubtitleConverter;

impl SubtitleConverter {
    /// Create a new subtitle converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SubtitleConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for SubtitleConverter {
    fn name(&self) -> &'static str {
        "subtitle"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read subtitle input: {e}")))?;
        let (text, encoding) = shared_text_decode(&bytes);

        let ext = input.extension().unwrap_or_default();
        let format_hint = SubtitleFormat::from_extension(&ext);
        let parser = SubtitleParser::new();
        let events = parser.parse(&text, format_hint);

        let markdown = if events.is_empty() {
            format!("*Subtitle file contains no cues ({ext}).*\n")
        } else {
            subtitles_to_simple_markdown(&events)
        };

        let metadata = serde_json::json!({
            "source_format": ext,
            "cue_count": events.len(),
            "encoding": encoding,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["srt", "vtt", "ass", "ssa"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        assert_eq!(
            SubtitleConverter::new().supported_extensions(),
            &["srt", "vtt", "ass", "ssa"]
        );
    }

    #[tokio::test]
    async fn convert_srt_three_cues() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:02,500 --> 00:00:03,500\nWorld\n\n3\n00:00:04,000 --> 00:00:05,000\nAgain\n";
        let input = Input::bytes(srt.as_bytes().to_vec(), "sample.srt");
        let result = SubtitleConverter::new().convert(input).await.unwrap();
        assert!(result.markdown.contains("Hello"));
        assert!(result.markdown.contains("Again"));
        assert_eq!(result.metadata["cue_count"], 3);
    }

    #[tokio::test]
    async fn convert_vtt_strips_header() {
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.000\nCue text\n";
        let input = Input::bytes(vtt.as_bytes().to_vec(), "sample.vtt");
        let result = SubtitleConverter::new().convert(input).await.unwrap();
        assert!(result.markdown.contains("Cue text"));
        assert!(!result.markdown.contains("WEBVTT"));
    }

    #[tokio::test]
    async fn convert_ass_dialogue() {
        let ass = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hello ASS\n";
        let input = Input::bytes(ass.as_bytes().to_vec(), "sample.ass");
        let result = SubtitleConverter::new().convert(input).await.unwrap();
        assert!(
            result.markdown.contains("Hello ASS"),
            "md: {}",
            result.markdown
        );
    }

    #[tokio::test]
    async fn convert_empty_subtitle_reports_cue_count() {
        let input = Input::bytes(b"\n".to_vec(), "empty.srt");
        let result = SubtitleConverter::new().convert(input).await.unwrap();
        assert!(result.markdown.contains("no cues"));
    }
}
