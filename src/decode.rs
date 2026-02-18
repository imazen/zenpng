//! PNG decoding and probing.

use std::io::Cursor;

use alloc::vec::Vec;
use imgref::ImgVec;
use rgb::{Gray, Rgb, Rgba};
use zencodec_types::{Cicp, ContentLightLevel, GrayAlpha, MasteringDisplay, PixelData};

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

    Ok(extract_png_info(info))
}

/// Decode PNG to pixels.
///
/// Preserves 16-bit depth when present in the source. Expands indexed
/// and sub-8-bit formats to their natural RGB/RGBA/Gray equivalents.
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

    // EXPAND only: expands indexed→RGB/RGBA, sub-8-bit grayscale→8-bit,
    // tRNS→alpha. Crucially does NOT strip 16-bit to 8-bit.
    decoder.set_transformations(png::Transformations::EXPAND);

    let mut reader = decoder.read_info()?;
    let info = reader.info();

    let png_info = extract_png_info(info);

    let (decoded_color_type, decoded_bit_depth) = reader.output_color_type();

    if let Some(lim) = limits {
        let bpp = output_bytes_per_pixel(decoded_color_type, decoded_bit_depth);
        lim.validate(png_info.width, png_info.height, bpp as u32)?;
    }

    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| PngError::InvalidInput("cannot determine PNG output buffer size".into()))?;
    let mut raw_pixels = alloc::vec![0u8; buffer_size];

    let output_info = reader.next_frame(&mut raw_pixels)?;
    raw_pixels.truncate(output_info.buffer_size());

    let w = png_info.width as usize;
    let h = png_info.height as usize;

    let pixels = match (decoded_color_type, decoded_bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Sixteen) => {
            let native = be_to_native_16(&raw_pixels);
            let rgba: &[Rgba<u16>] = bytemuck::cast_slice(&native);
            PixelData::Rgba16(ImgVec::new(rgba.to_vec(), w, h))
        }
        (png::ColorType::Rgba, _) => {
            let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&raw_pixels);
            PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h))
        }
        (png::ColorType::Rgb, png::BitDepth::Sixteen) => {
            let native = be_to_native_16(&raw_pixels);
            let rgb: &[Rgb<u16>] = bytemuck::cast_slice(&native);
            PixelData::Rgb16(ImgVec::new(rgb.to_vec(), w, h))
        }
        (png::ColorType::Rgb, _) => {
            let rgb: &[Rgb<u8>] = bytemuck::cast_slice(&raw_pixels);
            PixelData::Rgb8(ImgVec::new(rgb.to_vec(), w, h))
        }
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Sixteen) => {
            let native = be_to_native_16(&raw_pixels);
            let ga: &[[u16; 2]] = bytemuck::cast_slice(&native);
            let pixels: Vec<GrayAlpha<u16>> =
                ga.iter().map(|&[v, a]| GrayAlpha::new(v, a)).collect();
            PixelData::GrayAlpha16(ImgVec::new(pixels, w, h))
        }
        (png::ColorType::GrayscaleAlpha, _) => {
            // Expand gray+alpha to RGBA for 8-bit (matches previous behavior)
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
        (png::ColorType::Grayscale, png::BitDepth::Sixteen) => {
            let native = be_to_native_16(&raw_pixels);
            let gray: &[Gray<u16>] = bytemuck::cast_slice(&native);
            PixelData::Gray16(ImgVec::new(gray.to_vec(), w, h))
        }
        (png::ColorType::Grayscale, _) => {
            let gray: Vec<Gray<u8>> = raw_pixels.iter().map(|&g| Gray(g)).collect();
            PixelData::Gray8(ImgVec::new(gray, w, h))
        }
        (png::ColorType::Indexed, _) => {
            return Err(PngError::InvalidInput(
                "indexed PNG was not expanded by decoder transforms".into(),
            ));
        }
    };

    Ok(PngDecodeOutput {
        pixels,
        info: png_info,
    })
}

/// Extract all metadata from png::Info into PngInfo.
fn extract_png_info(info: &png::Info<'_>) -> PngInfo {
    let has_alpha = has_alpha_channel(info);
    let has_animation = info.animation_control.is_some();
    let frame_count = info
        .animation_control
        .as_ref()
        .map_or(1, |actl| actl.num_frames);

    let source_gamma = info.gama_chunk.map(|g| g.into_scaled());
    let srgb_intent = info.srgb.map(|s| s as u8);
    let chromaticities = info.chrm_chunk.map(|c| PngChromaticities {
        white_x: c.white.0.into_scaled(),
        white_y: c.white.1.into_scaled(),
        red_x: c.red.0.into_scaled(),
        red_y: c.red.1.into_scaled(),
        green_x: c.green.0.into_scaled(),
        green_y: c.green.1.into_scaled(),
        blue_x: c.blue.0.into_scaled(),
        blue_y: c.blue.1.into_scaled(),
    });

    let cicp = info.coding_independent_code_points.map(|c| {
        Cicp::new(
            c.color_primaries,
            c.transfer_function,
            c.matrix_coefficients,
            c.is_video_full_range_image,
        )
    });

    let content_light_level = info.content_light_level.map(|c| {
        ContentLightLevel::new(
            (c.max_content_light_level / 10000).min(65535) as u16,
            (c.max_frame_average_light_level / 10000).min(65535) as u16,
        )
    });

    let mastering_display = info.mastering_display_color_volume.map(|m| {
        // png crate uses ScaledFloat (u32/100000), zencodec uses u16 * 0.00002 (u16/50000)
        let sc = |v: png::ScaledFloat| (v.into_scaled() / 2).min(65535) as u16;
        MasteringDisplay::new(
            [
                [sc(m.chromaticities.red.0), sc(m.chromaticities.red.1)],
                [sc(m.chromaticities.green.0), sc(m.chromaticities.green.1)],
                [sc(m.chromaticities.blue.0), sc(m.chromaticities.blue.1)],
            ],
            [sc(m.chromaticities.white.0), sc(m.chromaticities.white.1)],
            m.max_luminance,
            m.min_luminance,
        )
    });

    PngInfo {
        width: info.width,
        height: info.height,
        has_alpha,
        has_animation,
        frame_count,
        bit_depth: info.bit_depth as u8,
        icc_profile: info.icc_profile.as_ref().map(|p| p.to_vec()),
        exif: info.exif_metadata.as_ref().map(|p| p.to_vec()),
        xmp: extract_xmp_from_itxt(info),
        source_gamma,
        srgb_intent,
        chromaticities,
        cicp,
        content_light_level,
        mastering_display,
    }
}

/// Compute output bytes per pixel for a given color type and bit depth.
fn output_bytes_per_pixel(color_type: png::ColorType, bit_depth: png::BitDepth) -> usize {
    let channels: usize = match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => 1,
    };
    let depth_bytes = match bit_depth {
        png::BitDepth::Sixteen => 2,
        _ => 1,
    };
    channels * depth_bytes
}

/// Byte-swap big-endian u16 samples from PNG to native endian.
fn be_to_native_16(bytes: &[u8]) -> Vec<u8> {
    if cfg!(target_endian = "big") {
        return bytes.to_vec();
    }
    let mut out = bytes.to_vec();
    for chunk in out.chunks_exact_mut(2) {
        chunk.swap(0, 1);
    }
    out
}

/// Determine whether a PNG image has an alpha channel.
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
