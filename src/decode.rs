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

/// Non-fatal issues detected during PNG decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PngWarning {
    /// Both sRGB and cICP chunks present (conflicting color space signals).
    SrgbCicpConflict,
    /// Both iCCP and sRGB chunks present (redundant/conflicting).
    IccpSrgbConflict,
    /// Both cICP and iCCP chunks present (conflicting color space signals).
    CicpIccpConflict,
    /// Both cICP and cHRM chunks present (cICP supersedes primaries).
    CicpChrmConflict,
    /// sRGB chunk present but gAMA value is not the expected 45455.
    SrgbGamaMismatch { actual_gamma: u32 },
    /// sRGB chunk present but cHRM values don't match standard sRGB primaries.
    SrgbChrmMismatch,
    /// A critical chunk's CRC was skipped (checksum leniency enabled).
    CriticalChunkCrcSkipped { chunk_type: [u8; 4] },
    /// The zlib decompression checksum (Adler-32) was skipped.
    DecompressionChecksumSkipped,
}

/// PNG decode output.
#[derive(Debug)]
pub struct PngDecodeOutput {
    /// Decoded pixel data.
    pub pixels: PixelData,
    /// Image metadata.
    pub info: PngInfo,
    /// Non-fatal warnings detected during decoding.
    pub warnings: Vec<PngWarning>,
}

/// Decode configuration for PNG operations.
///
/// Controls resource limits and checksum leniency. The default is safe for
/// general use: 100 MP pixel count, 4 GiB memory, strict checksums.
///
/// Use [`PngDecodeConfig::none()`] to disable resource limits (strict checksums).
/// Use [`PngDecodeConfig::lenient()`] for maximum permissiveness.
///
/// # Examples
///
/// ```no_run
/// use zenpng::PngDecodeConfig;
///
/// // Custom config via builder pattern
/// let config = PngDecodeConfig::default()
///     .with_max_pixels(1_000_000_000)
///     .with_skip_decompression_checksum(true);
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PngDecodeConfig {
    /// Maximum total pixels (width × height). `None` = no limit.
    pub max_pixels: Option<u64>,
    /// Maximum memory allocation in bytes. `None` = no limit.
    pub max_memory_bytes: Option<u64>,
    /// Skip zlib Adler-32 checksum verification (still computed for reporting).
    pub skip_decompression_checksum: bool,
    /// Skip CRC verification on critical chunks (IHDR, PLTE, IDAT).
    pub skip_critical_chunk_crc: bool,
}

impl PngDecodeConfig {
    /// Default maximum pixel count: 100 million.
    ///
    /// Covers all displays through 8K and most camera sensors.
    pub const DEFAULT_MAX_PIXELS: u64 = 100_000_000;

    /// Default maximum memory: 4 GiB.
    ///
    /// 100 MP × RGBA8 = 400 MB, × RGBA16 = 800 MB — both well within this limit.
    pub const DEFAULT_MAX_MEMORY: u64 = 4 * 1024 * 1024 * 1024;

    /// No resource limits, strict checksums.
    ///
    /// Caller takes responsibility for resource management.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            max_pixels: None,
            max_memory_bytes: None,
            skip_decompression_checksum: false,
            skip_critical_chunk_crc: false,
        }
    }

    /// Maximum permissiveness: no resource limits, skip all checksums.
    #[must_use]
    pub const fn lenient() -> Self {
        Self {
            max_pixels: None,
            max_memory_bytes: None,
            skip_decompression_checksum: true,
            skip_critical_chunk_crc: true,
        }
    }

    /// Set maximum pixel count (width × height).
    #[must_use]
    pub const fn with_max_pixels(mut self, max: u64) -> Self {
        self.max_pixels = Some(max);
        self
    }

    /// Set maximum memory allocation in bytes.
    #[must_use]
    pub const fn with_max_memory(mut self, max: u64) -> Self {
        self.max_memory_bytes = Some(max);
        self
    }

    /// Skip zlib decompression checksum (Adler-32) verification.
    ///
    /// When true, corrupt checksums produce a [`PngWarning::DecompressionChecksumSkipped`]
    /// instead of an error. Pixels are still decompressed and returned.
    #[must_use]
    pub const fn with_skip_decompression_checksum(mut self, skip: bool) -> Self {
        self.skip_decompression_checksum = skip;
        self
    }

    /// Skip CRC verification on critical PNG chunks.
    ///
    /// When true, critical chunk CRC mismatches produce a
    /// [`PngWarning::CriticalChunkCrcSkipped`] instead of an error.
    #[must_use]
    pub const fn with_skip_critical_chunk_crc(mut self, skip: bool) -> Self {
        self.skip_critical_chunk_crc = skip;
        self
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

impl Default for PngDecodeConfig {
    fn default() -> Self {
        Self {
            max_pixels: Some(Self::DEFAULT_MAX_PIXELS),
            max_memory_bytes: Some(Self::DEFAULT_MAX_MEMORY),
            skip_decompression_checksum: false,
            skip_critical_chunk_crc: false,
        }
    }
}

/// Deprecated: use [`PngDecodeConfig`] instead.
#[deprecated(note = "renamed to PngDecodeConfig")]
pub type PngLimits = PngDecodeConfig;

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
    config: &PngDecodeConfig,
    cancel: &dyn Stop,
) -> Result<PngDecodeOutput, PngError> {
    crate::png_reader::decode_png(data, config, cancel)
}

// ── sRGB standard chromaticities ─────────────────────────────────────

