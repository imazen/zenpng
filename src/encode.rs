//! PNG encoding.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::{Gray, Rgb, Rgba};

use zencodec_types::ImageMetadata;

use crate::error::PngError;

/// PNG encode configuration.
#[derive(Clone, Debug, Default)]
pub struct EncodeConfig {
    /// PNG compression level.
    pub compression: png::Compression,
    /// PNG row filter type.
    pub filter: png::Filter,
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
    encode_raw(bytes, width, height, png::ColorType::Rgb, metadata, config)
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
    encode_raw(bytes, width, height, png::ColorType::Rgba, metadata, config)
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
        metadata,
        config,
    )
}

/// Low-level encode: raw bytes to PNG with metadata and config applied.
pub(crate) fn encode_raw(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: png::ColorType,
    metadata: Option<&ImageMetadata<'_>>,
    config: &EncodeConfig,
) -> Result<Vec<u8>, PngError> {
    let mut output = Vec::new();

    let info = make_png_info(width, height, color_type, metadata);
    let mut encoder = png::Encoder::with_info(&mut output, info)?;
    encoder.set_compression(config.compression);
    encoder.set_filter(config.filter);

    let mut writer = encoder.write_header()?;
    writer.write_image_data(bytes)?;
    drop(writer);

    Ok(output)
}

/// Create a PNG Info struct with metadata applied.
pub(crate) fn make_png_info<'a>(
    width: u32,
    height: u32,
    color_type: png::ColorType,
    metadata: Option<&'a ImageMetadata<'a>>,
) -> png::Info<'a> {
    let mut info = png::Info::with_size(width, height);
    info.color_type = color_type;
    info.bit_depth = png::BitDepth::Eight;

    if let Some(meta) = metadata {
        if let Some(icc) = meta.icc_profile {
            info.icc_profile = Some(icc.into());
        }
        if let Some(exif) = meta.exif {
            info.exif_metadata = Some(exif.into());
        }
        if let Some(xmp) = meta.xmp {
            let xmp_str = core::str::from_utf8(xmp).unwrap_or_default();
            if !xmp_str.is_empty() {
                info.utf8_text.push(png::text_metadata::ITXtChunk::new(
                    "XML:com.adobe.xmp",
                    xmp_str,
                ));
            }
        }
    }

    info
}
