//! PNG decoding and probing.

use alloc::string::String;
use alloc::vec::Vec;
use enough::Stop;
use zencodec::{Cicp, ContentLightLevel, MasteringDisplay};
use zenpixels::PixelBuffer;

use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

/// Physical pixel dimensions unit (pHYs chunk).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhysUnit {
    /// Unit is unknown (aspect ratio only).
    Unknown,
    /// Pixels per meter.
    Meter,
}

/// A text chunk from tEXt or zTXt.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextChunk {
    /// Latin-1 keyword (1-79 bytes). Standard keywords include "Title",
    /// "Author", "Description", "Copyright", "Creation Time", "Software",
    /// "Disclaimer", "Warning", "Source", "Comment".
    pub keyword: String,
    /// Text value (Latin-1 for tEXt/zTXt, decoded to UTF-8 on best-effort).
    pub text: String,
    /// Whether this chunk was zTXt (compressed) rather than tEXt.
    pub compressed: bool,
}

/// Background color from bKGD chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PngBackground {
    /// Palette index (color type 3).
    Indexed(u8),
    /// Grayscale value (color types 0, 4). 16-bit range.
    Gray(u16),
    /// RGB value (color types 2, 6). 16-bit range per channel.
    Rgb(u16, u16, u16),
}

/// Last modification time from tIME chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PngTime {
    /// Year (e.g. 2026). Full four-digit year.
    pub year: u16,
    /// Month (1-12).
    pub month: u8,
    /// Day (1-31).
    pub day: u8,
    /// Hour (0-23).
    pub hour: u8,
    /// Minute (0-59).
    pub minute: u8,
    /// Second (0-60, 60 for leap second).
    pub second: u8,
}

/// Significant bits per channel from sBIT chunk.
///
/// Indicates the original precision of the image data before it was
/// scaled to the PNG bit depth. For example, a 5-bit-per-channel image
/// stored in 8-bit PNG channels would have `sBIT` values of 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignificantBits {
    /// Grayscale: significant bits in the gray channel.
    Gray(u8),
    /// RGB: significant bits per channel (r, g, b).
    Rgb(u8, u8, u8),
    /// Grayscale + alpha: significant bits in gray and alpha channels.
    GrayAlpha(u8, u8),
    /// RGBA: significant bits per channel (r, g, b, a).
    Rgba(u8, u8, u8, u8),
}

/// PNG chromaticity values (cHRM chunk).
///
/// All values are scaled by 100000, matching the PNG spec's `ScaledFloat`.
/// For example, the sRGB red primary (0.64, 0.33) is stored as (64000, 33000).
///
/// Values are signed (`i32`) to support wide-gamut color spaces with
/// imaginary primaries (e.g., ACES AP1), as allowed by libpng 1.6.44+.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PngChromaticities {
    /// White point x (scaled by 100000).
    pub white_x: i32,
    /// White point y (scaled by 100000).
    pub white_y: i32,
    /// Red primary x (scaled by 100000).
    pub red_x: i32,
    /// Red primary y (scaled by 100000).
    pub red_y: i32,
    /// Green primary x (scaled by 100000).
    pub green_x: i32,
    /// Green primary y (scaled by 100000).
    pub green_y: i32,
    /// Blue primary x (scaled by 100000).
    pub blue_x: i32,
    /// Blue primary y (scaled by 100000).
    pub blue_y: i32,
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
    /// What kind of image sequence the file contains.
    pub sequence: zencodec::ImageSequence,
    /// Source bit depth per channel (before any transformations).
    pub bit_depth: u8,
    /// PNG color type from IHDR (0=Grayscale, 2=RGB, 3=Indexed, 4=GrayAlpha, 6=RGBA).
    pub color_type: u8,
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
    /// Physical pixel dimensions X (pHYs chunk).
    pub pixels_per_unit_x: Option<u32>,
    /// Physical pixel dimensions Y (pHYs chunk).
    pub pixels_per_unit_y: Option<u32>,
    /// Unit for physical pixel dimensions.
    pub phys_unit: Option<PhysUnit>,
    /// Text chunks (tEXt and zTXt). Excludes XMP (extracted separately).
    pub text_chunks: Vec<TextChunk>,
    /// Background color from bKGD chunk.
    pub background: Option<PngBackground>,
    /// Last modification time from tIME chunk.
    pub last_modified: Option<PngTime>,
    /// Significant bits per channel from sBIT chunk.
    pub significant_bits: Option<SignificantBits>,
    /// Whether the image uses Adam7 interlacing.
    pub interlaced: bool,
    /// Number of entries in the PLTE chunk, if present. `None` for
    /// non-indexed images with no palette.
    pub palette_size: Option<u16>,
    /// Total bytes of IDAT/fdAT chunk payloads in the source file.
    ///
    /// Populated by the decoder while walking chunks; used to compute the
    /// compression ratio reported by [`crate::detect::PngProbe`].
    pub compressed_data_size: u64,
    /// First creating-tool string extracted from `Software`/`Creator`/`Comment`
    /// tEXt chunks or `Software`/`Creator` iTXt chunks, matching the rules
    /// used by [`crate::detect::probe`]. `None` if no such chunk is present.
    pub creating_tool: Option<alloc::string::String>,
}

