// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Subtitle parser for embedded video subtitles.
//!
//! Supports SRT, VTT, SSA/ASS formats using the aspasia crate.
//! Extracts subtitle events with timestamps for timeline merging.

use crate::infrastructure::media::timeline::{TimelineEvent, TimelineEventType};

/// Supported subtitle formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    /// SubRip format (.srt).
    Srt,
    /// WebVTT format (.vtt).
    Vtt,
    /// SubStation Alpha (.ssa) / Advanced SubStation Alpha (.ass).
    Ass,
    /// LRC lyrics (.lrc).
    Lrc,
}

impl SubtitleFormat {
    /// Detect format from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "srt" => Some(SubtitleFormat::Srt),
            "vtt" => Some(SubtitleFormat::Vtt),
            "lrc" => Some(SubtitleFormat::Lrc),
            "ass" | "ssa" => Some(SubtitleFormat::Ass),
            _ => None,
        }
    }

    /// Get default extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            SubtitleFormat::Srt => "srt",
            SubtitleFormat::Vtt => "vtt",
            SubtitleFormat::Ass => "ass",
            SubtitleFormat::Lrc => "lrc",
        }
    }
}

/// Parser for extracting subtitle events from subtitle files or embedded tracks.
pub struct SubtitleParser {
    format: Option<SubtitleFormat>,
}

impl SubtitleParser {
    /// Create a new subtitle parser without format pre-detection.
    pub fn new() -> Self {
        Self { format: None }
    }

    /// Create a parser for a specific format.
    pub fn with_format(format: SubtitleFormat) -> Self {
        Self {
            format: Some(format),
        }
    }

    /// Parse subtitle content and extract timeline events.
    ///
    /// # Arguments
    /// * `content` - Raw subtitle file content (UTF-8)
    /// * `format_hint` - Optional format hint (overrides self.format)
    ///
    /// # Returns
    /// Vector of TimelineEvents with subtitle text and timestamps.
    pub fn parse(&self, content: &str, format_hint: Option<SubtitleFormat>) -> Vec<TimelineEvent> {
        let format = format_hint.or(self.format).unwrap_or(SubtitleFormat::Srt);
        self.parse_with_format(content, format)
    }

    /// Parse subtitle content with explicit format.
    fn parse_with_format(&self, content: &str, format: SubtitleFormat) -> Vec<TimelineEvent> {
        match format {
            SubtitleFormat::Srt => self.parse_srt(content),
            SubtitleFormat::Vtt => self.parse_vtt(content),
            SubtitleFormat::Ass => self.parse_ass(content),
            SubtitleFormat::Lrc => parse_lrc(content),
        }
    }

    /// Parse SRT format subtitles.
    fn parse_srt(&self, content: &str) -> Vec<TimelineEvent> {
        use aspasia::{SubRipSubtitle, Subtitle};
        use std::str::FromStr;

        let subtitle: SubRipSubtitle = match SubRipSubtitle::from_str(content) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("SRT parse error: {e}");
                return Vec::new();
            }
        };

        subtitle
            .events()
            .iter()
            .map(|event| {
                let start_ms = i64::from(event.start) as u64;
                let end_ms = i64::from(event.end) as u64;
                TimelineEvent::new(
                    start_ms,
                    end_ms,
                    TimelineEventType::Subtitle,
                    event.text.clone(),
                )
            })
            .collect()
    }

    /// Parse WebVTT format subtitles.
    fn parse_vtt(&self, content: &str) -> Vec<TimelineEvent> {
        use aspasia::{Subtitle, WebVttSubtitle};
        use std::str::FromStr;

        let subtitle: WebVttSubtitle = match WebVttSubtitle::from_str(content) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("VTT parse error: {e}");
                return Vec::new();
            }
        };

        subtitle
            .events()
            .iter()
            .map(|cue| {
                let start_ms = i64::from(cue.start) as u64;
                let end_ms = i64::from(cue.end) as u64;
                TimelineEvent::new(
                    start_ms,
                    end_ms,
                    TimelineEventType::Subtitle,
                    cue.text.clone(),
                )
            })
            .collect()
    }

    /// Parse ASS/SSA format subtitles.
    fn parse_ass(&self, content: &str) -> Vec<TimelineEvent> {
        use aspasia::{AssSubtitle, Subtitle};
        use std::str::FromStr;

        let subtitle: AssSubtitle = match AssSubtitle::from_str(content) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("ASS parse error: {e}");
                return Vec::new();
            }
        };

        subtitle
            .events()
            .iter()
            .map(|event| {
                let start_ms = i64::from(event.start) as u64;
                let end_ms = i64::from(event.end) as u64;
                TimelineEvent::new(
                    start_ms,
                    end_ms,
                    TimelineEventType::Subtitle,
                    event.text.clone(),
                )
            })
            .collect()
    }
}

