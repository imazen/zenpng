//! PNG encoding.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::{Gray, Rgb, Rgba};

use zencodec_types::MetadataView;

use enough::Stop;

use crate::decode::PngChromaticities;
use crate::encoder::PngWriteMetadata;
use crate::error::PngError;
use crate::types::{Compression, Filter};

/// PNG encode configuration.
#[derive(Clone, Debug, Default)]
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
    pub source_gamma: Option<u32>,
    /// sRGB rendering intent for sRGB chunk (0-3).
    pub srgb_intent: Option<u8>,
    /// Chromaticities for cHRM chunk.
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

    /// Build compression options from this config and caller-provided stops.
    pub(crate) fn compress_options<'a>(
        &self,
        cancel: &'a dyn Stop,
        deadline: &'a dyn Stop,
        remaining_ns: Option<&'a dyn Fn() -> Option<u64>>,
    ) -> crate::encoder::CompressOptions<'a> {
        crate::encoder::CompressOptions {
            parallel: self.parallel,
            cancel,
            deadline,
            remaining_ns,
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
                    (None, color_type.to_png_byte(), bit_depth.to_png_byte(), None)
                }
                crate::optimize::OptimalEncoding::Original => {
                    (None, color_type.to_png_byte(), bit_depth.to_png_byte(), None)
                }
            }
        }
        (ColorType::Rgb, BitDepth::Eight) => match crate::optimize::optimize_rgb8(bytes, w, h) {
            crate::optimize::OptimalEncoding::Truecolor {
                bytes: ob,
                color_type: ct,
                bit_depth: bd,
                trns,
            } => (Some(ob), ct, bd, trns),
            _ => (None, color_type.to_png_byte(), bit_depth.to_png_byte(), None),
        },
        (_, BitDepth::Sixteen) => {
            match crate::optimize::optimize_16bit(bytes, w, h, color_type.to_png_byte()) {
                crate::optimize::OptimalEncoding::Truecolor {
                    bytes: ob,
                    color_type: ct,
                    bit_depth: bd,
                    trns,
                } => (Some(ob), ct, bd, trns),
                _ => (None, color_type.to_png_byte(), bit_depth.to_png_byte(), None),
            }
        }
        _ => (None, color_type.to_png_byte(), bit_depth.to_png_byte(), None),
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
pub struct ApngFrameInput<'a> {
    /// Canvas-sized RGBA8 pixel data.
    pub pixels: &'a [u8],
    /// Numerator of the frame delay fraction (in seconds).
    pub delay_num: u16,
    /// Denominator of the frame delay fraction.
    /// Per the APNG spec, 0 is treated as 100 (i.e., delay_num/100 seconds).
    pub delay_den: u16,
}

/// APNG encode configuration.
#[derive(Clone, Debug, Default)]
pub struct ApngEncodeConfig {
    /// PNG compression/filter settings.
    pub encode: EncodeConfig,
    /// Animation loop count. 0 = infinite loop.
    pub num_plays: u32,
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
