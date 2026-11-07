// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Media format converters: image, audio, video, subtitle, timeline, dedup.

pub mod asr;
pub mod audio;
pub mod dedup;
pub mod image;
pub mod subtitle;
pub mod subtitle_converter;
pub mod timeline;
pub mod video;

pub use audio::AudioConverter;
pub use dedup::{FrameDeduplicator, compute_entropy, pixel_diff_ratio};
pub use image::ImageConverter;
pub use subtitle::{SubtitleFormat, SubtitleParser, subtitles_to_simple_markdown};
pub use subtitle_converter::SubtitleConverter;
pub use timeline::{TimelineEvent, TimelineEventType, merge_timeline, timeline_to_markdown};
pub use video::VideoConverter;
