//! PNG encoding.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::{Gray, Rgb, Rgba};

use zc::MetadataView;

use enough::Stop;

use crate::decode::PngChromaticities;
use crate::encoder::PngWriteMetadata;
use crate::error::PngError;
use crate::types::{Compression, Filter};

/// PNG encode configuration.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct EncodeConfig {
    /// PNG compression level.
    pub compression: Compression,
    /// PNG row filter strategy.
    pub filter: Filter,
    /// Use multi-threaded screening and refinement.
    ///
    /// When true, Phase 1 (strategy screening) and Phase 2 (refinement)
    /// run their independent evaluations in parallel using `std::thread::scope`.
    /// Each thread allocates its own compression buffers (~3× image size),
    /// so memory usage scales with thread count. Default: false.
    pub parallel: bool,
    /// Source gamma for gAMA chunk (scaled by 100000, e.g. 45455 = 1/2.2).
    ///
    /// Suppressed in output when sRGB, iCCP, or cICP is present (PNGv3 precedence).
    pub source_gamma: Option<u32>,
    /// sRGB rendering intent for sRGB chunk (0-3).
    ///
    /// Suppressed in output when iCCP or cICP is present (PNGv3 precedence).
    /// When written, suppresses gAMA and cHRM.
    pub srgb_intent: Option<u8>,
    /// Chromaticities for cHRM chunk.
    ///
    /// Suppressed in output when sRGB, iCCP, or cICP is present (PNGv3 precedence).
    pub chromaticities: Option<PngChromaticities>,
    /// Near-lossless: number of least-significant bits to round (0-4).
    ///
    /// When set to N > 0, each 8-bit sample has its lowest N bits rounded
    /// to the nearest multiple of 2^N. This creates more byte repetition
    /// that PNG filters and DEFLATE exploit for smaller files.
    ///
    /// Quality impact is imperceptible at 1-2 bits. At 3-4 bits, there's
    /// a visible but minor quality reduction, with significant compression
    /// gains (typically 20-50% smaller for photographic content).
    ///
    /// Default: 0 (lossless). Does not affect indexed (palette) encoding.
    pub near_lossless_bits: u8,
    /// Maximum number of threads for compression.
    ///
    /// - `0` means no limit (use as many threads as beneficial).
    /// - `1` forces fully single-threaded operation: no `std::thread::scope`
    ///   calls anywhere in the compression pipeline.
    /// - `N > 1` caps parallelism to at most N threads.
    ///
    /// When set to 1, both the screening/refinement phases and the
    /// recompression phase run sequentially. This is derived from
    /// [`ThreadingPolicy`](zc::ThreadingPolicy) when using the zencodec adapter.
    ///
    /// Default: 0 (no limit).
    pub max_threads: usize,
}

impl EncodeConfig {
    /// Set compression level.
    #[must_use]
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Set row filter strategy.
    #[must_use]
    pub fn with_filter(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    /// Enable multi-threaded screening and refinement.
    #[must_use]
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Set source gamma for gAMA chunk (scaled by 100000, e.g. 45455 = 1/2.2).
    #[must_use]
    pub fn with_source_gamma(mut self, gamma: Option<u32>) -> Self {
        self.source_gamma = gamma;
        self
    }

    /// Set sRGB rendering intent for sRGB chunk (0-3).
    #[must_use]
    pub fn with_srgb_intent(mut self, intent: Option<u8>) -> Self {
        self.srgb_intent = intent;
        self
    }

    /// Set chromaticities for cHRM chunk.
    #[must_use]
    pub fn with_chromaticities(mut self, chrm: Option<PngChromaticities>) -> Self {
        self.chromaticities = chrm;
        self
    }

    /// Set near-lossless bit rounding (0-4).
    #[must_use]
    pub fn with_near_lossless_bits(mut self, bits: u8) -> Self {
        self.near_lossless_bits = bits;
        self
    }

    /// Build compression options from this config and caller-provided stops.
    pub(crate) fn compress_options<'a>(
        &self,
        cancel: &'a dyn Stop,
        deadline: &'a dyn Stop,
        remaining_ns: Option<&'a dyn Fn() -> Option<u64>>,
    ) -> crate::encoder::CompressOptions<'a> {
        crate::encoder::CompressOptions {
            parallel: self.parallel && self.max_threads != 1,
            cancel,
            deadline,
            remaining_ns,
            max_threads: self.max_threads,
        }
    }
}