/// Non-fatal issues detected during PNG decoding.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    SrgbGamaMismatch {
        /// The actual gamma value found in the gAMA chunk.
        actual_gamma: u32,
    },
    /// sRGB chunk present but cHRM values don't match standard sRGB primaries.
    SrgbChrmMismatch,
    /// The zlib decompression checksum (Adler-32) mismatched but was tolerated.
    DecompressionChecksumSkipped,
}

/// PNG decode output.
#[derive(Debug)]
#[non_exhaustive]
pub struct PngDecodeOutput {
    /// Decoded pixel data.
    pub pixels: PixelBuffer,
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
/// By default, checksums (Adler-32 and CRC-32) are **not** verified for speed.
/// Use [`PngDecodeConfig::strict()`] to enable checksum verification.
///
/// # Examples
///
/// ```no_run
/// use zenpng::PngDecodeConfig;
///
/// // Custom config via builder pattern
/// let config = PngDecodeConfig::default()
///     .with_max_pixels(1_000_000_000)
///     .with_skip_decompression_checksum(false); // enable Adler-32 verification
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

    /// No resource limits, no checksum verification.
    ///
    /// Caller takes responsibility for resource management.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            max_pixels: None,
            max_memory_bytes: None,
            skip_decompression_checksum: true,
            skip_critical_chunk_crc: true,
        }
    }

    /// Maximum permissiveness: no resource limits, skip all checksums.
    ///
    /// Equivalent to [`PngDecodeConfig::none()`].
    #[must_use]
    pub const fn lenient() -> Self {
        Self::none()
    }

    /// Strict checksums: verifies both Adler-32 and CRC-32.
    ///
    /// No resource limits. Use builder methods to add limits.
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            max_pixels: None,
            max_memory_bytes: None,
            skip_decompression_checksum: false,
            skip_critical_chunk_crc: false,
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
    /// When true (the default), CRC-32 is not computed or verified.
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
    ) -> crate::error::Result<()> {
        if let Some(max_px) = self.max_pixels {
            let pixels = width as u64 * height as u64;
            if pixels > max_px {
                return Err(at!(PngError::LimitExceeded(
                    "pixel count exceeds limit".into()
                )));
            }
        }
        if let Some(max_mem) = self.max_memory_bytes {
            let estimated = width as u64 * height as u64 * bytes_per_pixel as u64;
            if estimated > max_mem {
                return Err(at!(PngError::LimitExceeded(
                    "estimated memory exceeds limit".into(),
                )));
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
            skip_decompression_checksum: true,
            skip_critical_chunk_crc: true,
        }
    }
}