/// Standard sRGB chromaticities (cHRM values × 100000).
const SRGB_CHRM: [u32; 8] = [
    31270, 32900, // white point
    64000, 33000, // red
    30000, 60000, // green
    15000, 6000, // blue
];

/// Detect color management metadata conflicts.
pub(crate) fn detect_color_warnings(
    srgb_intent: Option<u8>,
    gamma: Option<u32>,
    chrm: Option<&[u32; 8]>,
    cicp: Option<&[u8; 4]>,
    icc_profile: Option<&[u8]>,
) -> Vec<PngWarning> {
    let mut warnings = Vec::new();
    let has_srgb = srgb_intent.is_some();
    let has_cicp = cicp.is_some();
    let has_iccp = icc_profile.is_some();

    if has_srgb && has_cicp {
        warnings.push(PngWarning::SrgbCicpConflict);
    }
    if has_iccp && has_srgb {
        warnings.push(PngWarning::IccpSrgbConflict);
    }
    if has_cicp && has_iccp {
        warnings.push(PngWarning::CicpIccpConflict);
    }
    if has_cicp && chrm.is_some() {
        warnings.push(PngWarning::CicpChrmConflict);
    }
    if has_srgb {
        if let Some(g) = gamma {
            if g != 45455 {
                warnings.push(PngWarning::SrgbGamaMismatch { actual_gamma: g });
            }
        }
        if let Some(c) = chrm {
            if c != &SRGB_CHRM {
                warnings.push(PngWarning::SrgbChrmMismatch);
            }
        }
    }
    warnings
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
        let png = craft_ihdr_png(65535, 65535, 6, 8);
        let result = decode(&png, &PngDecodeConfig::default(), &enough::Unstoppable);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, PngError::LimitExceeded(_)),
            "expected LimitExceeded, got: {err:?}"
        );
    }

    #[test]
    fn limits_none_skips_checks() {
        let png = craft_ihdr_png(65535, 65535, 6, 8);
        let result = decode(&png, &PngDecodeConfig::none(), &enough::Unstoppable);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            !matches!(err, PngError::LimitExceeded(_)),
            "expected non-limits error, got: {err:?}"
        );
    }

    #[test]
    fn limits_custom_pixel_threshold() {
        let png = craft_ihdr_png(100, 100, 6, 8);
        let config = PngDecodeConfig::none().with_max_pixels(5_000);
        let result = decode(&png, &config, &enough::Unstoppable);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PngError::LimitExceeded(_)));
    }

    #[test]
    fn limits_custom_memory_threshold() {
        let png = craft_ihdr_png(100, 100, 6, 8);
        let config = PngDecodeConfig::none().with_max_memory(20_000);
        let result = decode(&png, &config, &enough::Unstoppable);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PngError::LimitExceeded(_)));
    }

    #[test]
    fn default_has_expected_values() {
        let config = PngDecodeConfig::default();
        assert_eq!(config.max_pixels, Some(100_000_000));
        assert_eq!(config.max_memory_bytes, Some(4 * 1024 * 1024 * 1024));
        assert!(!config.skip_decompression_checksum);
        assert!(!config.skip_critical_chunk_crc);
    }

    #[test]
    fn none_has_no_limits() {
        let config = PngDecodeConfig::none();
        assert!(config.max_pixels.is_none());
        assert!(config.max_memory_bytes.is_none());
        assert!(!config.skip_decompression_checksum);
        assert!(!config.skip_critical_chunk_crc);
    }

    #[test]
    fn lenient_has_no_limits_and_skips_checksums() {
        let config = PngDecodeConfig::lenient();
        assert!(config.max_pixels.is_none());
        assert!(config.max_memory_bytes.is_none());
        assert!(config.skip_decompression_checksum);
        assert!(config.skip_critical_chunk_crc);
    }

    #[test]
    fn detect_srgb_cicp_conflict() {
        let w = detect_color_warnings(Some(0), None, None, Some(&[1, 13, 0, 1]), None);
        assert!(w.contains(&PngWarning::SrgbCicpConflict));
    }

    #[test]
    fn detect_iccp_srgb_conflict() {
        let w = detect_color_warnings(Some(0), None, None, None, Some(&[0]));
        assert!(w.contains(&PngWarning::IccpSrgbConflict));
    }

    #[test]
    fn detect_srgb_gama_mismatch() {
        let w = detect_color_warnings(Some(0), Some(50000), None, None, None);
        assert!(w.contains(&PngWarning::SrgbGamaMismatch {
            actual_gamma: 50000
        }));
    }

    #[test]
    fn detect_srgb_gama_correct() {
        let w = detect_color_warnings(Some(0), Some(45455), None, None, None);
        assert!(
            !w.iter()
                .any(|w| matches!(w, PngWarning::SrgbGamaMismatch { .. }))
        );
    }

    #[test]
    fn detect_srgb_chrm_mismatch() {
        let bad_chrm = [31270, 32900, 64000, 33000, 30000, 60000, 15000, 7000];
        let w = detect_color_warnings(Some(0), None, Some(&bad_chrm), None, None);
        assert!(w.contains(&PngWarning::SrgbChrmMismatch));
    }

    #[test]
    fn detect_srgb_chrm_correct() {
        let w = detect_color_warnings(Some(0), None, Some(&SRGB_CHRM), None, None);
        assert!(!w.contains(&PngWarning::SrgbChrmMismatch));
    }

    #[test]
    fn no_warnings_when_clean() {
        let w = detect_color_warnings(Some(0), Some(45455), Some(&SRGB_CHRM), None, None);
        assert!(w.is_empty());
    }
}
