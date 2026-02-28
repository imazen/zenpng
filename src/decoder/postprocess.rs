//! Post-processing: raw unfiltered rows → output pixels.

use alloc::vec::Vec;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::error::PngError;

use imgref::ImgVec;
use rgb::{Gray, Rgb, Rgba};
use zencodec_types::{GrayAlpha16, Pixel, PixelBuffer};

// ── Post-processing ─────────────────────────────────────────────────

/// Compute output bytes per pixel after post-processing (for limits checks).
pub(crate) fn output_bytes_per_pixel(ihdr: &Ihdr, ancillary: &PngAncillary) -> usize {
    match ihdr.color_type {
        0 => {
            // Grayscale
            if ancillary.trns.is_some() {
                // Gray + tRNS → RGBA8 (for 8-bit) or GrayAlpha16 (4 bytes either way)
                4
            } else if ihdr.bit_depth == 16 {
                2
            } else {
                1
            }
        }
        2 => {
            // RGB
            if ancillary.trns.is_some() {
                if ihdr.bit_depth == 16 { 8 } else { 4 }
            } else if ihdr.bit_depth == 16 {
                6
            } else {
                3
            }
        }
        3 => {
            // Indexed → RGB8 or RGBA8
            if ancillary.trns.is_some() { 4 } else { 3 }
        }
        4 => {
            // GrayAlpha: GA8 → RGBA8 (4 bytes), GA16 → GrayAlpha16 (4 bytes)
            4
        }
        6 => {
            // RGBA
            if ihdr.bit_depth == 16 { 8 } else { 4 }
        }
        _ => 4,
    }
}

/// Scale sub-8-bit gray value to 8-bit.
pub(crate) fn scale_to_8bit(value: u8, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => {
            if value != 0 {
                255
            } else {
                0
            }
        }
        2 => value * 85, // 0→0, 1→85, 2→170, 3→255
        4 => value * 17, // 0→0, 1→17, ..., 15→255
        _ => value,
    }
}

/// Unpack sub-8-bit grayscale pixels from a packed row.
fn unpack_sub_byte_gray(raw: &[u8], width: usize, bit_depth: u8, out: &mut Vec<u8>) {
    let pixels_per_byte = 8 / bit_depth as usize;
    let mask = (1u8 << bit_depth) - 1;

    for x in 0..width {
        let byte_idx = x / pixels_per_byte;
        let bit_offset = (pixels_per_byte - 1 - x % pixels_per_byte) * bit_depth as usize;
        let value = (raw[byte_idx] >> bit_offset) & mask;
        out.push(scale_to_8bit(value, bit_depth));
    }
}

/// Unpack sub-8-bit indexed pixels from a packed row.
fn unpack_sub_byte_indexed(raw: &[u8], width: usize, bit_depth: u8, out: &mut Vec<u8>) {
    let pixels_per_byte = 8 / bit_depth as usize;
    let mask = (1u8 << bit_depth) - 1;

    for x in 0..width {
        let byte_idx = x / pixels_per_byte;
        let bit_offset = (pixels_per_byte - 1 - x % pixels_per_byte) * bit_depth as usize;
        let index = (raw[byte_idx] >> bit_offset) & mask;
        out.push(index);
    }
}

