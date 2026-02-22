//! PNG encoding.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::{Gray, Rgb, Rgba};

use zencodec_types::ImageMetadata;

use enough::Stop;

use crate::decode::PngChromaticities;
use crate::error::PngError;
use crate::png_writer::{self, PngWriteMetadata};
use crate::types::{Compression, Filter};

/// PNG encode configuration.
#[derive(Clone, Debug, Default)]
pub struct EncodeConfig {
    /// PNG compression level.
    pub compression: Compression,
    /// PNG row filter strategy.
    pub filter: Filter,
    /// Source gamma for gAMA chunk (scaled by 100000, e.g. 45455 = 1/2.2).
    pub source_gamma: Option<u32>,
    /// sRGB rendering intent for sRGB chunk (0-3).
    pub srgb_intent: Option<u8>,
    /// Chromaticities for cHRM chunk.
    pub chromaticities: Option<PngChromaticities>,
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
    ) -> png_writer::CompressOptions<'a> {
        png_writer::CompressOptions {
            use_zopfli: self.compression.use_zopfli(),
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
    metadata: Option<&ImageMetadata<'_>>,
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
    metadata: Option<&ImageMetadata<'_>>,
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
    metadata: Option<&ImageMetadata<'_>>,
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
    metadata: Option<&ImageMetadata<'_>>,
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
    metadata: Option<&ImageMetadata<'_>>,
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
    metadata: Option<&ImageMetadata<'_>>,
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
/// Uses zenflate multi-strategy compression for all color types.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_raw(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: BitDepth,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let level = config.compression.to_zenflate_level();
    let opts = config.compress_options(cancel, deadline, None);

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.source_gamma;
    write_meta.srgb_intent = config.srgb_intent;
    write_meta.chromaticities = config.chromaticities;

    png_writer::write_truecolor_png(
        bytes,
        width,
        height,
        color_type.to_png_byte(),
        bit_depth.to_png_byte(),
        &write_meta,
        level,
        opts,
    )
}

/// Encode RGB8 pixels to PNG, returning per-phase compression statistics.
#[cfg(feature = "_dev")]
pub fn encode_rgb8_with_stats(
    img: ImgRef<Rgb<u8>>,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<(Vec<u8>, crate::png_writer::PhaseStats), PngError> {
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
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<(Vec<u8>, crate::png_writer::PhaseStats), PngError> {
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
#[cfg(feature = "_dev")]
#[allow(clippy::too_many_arguments)]
fn encode_raw_with_stats(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: BitDepth,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<(Vec<u8>, crate::png_writer::PhaseStats), PngError> {
    let level = config.compression.to_zenflate_level();
    let opts = config.compress_options(cancel, deadline, None);

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.source_gamma;
    write_meta.srgb_intent = config.srgb_intent;
    write_meta.chromaticities = config.chromaticities;

    let mut stats = crate::png_writer::PhaseStats::default();
    let png = png_writer::write_truecolor_png_with_stats(
        bytes,
        width,
        height,
        color_type.to_png_byte(),
        bit_depth.to_png_byte(),
        &write_meta,
        level,
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
