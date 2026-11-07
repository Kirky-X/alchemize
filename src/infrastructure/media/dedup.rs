// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Frame deduplication for video processing.
//!
//! Uses perceptual hashing (pHash), entropy filtering, and pixel difference ratio
//! to identify meaningful frames that should be sent to VisionProvider for analysis.

use img_hash::{HashAlg, HasherConfig, ImageHash};
// Use image crate re-exported by img_hash
use img_hash::image::{DynamicImage, ImageBuffer, Rgb};

/// Default Hamming distance threshold for perceptual hash comparison.
/// Frames with distance below this threshold are considered duplicates.
const DEFAULT_HASH_THRESHOLD: u32 = 10;

/// Default entropy threshold. Frames with entropy below this are considered
/// low-information (black screens, blurry frames, transition animations).
const DEFAULT_ENTROPY_THRESHOLD: f64 = 0.3;

/// Default pixel difference ratio threshold. Frames with less than this
/// proportion of changed pixels compared to the last accepted frame are skipped.
const DEFAULT_DIFF_RATIO_THRESHOLD: f64 = 0.05;

/// Frame deduplicator using multi-layer filtering.
///
/// Three-layer filtering strategy:
/// 1. Perceptual hash (pHash) - skip frames similar to the last frame
/// 2. Entropy filtering - skip low-information frames (black/blur/transition)
/// 3. Pixel difference ratio - skip frames with minimal changes
pub struct FrameDeduplicator {
    hasher: img_hash::Hasher,
    last_hash: Option<ImageHash<Box<[u8]>>>,
    last_accepted_frame: Option<Vec<u8>>,
    hash_threshold: u32,
    entropy_threshold: f64,
    diff_ratio_threshold: f64,
}

impl FrameDeduplicator {
    /// Create a new deduplicator with default thresholds.
    pub fn new() -> Self {
        let hasher = HasherConfig::new()
            .hash_alg(HashAlg::Mean) // pHash-like algorithm
            .hash_size(8, 8) // 8x8 hash for speed
            .to_hasher();

        Self {
            hasher,
            last_hash: None,
            last_accepted_frame: None,
            hash_threshold: DEFAULT_HASH_THRESHOLD,
            entropy_threshold: DEFAULT_ENTROPY_THRESHOLD,
            diff_ratio_threshold: DEFAULT_DIFF_RATIO_THRESHOLD,
        }
    }

    /// Create a deduplicator with custom thresholds.
    pub fn with_thresholds(
        hash_threshold: u32,
        entropy_threshold: f64,
        diff_ratio_threshold: f64,
    ) -> Self {
        let mut dedup = Self::new();
        dedup.hash_threshold = hash_threshold;
        dedup.entropy_threshold = entropy_threshold;
        dedup.diff_ratio_threshold = diff_ratio_threshold;
        dedup
    }

    /// Determine if a frame should be processed (is meaningful).
    ///
    /// # Arguments
    /// * `frame` - Raw pixel data (RGB or RGBA)
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    ///
    /// # Returns
    /// `true` if the frame is meaningful and should be sent to VisionProvider.
    pub fn should_process(&mut self, frame: &[u8], width: u32, height: u32) -> bool {
        // Layer 1: Perceptual hash deduplication
        let hash = self.compute_hash(frame, width, height);
        if let Some(ref last_hash) = self.last_hash {
            let distance = hash.dist(last_hash);
            if distance < self.hash_threshold {
                return false; // Too similar to previous frame
            }
        }
        self.last_hash = Some(hash);

        // Layer 2: Entropy filtering (skip low-information frames)
        let entropy = compute_entropy(frame);
        if entropy < self.entropy_threshold {
            return false; // Low entropy: black screen, blur, or transition
        }

        // Layer 3: Pixel difference ratio
        if let Some(ref prev_frame) = self.last_accepted_frame {
            let diff_ratio = pixel_diff_ratio(frame, prev_frame, width, height);
            if diff_ratio < self.diff_ratio_threshold {
                return false; // Minimal pixel changes
            }
        }

        // Frame is meaningful - store for next comparison
        self.last_accepted_frame = Some(frame.to_vec());
        true
    }