/// Post-process a raw unfiltered row into output pixels.
/// Returns the output pixel data for this row.
pub(crate) fn post_process_row(
    raw: &[u8],
    ihdr: &Ihdr,
    ancillary: &PngAncillary,
    out: &mut Vec<u8>,
) {
    out.clear();
    let width = ihdr.width as usize;

    match ihdr.color_type {
        0 => {
            // Grayscale
            if ihdr.is_sub_byte() {
                if let Some(ref trns) = ancillary.trns {
                    // tRNS value is in original bit depth range
                    let trns_val = if trns.len() >= 2 {
                        u16::from_be_bytes([trns[0], trns[1]])
                    } else {
                        0
                    };
                    // Unpack, compare raw values against tRNS, then scale
                    let pixels_per_byte = 8 / ihdr.bit_depth as usize;
                    let mask = (1u8 << ihdr.bit_depth) - 1;
                    for x in 0..width {
                        let byte_idx = x / pixels_per_byte;
                        let bit_offset =
                            (pixels_per_byte - 1 - x % pixels_per_byte) * ihdr.bit_depth as usize;
                        let raw_val = (raw[byte_idx] >> bit_offset) & mask;
                        let alpha = if raw_val as u16 == trns_val { 0u8 } else { 255 };
                        let g = scale_to_8bit(raw_val, ihdr.bit_depth);
                        out.extend_from_slice(&[g, g, g, alpha]);
                    }
                } else {
                    // Sub-8-bit without tRNS: unpack and scale to 8-bit
                    let mut gray_pixels = Vec::with_capacity(width);
                    unpack_sub_byte_gray(raw, width, ihdr.bit_depth, &mut gray_pixels);
                    out.extend_from_slice(&gray_pixels);
                }
            } else if ihdr.bit_depth == 16 {
                if let Some(ref trns) = ancillary.trns {
                    let trns_val = if trns.len() >= 2 {
                        u16::from_be_bytes([trns[0], trns[1]])
                    } else {
                        0
                    };
                    // Gray16 + tRNS → GrayAlpha16 (4 bytes per pixel, native endian)
                    for chunk in raw.chunks_exact(2) {
                        let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                        let alpha: u16 = if val == trns_val { 0 } else { 65535 };
                        out.extend_from_slice(&val.to_ne_bytes());
                        out.extend_from_slice(&alpha.to_ne_bytes());
                    }
                } else {
                    // Gray16 → native endian
                    for chunk in raw.chunks_exact(2) {
                        let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                        out.extend_from_slice(&val.to_ne_bytes());
                    }
                }
            } else {
                // Gray8
                if let Some(ref trns) = ancillary.trns {
                    let trns_val = if trns.len() >= 2 {
                        u16::from_be_bytes([trns[0], trns[1]])
                    } else {
                        0
                    };
                    // Gray8 + tRNS → RGBA8
                    for &g in raw.iter().take(width) {
                        let alpha = if g as u16 == trns_val { 0u8 } else { 255 };
                        out.extend_from_slice(&[g, g, g, alpha]);
                    }
                } else {
                    out.extend_from_slice(&raw[..width]);
                }
            }
        }
        2 => {
            // RGB
            if ihdr.bit_depth == 16 {
                if let Some(ref trns) = ancillary.trns {
                    // tRNS for RGB: 6 bytes (R16, G16, B16)
                    let (tr, tg, tb) = if trns.len() >= 6 {
                        (
                            u16::from_be_bytes([trns[0], trns[1]]),
                            u16::from_be_bytes([trns[2], trns[3]]),
                            u16::from_be_bytes([trns[4], trns[5]]),
                        )
                    } else {
                        (0, 0, 0)
                    };
                    // RGB16 + tRNS → RGBA16 native endian
                    for chunk in raw.chunks_exact(6) {
                        let r = u16::from_be_bytes([chunk[0], chunk[1]]);
                        let g = u16::from_be_bytes([chunk[2], chunk[3]]);
                        let b = u16::from_be_bytes([chunk[4], chunk[5]]);
                        let alpha: u16 = if r == tr && g == tg && b == tb {
                            0
                        } else {
                            65535
                        };
                        out.extend_from_slice(&r.to_ne_bytes());
                        out.extend_from_slice(&g.to_ne_bytes());
                        out.extend_from_slice(&b.to_ne_bytes());
                        out.extend_from_slice(&alpha.to_ne_bytes());
                    }
                } else {
                    // RGB16 → native endian
                    for chunk in raw.chunks_exact(2) {
                        let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                        out.extend_from_slice(&val.to_ne_bytes());
                    }
                }
            } else {
                // RGB8
                if let Some(ref trns) = ancillary.trns {
                    let (tr, tg, tb) = if trns.len() >= 6 {
                        (
                            u16::from_be_bytes([trns[0], trns[1]]) as u8,
                            u16::from_be_bytes([trns[2], trns[3]]) as u8,
                            u16::from_be_bytes([trns[4], trns[5]]) as u8,
                        )
                    } else {
                        (0, 0, 0)
                    };
                    // RGB8 + tRNS → RGBA8
                    for chunk in raw.chunks_exact(3).take(width) {
                        let alpha = if chunk[0] == tr && chunk[1] == tg && chunk[2] == tb {
                            0u8
                        } else {
                            255
                        };
                        out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], alpha]);
                    }
                } else {
                    let row_bytes = width * 3;
                    out.extend_from_slice(&raw[..row_bytes]);
                }
            }
        }
        3 => {
            // Indexed
            let palette = ancillary.palette.as_deref().unwrap_or(&[]);
            let trns = ancillary.trns.as_deref();
            let has_trns = trns.is_some();

            let indices: Vec<u8> = if ihdr.is_sub_byte() {
                let mut idx = Vec::with_capacity(width);
                unpack_sub_byte_indexed(raw, width, ihdr.bit_depth, &mut idx);
                idx
            } else {
                raw[..width].to_vec()
            };

            for &index in &indices {
                let i = index as usize;
                let (r, g, b) = if i * 3 + 2 < palette.len() {
                    (palette[i * 3], palette[i * 3 + 1], palette[i * 3 + 2])
                } else {
                    (0, 0, 0) // Out of range index
                };
                if has_trns {
                    let alpha = trns.and_then(|t| t.get(i)).copied().unwrap_or(255);
                    out.extend_from_slice(&[r, g, b, alpha]);
                } else {
                    out.extend_from_slice(&[r, g, b]);
                }
            }
        }
        4 => {
            // GrayAlpha
            if ihdr.bit_depth == 16 {
                // GrayAlpha16 → native endian
                for chunk in raw.chunks_exact(4) {
                    let v = u16::from_be_bytes([chunk[0], chunk[1]]);
                    let a = u16::from_be_bytes([chunk[2], chunk[3]]);
                    out.extend_from_slice(&v.to_ne_bytes());
                    out.extend_from_slice(&a.to_ne_bytes());
                }
            } else {
                // GrayAlpha8 → RGBA8 (matches decode.rs:182-192 behavior)
                for chunk in raw.chunks_exact(2).take(width) {
                    let g = chunk[0];
                    let a = chunk[1];
                    out.extend_from_slice(&[g, g, g, a]);
                }
            }
        }
        6 => {
            // RGBA
            if ihdr.bit_depth == 16 {
                // RGBA16 → native endian
                for chunk in raw.chunks_exact(2) {
                    let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                    out.extend_from_slice(&val.to_ne_bytes());
                }
            } else {
                // RGBA8 — pass through
                let row_bytes = width * 4;
                out.extend_from_slice(&raw[..row_bytes]);
            }
        }
        _ => unreachable!("validated in IHDR parsing"),
    }
}

