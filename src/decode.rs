//! PNG decoding and probing.

use std::io::Cursor;

use alloc::vec::Vec;
use imgref::ImgVec;
use rgb::{Gray, Rgb, Rgba};
use zencodec_types::PixelData;

use crate::error::PngError;

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
    /// Embedded ICC color profile.
    pub icc_profile: Option<Vec<u8>>,
    /// Embedded EXIF metadata.
    pub exif: Option<Vec<u8>>,
    /// Embedded XMP metadata.
    pub xmp: Option<Vec<u8>>,
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
    fn validate(&self, width: u32, height: u32, bytes_per_pixel: u32) -> Result<(), PngError> {
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
    let cursor = Cursor::new(data);
    let decoder = png::Decoder::new(cursor);
    let reader = decoder.read_info()?;
    let info = reader.info();

    let has_alpha = has_alpha_channel(info);
    let has_animation = info.animation_control.is_some();
    let frame_count = info
        .animation_control
        .as_ref()
        .map_or(1, |actl| actl.num_frames);
    let icc_profile = info.icc_profile.as_ref().map(|p| p.to_vec());
    let exif = info.exif_metadata.as_ref().map(|p| p.to_vec());
    let xmp = extract_xmp_from_itxt(info);

    Ok(PngInfo {
        width: info.width,
        height: info.height,
        has_alpha,
        has_animation,
        frame_count,
        icc_profile,
        exif,
        xmp,
    })
}

/// Decode PNG to pixels.
pub fn decode(data: &[u8], limits: Option<&PngLimits>) -> Result<PngDecodeOutput, PngError> {
    let cursor = Cursor::new(data);
    let mut decoder = if let Some(lim) = limits {
        let png_limits = png::Limits {
            bytes: lim.max_memory_bytes.unwrap_or(64 * 1024 * 1024) as usize,
        };
        png::Decoder::new_with_limits(cursor, png_limits)
    } else {
        png::Decoder::new(cursor)
    };

    // Normalize to 8-bit color: expands indexed→RGB/RGBA, sub-8-bit
    // grayscale→8-bit, strips 16-bit→8-bit, and applies tRNS transparency.
    // Without this, indexed PNGs return raw packed palette indices and
    // sub-8-bit grayscale returns packed values, neither of which our
    // PixelData types can represent.
    decoder.set_transformations(png::Transformations::normalize_to_color8());

    let mut reader = decoder.read_info()?;
    let info = reader.info();

    let width = info.width;
    let height = info.height;
    let has_alpha = has_alpha_channel(info);
    let has_animation = info.animation_control.is_some();
    let frame_count = info
        .animation_control
        .as_ref()
        .map_or(1, |actl| actl.num_frames);

    if let Some(lim) = limits {
        // Output will be 4 bpp (RGBA) if alpha, 3 bpp (RGB) otherwise
        let bpp: u32 = if has_alpha { 4 } else { 3 };
        lim.validate(width, height, bpp)?;
    }

    let icc_profile = info.icc_profile.as_ref().map(|p| p.to_vec());
    let exif = info.exif_metadata.as_ref().map(|p| p.to_vec());
    let xmp = extract_xmp_from_itxt(info);

    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| PngError::InvalidInput("cannot determine PNG output buffer size".into()))?;
    let mut raw_pixels = alloc::vec![0u8; buffer_size];

    let output_info = reader.next_frame(&mut raw_pixels)?;
    raw_pixels.truncate(output_info.buffer_size());

    let (decoded_color_type, _bit_depth) = reader.output_color_type();
    let w = width as usize;
    let h = height as usize;

    let pixels = match decoded_color_type {
        png::ColorType::Rgba => {
            let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&raw_pixels);
            PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h))
        }
        png::ColorType::Rgb => {
            let rgb: &[Rgb<u8>] = bytemuck::cast_slice(&raw_pixels);
            PixelData::Rgb8(ImgVec::new(rgb.to_vec(), w, h))
        }
        png::ColorType::GrayscaleAlpha => {
            let rgba: Vec<Rgba<u8>> = raw_pixels
                .chunks_exact(2)
                .map(|ga| Rgba {
                    r: ga[0],
                    g: ga[0],
                    b: ga[0],
                    a: ga[1],
                })
                .collect();
            PixelData::Rgba8(ImgVec::new(rgba, w, h))
        }
        png::ColorType::Grayscale => {
            let gray: Vec<Gray<u8>> = raw_pixels.iter().map(|&g| Gray(g)).collect();
            PixelData::Gray8(ImgVec::new(gray, w, h))
        }
        png::ColorType::Indexed => {
            // Should not be reached with normalize_to_color8(), which expands
            // indexed images to RGB/RGBA. If it somehow does, error cleanly.
            return Err(PngError::InvalidInput(
                "indexed PNG was not expanded by decoder transforms".into(),
            ));
        }
    };

    Ok(PngDecodeOutput {
        pixels,
        info: PngInfo {
            width,
            height,
            has_alpha,
            has_animation,
            frame_count,
            icc_profile,
            exif,
            xmp,
        },
    })
}

/// Determine whether a PNG image has an alpha channel.
///
/// Checks for native alpha (RGBA, GrayscaleAlpha) and tRNS transparency
/// on any color type (indexed palette alpha, truecolor/grayscale transparent color).
fn has_alpha_channel(info: &png::Info<'_>) -> bool {
    match info.color_type {
        png::ColorType::Rgba | png::ColorType::GrayscaleAlpha => true,
        _ => info.trns.is_some(),
    }
}

/// Extract XMP from iTXt chunks with keyword "XML:com.adobe.xmp".
pub(crate) fn extract_xmp_from_itxt(info: &png::Info<'_>) -> Option<Vec<u8>> {
    for chunk in &info.utf8_text {
        if chunk.keyword == "XML:com.adobe.xmp" {
            if let Ok(text) = chunk.get_text() {
                if !text.is_empty() {
                    return Some(text.into_bytes());
                }
            }
        }
    }
    None
}
