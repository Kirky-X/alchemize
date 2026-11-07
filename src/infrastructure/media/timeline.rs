// Copyright © 2026 Kirky.X. All rights reserved.
//! Timeline event types and merging for video processing.
//!
//! Supports synchronization of audio transcription, scene boundaries,
//! keyframe OCR, and embedded subtitles into a unified timeline.

use serde::{Deserialize, Serialize};

/// Type of timeline event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelineEventType {
    /// ASR word-level timestamp (primary axis).
    AsrWord,
    /// Scene boundary point (visual transition).
    SceneBoundary,
    /// Keyframe OCR text (visual content).
    KeyframeOcr,
    /// Embedded subtitle text (from video file).
    Subtitle,
}

impl std::fmt::Display for TimelineEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimelineEventType::AsrWord => write!(f, "asr"),
            TimelineEventType::SceneBoundary => write!(f, "scene"),
            TimelineEventType::KeyframeOcr => write!(f, "ocr"),
            TimelineEventType::Subtitle => write!(f, "subtitle"),
        }
    }
}

/// A single event on the video timeline.
///
/// All timestamps are in milliseconds for consistent precision across:
/// - Video frame PTS (microseconds, converted)
/// - ASR word-level (milliseconds, from forced alignment)
/// - Subtitles (milliseconds, from aspasia parser)
/// - Scene boundaries (frame-level, converted)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    /// Start timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// End timestamp in milliseconds (for spans like subtitles/ASR segments).
    pub end_ms: u64,
    /// Type of this event.
    pub event_type: TimelineEventType,
    /// Content text (transcript word, OCR text, subtitle text, etc.).
    pub content: String,
    /// Optional confidence score (0.0-1.0 for ASR/OCR).
    pub confidence: Option<f64>,
}

impl TimelineEvent {
    /// Create a new timeline event.
    pub fn new(
        timestamp_ms: u64,
        end_ms: u64,
        event_type: TimelineEventType,
        content: String,
    ) -> Self {
        Self {
            timestamp_ms,
            end_ms,
            event_type,
            content,
            confidence: None,
        }
    }

    /// Add confidence score.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.timestamp_ms)
    }

    /// Check if this event overlaps with another.
    pub fn overlaps(&self, other: &TimelineEvent) -> bool {
        self.timestamp_ms < other.end_ms && other.timestamp_ms < self.end_ms
    }
}

/// Merge all timeline sources into a unified, sorted timeline.
///
/// Priority when timestamps overlap:
/// - Subtitles > ASR (subtitles are usually more accurate for timing)
/// - Scene boundaries and keyframe OCR are inserted as visual supplements
///
/// # Arguments
/// * `asr_words` - Word-level ASR transcription events
/// * `scene_boundaries` - Scene transition points
/// * `keyframe_ocrs` - OCR text from meaningful frames
/// * `subtitles` - Embedded subtitle events from video
///
/// # Returns
/// Unified timeline sorted by timestamp_ms, with visual events interleaved.
pub fn merge_timeline(
    asr_words: Vec<TimelineEvent>,
    scene_boundaries: Vec<TimelineEvent>,
    keyframe_ocrs: Vec<TimelineEvent>,
    subtitles: Vec<TimelineEvent>,
) -> Vec<TimelineEvent> {
    // Combine all events
    let mut all_events: Vec<TimelineEvent> = Vec::with_capacity(
        asr_words.len() + scene_boundaries.len() + keyframe_ocrs.len() + subtitles.len(),
    );

    all_events.extend(asr_words);
    all_events.extend(scene_boundaries);
    all_events.extend(keyframe_ocrs);
    all_events.extend(subtitles);

    all_events.sort_by_key(|e| e.timestamp_ms);

    // Resolve overlaps: prefer subtitles over ASR for same time range
    let mut resolved: Vec<TimelineEvent> = Vec::new();
    let mut i = 0;

    while i < all_events.len() {
        let current = &all_events[i];

        // If current is ASR and there's a subtitle at same time, skip ASR
        if current.event_type == TimelineEventType::AsrWord {
            let has_subtitle_overlap = all_events
                .iter()
                .any(|e| e.event_type == TimelineEventType::Subtitle && e.overlaps(current));
            if has_subtitle_overlap {
                i += 1;
                continue;
            }
        }

        resolved.push(current.clone());
        i += 1;
    }

    resolved
}

/// Convert unified timeline to structured markdown.
///
/// Format:
/// ```markdown
/// ## [HH:MM:SS] Scene Boundary
/// ### [HH:MM:SS - HH:MM:SS] Subtitle/ASR
/// Content text...
///
/// ### [HH:MM:SS] Keyframe OCR
/// OCR text...
/// ```
pub fn timeline_to_markdown(events: &[TimelineEvent]) -> String {
    let mut markdown = String::new();

    for event in events.iter() {
        let time_start = format_timestamp(event.timestamp_ms);
        let time_end = format_timestamp(event.end_ms);

        match event.event_type {
            TimelineEventType::SceneBoundary => {
                // Scene boundary as section header
                markdown.push_str(&format!("## [{time_start}] Scene Change\n\n"));
            }
            TimelineEventType::Subtitle | TimelineEventType::AsrWord => {
                // Subtitle/ASR as content block
                if event.timestamp_ms != event.end_ms {
                    markdown.push_str(&format!("### [{time_start} - {time_end}]\n"));
                } else {
                    markdown.push_str(&format!("### [{time_start}]\n"));
                }
                markdown.push_str(&event.content);
                markdown.push_str("\n\n");
            }
            TimelineEventType::KeyframeOcr => {
                // OCR as visual supplement
                markdown.push_str(&format!("**[{time_start}] OCR:**\n"));
                markdown.push_str(&event.content);
                markdown.push_str("\n\n");
            }
        }
    }

    markdown
}

