//! PNG decoding and probing.

use alloc::vec::Vec;
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
#[derive(Clone, Debug, Default)]
pub struct PngLimits {
    /// Maximum total pixels (width * height).
    pub max_pixels: Option<u64>,
    /// Maximum memory allocation in bytes.
    pub max_memory_bytes: Option<u64>,
}

impl PngLimits {
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

/// Probe PNG metadata without decoding pixels.
pub fn probe(data: &[u8]) -> Result<PngInfo, PngError> {
    crate::png_reader::probe_png(data)
}

/// Decode PNG to pixels.
///
/// Preserves 16-bit depth when present in the source. Expands indexed
/// and sub-8-bit formats to their natural RGB/RGBA/Gray equivalents.
pub fn decode(data: &[u8], limits: Option<&PngLimits>) -> Result<PngDecodeOutput, PngError> {
    crate::png_reader::decode_png(data, limits)
}