/// Determine the output PixelData variant info for `PngInfo` construction.
pub(crate) struct OutputFormat {
    pub channels: usize,
    pub bytes_per_channel: usize,
}

impl OutputFormat {
    pub fn from_ihdr(ihdr: &Ihdr, ancillary: &PngAncillary) -> Self {
        match ihdr.color_type {
            0 => {
                if ancillary.trns.is_some() {
                    // Gray + tRNS → RGBA
                    if ihdr.bit_depth == 16 {
                        Self {
                            channels: 2,
                            bytes_per_channel: 2,
                        }
                    } else {
                        Self {
                            channels: 4,
                            bytes_per_channel: 1,
                        }
                    }
                } else if ihdr.bit_depth == 16 {
                    Self {
                        channels: 1,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 1,
                        bytes_per_channel: 1,
                    }
                }
            }
            2 => {
                if ancillary.trns.is_some() {
                    if ihdr.bit_depth == 16 {
                        Self {
                            channels: 4,
                            bytes_per_channel: 2,
                        }
                    } else {
                        Self {
                            channels: 4,
                            bytes_per_channel: 1,
                        }
                    }
                } else if ihdr.bit_depth == 16 {
                    Self {
                        channels: 3,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 3,
                        bytes_per_channel: 1,
                    }
                }
            }
            3 => {
                if ancillary.trns.is_some() {
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                } else {
                    Self {
                        channels: 3,
                        bytes_per_channel: 1,
                    }
                }
            }
            4 => {
                if ihdr.bit_depth == 16 {
                    Self {
                        channels: 2,
                        bytes_per_channel: 2,
                    }
                } else {
                    // GA8 → RGBA8
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                }
            }
            6 => {
                if ihdr.bit_depth == 16 {
                    Self {
                        channels: 4,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Build PixelBuffer from the fully assembled pixel bytes.
pub(crate) fn build_pixel_data(
    ihdr: &Ihdr,
    ancillary: &PngAncillary,
    pixels: Vec<u8>,
    w: usize,
    h: usize,
) -> Result<PixelBuffer, PngError> {
    let w32 = w as u32;
    let h32 = h as u32;
    match (ihdr.color_type, ihdr.bit_depth, ancillary.trns.is_some()) {
        // Grayscale
        (0, 16, false) => {
            let gray: &[Gray<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(gray.to_vec(), w, h)).into())
        }
        (0, 16, true) => {
            // Gray16 + tRNS → GrayAlpha16 (already processed to native u16 pairs)
            // GrayAlpha16 now impls Pod; construct via raw bytes
            let ga_bytes: Vec<u8> = pixels;
            PixelBuffer::from_vec(ga_bytes, w as u32, h as u32, GrayAlpha16::DESCRIPTOR)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))
        }
        (0, _, false) if ihdr.bit_depth <= 8 => {
            let gray: Vec<Gray<u8>> = pixels.iter().map(|&g| Gray(g)).collect();
            Ok(PixelBuffer::from_imgvec(ImgVec::new(gray, w, h)).into())
        }
        (0, _, true) if ihdr.bit_depth <= 8 => {
            // Gray + tRNS → RGBA8
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))?)
        }
        // RGB
        (2, 16, false) => {
            let rgb: &[Rgb<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgb.to_vec(), w, h)).into())
        }
        (2, 16, true) => {
            let rgba: &[Rgba<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgba.to_vec(), w, h)).into())
        }
        (2, 8, false) => {
            let rgb: Vec<Rgb<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgb, w32, h32)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))?)
        }
        (2, 8, true) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))?)
        }
        // Indexed
        (3, _, true) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))?)
        }
        (3, _, false) => {
            let rgb: Vec<Rgb<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgb, w32, h32)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))?)
        }
        // GrayAlpha
        (4, 16, _) => {
            // GrayAlpha16 now impls Pod; construct via raw bytes
            // pixels are already native-endian u16 pairs (v, a) = same layout as GrayAlpha16
            PixelBuffer::from_vec(pixels, w as u32, h as u32, GrayAlpha16::DESCRIPTOR)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))
        }
        (4, 8, _) => {
            // GA8 already expanded to RGBA8
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))?)
        }
        // RGBA
        (6, 16, _) => {
            let rgba: &[Rgba<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgba.to_vec(), w, h)).into())
        }
        (6, 8, _) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| PngError::Decode(alloc::format!("{e}")))?)
        }
        _ => Err(PngError::Decode(alloc::format!(
            "unsupported color_type={} bit_depth={}",
            ihdr.color_type,
            ihdr.bit_depth
        ))),
    }
}