/// Deprecated: use [`PngDecodeConfig`] instead.
#[deprecated(note = "renamed to PngDecodeConfig")]
pub type PngLimits = PngDecodeConfig;

/// Probe PNG metadata without decoding pixels.
pub fn probe(data: &[u8]) -> crate::error::Result<PngInfo> {
    crate::decoder::probe_png(data)
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
) -> crate::error::Result<PngDecodeOutput> {
    crate::decoder::decode_png(data, config, cancel)
}

// ── APNG decode ──────────────────────────────────────────────────────

/// Per-frame APNG timing metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApngFrameInfo {
    /// Numerator of the frame delay fraction.
    pub delay_num: u16,
    /// Denominator of the frame delay fraction.
    /// Per the APNG spec, 0 is treated as 100 (i.e., delay_num/100 seconds).
    pub delay_den: u16,
}

/// A single composed APNG frame (canvas-sized pixels).
#[derive(Debug)]
#[non_exhaustive]
pub struct ApngFrame {
    /// Fully composited pixel data at the canvas dimensions.
    pub pixels: PixelBuffer,
    /// Frame timing metadata.
    pub frame_info: ApngFrameInfo,
}

/// APNG decode output containing fully composed frames.
#[derive(Debug)]
#[non_exhaustive]
pub struct ApngDecodeOutput {
    /// All composed frames, each at the canvas dimensions.
    pub frames: Vec<ApngFrame>,
    /// Image metadata (canvas dimensions, color info, etc.).
    pub info: PngInfo,
    /// Animation loop count. 0 means infinite looping.
    pub num_plays: u32,
    /// Non-fatal warnings detected during decoding.
    pub warnings: Vec<PngWarning>,
}

/// Decode APNG with full compositing, returning canvas-sized frames.
///
/// For non-animated PNGs, returns the single image as frame 0 with delay=0.
///
/// Each frame is fully composited (dispose_op and blend_op applied) so callers
/// get ready-to-display canvas-sized frames.
pub fn decode_apng(
    data: &[u8],
    config: &PngDecodeConfig,
    cancel: &dyn Stop,
) -> crate::error::Result<ApngDecodeOutput> {
    // Check if this is actually an APNG
    let probe_info = crate::decoder::probe_png(data)?;
    if !probe_info.sequence.is_animation() {
        // Non-animated PNG: decode normally, wrap as single frame
        let output = crate::decoder::decode_png(data, config, cancel)?;
        let frame = ApngFrame {
            pixels: output.pixels,
            frame_info: ApngFrameInfo {
                delay_num: 0,
                delay_den: 100,
            },
        };
        return Ok(ApngDecodeOutput {
            frames: vec![frame],
            info: output.info,
            num_plays: 0,
            warnings: output.warnings,
        });
    }

    let result = crate::decoder::apng::decode_apng_composed(data, config, cancel)?;

    let info = crate::decoder::build_png_info(&result.ihdr, &result.ancillary);

    Ok(ApngDecodeOutput {
        frames: result.frames,
        info,
        num_plays: result.num_plays,
        warnings: result.warnings,
    })
}

// ── sRGB standard chromaticities ─────────────────────────────────────

/// Standard sRGB chromaticities (cHRM values × 100000).
const SRGB_CHRM: [i32; 8] = [
    31270, 32900, // white point
    64000, 33000, // red
    30000, 60000, // green
    15000, 6000, // blue
];