    /// Compute perceptual hash for a frame.
    fn compute_hash(&self, frame: &[u8], width: u32, height: u32) -> ImageHash<Box<[u8]>> {
        // img_hash expects image::DynamicImage with owned pixel buffer
        let img_buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
            match ImageBuffer::from_raw(width, height, frame.to_vec()) {
                Some(buf) => buf,
                None => ImageBuffer::new(width, height),
            };
        let dynamic_image = DynamicImage::ImageRgb8(img_buffer);
        self.hasher.hash_image(&dynamic_image)
    }

    /// Reset state for a new video segment.
    pub fn reset(&mut self) {
        self.last_hash = None;
        self.last_accepted_frame = None;
    }

    /// Get current hash threshold.
    pub fn hash_threshold(&self) -> u32 {
        self.hash_threshold
    }

    /// Get current entropy threshold.
    pub fn entropy_threshold(&self) -> f64 {
        self.entropy_threshold
    }

    /// Get current diff ratio threshold.
    pub fn diff_ratio_threshold(&self) -> f64 {
        self.diff_ratio_threshold
    }
}

impl Default for FrameDeduplicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute Shannon entropy of pixel values.
///
/// Higher entropy indicates more visual information content.
/// Low entropy suggests uniform regions (black screens, simple gradients).
pub fn compute_entropy(pixels: &[u8]) -> f64 {
    if pixels.is_empty() {
        return 0.0;
    }

    // Count frequency of each pixel value
    let mut histogram = [0usize; 256];
    for &pixel in pixels {
        histogram[pixel as usize] += 1;
    }

    let total = pixels.len() as f64;
    let mut entropy = 0.0;
    for &count in &histogram {
        if count > 0 {
            let probability = count as f64 / total;
            entropy -= probability * probability.log2();
        }
    }

    // Normalize to [0, 1] range (max entropy for 256 values is 8.0)
    entropy / 8.0
}

/// Compute pixel difference ratio between two frames.
///
/// Returns the proportion of pixels that have changed significantly.
pub fn pixel_diff_ratio(current: &[u8], previous: &[u8], width: u32, height: u32) -> f64 {
    if current.len() != previous.len() || current.is_empty() {
        return 1.0; // Assume significant change if sizes differ
    }

    // For RGB frames, compare each channel
    let channels = if current.len() == (width * height * 3) as usize {
        3 // RGB
    } else if current.len() == (width * height * 4) as usize {
        4 // RGBA
    } else {
        3 // Assume RGB by default
    };

    let threshold: u32 = 30; // Pixel difference threshold
    let mut changed_pixels = 0usize;
    let pixel_count = (width * height) as usize;
    let channels_as_u32 = channels as u32;

    for i in 0..pixel_count {
        let base = i * channels;
        let mut pixel_diff = 0u32;
        for c in 0..channels {
            if base + c < current.len() && base + c < previous.len() {
                let diff = (current[base + c] as i32 - previous[base + c] as i32).unsigned_abs();
                pixel_diff += diff;
            }
        }
        // Average channel difference exceeds threshold
        if pixel_diff / channels_as_u32 > threshold {
            changed_pixels += 1;
        }
    }

    changed_pixels as f64 / pixel_count as f64
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_thresholds() {
        let dedup = FrameDeduplicator::default();
        assert_eq!(dedup.hash_threshold(), 10);
        assert!((dedup.entropy_threshold() - 0.3).abs() < 0.01);
        assert!((dedup.diff_ratio_threshold() - 0.05).abs() < 0.01);
    }

    #[test]
    fn custom_thresholds() {
        let dedup = FrameDeduplicator::with_thresholds(15, 0.4, 0.1);
        assert_eq!(dedup.hash_threshold(), 15);
        assert!((dedup.entropy_threshold() - 0.4).abs() < 0.01);
        assert!((dedup.diff_ratio_threshold() - 0.1).abs() < 0.01);
    }

    #[test]
    fn compute_entropy_uniform() {
        // All same value = zero entropy
        let uniform: Vec<u8> = vec![128; 100];
        let entropy = compute_entropy(&uniform);
        assert!((entropy - 0.0).abs() < 0.01);
    }

    #[test]
    fn compute_entropy_max() {
        // All 256 values equally represented = max entropy (1.0 normalized)
        let max_entropy: Vec<u8> = (0u8..=255).cycle().take(256).collect();
        let entropy = compute_entropy(&max_entropy);
        assert!((entropy - 1.0).abs() < 0.01);
    }

    #[test]
    fn pixel_diff_ratio_identical() {
        let frame: Vec<u8> = vec![100; 300]; // 10x10 RGB
        let ratio = pixel_diff_ratio(&frame, &frame, 10, 10);
        assert!((ratio - 0.0).abs() < 0.01);
    }

    #[test]
    fn pixel_diff_ratio_different() {
        let frame1: Vec<u8> = vec![0; 300]; // 10x10 RGB, all black
        let frame2: Vec<u8> = vec![255; 300]; // 10x10 RGB, all white
        let ratio = pixel_diff_ratio(&frame1, &frame2, 10, 10);
        assert!((ratio - 1.0).abs() < 0.01);
    }

    #[test]
    fn reset_clears_state() {
        let mut dedup = FrameDeduplicator::new();
        dedup.last_hash = Some(
            HasherConfig::new()
                .to_hasher()
                .hash_image(&DynamicImage::new_rgb8(8, 8)),
        );
        dedup.last_accepted_frame = Some(vec![1, 2, 3]);
        dedup.reset();
        assert!(dedup.last_hash.is_none());
        assert!(dedup.last_accepted_frame.is_none());
    }
}