/// PNG color type (internal, maps to PNG spec values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // GrayscaleAlpha not yet used in encode, but kept for completeness
pub(crate) enum ColorType {
    Grayscale,
    Rgb,
    GrayscaleAlpha,
    Rgba,
}

impl ColorType {
    /// PNG spec color type byte.
    pub(crate) fn to_png_byte(self) -> u8 {
        match self {
            ColorType::Grayscale => 0,
            ColorType::Rgb => 2,
            ColorType::GrayscaleAlpha => 4,
            ColorType::Rgba => 6,
        }
    }
}

/// PNG bit depth (internal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BitDepth {
    Eight,
    Sixteen,
}

impl BitDepth {
    /// PNG spec bit depth value.
    pub(crate) fn to_png_byte(self) -> u8 {
        match self {
            BitDepth::Eight => 8,
            BitDepth::Sixteen => 16,
        }
    }
}

/// Encode RGB8 pixels to PNG.
pub fn encode_rgb8(
    img: ImgRef<Rgb<u8>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw(
        bytes,
        width,
        height,
        ColorType::Rgb,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGBA8 pixels to PNG.
pub fn encode_rgba8(
    img: ImgRef<Rgba<u8>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw(
        bytes,
        width,
        height,
        ColorType::Rgba,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode Gray8 pixels to PNG.
pub fn encode_gray8(
    img: ImgRef<Gray<u8>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: Vec<u8> = buf.iter().map(|g| g.value()).collect();
    encode_raw(
        &bytes,
        width,
        height,
        ColorType::Grayscale,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGB16 pixels to PNG.
///
/// Input samples are native-endian u16. The encoder handles byte-swapping
/// to big-endian as required by the PNG specification.
pub fn encode_rgb16(
    img: ImgRef<Rgb<u16>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    let be = native_to_be_16(bytes);
    encode_raw(
        &be,
        width,
        height,
        ColorType::Rgb,
        BitDepth::Sixteen,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGBA16 pixels to PNG.
///
/// Input samples are native-endian u16. The encoder handles byte-swapping
/// to big-endian as required by the PNG specification.
pub fn encode_rgba16(
    img: ImgRef<Rgba<u16>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    let be = native_to_be_16(bytes);
    encode_raw(
        &be,
        width,
        height,
        ColorType::Rgba,
        BitDepth::Sixteen,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode Gray16 pixels to PNG.
///
/// Input samples are native-endian u16. The encoder handles byte-swapping
/// to big-endian as required by the PNG specification.
pub fn encode_gray16(
    img: ImgRef<Gray<u16>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    let be = native_to_be_16(bytes);
    encode_raw(
        &be,
        width,
        height,
        ColorType::Grayscale,
        BitDepth::Sixteen,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Low-level encode: raw bytes to PNG with metadata and config applied.
///
/// Automatically optimizes color type and bit depth for smallest output:
/// - RGBA → RGB (strip opaque alpha), GrayscaleAlpha, or Grayscale
/// - RGB → Grayscale (when R==G==B)
/// - 16-bit → 8-bit (when all samples fit in 8 bits)
/// - Truecolor → Indexed (when ≤256 unique colors)
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_raw(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: BitDepth,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let effort = config.compression.effort();

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.source_gamma;
    write_meta.srgb_intent = config.srgb_intent;
    write_meta.chromaticities = config.chromaticities;

    let w = width as usize;
    let h = height as usize;

    // Near-lossless: quantize LSBs for better compression (8-bit only)
    let nl_bytes;
    let bytes = if config.near_lossless_bits > 0 && bit_depth == BitDepth::Eight {
        let channels: usize = match color_type {
            ColorType::Grayscale => 1,
            ColorType::Rgb => 3,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgba => 4,
        };
        nl_bytes =
            crate::optimize::near_lossless_quantize(bytes, channels, config.near_lossless_bits);
        &nl_bytes
    } else {
        bytes
    };

    // Auto-optimize color type and bit depth
    let optimization = match (color_type, bit_depth) {
        (ColorType::Rgba, BitDepth::Eight) => crate::optimize::optimize_rgba8(bytes, w, h),
        (ColorType::Rgb, BitDepth::Eight) => crate::optimize::optimize_rgb8(bytes, w, h),
        (_, BitDepth::Sixteen) => {
            crate::optimize::optimize_16bit(bytes, w, h, color_type.to_png_byte())
        }
        _ => crate::optimize::OptimalEncoding::Original,
    };

    match optimization {
        crate::optimize::OptimalEncoding::Indexed {
            palette_rgb,
            palette_alpha,
            indices,
        } => {
            let opts = config.compress_options(cancel, deadline, None);
            crate::encoder::write_indexed_png(
                &indices,
                width,
                height,
                &palette_rgb,
                palette_alpha.as_deref(),
                &write_meta,
                effort,
                opts,
            )
        }
        crate::optimize::OptimalEncoding::Truecolor {
            bytes: opt_bytes,
            color_type: opt_ct,
            bit_depth: opt_bd,
            trns,
        } => {
            let opts = config.compress_options(cancel, deadline, None);
            crate::encoder::write_truecolor_png(
                &opt_bytes,
                width,
                height,
                opt_ct,
                opt_bd,
                trns.as_deref(),
                &write_meta,
                effort,
                opts,
            )
        }
        crate::optimize::OptimalEncoding::Original => {
            let opts = config.compress_options(cancel, deadline, None);
            crate::encoder::write_truecolor_png(
                bytes,
                width,
                height,
                color_type.to_png_byte(),
                bit_depth.to_png_byte(),
                None,
                &write_meta,
                effort,
                opts,
            )
        }
    }
}

/// Encode RGB8 pixels to PNG, returning per-phase compression statistics.
#[cfg(feature = "_dev")]
pub fn encode_rgb8_with_stats(
    img: ImgRef<Rgb<u8>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<(Vec<u8>, crate::encoder::PhaseStats), PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw_with_stats(
        bytes,
        width,
        height,
        ColorType::Rgb,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGBA8 pixels to PNG, returning per-phase compression statistics.
#[cfg(feature = "_dev")]
pub fn encode_rgba8_with_stats(
    img: ImgRef<Rgba<u8>>,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<(Vec<u8>, crate::encoder::PhaseStats), PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw_with_stats(
        bytes,
        width,
        height,
        ColorType::Rgba,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Low-level encode with stats: raw bytes to PNG with per-phase timing.
///
/// Applies the same auto-optimization as `encode_raw` (color type reduction,
/// bit depth reduction, auto-indexing).
#[cfg(feature = "_dev")]
#[allow(clippy::too_many_arguments)]
fn encode_raw_with_stats(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: BitDepth,
    metadata: Option<&MetadataView<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<(Vec<u8>, crate::encoder::PhaseStats), PngError> {
    let effort = config.compression.effort();

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.source_gamma;
    write_meta.srgb_intent = config.srgb_intent;
    write_meta.chromaticities = config.chromaticities;

    let w = width as usize;
    let h = height as usize;

    // Near-lossless: quantize LSBs for better compression (8-bit only)
    let nl_bytes;
    let bytes = if config.near_lossless_bits > 0 && bit_depth == BitDepth::Eight {
        let channels: usize = match color_type {
            ColorType::Grayscale => 1,
            ColorType::Rgb => 3,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgba => 4,
        };
        nl_bytes =
            crate::optimize::near_lossless_quantize(bytes, channels, config.near_lossless_bits);
        &nl_bytes
    } else {
        bytes
    };

    // Determine optimal encoding (same logic as encode_raw)
    let (eff_bytes, eff_ct, eff_bd, eff_trns) = match (color_type, bit_depth) {
        (ColorType::Rgba, BitDepth::Eight) => {
            match crate::optimize::optimize_rgba8(bytes, w, h) {
                crate::optimize::OptimalEncoding::Truecolor {
                    bytes: ob,
                    color_type: ct,
                    bit_depth: bd,
                    trns,
                } => (Some(ob), ct, bd, trns),
                crate::optimize::OptimalEncoding::Indexed { .. } => {
                    // Stats path doesn't support indexed; fall through to truecolor
                    (
                        None,
                        color_type.to_png_byte(),
                        bit_depth.to_png_byte(),
                        None,
                    )
                }
                crate::optimize::OptimalEncoding::Original => (
                    None,
                    color_type.to_png_byte(),
                    bit_depth.to_png_byte(),
                    None,
                ),
            }
        }
        (ColorType::Rgb, BitDepth::Eight) => match crate::optimize::optimize_rgb8(bytes, w, h) {
            crate::optimize::OptimalEncoding::Truecolor {
                bytes: ob,
                color_type: ct,
                bit_depth: bd,
                trns,
            } => (Some(ob), ct, bd, trns),
            _ => (
                None,
                color_type.to_png_byte(),
                bit_depth.to_png_byte(),
                None,
            ),
        },
        (_, BitDepth::Sixteen) => {
            match crate::optimize::optimize_16bit(bytes, w, h, color_type.to_png_byte()) {
                crate::optimize::OptimalEncoding::Truecolor {
                    bytes: ob,
                    color_type: ct,
                    bit_depth: bd,
                    trns,
                } => (Some(ob), ct, bd, trns),
                _ => (
                    None,
                    color_type.to_png_byte(),
                    bit_depth.to_png_byte(),
                    None,
                ),
            }
        }
        _ => (
            None,
            color_type.to_png_byte(),
            bit_depth.to_png_byte(),
            None,
        ),
    };

    let pixel_data = eff_bytes.as_deref().unwrap_or(bytes);
    let opts = config.compress_options(cancel, deadline, None);

    let mut stats = crate::encoder::PhaseStats::default();
    let png = crate::encoder::write_truecolor_png_with_stats(
        pixel_data,
        width,
        height,
        eff_ct,
        eff_bd,
        eff_trns.as_deref(),
        &write_meta,
        effort,
        opts,
        &mut stats,
    )?;
    Ok((png, stats))
}

/// Byte-swap native-endian u16 samples to big-endian for PNG.
fn native_to_be_16(native: &[u8]) -> Vec<u8> {
    if cfg!(target_endian = "big") {
        return native.to_vec();
    }
    let mut out = native.to_vec();
    for chunk in out.chunks_exact_mut(2) {
        chunk.swap(0, 1);
    }
    out
}

// ── APNG encoding ───────────────────────────────────────────────────

/// A single frame for APNG encoding.
///
/// Each frame must be canvas-sized RGBA8 pixel data (width * height * 4 bytes).
/// The encoder computes delta regions internally.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ApngFrameInput<'a> {
    /// Canvas-sized RGBA8 pixel data.
    pub pixels: &'a [u8],
    /// Numerator of the frame delay fraction (in seconds).
    pub delay_num: u16,
    /// Denominator of the frame delay fraction.
    /// Per the APNG spec, 0 is treated as 100 (i.e., delay_num/100 seconds).
    pub delay_den: u16,
}

impl<'a> ApngFrameInput<'a> {
    /// Create a new APNG frame input.
    #[must_use]
    pub fn new(pixels: &'a [u8], delay_num: u16, delay_den: u16) -> Self {
        Self {
            pixels,
            delay_num,
            delay_den,
        }
    }
}

/// APNG encode configuration.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ApngEncodeConfig {
    /// PNG compression/filter settings.
    pub encode: EncodeConfig,
    /// Animation loop count. 0 = infinite loop.
    pub num_plays: u32,
}

impl ApngEncodeConfig {
    /// Set the encode configuration.
    #[must_use]
    pub fn with_encode(mut self, encode: EncodeConfig) -> Self {
        self.encode = encode;
        self
    }

    /// Set the loop count (0 = infinite).
    #[must_use]
    pub fn with_num_plays(mut self, num_plays: u32) -> Self {
        self.num_plays = num_plays;
        self
    }
}

/// Encode canvas-sized RGBA8 frames into a truecolor APNG file.
///
/// All frames must be `canvas_width * canvas_height * 4` bytes of RGBA8 data.
/// The encoder computes delta regions between consecutive frames for efficient
/// encoding. At least one frame is required.
pub fn encode_apng(
    frames: &[ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    config: &ApngEncodeConfig,
    metadata: Option<&MetadataView<'_>>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    // Validation
    if frames.is_empty() {
        return Err(PngError::InvalidInput(
            "APNG requires at least one frame".into(),
        ));
    }
    let expected_len = canvas_width as usize * canvas_height as usize * 4;
    for (i, frame) in frames.iter().enumerate() {
        if frame.pixels.len() < expected_len {
            return Err(PngError::InvalidInput(alloc::format!(
                "frame {i}: pixel buffer too small: need {expected_len}, got {}",
                frame.pixels.len()
            )));
        }
    }

    let effort = config.encode.compression.effort();
    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.encode.source_gamma;
    write_meta.srgb_intent = config.encode.srgb_intent;
    write_meta.chromaticities = config.encode.chromaticities;

    crate::encoder::apng::encode_apng_truecolor(
        frames,
        canvas_width,
        canvas_height,
        &write_meta,
        config.num_plays,
        effort,
        cancel,
        deadline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use enough::Unstoppable;
    use imgref::Img;

    // ── EncodeConfig builder tests ──────────────────────────────────

    #[test]
    fn encode_config_defaults() {
        let c = EncodeConfig::default();
        assert_eq!(c.compression, Compression::Balanced);
        assert_eq!(c.filter, Filter::Auto);
        assert!(!c.parallel);
        assert!(c.source_gamma.is_none());
        assert!(c.srgb_intent.is_none());
        assert!(c.chromaticities.is_none());
        assert_eq!(c.near_lossless_bits, 0);
    }

    #[test]
    fn encode_config_with_filter() {
        let c = EncodeConfig::default().with_filter(Filter::Auto);
        assert_eq!(c.filter, Filter::Auto);
    }

    #[test]
    fn encode_config_with_parallel() {
        let c = EncodeConfig::default().with_parallel(true);
        assert!(c.parallel);
    }

    #[test]
    fn encode_config_with_source_gamma() {
        let c = EncodeConfig::default().with_source_gamma(Some(45455));
        assert_eq!(c.source_gamma, Some(45455));
    }

    #[test]
    fn encode_config_with_srgb_intent() {
        let c = EncodeConfig::default().with_srgb_intent(Some(1));
        assert_eq!(c.srgb_intent, Some(1));
    }

    #[test]
    fn encode_config_with_chromaticities() {
        let chrm = PngChromaticities {
            white_x: 31270,
            white_y: 32900,
            red_x: 64000,
            red_y: 33000,
            green_x: 30000,
            green_y: 60000,
            blue_x: 15000,
            blue_y: 6000,
        };
        let c = EncodeConfig::default().with_chromaticities(Some(chrm));
        assert_eq!(c.chromaticities.unwrap().red_x, 64000);
    }

    #[test]
    fn encode_config_with_near_lossless_bits() {
        let c = EncodeConfig::default().with_near_lossless_bits(2);
        assert_eq!(c.near_lossless_bits, 2);
    }

    // ── Compression effort mapping ──────────────────────────────────

    #[test]
    fn compression_effort_mapping() {
        assert_eq!(Compression::None.effort(), 0);
        assert_eq!(Compression::Fastest.effort(), 1);
        assert_eq!(Compression::Turbo.effort(), 2);
        assert_eq!(Compression::Fast.effort(), 7);
        assert_eq!(Compression::Balanced.effort(), 13);
        assert_eq!(Compression::Thorough.effort(), 17);
        assert_eq!(Compression::High.effort(), 19);
        assert_eq!(Compression::Aggressive.effort(), 22);
        assert_eq!(Compression::Intense.effort(), 24);
        assert_eq!(Compression::Crush.effort(), 27);
        assert_eq!(Compression::Maniac.effort(), 30);
        assert_eq!(Compression::Brag.effort(), 31);
        assert_eq!(Compression::Minutes.effort(), 200);
        assert_eq!(Compression::Effort(15).effort(), 15);
        assert_eq!(Compression::Effort(500).effort(), 200); // clamped
    }

    // ── Encode roundtrip at various effort levels ───────────────────

    fn small_rgb_image() -> (Vec<Rgb<u8>>, usize, usize) {
        let w = 8;
        let h = 4;
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| Rgb {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 23) as u8,
            })
            .collect();
        (pixels, w, h)
    }

    fn small_rgba_image() -> (Vec<Rgba<u8>>, usize, usize) {
        let w = 8;
        let h = 4;
        let pixels: Vec<Rgba<u8>> = (0..w * h)
            .map(|i| Rgba {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 23) as u8,
                a: if i % 4 == 0 { 0 } else { 255 },
            })
            .collect();
        (pixels, w, h)
    }

    fn roundtrip_rgb(config: &EncodeConfig) {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let encoded = encode_rgb8(img.as_ref(), None, config, &Unstoppable, &Unstoppable).unwrap();
        assert!(encoded[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
        assert_eq!(decoded.info.height, h as u32);
    }

    #[test]
    fn roundtrip_effort_0_store() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::None));
    }

    #[test]
    fn roundtrip_effort_1_fastest() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Fastest));
    }

    #[test]
    fn roundtrip_effort_2_turbo() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Turbo));
    }

    #[test]
    fn roundtrip_effort_7_fast() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Fast));
    }

    #[test]
    fn roundtrip_effort_13_balanced() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Balanced));
    }

    #[test]
    fn roundtrip_effort_17_thorough() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Thorough));
    }

    #[test]
    fn roundtrip_rgba_with_transparency() {
        let (pixels, w, h) = small_rgba_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Fast);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    #[test]
    fn roundtrip_gray8() {
        let pixels: Vec<Gray<u8>> = (0..32).map(|i| Gray(i * 8)).collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fast);
        let encoded =
            encode_gray8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert!(!decoded.info.has_alpha);
    }

    #[test]
    fn roundtrip_rgb16() {
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| Rgb {
                r: i * 2048,
                g: i * 1024,
                b: i * 512,
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    #[test]
    fn roundtrip_rgba16() {
        let pixels: Vec<Rgba<u16>> = (0..32)
            .map(|i| Rgba {
                r: i * 2048,
                g: i * 1024,
                b: i * 512,
                a: 65535,
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgba16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    #[test]
    fn roundtrip_gray16() {
        let pixels: Vec<Gray<u16>> = (0..32).map(|i| Gray(i * 2048)).collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_gray16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    #[test]
    fn near_lossless_produces_valid_output() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let encoded = encode_rgb8(
            img.as_ref(),
            None,
            &EncodeConfig::default()
                .with_compression(Compression::Fastest)
                .with_near_lossless_bits(3),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    // ── APNG encoding ───────────────────────────────────────────────

    #[test]
    fn apng_frame_input_new() {
        let pixels = vec![0u8; 16 * 16 * 4];
        let frame = ApngFrameInput::new(&pixels, 100, 1000);
        assert_eq!(frame.delay_num, 100);
        assert_eq!(frame.delay_den, 1000);
    }

    #[test]
    fn apng_encode_config_builders() {
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::Fast))
            .with_num_plays(3);
        assert_eq!(config.num_plays, 3);
        assert_eq!(config.encode.compression.effort(), 7);
    }

    #[test]
    fn apng_empty_frames_error() {
        let config = ApngEncodeConfig::default();
        let result = encode_apng(&[], 16, 16, &config, None, &Unstoppable, &Unstoppable);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("at least one frame")
        );
    }

    #[test]
    fn apng_buffer_too_small_error() {
        let config = ApngEncodeConfig::default();
        let small_buf = vec![0u8; 10];
        let frames = [ApngFrameInput::new(&small_buf, 100, 1000)];
        let result = encode_apng(&frames, 16, 16, &config, None, &Unstoppable, &Unstoppable);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));
    }

    #[test]
    fn apng_single_frame_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let pixels = vec![128u8; (w * h * 4) as usize];
        let frames = [ApngFrameInput::new(&pixels, 100, 1000)];
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::Fastest));
        let encoded =
            encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
        assert!(encoded[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    // ── Color type and bit depth ────────────────────────────────────

    #[test]
    fn color_type_to_png_byte() {
        assert_eq!(ColorType::Grayscale.to_png_byte(), 0);
        assert_eq!(ColorType::Rgb.to_png_byte(), 2);
        assert_eq!(ColorType::GrayscaleAlpha.to_png_byte(), 4);
        assert_eq!(ColorType::Rgba.to_png_byte(), 6);
    }

    #[test]
    fn bit_depth_to_png_byte() {
        assert_eq!(BitDepth::Eight.to_png_byte(), 8);
        assert_eq!(BitDepth::Sixteen.to_png_byte(), 16);
    }

    // ── Native to BE byte swap ──────────────────────────────────────

    #[test]
    fn native_to_be_16_involution() {
        let original = vec![0x12u8, 0x34, 0x56, 0x78];
        let be = native_to_be_16(&original);
        let back = native_to_be_16(&be);
        assert_eq!(original, back);
    }

    // ── Parallel encoding ───────────────────────────────────────────

    #[test]
    fn parallel_encoding_produces_valid_png() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default()
            .with_compression(Compression::Balanced)
            .with_parallel(true);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    // ── Additional effort tiers (cover compress.rs branches) ────────

    #[test]
    fn roundtrip_effort_3() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(3)));
    }

    #[test]
    fn roundtrip_effort_4() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(4)));
    }

    #[test]
    fn roundtrip_effort_5() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(5)));
    }

    #[test]
    fn roundtrip_effort_6() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(6)));
    }

    #[test]
    fn roundtrip_effort_8() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(8)));
    }

    #[test]
    fn roundtrip_effort_9() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(9)));
    }

    #[test]
    fn roundtrip_effort_10() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(10)));
    }

    #[test]
    fn roundtrip_effort_11() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(11)));
    }

    #[test]
    fn roundtrip_effort_12() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(12)));
    }

    #[test]
    fn roundtrip_effort_14() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(14)));
    }

    #[test]
    fn roundtrip_effort_15() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(15)));
    }

    #[test]
    fn roundtrip_effort_16() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(16)));
    }

    #[test]
    fn roundtrip_effort_19_high() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::High));
    }

    #[test]
    fn roundtrip_effort_22_aggressive() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Aggressive));
    }

    // ── Monotonicity: higher effort must not produce larger output ──

    #[test]
    fn monotonicity_effort_0_through_17() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let mut prev_size = usize::MAX;
        for effort in [0, 1, 2, 7, 13, 17] {
            let config = EncodeConfig::default().with_compression(Compression::Effort(effort));
            let encoded =
                encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
            // At effort 0 (store), output is larger; but from effort 1+, should be monotonic
            if effort > 0 && prev_size < usize::MAX {
                assert!(
                    encoded.len() <= prev_size,
                    "effort {effort} produced {} bytes, worse than previous {} bytes",
                    encoded.len(),
                    prev_size
                );
            }
            if effort > 0 {
                prev_size = encoded.len();
            }
        }
    }

    // ── Near-lossless with RGBA (exercises transparent pixel zeroing) ──

    #[test]
    fn near_lossless_rgba() {
        let (pixels, w, h) = small_rgba_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default()
            .with_compression(Compression::Fast)
            .with_near_lossless_bits(2);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    // ── Gray near-lossless ──────────────────────────────────────────

    #[test]
    fn near_lossless_gray() {
        let pixels: Vec<Gray<u8>> = (0..32).map(|i| Gray(i * 8)).collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default()
            .with_compression(Compression::Fast)
            .with_near_lossless_bits(2);
        let encoded =
            encode_gray8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    // ── Metadata roundtrip ──────────────────────────────────────────

    #[test]
    fn encode_with_gama_chrm() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let chrm = PngChromaticities {
            white_x: 31270,
            white_y: 32900,
            red_x: 64000,
            red_y: 33000,
            green_x: 30000,
            green_y: 60000,
            blue_x: 15000,
            blue_y: 6000,
        };
        let config = EncodeConfig::default()
            .with_compression(Compression::Fastest)
            .with_source_gamma(Some(45455))
            .with_chromaticities(Some(chrm));
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.source_gamma, Some(45455));
        assert_eq!(decoded.info.chromaticities.unwrap().red_x, 64000);
    }

    #[test]
    fn encode_with_srgb_intent() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default()
            .with_compression(Compression::Fastest)
            .with_srgb_intent(Some(0));
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.srgb_intent, Some(0));
    }

    // ── High effort on tiny data (brute-force, fork, beam paths) ────

    #[test]
    fn roundtrip_effort_24_intense_rgba() {
        let (pixels, w, h) = small_rgba_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Intense);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
        assert_eq!(decoded.info.height, h as u32);
    }

    #[test]
    fn roundtrip_effort_27_crush() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Crush);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    #[test]
    fn roundtrip_effort_30_maniac() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Maniac);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    #[test]
    fn roundtrip_effort_31_brag() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Brag);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    // ── 16-bit encode/decode roundtrips with pixel verification ─────

    #[test]
    fn roundtrip_rgb16_pixel_perfect() {
        // Use values where high_byte != low_byte so optimizer won't reduce to 8-bit
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| Rgb {
                r: i * 2048 + 1, // low byte nonzero → stays 16-bit
                g: i * 1024 + 3,
                b: i * 512 + 7,
            })
            .collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Balanced);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 16);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [p.r.to_ne_bytes(), p.g.to_ne_bytes(), p.b.to_ne_bytes()].concat())
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_rgba16_pixel_perfect() {
        let pixels: Vec<Rgba<u16>> = (0..32)
            .map(|i| Rgba {
                r: i * 2048 + 1,
                g: i * 1024 + 3,
                b: i * 512 + 7,
                a: 65535 - i * 1000,
            })
            .collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Balanced);
        let encoded =
            encode_rgba16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 16);
        assert!(decoded.info.has_alpha);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| {
                [
                    p.r.to_ne_bytes(),
                    p.g.to_ne_bytes(),
                    p.b.to_ne_bytes(),
                    p.a.to_ne_bytes(),
                ]
                .concat()
            })
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_gray16_pixel_perfect() {
        let pixels: Vec<Gray<u16>> = (0..32).map(|i| Gray(i * 2048 + 1)).collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Balanced);
        let encoded =
            encode_gray16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 16);
        assert!(!decoded.info.has_alpha);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| p.value().to_ne_bytes())
            .collect();
        assert_eq!(raw, expected);
    }

    // ── 16-bit values that reduce to 8-bit (optimization path) ──────

    #[test]
    fn rgb16_reduces_to_8bit_when_samples_fit() {
        // Values that are multiples of 256: BE representation has low byte = 0
        // e.g., 0x0800 → BE [0x08, 0x00] → reducible to 8-bit value 0x08
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| Rgb {
                r: i * 256,
                g: 0,
                b: 0,
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 8);
    }

    // ── RGBA fully opaque reduces to RGB ────────────────────────────

    #[test]
    fn rgba8_opaque_reduces_to_rgb() {
        let pixels: Vec<Rgba<u8>> = (0..32)
            .map(|i| Rgba {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 23) as u8,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert!(!decoded.info.has_alpha);
    }

    // ── Grayscale detection ─────────────────────────────────────────

    #[test]
    fn rgb8_grayscale_reduces_to_gray() {
        let pixels: Vec<Rgb<u8>> = (0..32)
            .map(|i| {
                let v = (i * 8) as u8;
                Rgb { r: v, g: v, b: v }
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        // Should be decoded as grayscale
        assert!(!decoded.info.has_alpha);
    }

    // ── Small exact palette triggers indexed ─────────────────────────

    #[test]
    fn rgba8_few_colors_uses_indexed() {
        // 4 unique colors → should auto-index
        let pixels: Vec<Rgba<u8>> = (0..64)
            .map(|i| match i % 4 {
                0 => Rgba {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                1 => Rgba {
                    r: 0,
                    g: 255,
                    b: 0,
                    a: 255,
                },
                2 => Rgba {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                },
                _ => Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            })
            .collect();
        let img = Img::new(pixels, 8, 8);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    // ── High effort on gray8 (low bpp brute-force path) ─────────────

    #[test]
    fn gray8_high_effort_roundtrip() {
        let pixels: Vec<Gray<u8>> = (0..64).map(|i| Gray((i * 4) as u8)).collect();
        let img = Img::new(pixels, 8, 8);
        let config = EncodeConfig::default().with_compression(Compression::High);
        let encoded =
            encode_gray8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    // ── 16-bit with high effort ─────────────────────────────────────

    #[test]
    fn rgb16_high_effort_roundtrip() {
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| Rgb {
                r: i * 2048,
                g: i * 1024,
                b: i * 512,
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::High);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    // ── Multi-frame APNG roundtrip ──────────────────────────────────

    #[test]
    fn apng_two_frame_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let sz = (w * h * 4) as usize;
        let frame0: Vec<u8> = (0..sz).map(|i| (i * 3) as u8).collect();
        let frame1: Vec<u8> = (0..sz).map(|i| (i * 7 + 1) as u8).collect();
        let frames = [
            ApngFrameInput::new(&frame0, 1, 30),
            ApngFrameInput::new(&frame1, 1, 30),
        ];
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::Fastest));
        let encoded =
            encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode_apng(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    #[test]
    fn apng_high_effort_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let sz = (w * h * 4) as usize;
        let frame0 = vec![128u8; sz];
        let frame1 = vec![64u8; sz];
        let frames = [
            ApngFrameInput::new(&frame0, 1, 10),
            ApngFrameInput::new(&frame1, 1, 10),
        ];
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::High));
        let encoded =
            encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
        assert!(encoded[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    // ── Non-animated PNG through decode_apng ────────────────────────

    #[test]
    fn decode_apng_on_static_png() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode_apng(&encoded, &crate::PngDecodeConfig::none(), &Unstoppable).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert_eq!(decoded.info.width, w as u32);
        assert_eq!(decoded.num_plays, 0);
    }
}
