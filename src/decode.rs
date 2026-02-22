//! PNG decoding and probing.

use alloc::vec::Vec;
use enough::Stop;
use zencodec_types::{Cicp, ContentLightLevel, MasteringDisplay, PixelData};

use crate::error::PngError;

/// PNG chromaticity values (cHRM chunk).
///
/// All values are scaled by 100000, matching the PNG spec's `ScaledFloat`.
/// For example, the sRGB red primary (0.64, 0.33) is stored as (64000, 33000).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngChromaticities {
    pub white_x: u32,
    pub white_y: u32,
    pub red_x: u32,
    pub red_y: u32,
    pub green_x: u32,
    pub green_y: u32,
    pub blue_x: u32,
    pub blue_y: u32,
}

/// PNG image metadata from probing.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PngInfo {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Whether the image has an alpha channel.
    pub has_alpha: bool,
    /// Whether the image contains animation (APNG).
    pub has_animation: bool,
    /// Number of frames.
    pub frame_count: u32,
    /// Source bit depth per channel (before any transformations).
    pub bit_depth: u8,
    /// Embedded ICC color profile.
    pub icc_profile: Option<Vec<u8>>,
    /// Embedded EXIF metadata.
    pub exif: Option<Vec<u8>>,
    /// Embedded XMP metadata.
    pub xmp: Option<Vec<u8>>,
    /// Source gamma from gAMA chunk (scaled by 100000, e.g. 45455 = 1/2.2).
    pub source_gamma: Option<u32>,
    /// sRGB rendering intent from sRGB chunk.
    /// 0=Perceptual, 1=RelativeColorimetric, 2=Saturation, 3=AbsoluteColorimetric.
    pub srgb_intent: Option<u8>,
    /// Chromaticities from cHRM chunk.
    pub chromaticities: Option<PngChromaticities>,
    /// CICP color description from cICP chunk.
    pub cicp: Option<Cicp>,
    /// Content light level from cLLi chunk (HDR).
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering display color volume from mDCV chunk (HDR).
    pub mastering_display: Option<MasteringDisplay>,
}

/// PNG decode output.
#[derive(Debug)]
pub struct PngDecodeOutput {
    /// Decoded pixel data.
    pub pixels: PixelData,
    /// Image metadata.
    pub info: PngInfo,
}

/// Decode limits for PNG operations.
///
/// The default limits are safe for general use: 256 MP pixel count and 4 GiB memory.
/// Use [`PngLimits::none()`] to explicitly disable all limits.
#[derive(Clone, Debug)]
pub struct PngLimits {
    /// Maximum total pixels (width * height).
    pub max_pixels: Option<u64>,
    /// Maximum memory allocation in bytes.
    pub max_memory_bytes: Option<u64>,
}

impl PngLimits {
    /// Default maximum pixel count: 256 million.
    ///
    /// Covers all displays through 8K+ and most camera sensors.
    pub const DEFAULT_MAX_PIXELS: u64 = 256_000_000;

    /// Default maximum memory: 4 GiB.
    ///
    /// 256 MP × RGBA8 = 1 GB, × RGBA16 = 2 GB — both well within this limit.
    pub const DEFAULT_MAX_MEMORY: u64 = 4 * 1024 * 1024 * 1024;

    /// No limits. Caller takes responsibility for resource management.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            max_pixels: None,
            max_memory_bytes: None,
        }
    }

    pub(crate) fn validate(
        &self,
        width: u32,
        height: u32,
        bytes_per_pixel: u32,
    ) -> Result<(), PngError> {
        if let Some(max_px) = self.max_pixels {
            let pixels = width as u64 * height as u64;
            if pixels > max_px {
                return Err(PngError::LimitExceeded("pixel count exceeds limit".into()));
            }
        }
        if let Some(max_mem) = self.max_memory_bytes {
            let estimated = width as u64 * height as u64 * bytes_per_pixel as u64;
            if estimated > max_mem {
                return Err(PngError::LimitExceeded(
                    "estimated memory exceeds limit".into(),
                ));
            }
        }
        Ok(())
    }
}

impl Default for PngLimits {
    fn default() -> Self {
        Self {
            max_pixels: Some(Self::DEFAULT_MAX_PIXELS),
            max_memory_bytes: Some(Self::DEFAULT_MAX_MEMORY),
        }
    }
}

/// Probe PNG metadata without decoding pixels.
pub fn probe(data: &[u8]) -> Result<PngInfo, PngError> {
    crate::png_reader::probe_png(data)
}