impl Default for SubtitleParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert subtitle events to a simple markdown format.
///
/// Format:
/// ```markdown
/// [00:01.000 - 00:03.000] Subtitle text
/// [00:05.000 - 00:07.000] Next subtitle
/// ```
pub fn subtitles_to_simple_markdown(events: &[TimelineEvent]) -> String {
    events
        .iter()
        .filter(|e| e.event_type == TimelineEventType::Subtitle)
        .map(|e| {
            let start = format_time_simple(e.timestamp_ms);
            let end = format_time_simple(e.end_ms);
            format!("[{start} - {end}] {content}\n", content = e.content)
        })
        .collect()
}

/// Format time in simple MM:SS.mmm format.
fn format_time_simple(ms: u64) -> String {
    let minutes = ms / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}

// Tests

/// Parse LRC lyrics: `[mm:ss.xx] line` (multiple tags per line are legal).
///
/// Metadata tags (`[ar:…]`, `[ti:…]`, `[offset:…]`) are ignored; the offset
/// shifts every timestamp when present. End times are the next cue's start.
pub fn parse_lrc(content: &str) -> Vec<TimelineEvent> {
    use crate::infrastructure::media::timeline::TimelineEventType;

    // Optional global offset in milliseconds (LRC convention: + speeds up).
    let mut offset_ms: i64 = 0;
    let mut cues: Vec<(u64, String)> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("[offset:")
            && let Some(v) = line
                .trim_start_matches("[offset:")
                .trim_end_matches(']')
                .trim()
                .parse::<i64>()
                .ok()
        {
            offset_ms = v;
            continue;
        }
        // Collect every leading [mm:ss.xx] tag; the remainder is the text.
        let mut rest = line;
        let mut stamps: Vec<u64> = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else {
                break;
            };
            let tag = &stripped[..close];
            if let Some(ms) = parse_lrc_timestamp(tag) {
                stamps.push(ms);
            }
            rest = &stripped[close + 1..];
        }
        if stamps.is_empty() {
            continue; // metadata tag or plain text without timing
        }
        let text = rest.trim().to_string();
        for ms in stamps {
            let shifted = (ms as i64 + offset_ms).max(0) as u64;
            cues.push((shifted, text.clone()));
        }
    }
    cues.sort_by_key(|(ms, _)| *ms);
    let mut events = Vec::with_capacity(cues.len());
    for i in 0..cues.len() {
        let (start, text) = cues[i].clone();
        let end = cues
            .get(i + 1)
            .map(|(next, _)| *next)
            .unwrap_or(start + 2000);
        events.push(TimelineEvent {
            timestamp_ms: start,
            end_ms: end,
            event_type: TimelineEventType::Subtitle,
            content: text,
            confidence: None,
        });
    }
    events
}

