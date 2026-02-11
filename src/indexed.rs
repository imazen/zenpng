//! Indexed (palette) PNG encoding via zenquant.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::Rgba;

use zencodec_types::ImageMetadata;
use zenquant::{OutputFormat, QuantizeConfig};

use crate::encode::EncodeConfig;
use crate::error::PngError;

/// Encode RGBA8 pixels to indexed PNG using zenquant for palette quantization.
///
/// Quantizes the image to at most 256 colors, then writes an indexed PNG
/// with PLTE and optional tRNS chunks.
pub fn encode_indexed_rgba8(
    img: ImgRef<Rgba<u8>>,
    encode_config: &EncodeConfig,
    quant_config: &QuantizeConfig,
    metadata: Option<&ImageMetadata<'_>>,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, w, h) = img.to_contiguous_buf();

    // Convert Rgba<u8> to zenquant's RGBA type
    let rgba_slice: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(buf.as_ref());
    let result = zenquant::quantize_rgba(rgba_slice, w, h, quant_config)?;

    let mut output = Vec::new();

    // Build PNG info with indexed color type
    let mut info = crate::encode::make_png_info(width, height, png::ColorType::Indexed, metadata);

    // Set PLTE from quantized palette
    let palette_rgba = result.palette_rgba();
    let mut plte = Vec::with_capacity(palette_rgba.len() * 3);
    let mut trns = Vec::with_capacity(palette_rgba.len());
    let mut has_transparency = false;

    for entry in palette_rgba {
        plte.push(entry[0]);
        plte.push(entry[1]);
        plte.push(entry[2]);
        trns.push(entry[3]);
        if entry[3] < 255 {
            has_transparency = true;
        }
    }

    info.palette = Some(plte.into());
    if has_transparency {
        info.trns = Some(trns.into());
    }

    let mut encoder = png::Encoder::with_info(&mut output, info)?;
    encoder.set_compression(encode_config.compression);
    encoder.set_filter(encode_config.filter);

    let mut writer = encoder.write_header()?;
    writer.write_image_data(result.indices())?;
    drop(writer);

    Ok(output)
}

/// Create a default [`QuantizeConfig`] tuned for PNG output.
pub fn default_quantize_config() -> QuantizeConfig {
    QuantizeConfig::new(OutputFormat::Png)
}