/// Decode PNG to pixels.
///
/// Preserves 16-bit depth when present in the source. Expands indexed
/// and sub-8-bit formats to their natural RGB/RGBA/Gray equivalents.
///
/// The `cancel` signal is checked between rows; pass `&Unstoppable` when
/// cancellation is not needed.
pub fn decode(
    data: &[u8],
    limits: &PngLimits,
    cancel: &dyn Stop,
) -> Result<PngDecodeOutput, PngError> {
    crate::png_reader::decode_png(data, limits, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal PNG with a custom IHDR (valid signature + IHDR + IEND, no IDAT).
    /// The image will fail to decode fully but will hit limits checks first.
    fn craft_ihdr_png(width: u32, height: u32, color_type: u8, bit_depth: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        // PNG signature
        buf.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR chunk: length=13
        buf.extend_from_slice(&13u32.to_be_bytes());
        let ihdr_type = b"IHDR";
        buf.extend_from_slice(ihdr_type);
        buf.extend_from_slice(&width.to_be_bytes());
        buf.extend_from_slice(&height.to_be_bytes());
        buf.push(bit_depth);
        buf.push(color_type);
        buf.push(0); // compression
        buf.push(0); // filter
        buf.push(0); // interlace
        let crc = zenflate::crc32(zenflate::crc32(0, ihdr_type), &buf[16..29]);
        buf.extend_from_slice(&crc.to_be_bytes());
        // Empty IDAT (needed to get past chunk parsing to limits check)
        let idat_data: &[u8] = &[];
        buf.extend_from_slice(&0u32.to_be_bytes());
        let idat_type = b"IDAT";
        buf.extend_from_slice(idat_type);
        let crc = zenflate::crc32(zenflate::crc32(0, idat_type), idat_data);
        buf.extend_from_slice(&crc.to_be_bytes());
        // IEND
        buf.extend_from_slice(&0u32.to_be_bytes());
        let iend_type = b"IEND";
        buf.extend_from_slice(iend_type);
        let crc = zenflate::crc32(0, iend_type);
        buf.extend_from_slice(&crc.to_be_bytes());
        buf
    }

    #[test]
    fn limits_default_rejects_oversized() {
        // 65535×65535 RGBA = 4.3 GP, far exceeding 256 MP default
        let png = craft_ihdr_png(65535, 65535, 6, 8);
        let result = decode(&png, &PngLimits::default(), &enough::Unstoppable);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, PngError::LimitExceeded(_)),
            "expected LimitExceeded, got: {err:?}"
        );
    }

    #[test]
    fn limits_none_skips_checks() {
        // Same oversized PNG, but with no limits — should fail for a different reason
        // (decompression, not limits), proving limits were not enforced
        let png = craft_ihdr_png(65535, 65535, 6, 8);
        let result = decode(&png, &PngLimits::none(), &enough::Unstoppable);
        assert!(result.is_err());
        // Should NOT be a limits error
        let err = result.unwrap_err();
        assert!(
            !matches!(err, PngError::LimitExceeded(_)),
            "expected non-limits error, got: {err:?}"
        );
    }

    #[test]
    fn limits_custom_pixel_threshold() {
        // 100×100 RGBA = 10,000 pixels, set limit to 5,000
        let png = craft_ihdr_png(100, 100, 6, 8);
        let limits = PngLimits {
            max_pixels: Some(5_000),
            ..PngLimits::none()
        };
        let result = decode(&png, &limits, &enough::Unstoppable);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PngError::LimitExceeded(_)));
    }

    #[test]
    fn limits_custom_memory_threshold() {
        // 100×100 RGBA8 = 40,000 bytes, set limit to 20,000
        let png = craft_ihdr_png(100, 100, 6, 8);
        let limits = PngLimits {
            max_memory_bytes: Some(20_000),
            ..PngLimits::none()
        };
        let result = decode(&png, &limits, &enough::Unstoppable);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PngError::LimitExceeded(_)));
    }

    #[test]
    fn default_has_expected_values() {
        let limits = PngLimits::default();
        assert_eq!(limits.max_pixels, Some(256_000_000));
        assert_eq!(limits.max_memory_bytes, Some(4 * 1024 * 1024 * 1024));
    }

    #[test]
    fn none_has_no_limits() {
        let limits = PngLimits::none();
        assert!(limits.max_pixels.is_none());
        assert!(limits.max_memory_bytes.is_none());
    }
}