#[cfg(test)]
mod boost_tests {
    use super::*;

    /// Textured gradient frame (passes entropy filtering).
    fn textured(width: u32, height: u32, seed: u8) -> Vec<u8> {
        let mut buf = Vec::with_capacity((width * height * 3) as usize);
        for i in 0..width * height {
            let v = (i as u8).wrapping_mul(7).wrapping_add(seed);
            buf.extend_from_slice(&[v, v.wrapping_mul(3), 255 - v]);
        }
        buf
    }

    /// Flat frame of one value (low entropy by construction).
    fn flat(width: u32, height: u32, value: u8) -> Vec<u8> {
        vec![value; (width * height * 3) as usize]
    }

    #[test]
    fn should_process_accepts_first_textured_frame() {
        let mut d = FrameDeduplicator::new();
        assert!(d.should_process(&textured(8, 8, 1), 8, 8));
    }

    #[test]
    fn should_process_rejects_identical_frame() {
        let mut d = FrameDeduplicator::new();
        let frame = textured(8, 8, 3);
        assert!(d.should_process(&frame, 8, 8));
        assert!(!d.should_process(&frame, 8, 8), "identical frame must dedup");
    }

    #[test]
    fn should_process_rejects_low_entropy_flat() {
        let mut d = FrameDeduplicator::new();
        assert!(!d.should_process(&flat(8, 8, 7), 8, 8), "flat frame is low info");
    }

    #[test]
    fn should_process_accepts_large_change() {
        let mut d = FrameDeduplicator::new();
        assert!(d.should_process(&textured(8, 8, 10), 8, 8));
        assert!(
            d.should_process(&textured(8, 8, 200), 8, 8),
            "large change accepted"
        );
    }

    #[test]
    fn entropy_flat_vs_textured() {
        assert!(compute_entropy(&textured(8, 8, 1)) > compute_entropy(&flat(8, 8, 42)));
    }

    #[test]
    fn pixel_diff_zero_for_identical() {
        let f = textured(4, 4, 5);
        assert_eq!(pixel_diff_ratio(&f, &f, 4, 4), 0.0);
    }
}
