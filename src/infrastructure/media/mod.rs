// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Media format converters: image, audio, video, subtitle, timeline, dedup.
//!
//! The four converter families are gated by the `media` feature; HEIC is its
//! own feature (OS decoder); the ASR backend is gated by `audio`.

#[cfg(feature = "audio")]
pub mod asr;
#[cfg(feature = "media")]
pub mod audio;
#[cfg(feature = "media")]
pub mod dedup;
#[cfg(feature = "heic")]
pub mod heic;
#[cfg(feature = "media")]
pub mod image;
#[cfg(feature = "media")]
pub mod subtitle;
#[cfg(feature = "media")]
pub mod subtitle_converter;
#[cfg(feature = "media")]
pub mod timeline;
#[cfg(feature = "media")]
pub mod video;

#[cfg(feature = "media")]
pub use audio::AudioConverter;
#[cfg(feature = "media")]
pub use dedup::{FrameDeduplicator, compute_entropy, pixel_diff_ratio};
#[cfg(feature = "heic")]
pub use heic::HeicConverter;
#[cfg(feature = "media")]
pub use image::ImageConverter;
#[cfg(feature = "media")]
pub use subtitle::{SubtitleFormat, SubtitleParser, subtitles_to_simple_markdown};
#[cfg(feature = "media")]
pub use subtitle_converter::SubtitleConverter;
#[cfg(feature = "media")]
pub use timeline::{TimelineEvent, TimelineEventType, merge_timeline, timeline_to_markdown};
#[cfg(feature = "media")]
pub use video::VideoConverter;