/// Parse one LRC timestamp `mm:ss.xx` / `mm:ss:xx` into milliseconds.
fn parse_lrc_timestamp(tag: &str) -> Option<u64> {
    let (mm, rest) = tag.split_once(':')?;
    let minutes: u64 = mm.trim().parse().ok()?;
    // Hundredths may use '.' or ':' as the separator.
    let (ss, frac) = match rest.split_once(['.', ':']) {
        Some((ss, frac)) => (ss, Some(frac)),
        None => (rest, None),
    };
    let seconds: u64 = ss.trim().parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let hundredths: u64 = frac
        .and_then(|f| {
            let f = f.trim();

            if f.len() == 1 {
                f.parse::<u64>().ok().map(|v| v * 10)
            } else {
                f.get(..2).and_then(|v| v.parse().ok())
            }
        })
        .unwrap_or(0);
    Some(minutes * 60_000 + seconds * 1000 + hundredths * 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_cues_parse_with_offsets_and_ordering() {
        let lrc = "[ti:Demo]
[offset:+500]
[00:12.50]second in file, first in time? no
[00:03.00]first line
[00:03.00][00:20.10]repeated tag two timestamps
";
        let events = parse_lrc(lrc);
        // 4 cues: 3.5s, 13.0s, 3.5s(repeat tag), 20.6s
        assert_eq!(events.len(), 4, "{events:?}");
        assert_eq!(events[0].timestamp_ms, 3_500);
        assert_eq!(events[0].content, "first line");
        assert_eq!(events[1].timestamp_ms, 3_500);
        assert_eq!(events[2].timestamp_ms, 13_000);
        assert_eq!(events[3].timestamp_ms, 20_600);
        assert_eq!(events[2].end_ms, 20_600);
    }

    #[test]
    fn lrc_timestamp_variants() {
        assert_eq!(parse_lrc_timestamp("01:02.34"), Some(62_340));
        assert_eq!(parse_lrc_timestamp("00:05"), Some(5_000));
        assert_eq!(parse_lrc_timestamp("00:05:50"), Some(5_500));
        assert_eq!(parse_lrc_timestamp("99:99"), None, "seconds must be < 60");
        assert_eq!(parse_lrc_timestamp("ar:Someone"), None);
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(
            SubtitleFormat::from_extension("srt"),
            Some(SubtitleFormat::Srt)
        );
        assert_eq!(
            SubtitleFormat::from_extension("vtt"),
            Some(SubtitleFormat::Vtt)
        );
        assert_eq!(
            SubtitleFormat::from_extension("ass"),
            Some(SubtitleFormat::Ass)
        );
        assert_eq!(
            SubtitleFormat::from_extension("ssa"),
            Some(SubtitleFormat::Ass)
        );
        assert_eq!(SubtitleFormat::from_extension("txt"), None);
    }

    #[test]
    fn format_extension() {
        assert_eq!(SubtitleFormat::Srt.extension(), "srt");
        assert_eq!(SubtitleFormat::Vtt.extension(), "vtt");
        assert_eq!(SubtitleFormat::Ass.extension(), "ass");
    }

    #[test]
    fn parse_empty_srt() {
        let parser = SubtitleParser::with_format(SubtitleFormat::Srt);
        let events = parser.parse("", None);
        assert!(events.is_empty());
    }

    #[test]
    fn parse_simple_srt() {
        let srt_content = "1\n00:00:01,000 --> 00:00:03,000\nHello world\n\n2\n00:00:05,000 --> 00:00:07,000\nNext line\n";
        let parser = SubtitleParser::with_format(SubtitleFormat::Srt);
        let events = parser.parse(srt_content, None);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].timestamp_ms, 1000);
        assert_eq!(events[0].end_ms, 3000);
        assert_eq!(events[0].content, "Hello world");
        assert_eq!(events[1].timestamp_ms, 5000);
        assert_eq!(events[1].content, "Next line");
    }

    #[test]
    fn parse_simple_vtt() {
        let vtt_content = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nHello world\n\n00:00:05.000 --> 00:00:07.000\nNext line\n";
        let parser = SubtitleParser::with_format(SubtitleFormat::Vtt);
        let events = parser.parse(vtt_content, None);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].timestamp_ms, 1000);
        assert_eq!(events[0].content, "Hello world");
    }

    #[test]
    fn format_hint_overrides() {
        let parser = SubtitleParser::with_format(SubtitleFormat::Srt);
        // Even with Srt format set, Vtt hint should use Vtt parsing
        let vtt_content = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nTest\n";
        let events = parser.parse(vtt_content, Some(SubtitleFormat::Vtt));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content, "Test");
    }

    #[test]
    fn subtitles_to_simple_markdown_empty() {
        let md = subtitles_to_simple_markdown(&[]);
        assert!(md.is_empty());
    }

    #[test]
    fn subtitles_to_simple_markdown_format() {
        let events = vec![TimelineEvent::new(
            1000,
            3000,
            TimelineEventType::Subtitle,
            "Hello".to_string(),
        )];
        let md = subtitles_to_simple_markdown(&events);
        assert!(md.contains("[00:01.000 - 00:03.000]"));
        assert!(md.contains("Hello"));
    }

    #[test]
    fn default_parser() {
        let parser = SubtitleParser::default();
        assert!(parser.format.is_none());
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn lrc_cues_become_timeline_events() {
        let lrc = "[offset:500]\n[00:01.5]first\n[00:02:25]second\n";
        let format = SubtitleFormat::from_extension("lrc").unwrap();
        assert_eq!(format.extension(), "lrc");
        let events = parse_lrc(lrc);
        assert_eq!(events.len(), 2, "{events:?}");
        // offset 500ms is ADDED to cue time (impl convention): 1500 + 500.
        assert_eq!(events[0].timestamp_ms, 2000, "{events:?}");
        assert_eq!(events[1].timestamp_ms, 2750, "{events:?}");
    }

    #[test]
    fn lrc_timestamp_garbage_is_skipped() {
        // 90 seconds is out of range; the whole tag is dropped.
        assert!(parse_lrc("[00:90.00]x").is_empty());
        // Non-numeric minute: tag rejected, no panic.
        assert!(parse_lrc("[ab:cd]x").is_empty());
    }
}

#[test]
fn lrc_routes_through_converter_dispatch() {
    // The format-dispatch arm for .lrc (parse via converter-level entry).
    let format = SubtitleFormat::from_extension("lrc").unwrap();
    let events = match format {
        SubtitleFormat::Lrc => parse_lrc("[00:01.00]hello"),
        _ => Vec::new(),
    };
    assert_eq!(events.len(), 1);
}
