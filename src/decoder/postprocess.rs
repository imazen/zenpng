//! Post-processing: raw unfiltered rows → output pixels.

use alloc::vec::Vec;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use imgref::ImgVec;
use rgb::{Gray, Rgb, Rgba};
use zenpixels::{GrayAlpha16, Pixel, PixelBuffer};

/// Reinterpret `Vec<u8>` as `Vec<T>` without copying when possible.
/// Falls back to per-element construction only if alignment prevents zero-copy.
pub(crate) fn bytes_to_rgba16_vec(bytes: &[u8]) -> Vec<Rgba<u16>> {
    bytes
        .chunks_exact(8)
        .map(|c| Rgba {
            r: u16::from_ne_bytes([c[0], c[1]]),
            g: u16::from_ne_bytes([c[2], c[3]]),
            b: u16::from_ne_bytes([c[4], c[5]]),
            a: u16::from_ne_bytes([c[6], c[7]]),
        })
        .collect()
}

fn try_cast_vec_or<T: bytemuck::AnyBitPattern + bytemuck::NoUninit>(
    pixels: Vec<u8>,
    fallback: fn(&[u8]) -> Vec<T>,
) -> Vec<T> {
    match bytemuck::try_cast_vec(pixels) {
        Ok(v) => v,
        Err((bytemuck::PodCastError::AlignmentMismatch, bytes)) => fallback(&bytes),
        Err((e, _)) => panic!("unexpected cast error: {e:?}"),
    }
}

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
) -> crate::error::Result<PixelBuffer> {
    let w32 = w as u32;
    let h32 = h as u32;
    match (ihdr.color_type, ihdr.bit_depth, ancillary.trns.is_some()) {
        // Grayscale
        (0, 16, false) => {
            let gray = try_cast_vec_or(pixels, |b| {
                b.chunks_exact(2)
                    .map(|c| Gray(u16::from_ne_bytes([c[0], c[1]])))
                    .collect()
            });
            Ok(PixelBuffer::from_imgvec(ImgVec::new(gray, w, h)).into())
        }
        (0, 16, true) => {
            // Gray16 + tRNS → GrayAlpha16 (already processed to native u16 pairs)
            // GrayAlpha16 now impls Pod; construct via raw bytes
            let ga_bytes: Vec<u8> = pixels;
            PixelBuffer::from_vec(ga_bytes, w as u32, h as u32, GrayAlpha16::DESCRIPTOR)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))
        }
        (0, _, false) if ihdr.bit_depth <= 8 => {
            let gray: Vec<Gray<u8>> = pixels.iter().map(|&g| Gray(g)).collect();
            Ok(PixelBuffer::from_imgvec(ImgVec::new(gray, w, h)).into())
        }
        (0, _, true) if ihdr.bit_depth <= 8 => {
            // Gray + tRNS → RGBA8
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // RGB
        (2, 16, false) => {
            let rgb = try_cast_vec_or(pixels, |b| {
                b.chunks_exact(6)
                    .map(|c| Rgb {
                        r: u16::from_ne_bytes([c[0], c[1]]),
                        g: u16::from_ne_bytes([c[2], c[3]]),
                        b: u16::from_ne_bytes([c[4], c[5]]),
                    })
                    .collect()
            });
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgb, w, h)).into())
        }
        (2, 16, true) => {
            let rgba = try_cast_vec_or(pixels, bytes_to_rgba16_vec);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgba, w, h)).into())
        }
        (2, 8, false) => {
            let rgb: Vec<Rgb<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgb, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        (2, 8, true) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // Indexed
        (3, _, true) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        (3, _, false) => {
            let rgb: Vec<Rgb<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgb, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // GrayAlpha
        (4, 16, _) => {
            // GrayAlpha16 now impls Pod; construct via raw bytes
            // pixels are already native-endian u16 pairs (v, a) = same layout as GrayAlpha16
            PixelBuffer::from_vec(pixels, w as u32, h as u32, GrayAlpha16::DESCRIPTOR)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))
        }
        (4, 8, _) => {
            // GA8 already expanded to RGBA8
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        // RGBA
        (6, 16, _) => {
            let rgba = try_cast_vec_or(pixels, bytes_to_rgba16_vec);
            Ok(PixelBuffer::from_imgvec(ImgVec::new(rgba, w, h)).into())
        }
        (6, 8, _) => {
            let rgba: Vec<Rgba<u8>> = bytemuck::cast_vec(pixels);
            Ok(PixelBuffer::from_pixels_erased(rgba, w32, h32)
                .map_err(|e| at!(PngError::Decode(alloc::format!("{e}"))))?)
        }
        _ => Err(at!(PngError::Decode(alloc::format!(
            "unsupported color_type={} bit_depth={}",
            ihdr.color_type,
            ihdr.bit_depth
        )))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ihdr(color_type: u8, bit_depth: u8) -> Ihdr {
        Ihdr {
            width: 4,
            height: 1,
            bit_depth,
            color_type,
            interlace: 0,
        }
    }

    fn empty_anc() -> PngAncillary {
        PngAncillary::default()
    }

    fn anc_with_trns(trns: Vec<u8>) -> PngAncillary {
        PngAncillary {
            trns: Some(trns),
            ..Default::default()
        }
    }

    // ── scale_to_8bit ──

    #[test]
    fn scale_to_8bit_identity() {
        // bit_depth=8 hits the `_ => value` fallback
        assert_eq!(scale_to_8bit(42, 8), 42);
        assert_eq!(scale_to_8bit(0, 8), 0);
        assert_eq!(scale_to_8bit(255, 8), 255);
    }

    #[test]
    fn scale_to_8bit_1bit() {
        assert_eq!(scale_to_8bit(0, 1), 0);
        assert_eq!(scale_to_8bit(1, 1), 255);
    }

    #[test]
    fn scale_to_8bit_2bit() {
        assert_eq!(scale_to_8bit(0, 2), 0);
        assert_eq!(scale_to_8bit(1, 2), 85);
        assert_eq!(scale_to_8bit(2, 2), 170);
        assert_eq!(scale_to_8bit(3, 2), 255);
    }

    #[test]
    fn scale_to_8bit_4bit() {
        assert_eq!(scale_to_8bit(0, 4), 0);
        assert_eq!(scale_to_8bit(15, 4), 255);
    }

    // ── output_bytes_per_pixel ──

    #[test]
    fn output_bpp_gray_variants() {
        let anc = empty_anc();
        // Gray8 no tRNS → 1 byte
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 8), &anc), 1);
        // Gray16 no tRNS → 2 bytes
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 16), &anc), 2);
        // Gray + tRNS → 4 bytes (RGBA8 or GA16)
        let anc_trns = anc_with_trns(vec![0, 0]);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 8), &anc_trns), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(0, 16), &anc_trns), 4);
    }

    #[test]
    fn output_bpp_rgb_variants() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 8), &anc), 3);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 16), &anc), 6);
        let anc_trns = anc_with_trns(vec![0; 6]);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 8), &anc_trns), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(2, 16), &anc_trns), 8);
    }

    #[test]
    fn output_bpp_indexed() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(3, 8), &anc), 3);
        let anc_trns = anc_with_trns(vec![255]);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(3, 8), &anc_trns), 4);
    }

    #[test]
    fn output_bpp_gray_alpha() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(4, 8), &anc), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(4, 16), &anc), 4);
    }

    #[test]
    fn output_bpp_rgba() {
        let anc = empty_anc();
        assert_eq!(output_bytes_per_pixel(&make_ihdr(6, 8), &anc), 4);
        assert_eq!(output_bytes_per_pixel(&make_ihdr(6, 16), &anc), 8);
    }

    // ── post_process_row: Gray8 + tRNS ──

    #[test]
    fn gray8_trns_expansion() {
        let ihdr = make_ihdr(0, 8);
        // tRNS gray value = 100 (big-endian u16: [0, 100])
        let anc = anc_with_trns(vec![0, 100]);
        // Raw row: 4 gray pixels, second one matches tRNS
        let raw = vec![50, 100, 200, 100];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        // Each pixel → RGBA8: [g, g, g, alpha]
        assert_eq!(out.len(), 16);
        // pixel 0: g=50, alpha=255
        assert_eq!(&out[0..4], &[50, 50, 50, 255]);
        // pixel 1: g=100 matches tRNS → alpha=0
        assert_eq!(&out[4..8], &[100, 100, 100, 0]);
        // pixel 2: g=200, alpha=255
        assert_eq!(&out[8..12], &[200, 200, 200, 255]);
        // pixel 3: g=100 matches tRNS → alpha=0
        assert_eq!(&out[12..16], &[100, 100, 100, 0]);
    }

    #[test]
    fn gray8_trns_short_data() {
        // tRNS data shorter than 2 bytes → trns_val defaults to 0
        let ihdr = make_ihdr(0, 8);
        let anc = anc_with_trns(vec![]);
        let raw = vec![0, 1, 0, 255];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        // pixel 0: g=0 matches tRNS(0) → alpha=0
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        // pixel 1: g=1, alpha=255
        assert_eq!(&out[4..8], &[1, 1, 1, 255]);
    }

    // ── post_process_row: sub-byte Gray + tRNS (short data fallback) ──

    #[test]
    fn gray_subbyte_trns_short_data() {
        // 1-bit gray, width=4, tRNS with short data → trns_val=0
        let ihdr = Ihdr {
            width: 8,
            height: 1,
            bit_depth: 1,
            color_type: 0,
            interlace: 0,
        };
        // Short tRNS → defaults to 0 → val 0 is transparent
        let anc = anc_with_trns(vec![]);
        // 8 pixels packed in 1 byte: 0b10101010 = pixels [1,0,1,0,1,0,1,0]
        let raw = vec![0b10101010];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        assert_eq!(out.len(), 32); // 8 pixels × 4 bytes
        // pixel 0: val=1 → g=255, alpha=255
        assert_eq!(&out[0..4], &[255, 255, 255, 255]);
        // pixel 1: val=0 → g=0, alpha=0 (transparent)
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
    }

    // ── post_process_row: Gray16 + tRNS (short data fallback) ──

    #[test]
    fn gray16_trns_short_data() {
        let ihdr = make_ihdr(0, 16);
        // Short tRNS → trns_val=0
        let anc = anc_with_trns(vec![5]); // only 1 byte, needs 2
        // 4 pixels: big-endian gray16
        let raw: Vec<u8> = [0u16, 100, 200, 0]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        // Each pixel → GrayAlpha16 (4 bytes: val_ne + alpha_ne)
        assert_eq!(out.len(), 16);
        // pixel 0: val=0 matches tRNS → alpha=0
        let v0 = u16::from_ne_bytes([out[0], out[1]]);
        let a0 = u16::from_ne_bytes([out[2], out[3]]);
        assert_eq!(v0, 0);
        assert_eq!(a0, 0);
        // pixel 1: val=100, alpha=65535
        let a1 = u16::from_ne_bytes([out[6], out[7]]);
        assert_eq!(a1, 65535);
    }

    // ── post_process_row: RGB16 + tRNS (short data fallback) ──

    #[test]
    fn rgb16_trns_short_data() {
        // Short tRNS (less than 6 bytes) → defaults to (0,0,0)
        let anc = anc_with_trns(vec![0, 1]);
        // 1 pixel RGB16 = [0, 0, 0] (matches default tRNS)
        let raw: Vec<u8> = [0u16, 0, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut ihdr1 = make_ihdr(2, 16);
        ihdr1.width = 1;
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr1, &anc, &mut out);
        // 1 pixel → RGBA16 = 8 bytes
        assert_eq!(out.len(), 8);
        // alpha should be 0 (transparent)
        let a = u16::from_ne_bytes([out[6], out[7]]);
        assert_eq!(a, 0);
    }

    // ── post_process_row: RGB8 + tRNS (short data fallback) ──

    #[test]
    fn rgb8_trns_short_data() {
        let mut ihdr = make_ihdr(2, 8);
        ihdr.width = 2;
        // Short tRNS → defaults to (0,0,0)
        let anc = anc_with_trns(vec![0, 1, 0]);
        // 2 pixels: [0,0,0] and [1,2,3]
        let raw = vec![0, 0, 0, 1, 2, 3];
        let mut out = Vec::new();
        post_process_row(&raw, &ihdr, &anc, &mut out);
        assert_eq!(out.len(), 8); // 2 × RGBA8
        // pixel 0: matches (0,0,0) → alpha=0
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        // pixel 1: no match → alpha=255
        assert_eq!(&out[4..8], &[1, 2, 3, 255]);
    }

    // ── OutputFormat ──

    #[test]
    fn output_format_gray_trns_16() {
        let ihdr = make_ihdr(0, 16);
        let anc = anc_with_trns(vec![0, 0]);
        let fmt = OutputFormat::from_ihdr(&ihdr, &anc);
        assert_eq!(fmt.channels, 2);
        assert_eq!(fmt.bytes_per_channel, 2);
    }

    #[test]
    fn output_format_gray_trns_8() {
        let ihdr = make_ihdr(0, 8);
        let anc = anc_with_trns(vec![0, 0]);
        let fmt = OutputFormat::from_ihdr(&ihdr, &anc);
        assert_eq!(fmt.channels, 4);
        assert_eq!(fmt.bytes_per_channel, 1);
    }

    #[test]
    fn output_format_all_types() {
        let anc = empty_anc();
        // Gray8
        let f = OutputFormat::from_ihdr(&make_ihdr(0, 8), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (1, 1));
        // Gray16
        let f = OutputFormat::from_ihdr(&make_ihdr(0, 16), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (1, 2));
        // RGB8
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 8), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (3, 1));
        // RGB16
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 16), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (3, 2));
        // RGB8 + tRNS
        let at = anc_with_trns(vec![0; 6]);
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 8), &at);
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // RGB16 + tRNS
        let f = OutputFormat::from_ihdr(&make_ihdr(2, 16), &at);
        assert_eq!((f.channels, f.bytes_per_channel), (4, 2));
        // Indexed
        let f = OutputFormat::from_ihdr(&make_ihdr(3, 8), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (3, 1));
        let f = OutputFormat::from_ihdr(&make_ihdr(3, 8), &at);
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // GrayAlpha8
        let f = OutputFormat::from_ihdr(&make_ihdr(4, 8), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // GrayAlpha16
        let f = OutputFormat::from_ihdr(&make_ihdr(4, 16), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (2, 2));
        // RGBA8
        let f = OutputFormat::from_ihdr(&make_ihdr(6, 8), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (4, 1));
        // RGBA16
        let f = OutputFormat::from_ihdr(&make_ihdr(6, 16), &anc);
        assert_eq!((f.channels, f.bytes_per_channel), (4, 2));
    }

    // ── build_pixel_data ──

    #[test]
    fn build_pixel_data_gray8() {
        let ihdr = make_ihdr(0, 8);
        let anc = empty_anc();
        let pixels = vec![10, 20, 30, 40]; // 4 pixels
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray8_trns() {
        let ihdr = make_ihdr(0, 8);
        let anc = anc_with_trns(vec![0, 10]);
        // 4 RGBA8 pixels
        let pixels = vec![
            10, 10, 10, 0, 20, 20, 20, 255, 10, 10, 10, 0, 30, 30, 30, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray16() {
        let ihdr = make_ihdr(0, 16);
        let anc = empty_anc();
        // 4 Gray16 pixels in native endian
        let pixels: Vec<u8> = [100u16, 200, 300, 400]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray16_trns() {
        let anc = anc_with_trns(vec![0, 0]);
        // 2 GrayAlpha16 pixels (4 bytes each, native endian u16 pairs)
        let mut ihdr2 = make_ihdr(0, 16);
        ihdr2.width = 2;
        let pixels: Vec<u8> = [100u16, 65535, 0, 0]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr2, &anc, pixels, 2, 1).unwrap();
        assert_eq!(result.width(), 2);
    }

    #[test]
    fn build_pixel_data_rgb8() {
        let ihdr = make_ihdr(2, 8);
        let anc = empty_anc();
        let pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_rgb8_trns() {
        let ihdr = make_ihdr(2, 8);
        let anc = anc_with_trns(vec![0; 6]);
        let pixels = vec![
            0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 255, 255, 128, 128, 128, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_rgb16() {
        let anc = empty_anc();
        let mut ihdr1 = make_ihdr(2, 16);
        ihdr1.width = 1;
        let pixels: Vec<u8> = [100u16, 200, 300]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr1, &anc, pixels, 1, 1).unwrap();
        assert_eq!(result.width(), 1);
    }

    #[test]
    fn build_pixel_data_rgb16_trns() {
        let anc = anc_with_trns(vec![0; 6]);
        let mut ihdr1 = make_ihdr(2, 16);
        ihdr1.width = 1;
        let pixels: Vec<u8> = [100u16, 200, 300, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr1, &anc, pixels, 1, 1).unwrap();
        assert_eq!(result.width(), 1);
    }

    #[test]
    fn build_pixel_data_indexed_with_trns() {
        let ihdr = make_ihdr(3, 8);
        let anc = anc_with_trns(vec![255, 0]);
        let pixels = vec![
            255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 128, 128, 128, 128, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_indexed_no_trns() {
        let ihdr = make_ihdr(3, 8);
        let anc = empty_anc();
        let pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray_alpha_8() {
        let ihdr = make_ihdr(4, 8);
        let anc = empty_anc();
        // GA8 expanded to RGBA8: 4 pixels × 4 bytes
        let pixels = vec![
            100, 100, 100, 255, 200, 200, 200, 128, 50, 50, 50, 0, 0, 0, 0, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_gray_alpha_16() {
        let mut ihdr = make_ihdr(4, 16);
        ihdr.width = 2;
        let anc = empty_anc();
        let pixels: Vec<u8> = [100u16, 65535, 200, 0]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr, &anc, pixels, 2, 1).unwrap();
        assert_eq!(result.width(), 2);
    }

    #[test]
    fn build_pixel_data_rgba8() {
        let ihdr = make_ihdr(6, 8);
        let anc = empty_anc();
        let pixels = vec![
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 128, 128, 128, 255,
        ];
        let result = build_pixel_data(&ihdr, &anc, pixels, 4, 1).unwrap();
        assert_eq!(result.width(), 4);
    }

    #[test]
    fn build_pixel_data_rgba16() {
        let mut ihdr = make_ihdr(6, 16);
        ihdr.width = 1;
        let anc = empty_anc();
        let pixels: Vec<u8> = [100u16, 200, 300, 65535]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let result = build_pixel_data(&ihdr, &anc, pixels, 1, 1).unwrap();
        assert_eq!(result.width(), 1);
    }

    // ── unpack functions ──

    #[test]
    fn unpack_sub_byte_gray_1bit() {
        // 0b11001100 = pixels [1,1,0,0,1,1,0,0]
        let raw = vec![0b11001100];
        let mut out = Vec::new();
        unpack_sub_byte_gray(&raw, 8, 1, &mut out);
        assert_eq!(out, vec![255, 255, 0, 0, 255, 255, 0, 0]);
    }

    #[test]
    fn unpack_sub_byte_indexed_4bit() {
        // 0xA3 = high nibble 10, low nibble 3
        let raw = vec![0xA3];
        let mut out = Vec::new();
        unpack_sub_byte_indexed(&raw, 2, 4, &mut out);
        assert_eq!(out, vec![10, 3]);
    }
}