/// Format timestamp in milliseconds to HH:MM:SS.mmm format.
fn format_timestamp(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
    } else if minutes > 0 {
        format!("{minutes:02}:{seconds:02}.{millis:03}")
    } else {
        format!("{seconds:02}.{millis:03}")
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_creation() {
        let event = TimelineEvent::new(1000, 2000, TimelineEventType::AsrWord, "hello".to_string());
        assert_eq!(event.timestamp_ms, 1000);
        assert_eq!(event.end_ms, 2000);
        assert_eq!(event.event_type, TimelineEventType::AsrWord);
        assert_eq!(event.content, "hello");
        assert!(event.confidence.is_none());
    }

    #[test]
    fn event_with_confidence() {
        let event =
            TimelineEvent::new(100, 200, TimelineEventType::KeyframeOcr, "text".to_string())
                .with_confidence(0.95);
        assert_eq!(event.confidence, Some(0.95));
    }

    #[test]
    fn event_duration() {
        let event = TimelineEvent::new(1000, 2500, TimelineEventType::Subtitle, "test".to_string());
        assert_eq!(event.duration_ms(), 1500);
    }

    #[test]
    fn event_overlap() {
        let e1 = TimelineEvent::new(1000, 2000, TimelineEventType::AsrWord, "a".to_string());
        let e2 = TimelineEvent::new(1500, 2500, TimelineEventType::Subtitle, "b".to_string());
        assert!(e1.overlaps(&e2));
        assert!(e2.overlaps(&e1));

        let e3 = TimelineEvent::new(3000, 4000, TimelineEventType::AsrWord, "c".to_string());
        assert!(!e1.overlaps(&e3));
    }

    #[test]
    fn event_type_display() {
        assert_eq!(format!("{}", TimelineEventType::AsrWord), "asr");
        assert_eq!(format!("{}", TimelineEventType::SceneBoundary), "scene");
        assert_eq!(format!("{}", TimelineEventType::KeyframeOcr), "ocr");
        assert_eq!(format!("{}", TimelineEventType::Subtitle), "subtitle");
    }

    #[test]
    fn merge_empty() {
        let result = merge_timeline(vec![], vec![], vec![], vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn merge_single_source() {
        let asr = vec![
            TimelineEvent::new(100, 200, TimelineEventType::AsrWord, "word1".to_string()),
            TimelineEvent::new(300, 400, TimelineEventType::AsrWord, "word2".to_string()),
        ];
        let result = merge_timeline(asr, vec![], vec![], vec![]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "word1");
        assert_eq!(result[1].content, "word2");
    }

    #[test]
    fn merge_sorted_by_timestamp() {
        let scenes = vec![TimelineEvent::new(
            500,
            500,
            TimelineEventType::SceneBoundary,
            "".to_string(),
        )];
        let asr = vec![TimelineEvent::new(
            100,
            200,
            TimelineEventType::AsrWord,
            "first".to_string(),
        )];
        let result = merge_timeline(asr, scenes, vec![], vec![]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].timestamp_ms, 100); // ASR first
        assert_eq!(result[1].timestamp_ms, 500); // Scene second
    }

    #[test]
    fn merge_subtitle_overrides_asr() {
        let asr = vec![TimelineEvent::new(
            100,
            300,
            TimelineEventType::AsrWord,
            "ASR text".to_string(),
        )];
        let subtitles = vec![TimelineEvent::new(
            100,
            300,
            TimelineEventType::Subtitle,
            "Subtitle text".to_string(),
        )];
        let result = merge_timeline(asr, vec![], vec![], subtitles);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_type, TimelineEventType::Subtitle);
        assert_eq!(result[0].content, "Subtitle text");
    }

    #[test]
    fn format_timestamp_hours() {
        assert_eq!(format_timestamp(3_660_500), "01:01:00.500");
    }

    #[test]
    fn format_timestamp_minutes() {
        assert_eq!(format_timestamp(125_500), "02:05.500");
    }

    #[test]
    fn format_timestamp_seconds() {
        assert_eq!(format_timestamp(5_500), "05.500");
    }

    #[test]
    fn timeline_to_markdown_empty() {
        let md = timeline_to_markdown(&[]);
        assert!(md.is_empty());
    }

    #[test]
    fn timeline_to_markdown_scene() {
        let events = vec![TimelineEvent::new(
            1000,
            1000,
            TimelineEventType::SceneBoundary,
            "".to_string(),
        )];
        let md = timeline_to_markdown(&events);
        assert!(md.contains("## [01.000] Scene Change"));
    }

    #[test]
    fn timeline_to_markdown_subtitle() {
        let events = vec![TimelineEvent::new(
            1000,
            3000,
            TimelineEventType::Subtitle,
            "Hello world".to_string(),
        )];
        let md = timeline_to_markdown(&events);
        assert!(md.contains("### [01.000 - 03.000]"));
        assert!(md.contains("Hello world"));
    }

    #[test]
    fn timeline_to_markdown_ocr() {
        let events = vec![TimelineEvent::new(
            2000,
            2000,
            TimelineEventType::KeyframeOcr,
            "Slide Title".to_string(),
        )];
        let md = timeline_to_markdown(&events);
        assert!(md.contains("**[02.000] OCR:**"));
        assert!(md.contains("Slide Title"));
    }
}