/// Detect color management metadata conflicts.
pub(crate) fn detect_color_warnings(
    srgb_intent: Option<u8>,
    gamma: Option<u32>,
    chrm: Option<&[i32; 8]>,
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
        if let Some(g) = gamma
            && g != 45455
        {
            warnings.push(PngWarning::SrgbGamaMismatch { actual_gamma: g });
        }
        if let Some(c) = chrm
            && c != &SRGB_CHRM
        {
            warnings.push(PngWarning::SrgbChrmMismatch);
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
            matches!(err.error(), PngError::LimitExceeded(_)),
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
            !matches!(err.error(), PngError::LimitExceeded(_)),
            "expected non-limits error, got: {err:?}"
        );
    }

    #[test]
    fn limits_custom_pixel_threshold() {
        let png = craft_ihdr_png(100, 100, 6, 8);
        let config = PngDecodeConfig::none().with_max_pixels(5_000);
        let result = decode(&png, &config, &enough::Unstoppable);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().error(),
            PngError::LimitExceeded(_)
        ));
    }

    #[test]
    fn limits_custom_memory_threshold() {
        let png = craft_ihdr_png(100, 100, 6, 8);
        let config = PngDecodeConfig::none().with_max_memory(20_000);
        let result = decode(&png, &config, &enough::Unstoppable);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().error(),
            PngError::LimitExceeded(_)
        ));
    }

    #[test]
    fn default_skips_checksums() {
        let config = PngDecodeConfig::default();
        assert_eq!(config.max_pixels, Some(100_000_000));
        assert_eq!(config.max_memory_bytes, Some(4 * 1024 * 1024 * 1024));
        assert!(config.skip_decompression_checksum);
        assert!(config.skip_critical_chunk_crc);
    }

    #[test]
    fn none_has_no_limits_and_skips_checksums() {
        let config = PngDecodeConfig::none();
        assert!(config.max_pixels.is_none());
        assert!(config.max_memory_bytes.is_none());
        assert!(config.skip_decompression_checksum);
        assert!(config.skip_critical_chunk_crc);
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
    fn strict_verifies_checksums() {
        let config = PngDecodeConfig::strict();
        assert!(config.max_pixels.is_none());
        assert!(config.max_memory_bytes.is_none());
        assert!(!config.skip_decompression_checksum);
        assert!(!config.skip_critical_chunk_crc);
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
        let bad_chrm: [i32; 8] = [31270, 32900, 64000, 33000, 30000, 60000, 15000, 7000];
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

    #[test]
    fn with_skip_decompression_checksum_builder() {
        let config = PngDecodeConfig::strict().with_skip_decompression_checksum(true);
        assert!(config.skip_decompression_checksum);
        let config2 = PngDecodeConfig::none().with_skip_decompression_checksum(false);
        assert!(!config2.skip_decompression_checksum);
    }

    #[test]
    fn with_skip_critical_chunk_crc_builder() {
        let config = PngDecodeConfig::strict().with_skip_critical_chunk_crc(true);
        assert!(config.skip_critical_chunk_crc);
        let config2 = PngDecodeConfig::none().with_skip_critical_chunk_crc(false);
        assert!(!config2.skip_critical_chunk_crc);
    }

    // ── Ancillary chunk integration tests via probe() ──────────────

    /// Helper: build a PNG with IHDR + arbitrary ancillary chunks + IDAT + IEND.
    fn craft_png_with_chunks(
        width: u32,
        height: u32,
        color_type: u8,
        bit_depth: u8,
        chunks: &[(&[u8; 4], &[u8])],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        // PNG signature
        buf.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR
        let ihdr_start = buf.len();
        buf.extend_from_slice(&13u32.to_be_bytes());
        buf.extend_from_slice(b"IHDR");
        buf.extend_from_slice(&width.to_be_bytes());
        buf.extend_from_slice(&height.to_be_bytes());
        buf.push(bit_depth);
        buf.push(color_type);
        buf.push(0);
        buf.push(0);
        buf.push(0); // compression, filter, interlace
        let crc = zenflate::crc32(
            zenflate::crc32(0, b"IHDR"),
            &buf[ihdr_start + 8..ihdr_start + 8 + 13],
        );
        buf.extend_from_slice(&crc.to_be_bytes());
        // Ancillary chunks
        for &(ctype, cdata) in chunks {
            buf.extend_from_slice(&(cdata.len() as u32).to_be_bytes());
            buf.extend_from_slice(ctype);
            buf.extend_from_slice(cdata);
            let crc = zenflate::crc32(zenflate::crc32(0, ctype), cdata);
            buf.extend_from_slice(&crc.to_be_bytes());
        }
        // Empty IDAT
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(b"IDAT");
        let crc = zenflate::crc32(zenflate::crc32(0, b"IDAT"), &[]);
        buf.extend_from_slice(&crc.to_be_bytes());
        // IEND
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(b"IEND");
        let crc = zenflate::crc32(0, b"IEND");
        buf.extend_from_slice(&crc.to_be_bytes());
        buf
    }

    #[test]
    fn probe_phys_meter() {
        let mut phys_data = [0u8; 9];
        phys_data[0..4].copy_from_slice(&3780u32.to_be_bytes());
        phys_data[4..8].copy_from_slice(&3780u32.to_be_bytes());
        phys_data[8] = 1;
        let png = craft_png_with_chunks(4, 4, 2, 8, &[(b"pHYs", &phys_data)]);
        let info = probe(&png).unwrap();
        assert_eq!(info.pixels_per_unit_x, Some(3780));
        assert_eq!(info.pixels_per_unit_y, Some(3780));
        assert_eq!(info.phys_unit, Some(PhysUnit::Meter));
    }

    #[test]
    fn probe_phys_unknown() {
        let mut phys_data = [0u8; 9];
        phys_data[0..4].copy_from_slice(&1u32.to_be_bytes());
        phys_data[4..8].copy_from_slice(&2u32.to_be_bytes());
        phys_data[8] = 0;
        let png = craft_png_with_chunks(4, 4, 2, 8, &[(b"pHYs", &phys_data)]);
        let info = probe(&png).unwrap();
        assert_eq!(info.pixels_per_unit_x, Some(1));
        assert_eq!(info.pixels_per_unit_y, Some(2));
        assert_eq!(info.phys_unit, Some(PhysUnit::Unknown));
    }

    #[test]
    fn probe_text_chunks() {
        let mut text1 = Vec::new();
        text1.extend_from_slice(b"Author");
        text1.push(0);
        text1.extend_from_slice(b"Alice");
        let mut text2 = Vec::new();
        text2.extend_from_slice(b"Comment");
        text2.push(0);
        text2.extend_from_slice(b"test");
        let png = craft_png_with_chunks(4, 4, 2, 8, &[(b"tEXt", &text1), (b"tEXt", &text2)]);
        let info = probe(&png).unwrap();
        assert_eq!(info.text_chunks.len(), 2);
        assert_eq!(info.text_chunks[0].keyword, "Author");
        assert_eq!(info.text_chunks[0].text, "Alice");
        assert!(!info.text_chunks[0].compressed);
        assert_eq!(info.text_chunks[1].keyword, "Comment");
        assert_eq!(info.text_chunks[1].text, "test");
    }

    #[test]
    fn probe_bkgd_rgb() {
        let mut bkgd = [0u8; 6];
        bkgd[0..2].copy_from_slice(&255u16.to_be_bytes());
        bkgd[2..4].copy_from_slice(&128u16.to_be_bytes());
        bkgd[4..6].copy_from_slice(&0u16.to_be_bytes());
        let png = craft_png_with_chunks(4, 4, 2, 8, &[(b"bKGD", &bkgd)]);
        let info = probe(&png).unwrap();
        assert_eq!(info.background, Some(PngBackground::Rgb(255, 128, 0)));
    }

    #[test]
    fn probe_bkgd_gray() {
        let bkgd = 42u16.to_be_bytes();
        let png = craft_png_with_chunks(4, 4, 0, 8, &[(b"bKGD", &bkgd)]);
        let info = probe(&png).unwrap();
        assert_eq!(info.background, Some(PngBackground::Gray(42)));
    }

    #[test]
    fn probe_time() {
        let mut time_data = [0u8; 7];
        time_data[0..2].copy_from_slice(&2026u16.to_be_bytes());
        time_data[2] = 3;
        time_data[3] = 18;
        time_data[4] = 14;
        time_data[5] = 30;
        time_data[6] = 45;
        let png = craft_png_with_chunks(4, 4, 2, 8, &[(b"tIME", &time_data)]);
        let info = probe(&png).unwrap();
        let t = info.last_modified.unwrap();
        assert_eq!(t.year, 2026);
        assert_eq!(t.month, 3);
        assert_eq!(t.day, 18);
        assert_eq!(t.hour, 14);
        assert_eq!(t.minute, 30);
        assert_eq!(t.second, 45);
    }

    #[test]
    fn probe_sbit_rgb() {
        let png = craft_png_with_chunks(4, 4, 2, 8, &[(b"sBIT", &[5, 6, 5])]);
        let info = probe(&png).unwrap();
        assert_eq!(info.significant_bits, Some(SignificantBits::Rgb(5, 6, 5)));
    }

    #[test]
    fn probe_sbit_rgba() {
        let png = craft_png_with_chunks(4, 4, 6, 8, &[(b"sBIT", &[5, 6, 5, 8])]);
        let info = probe(&png).unwrap();
        assert_eq!(
            info.significant_bits,
            Some(SignificantBits::Rgba(5, 6, 5, 8))
        );
    }

    #[test]
    fn probe_no_ancillary_defaults() {
        let png = craft_png_with_chunks(4, 4, 2, 8, &[]);
        let info = probe(&png).unwrap();
        assert!(info.pixels_per_unit_x.is_none());
        assert!(info.pixels_per_unit_y.is_none());
        assert!(info.phys_unit.is_none());
        assert!(info.text_chunks.is_empty());
        assert!(info.background.is_none());
        assert!(info.last_modified.is_none());
        assert!(info.significant_bits.is_none());
    }

    // ── Encode→decode roundtrip tests for new ancillary chunks ──

    #[test]
    fn roundtrip_phys_text_time() {
        use imgref::ImgVec;
        use rgb::Rgb;

        let pixels = ImgVec::new(
            vec![
                Rgb {
                    r: 128u8,
                    g: 64,
                    b: 32
                };
                16
            ],
            4,
            4,
        );
        let config = crate::EncodeConfig::default()
            .with_phys(3780, 3780, PhysUnit::Meter)
            .with_text("Author", "zenpng test")
            .with_text("Comment", "roundtrip")
            .with_last_modified(PngTime {
                year: 2026,
                month: 3,
                day: 18,
                hour: 15,
                minute: 0,
                second: 0,
            });
        let encoded = crate::encode_rgb8(
            pixels.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let info = probe(&encoded).unwrap();
        assert_eq!(info.pixels_per_unit_x, Some(3780));
        assert_eq!(info.pixels_per_unit_y, Some(3780));
        assert_eq!(info.phys_unit, Some(PhysUnit::Meter));
        assert_eq!(info.text_chunks.len(), 2);
        assert_eq!(info.text_chunks[0].keyword, "Author");
        assert_eq!(info.text_chunks[0].text, "zenpng test");
        assert_eq!(info.text_chunks[1].keyword, "Comment");
        assert_eq!(info.text_chunks[1].text, "roundtrip");
        let t = info.last_modified.unwrap();
        assert_eq!(t.year, 2026);
        assert_eq!(t.month, 3);
        assert_eq!(t.day, 18);
        assert_eq!(t.hour, 15);
        assert_eq!(t.minute, 0);
        assert_eq!(t.second, 0);
    }

    #[test]
    fn roundtrip_phys_unknown_unit() {
        use imgref::ImgVec;
        use rgb::Rgb;

        let pixels = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 4], 2, 2);
        let config = crate::EncodeConfig::default().with_phys(1, 2, PhysUnit::Unknown);
        let encoded = crate::encode_rgb8(
            pixels.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let info = probe(&encoded).unwrap();
        assert_eq!(info.pixels_per_unit_x, Some(1));
        assert_eq!(info.pixels_per_unit_y, Some(2));
        assert_eq!(info.phys_unit, Some(PhysUnit::Unknown));
    }
}
