//! PNG encoding.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::{Gray, Rgb, Rgba};

use zencodec_types::ImageMetadata;

use crate::decode::PngChromaticities;
use crate::error::PngError;
use crate::png_writer::{self, PngWriteMetadata};

/// PNG encode configuration.
#[derive(Clone, Debug, Default)]
pub struct EncodeConfig {
    /// PNG compression level.
    pub compression: png::Compression,
    /// PNG row filter type (ignored — multi-strategy selection is always used).
    pub filter: png::Filter,
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
    pub fn with_compression(mut self, compression: png::Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Set row filter type.
    #[must_use]
    pub fn with_filter(mut self, filter: png::Filter) -> Self {
        self.filter = filter;
        self
    }
}

/// Encode RGB8 pixels to PNG.
pub fn encode_rgb8(
    img: ImgRef<Rgb<u8>>,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw(
        bytes,
        width,
        height,
        png::ColorType::Rgb,
        png::BitDepth::Eight,
        metadata,
        config,
    )
}

/// Encode RGBA8 pixels to PNG.
pub fn encode_rgba8(
    img: ImgRef<Rgba<u8>>,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw(
        bytes,
        width,
        height,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        metadata,
        config,
    )
}

/// Encode Gray8 pixels to PNG.
pub fn encode_gray8(
    img: ImgRef<Gray<u8>>,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: Vec<u8> = buf.iter().map(|g| g.value()).collect();
    encode_raw(
        &bytes,
        width,
        height,
        png::ColorType::Grayscale,
        png::BitDepth::Eight,
        metadata,
        config,
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
        png::ColorType::Rgb,
        png::BitDepth::Sixteen,
        metadata,
        config,
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
        png::ColorType::Rgba,
        png::BitDepth::Sixteen,
        metadata,
        config,
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
        png::ColorType::Grayscale,
        png::BitDepth::Sixteen,
        metadata,
        config,
    )
}

/// Low-level encode: raw bytes to PNG with metadata and config applied.
///
/// Uses zenflate multi-strategy compression for all color types.
pub(crate) fn encode_raw(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: png::ColorType,
    bit_depth: png::BitDepth,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
) -> Result<Vec<u8>, PngError> {
    let png_color_type: u8 = match color_type {
        png::ColorType::Grayscale => 0,
        png::ColorType::Rgb => 2,
        png::ColorType::GrayscaleAlpha => 4,
        png::ColorType::Rgba => 6,
        _ => {
            return Err(PngError::InvalidInput(alloc::format!(
                "unsupported color type for truecolor PNG: {color_type:?}"
            )));
        }
    };
    let png_bit_depth: u8 = match bit_depth {
        png::BitDepth::Eight => 8,
        png::BitDepth::Sixteen => 16,
        _ => {
            return Err(PngError::InvalidInput(alloc::format!(
                "unsupported bit depth for truecolor PNG: {bit_depth:?}"
            )));
        }
    };
    let level = compression_to_zenflate_level(config.compression);

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.source_gamma;
    write_meta.srgb_intent = config.srgb_intent;
    write_meta.chromaticities = config.chromaticities;

    png_writer::write_truecolor_png(
        bytes,
        width,
        height,
        png_color_type,
        png_bit_depth,
        &write_meta,
        level,
    )
}

/// Map `png::Compression` levels to zenflate compression levels (0-12).
pub(crate) fn compression_to_zenflate_level(compression: png::Compression) -> u8 {
    match compression {
        png::Compression::NoCompression => 0,
        png::Compression::Fastest => 1,
        png::Compression::Fast => 4,
        png::Compression::Balanced => 9,
        png::Compression::High => 12,
        _ => 9,
    }
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
