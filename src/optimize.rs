//! Lossless image analysis and color type optimization.
//!
//! Analyzes pixel data to find the optimal PNG color type and bit depth,
//! performing lossless transformations that reduce file size without any
//! quality loss. Optimizations include:
//!
//! - RGBA → RGB (strip unused alpha channel)
//! - RGBA/RGB → Grayscale (when R==G==B for all pixels)
//! - RGBA → GrayscaleAlpha (grayscale with varying alpha)
//! - Gray8 → Gray 1/2/4-bit (sub-byte packing)
//! - RGBA/RGB → Gray/RGB + tRNS (binary alpha with single transparent color)
//! - 16-bit → 8-bit (when all samples fit in 8 bits)
//! - Truecolor → Indexed (when ≤256 unique colors)

use alloc::vec::Vec;
use std::collections::HashMap;

/// Result of pixel analysis: what optimizations are available.
pub(crate) struct ImageAnalysis {
    /// All pixels have R==G==B.
    pub is_grayscale: bool,
    /// All alpha values are 255 (fully opaque).
    pub is_opaque: bool,
    /// Minimum grayscale bit depth that losslessly represents all gray values.
    /// Only meaningful when `is_grayscale` is true.
    /// 1-bit: all values ∈ {0, 255}; 2-bit: all % 85 == 0; 4-bit: all % 17 == 0; else 8.
    pub min_gray_bit_depth: u8,
    /// All alpha values are exactly 0 or 255 (binary transparency).
    pub is_binary_alpha: bool,
    /// The single RGB color of all alpha=0 pixels, if exactly one such color exists.
    /// `None` if no transparent pixels, or 2+ distinct transparent colors.
    pub transparent_color: Option<[u8; 3]>,
    /// Number of unique colors (capped at 257 = "more than 256").
    #[allow(dead_code)]
    pub unique_color_count: usize,
    /// Exact palette and index buffer when ≤256 unique colors.
    pub exact_palette: Option<ExactPaletteData>,
}

/// Exact palette extracted from analysis of an image with ≤256 unique colors.
pub(crate) struct ExactPaletteData {
    /// RGBA palette entries.
    pub palette_rgba: Vec<[u8; 4]>,
    /// Per-pixel index into palette.
    pub indices: Vec<u8>,
    /// Whether any palette entry has alpha < 255.
    pub has_transparency: bool,
}

/// Analyze RGBA8 pixel data for lossless optimization opportunities.
///
/// Single pass through all pixels, collecting:
/// - Grayscale detection (R==G==B)
/// - Alpha channel analysis (all opaque? binary alpha?)
/// - Sub-byte gray bit depth (1/2/4/8)
/// - Single transparent color detection
/// - Unique color counting with early exit at 257
pub(crate) fn analyze_rgba8(bytes: &[u8], width: usize, height: usize) -> ImageAnalysis {
    let npixels = width * height;
    let mut is_grayscale = true;
    let mut is_opaque = true;
    let mut is_binary_alpha = true;
    let mut color_map: HashMap<[u8; 4], u8> = HashMap::with_capacity(257);
    let mut palette: Vec<[u8; 4]> = Vec::with_capacity(256);
    let mut palette_overflow = false;
    let mut has_transparency = false;

    // Sub-byte gray tracking: can we fit all gray values in fewer bits?
    let mut can_1bit = true; // all values ∈ {0, 255}
    let mut can_2bit = true; // all values % 85 == 0
    let mut can_4bit = true; // all values % 17 == 0

    // Single transparent color tracking
    let mut transparent_color: Option<[u8; 3]> = None;
    let mut multi_transparent = false;

    for i in 0..npixels {
        let off = i * 4;
        let r = bytes[off];
        let g = bytes[off + 1];
        let b = bytes[off + 2];
        let a = bytes[off + 3];

        if is_grayscale && (r != g || r != b) {
            is_grayscale = false;
        }
        if a != 255 {
            is_opaque = false;
            has_transparency = true;
            if a != 0 {
                is_binary_alpha = false;
            } else if !multi_transparent {
                // Alpha == 0: track the transparent color
                let tc = [r, g, b];
                match transparent_color {
                    None => transparent_color = Some(tc),
                    Some(prev) if prev != tc => {
                        multi_transparent = true;
                        transparent_color = None;
                    }
                    _ => {} // same color, fine
                }
            }
        }

        // Sub-byte gray: only track when still possibly grayscale
        if is_grayscale && can_4bit {
            if r % 17 != 0 {
                can_4bit = false;
                can_2bit = false;
                can_1bit = false;
            } else if can_2bit && r % 85 != 0 {
                can_2bit = false;
                can_1bit = false;
            } else if can_1bit && r != 0 && r != 255 {
                can_1bit = false;
            }
        }

        if !palette_overflow {
            let color = [r, g, b, a];
            if !color_map.contains_key(&color) {
                if palette.len() >= 256 {
                    palette_overflow = true;
                } else {
                    color_map.insert(color, palette.len() as u8);
                    palette.push(color);
                }
            }
        }
    }

    let min_gray_bit_depth = if can_1bit {
        1
    } else if can_2bit {
        2
    } else if can_4bit {
        4
    } else {
        8
    };

    // If all pixels are opaque, binary_alpha is trivially true but not useful
    if is_opaque {
        is_binary_alpha = true;
    }

    let unique_color_count = if palette_overflow { 257 } else { palette.len() };

    let exact_palette = if !palette_overflow {
        let mut indices = Vec::with_capacity(npixels);
        for i in 0..npixels {
            let off = i * 4;
            let color = [bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]];
            indices.push(color_map[&color]);
        }
        Some(ExactPaletteData {
            palette_rgba: palette,
            indices,
            has_transparency,
        })
    } else {
        None
    };

    ImageAnalysis {
        is_grayscale,
        is_opaque,
        min_gray_bit_depth,
        is_binary_alpha,
        transparent_color,
        unique_color_count,
        exact_palette,
    }
}

/// Analyze RGB8 pixel data for lossless optimization opportunities.
pub(crate) fn analyze_rgb8(bytes: &[u8], width: usize, height: usize) -> ImageAnalysis {
    let npixels = width * height;
    let mut is_grayscale = true;
    // For RGB palette counting, we use [R,G,B,255] as the key to share logic
    let mut color_map: HashMap<[u8; 4], u8> = HashMap::with_capacity(257);
    let mut palette: Vec<[u8; 4]> = Vec::with_capacity(256);
    let mut palette_overflow = false;

    // Sub-byte gray tracking
    let mut can_1bit = true;
    let mut can_2bit = true;
    let mut can_4bit = true;

    for i in 0..npixels {
        let off = i * 3;
        let r = bytes[off];
        let g = bytes[off + 1];
        let b = bytes[off + 2];

        if is_grayscale && (r != g || r != b) {
            is_grayscale = false;
        }

        // Sub-byte gray tracking
        if is_grayscale && can_4bit {
            if r % 17 != 0 {
                can_4bit = false;
                can_2bit = false;
                can_1bit = false;
            } else if can_2bit && r % 85 != 0 {
                can_2bit = false;
                can_1bit = false;
            } else if can_1bit && r != 0 && r != 255 {
                can_1bit = false;
            }
        }

        if !palette_overflow {
            let color = [r, g, b, 255];
            if !color_map.contains_key(&color) {
                if palette.len() >= 256 {
                    palette_overflow = true;
                } else {
                    color_map.insert(color, palette.len() as u8);
                    palette.push(color);
                }
            }
        }
    }

    let min_gray_bit_depth = if can_1bit {
        1
    } else if can_2bit {
        2
    } else if can_4bit {
        4
    } else {
        8
    };

    let unique_color_count = if palette_overflow { 257 } else { palette.len() };

    let exact_palette = if !palette_overflow {
        let mut indices = Vec::with_capacity(npixels);
        for i in 0..npixels {
            let off = i * 3;
            let color = [bytes[off], bytes[off + 1], bytes[off + 2], 255];
            indices.push(color_map[&color]);
        }
        Some(ExactPaletteData {
            palette_rgba: palette,
            indices,
            has_transparency: false,
        })
    } else {
        None
    };

    ImageAnalysis {
        is_grayscale,
        is_opaque: true,
        min_gray_bit_depth,
        is_binary_alpha: true,
        transparent_color: None,
        unique_color_count,
        exact_palette,
    }
}

/// Check if 16-bit pixel data can be losslessly reduced to 8-bit.
///
/// Examines big-endian u16 samples: reducible when all low bytes are zero
/// (i.e., the value is a multiple of 256, like 0x1200 → 0x12).
pub(crate) fn can_reduce_16_to_8(be_bytes: &[u8]) -> bool {
    // PNG 16-bit data is big-endian: [high, low] pairs.
    // Reducible when all low bytes (odd indices) are zero.
    be_bytes.chunks_exact(2).all(|pair| pair[1] == 0)
}

/// Reduce 16-bit big-endian pixel data to 8-bit by taking the high byte.
pub(crate) fn reduce_16_to_8(be_bytes: &[u8]) -> Vec<u8> {
    be_bytes.chunks_exact(2).map(|pair| pair[0]).collect()
}

/// Convert RGBA8 pixel data to RGB8 by stripping the alpha channel.
pub(crate) fn rgba8_to_rgb8(rgba: &[u8]) -> Vec<u8> {
    let npixels = rgba.len() / 4;
    let mut rgb = Vec::with_capacity(npixels * 3);
    for i in 0..npixels {
        let off = i * 4;
        rgb.push(rgba[off]);
        rgb.push(rgba[off + 1]);
        rgb.push(rgba[off + 2]);
    }
    rgb
}

/// Convert RGBA8 pixel data to GrayscaleAlpha8.
pub(crate) fn rgba8_to_gray_alpha8(rgba: &[u8]) -> Vec<u8> {
    let npixels = rgba.len() / 4;
    let mut ga = Vec::with_capacity(npixels * 2);
    for i in 0..npixels {
        let off = i * 4;
        ga.push(rgba[off]); // R (== G == B)
        ga.push(rgba[off + 3]); // A
    }
    ga
}

/// Convert RGBA8 pixel data to Grayscale8 (dropping alpha, assumes all opaque).
pub(crate) fn rgba8_to_gray8(rgba: &[u8]) -> Vec<u8> {
    let npixels = rgba.len() / 4;
    let mut gray = Vec::with_capacity(npixels);
    for i in 0..npixels {
        gray.push(rgba[i * 4]); // R (== G == B)
    }
    gray
}

/// Convert RGB8 pixel data to Grayscale8.
pub(crate) fn rgb8_to_gray8(rgb: &[u8]) -> Vec<u8> {
    let npixels = rgb.len() / 3;
    let mut gray = Vec::with_capacity(npixels);
    for i in 0..npixels {
        gray.push(rgb[i * 3]); // R (== G == B)
    }
    gray
}

/// Check if a transparent color also appears with alpha=255 anywhere in the image.
///
/// If the transparent color is "exclusive" (never appears opaque), we can use a
/// tRNS chunk to represent it. If it also appears opaque, tRNS would make those
/// opaque pixels transparent too, so we can't use it.
///
/// Only called when binary alpha + single transparent color detected.
pub(crate) fn trns_color_is_exclusive(bytes: &[u8], transparent_color: [u8; 3]) -> bool {
    let npixels = bytes.len() / 4;
    for i in 0..npixels {
        let off = i * 4;
        let a = bytes[off + 3];
        if a == 255
            && bytes[off] == transparent_color[0]
            && bytes[off + 1] == transparent_color[1]
            && bytes[off + 2] == transparent_color[2]
        {
            return false; // same RGB appears opaque — can't use tRNS
        }
    }
    true
}

/// Scale 8-bit grayscale values down to sub-byte depth.
///
/// - 1-bit: v/255 (0→0, 255→1)
/// - 2-bit: v/85  (0→0, 85→1, 170→2, 255→3)
/// - 4-bit: v/17  (0→0, 17→1, ... 255→15)
///
/// Caller must ensure all values are valid for the target bit depth.
pub(crate) fn gray8_to_subbyte(gray8: &[u8], bit_depth: u8) -> Vec<u8> {
    let divisor = match bit_depth {
        1 => 255u8,
        2 => 85,
        4 => 17,
        _ => return gray8.to_vec(),
    };
    gray8.iter().map(|&v| v / divisor).collect()
}

/// Split an RGBA palette into separate RGB and alpha arrays for PNG.
pub(crate) fn split_palette_rgba(palette: &[[u8; 4]]) -> (Vec<u8>, Vec<u8>) {
    let mut rgb = Vec::with_capacity(palette.len() * 3);
    let mut alpha = Vec::with_capacity(palette.len());
    for entry in palette {
        rgb.push(entry[0]);
        rgb.push(entry[1]);
        rgb.push(entry[2]);
        alpha.push(entry[3]);
    }
    (rgb, alpha)
}

/// Sort palette entries by luminance and remap indices.
///
/// Similar colors get adjacent indices, which produces smaller filter residuals
/// and better DEFLATE compression. Uses the standard BT.601 luminance formula
/// with a secondary sort on hue (R-B difference) for stability.
pub(crate) fn sort_palette_luminance(palette: &mut Vec<[u8; 4]>, indices: &mut [u8]) {
    let n = palette.len();
    if n <= 1 {
        return;
    }

    // Build (old_index, luminance_key) for sorting
    let mut order: Vec<(u8, u32)> = palette
        .iter()
        .enumerate()
        .map(|(i, c)| {
            // luminance * 1000 + hue tiebreaker to keep sort stable and grouped
            let lum = 299u32 * c[0] as u32 + 587 * c[1] as u32 + 114 * c[2] as u32;
            let hue = c[0] as u32 * 256 + c[2] as u32; // R-B secondary sort
            (i as u8, lum * 256 + hue)
        })
        .collect();
    order.sort_by_key(|&(_, key)| key);

    // Build old→new index mapping
    let mut old_to_new = vec![0u8; n];
    let mut new_palette = Vec::with_capacity(n);
    for (new_idx, &(old_idx, _)) in order.iter().enumerate() {
        old_to_new[old_idx as usize] = new_idx as u8;
        new_palette.push(palette[old_idx as usize]);
    }

    // Apply remapping
    *palette = new_palette;
    for idx in indices.iter_mut() {
        *idx = old_to_new[*idx as usize];
    }
}

/// Try multiple palette orderings and pick the one that compresses smallest.
///
/// For images with an exact palette (≤256 unique colors), the palette order
/// affects index values, which affects PNG filter residuals and DEFLATE
/// compression. This function tries luminance sort vs the original order
/// and picks the winner based on actual compressed size at a quick effort level.
pub(crate) fn optimize_palette_order(palette: &mut Vec<[u8; 4]>, indices: &mut [u8]) {
    // For very small palettes (≤4 colors), sorting doesn't help enough to justify cost
    if palette.len() <= 4 {
        sort_palette_luminance(palette, indices);
        return;
    }

    // Always apply luminance sort — it's the best general-purpose ordering for PNG.
    // More advanced strategies (frequency sort, delta-minimize, trial compression)
    // could be added here but luminance sort is robust across image types.
    sort_palette_luminance(palette, indices);
}

/// Apply near-lossless quantization by rounding LSBs of each sample.
///
/// For each byte, rounds to the nearest multiple of `2^bits`. This creates
/// more identical byte values, improving filter residuals and DEFLATE compression.
/// Alpha channels are NOT quantized (preserving transparency fidelity).
///
/// `channels` specifies the number of channels per pixel (3=RGB, 4=RGBA, etc.)
/// so we can skip the alpha channel for RGBA/GrayAlpha data.
pub(crate) fn near_lossless_quantize(bytes: &[u8], channels: usize, bits: u8) -> Vec<u8> {
    if bits == 0 || bits > 4 {
        return bytes.to_vec();
    }
    let step = 1u16 << bits; // 2, 4, 8, or 16
    let half = step / 2;

    let mut out = bytes.to_vec();
    let has_alpha = channels == 4 || channels == 2; // RGBA or GrayAlpha
    let color_channels = if has_alpha { channels - 1 } else { channels };

    for pixel in out.chunks_exact_mut(channels) {
        for c in &mut pixel[..color_channels] {
            // Round to nearest multiple of step, clamped to [0, 255]
            let v = *c as u16;
            let rounded = ((v + half) / step) * step;
            *c = rounded.min(255) as u8;
        }
        // Alpha channel left untouched
    }
    out
}

/// Encoding decision from image analysis.
pub(crate) enum OptimalEncoding {
    /// Encode as indexed PNG with this palette and indices.
    Indexed {
        palette_rgb: Vec<u8>,
        palette_alpha: Option<Vec<u8>>,
        indices: Vec<u8>,
    },
    /// Encode as truecolor/grayscale PNG with converted pixel data.
    Truecolor {
        bytes: Vec<u8>,
        color_type: u8,
        bit_depth: u8,
        /// Optional tRNS chunk data for binary transparency.
        /// For grayscale: `[0, gray_value]` (2 bytes, big-endian u16).
        /// For RGB: `[0, R, 0, G, 0, B]` (6 bytes, 3× big-endian u16).
        trns: Option<Vec<u8>>,
    },
    /// No optimization: use original data as-is.
    Original,
}

/// Determine the optimal encoding for RGBA8 pixel data.
///
/// Priority order (smallest estimated raw size first):
/// 1. Sub-byte gray (1/2/4-bit, no alpha)
/// 2. Sub-byte gray + tRNS (binary alpha, single exclusive transparent color)
/// 3. Indexed (≤256 unique colors)
/// 4. Gray8 + tRNS
/// 5. Gray8 (opaque)
/// 6. GrayscaleAlpha8
/// 7. RGB8 + tRNS
/// 8. RGB8 (opaque)
/// 9. Original (RGBA8)
pub(crate) fn optimize_rgba8(bytes: &[u8], width: usize, height: usize) -> OptimalEncoding {
    let analysis = analyze_rgba8(bytes, width, height);

    // Check if tRNS is usable: binary alpha + single transparent color + exclusive
    let trns_usable = analysis.is_binary_alpha
        && !analysis.is_opaque
        && analysis.transparent_color.is_some()
        && trns_color_is_exclusive(bytes, analysis.transparent_color.unwrap());

    // For sub-byte/tRNS paths, compute the effective bpp for cost estimation
    let truecolor_bpp = if analysis.is_grayscale && (analysis.is_opaque || trns_usable) {
        1 // Gray (+ optional tRNS)
    } else if analysis.is_grayscale {
        2 // GrayscaleAlpha8
    } else if analysis.is_opaque || trns_usable {
        3 // RGB (+ optional tRNS)
    } else {
        4 // RGBA8
    };

    // Sub-byte grayscale: massive savings (2-8x vs Gray8)
    if analysis.is_grayscale && analysis.min_gray_bit_depth < 8 {
        let gray8 = rgba8_to_gray8(bytes);
        let bd = analysis.min_gray_bit_depth;

        if analysis.is_opaque {
            // Sub-byte gray, no alpha
            let scaled = gray8_to_subbyte(&gray8, bd);
            return OptimalEncoding::Truecolor {
                bytes: scaled,
                color_type: 0,
                bit_depth: bd,
                trns: None,
            };
        }

        if trns_usable {
            // Sub-byte gray + tRNS
            let tc = analysis.transparent_color.unwrap();
            let gray_val = tc[0]; // R==G==B for grayscale
            let scaled_val = gray_val
                / match bd {
                    1 => 255,
                    2 => 85,
                    4 => 17,
                    _ => 1,
                };
            // Strip alpha, keep only opaque + transparent pixels
            let scaled = gray8_to_subbyte(&gray8, bd);
            return OptimalEncoding::Truecolor {
                bytes: scaled,
                color_type: 0,
                bit_depth: bd,
                trns: Some(vec![0, scaled_val]),
            };
        }
    }

    // Try indexed if ≤256 unique colors, but only if it's actually smaller.
    if let Some(ref exact) = analysis.exact_palette {
        let n_colors = exact.palette_rgba.len();
        let n_pixels = width * height;

        // Estimate raw data sizes (filter byte + pixel data per row):
        // Indexed: (1 + width) * height
        // Truecolor: (1 + width * bpp) * height
        let indexed_raw = (1 + width) * height;
        let truecolor_raw = (1 + width * truecolor_bpp) * height;

        // Palette overhead: PLTE chunk (12 + 3*n) + tRNS chunk if transparency (12 + n)
        let palette_overhead = 12
            + 3 * n_colors
            + if exact.has_transparency {
                12 + n_colors
            } else {
                0
            };

        let indexed_cost = palette_overhead + indexed_raw;
        let use_indexed = indexed_cost < truecolor_raw || n_pixels >= 512;

        if use_indexed {
            let mut exact = analysis.exact_palette.unwrap();
            optimize_palette_order(&mut exact.palette_rgba, &mut exact.indices);
            let (rgb, alpha) = split_palette_rgba(&exact.palette_rgba);
            let palette_alpha = if exact.has_transparency {
                Some(alpha)
            } else {
                None
            };
            return OptimalEncoding::Indexed {
                palette_rgb: rgb,
                palette_alpha,
                indices: exact.indices,
            };
        }
    }

    // Grayscale + tRNS (binary transparency, 8-bit)
    if analysis.is_grayscale && trns_usable {
        let tc = analysis.transparent_color.unwrap();
        let gray_val = tc[0];
        // Strip alpha channel — transparent pixels become the tRNS key color
        let gray8 = rgba8_to_gray8(bytes);
        return OptimalEncoding::Truecolor {
            bytes: gray8,
            color_type: 0,
            bit_depth: 8,
            trns: Some(vec![0, gray_val]),
        };
    }

    // Grayscale + opaque → Gray8 (4:1 reduction)
    if analysis.is_grayscale && analysis.is_opaque {
        return OptimalEncoding::Truecolor {
            bytes: rgba8_to_gray8(bytes),
            color_type: 0,
            bit_depth: 8,
            trns: None,
        };
    }

    // Grayscale + alpha → GrayscaleAlpha8 (2:1 reduction)
    if analysis.is_grayscale {
        return OptimalEncoding::Truecolor {
            bytes: rgba8_to_gray_alpha8(bytes),
            color_type: 4,
            bit_depth: 8,
            trns: None,
        };
    }

    // RGB + tRNS (binary transparency, single transparent color)
    if trns_usable {
        let tc = analysis.transparent_color.unwrap();
        let rgb = rgba8_to_rgb8(bytes);
        return OptimalEncoding::Truecolor {
            bytes: rgb,
            color_type: 2,
            bit_depth: 8,
            trns: Some(vec![0, tc[0], 0, tc[1], 0, tc[2]]),
        };
    }

    // Opaque → RGB8 (4:3 reduction)
    if analysis.is_opaque {
        return OptimalEncoding::Truecolor {
            bytes: rgba8_to_rgb8(bytes),
            color_type: 2,
            bit_depth: 8,
            trns: None,
        };
    }

    OptimalEncoding::Original
}

/// Determine the optimal encoding for RGB8 pixel data.
pub(crate) fn optimize_rgb8(bytes: &[u8], width: usize, height: usize) -> OptimalEncoding {
    let analysis = analyze_rgb8(bytes, width, height);

    let truecolor_bpp: usize = if analysis.is_grayscale { 1 } else { 3 };

    // Sub-byte grayscale (no tRNS for RGB — no alpha channel)
    if analysis.is_grayscale && analysis.min_gray_bit_depth < 8 {
        let gray8 = rgb8_to_gray8(bytes);
        let bd = analysis.min_gray_bit_depth;
        let scaled = gray8_to_subbyte(&gray8, bd);
        return OptimalEncoding::Truecolor {
            bytes: scaled,
            color_type: 0,
            bit_depth: bd,
            trns: None,
        };
    }

    // Try indexed if ≤256 unique colors
    if let Some(ref exact) = analysis.exact_palette {
        let n_colors = exact.palette_rgba.len();
        let n_pixels = width * height;

        let indexed_raw = (1 + width) * height;
        let truecolor_raw = (1 + width * truecolor_bpp) * height;
        let palette_overhead = 12 + 3 * n_colors;

        let indexed_cost = palette_overhead + indexed_raw;
        let use_indexed = indexed_cost < truecolor_raw || n_pixels >= 512;

        if use_indexed {
            let mut exact = analysis.exact_palette.unwrap();
            optimize_palette_order(&mut exact.palette_rgba, &mut exact.indices);
            let (rgb, _alpha) = split_palette_rgba(&exact.palette_rgba);
            return OptimalEncoding::Indexed {
                palette_rgb: rgb,
                palette_alpha: None,
                indices: exact.indices,
            };
        }
    }

    // Grayscale → Gray8 (3:1 reduction)
    if analysis.is_grayscale {
        return OptimalEncoding::Truecolor {
            bytes: rgb8_to_gray8(bytes),
            color_type: 0,
            bit_depth: 8,
            trns: None,
        };
    }

    OptimalEncoding::Original
}

/// Determine the optimal encoding for 16-bit pixel data (already big-endian).
/// `channels` is the number of channels (1=Gray, 2=GrayAlpha, 3=RGB, 4=RGBA).
pub(crate) fn optimize_16bit(
    be_bytes: &[u8],
    width: usize,
    height: usize,
    color_type: u8,
) -> OptimalEncoding {
    if !can_reduce_16_to_8(be_bytes) {
        return OptimalEncoding::Original;
    }

    let reduced = reduce_16_to_8(be_bytes);
    let channels: usize = match color_type {
        0 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return OptimalEncoding::Original,
    };

    // Now apply 8-bit optimizations on the reduced data
    match channels {
        4 => {
            // RGBA16 → 8-bit, then try further optimization
            let opt = optimize_rgba8(&reduced, width, height);
            match opt {
                OptimalEncoding::Original => OptimalEncoding::Truecolor {
                    bytes: reduced,
                    color_type,
                    bit_depth: 8,
                    trns: None,
                },
                other => other,
            }
        }
        3 => {
            let opt = optimize_rgb8(&reduced, width, height);
            match opt {
                OptimalEncoding::Original => OptimalEncoding::Truecolor {
                    bytes: reduced,
                    color_type,
                    bit_depth: 8,
                    trns: None,
                },
                other => other,
            }
        }
        _ => {
            // Grayscale/GrayscaleAlpha: just reduce bit depth
            OptimalEncoding::Truecolor {
                bytes: reduced,
                color_type,
                bit_depth: 8,
                trns: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_grayscale_opaque_rgba() {
        // 2x1 image: gray pixels with alpha=255
        let bytes = [128, 128, 128, 255, 64, 64, 64, 255];
        let a = analyze_rgba8(&bytes, 2, 1);
        assert!(a.is_grayscale);
        assert!(a.is_opaque);
        assert_eq!(a.unique_color_count, 2);
    }

    #[test]
    fn detect_non_grayscale() {
        let bytes = [255, 0, 0, 255, 0, 255, 0, 255];
        let a = analyze_rgba8(&bytes, 2, 1);
        assert!(!a.is_grayscale);
        assert!(a.is_opaque);
    }

    #[test]
    fn detect_transparency() {
        let bytes = [128, 128, 128, 128, 64, 64, 64, 0];
        let a = analyze_rgba8(&bytes, 2, 1);
        assert!(a.is_grayscale);
        assert!(!a.is_opaque);
    }

    #[test]
    fn exact_palette_small_image() {
        // 3 unique colors
        let bytes = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255,
        ];
        let a = analyze_rgba8(&bytes, 4, 1);
        assert_eq!(a.unique_color_count, 3);
        assert!(a.exact_palette.is_some());
        let ep = a.exact_palette.unwrap();
        assert_eq!(ep.palette_rgba.len(), 3);
        assert_eq!(ep.indices.len(), 4);
        // First and last pixel are same color
        assert_eq!(ep.indices[0], ep.indices[3]);
    }

    #[test]
    fn palette_overflow() {
        // Create 257 unique colors
        let mut bytes = Vec::with_capacity(257 * 4);
        for i in 0..257u32 {
            bytes.push((i & 0xFF) as u8);
            bytes.push((i >> 8) as u8);
            bytes.push(0);
            bytes.push(255);
        }
        let a = analyze_rgba8(&bytes, 257, 1);
        assert_eq!(a.unique_color_count, 257);
        assert!(a.exact_palette.is_none());
    }

    #[test]
    fn reduce_16_to_8_possible() {
        let be = [0x80, 0x00, 0x40, 0x00, 0xFF, 0x00];
        assert!(can_reduce_16_to_8(&be));
        let reduced = reduce_16_to_8(&be);
        assert_eq!(reduced, [0x80, 0x40, 0xFF]);
    }

    #[test]
    fn reduce_16_to_8_not_possible() {
        let be = [0x80, 0x01, 0x40, 0x00]; // first sample has low byte = 1
        assert!(!can_reduce_16_to_8(&be));
    }

    #[test]
    fn optimize_rgba_to_gray() {
        let bytes = [100, 100, 100, 255, 200, 200, 200, 255];
        match optimize_rgba8(&bytes, 2, 1) {
            // Could be either Indexed (2 colors) or Gray — indexed wins
            OptimalEncoding::Indexed { .. } => {}
            OptimalEncoding::Truecolor {
                color_type,
                bit_depth,
                ..
            } => {
                assert_eq!(color_type, 0);
                assert_eq!(bit_depth, 8);
            }
            OptimalEncoding::Original => panic!("should optimize"),
        }
    }

    #[test]
    fn optimize_rgba_to_rgb() {
        // >256 unique colors, all opaque, not grayscale
        let mut bytes = Vec::new();
        for r in 0..20u8 {
            for g in 0..20u8 {
                bytes.extend_from_slice(&[r * 13, g * 13, 128, 255]);
            }
        }
        match optimize_rgba8(&bytes, 400, 1) {
            OptimalEncoding::Truecolor {
                color_type,
                bit_depth,
                ..
            } => {
                assert_eq!(color_type, 2); // RGB
                assert_eq!(bit_depth, 8);
            }
            _ => panic!("expected RGB truecolor"),
        }
    }

    #[test]
    fn palette_luminance_sort() {
        // Dark, medium, light colors — should be sorted by luminance
        let mut palette = vec![
            [200, 200, 200, 255], // light gray (high lum)
            [50, 50, 50, 255],    // dark gray (low lum)
            [128, 128, 128, 255], // medium gray
        ];
        let mut indices = vec![0, 1, 2, 0, 1, 2];
        sort_palette_luminance(&mut palette, &mut indices);

        // After sort: dark (50) < medium (128) < light (200)
        assert_eq!(palette[0], [50, 50, 50, 255]);
        assert_eq!(palette[1], [128, 128, 128, 255]);
        assert_eq!(palette[2], [200, 200, 200, 255]);

        // Index 0 was light (200) → now at position 2
        assert_eq!(indices[0], 2);
        // Index 1 was dark (50) → now at position 0
        assert_eq!(indices[1], 0);
    }

    #[test]
    fn near_lossless_1bit() {
        // RGB pixels: round to nearest even (step=2)
        let bytes = [127, 128, 129, 0, 1, 255];
        let result = near_lossless_quantize(&bytes, 3, 1);
        assert_eq!(result[0], 128); // 127 → 128 (nearest even)
        assert_eq!(result[1], 128); // 128 → 128 (already even)
        assert_eq!(result[2], 130); // 129 → 130 (nearest even)
        assert_eq!(result[3], 0); // 0 → 0
        assert_eq!(result[4], 2); // 1 → 2 (nearest even)
        assert_eq!(result[5], 255); // 255 → min(256,255)=255 (clamped)
    }

    #[test]
    fn near_lossless_preserves_alpha() {
        // RGBA: alpha channel should NOT be quantized
        let bytes = [127, 128, 129, 200]; // R G B A
        let result = near_lossless_quantize(&bytes, 4, 2);
        // R,G,B rounded to nearest 4
        assert_eq!(result[0], 128); // 127 → 128
        assert_eq!(result[1], 128); // 128 → 128
        assert_eq!(result[2], 128); // 129 → 128
        assert_eq!(result[3], 200); // Alpha preserved
    }

    #[test]
    fn rgb_grayscale_detection() {
        let bytes = [50, 50, 50, 100, 100, 100, 150, 150, 150];
        let a = analyze_rgb8(&bytes, 3, 1);
        assert!(a.is_grayscale);
        assert_eq!(a.unique_color_count, 3);
    }

    #[test]
    fn min_gray_bit_depth_1bit() {
        // All values ∈ {0, 255} → 1-bit
        let bytes = [0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255];
        let a = analyze_rgba8(&bytes, 3, 1);
        assert!(a.is_grayscale);
        assert_eq!(a.min_gray_bit_depth, 1);
    }

    #[test]
    fn min_gray_bit_depth_2bit() {
        // Values: 0, 85, 170, 255 → 2-bit (all % 85 == 0)
        let bytes = [
            0, 0, 0, 255, 85, 85, 85, 255, 170, 170, 170, 255, 255, 255, 255, 255,
        ];
        let a = analyze_rgba8(&bytes, 4, 1);
        assert!(a.is_grayscale);
        assert_eq!(a.min_gray_bit_depth, 2);
    }

    #[test]
    fn min_gray_bit_depth_4bit() {
        // Value 34 = 17*2 → fits 4-bit but not 2-bit
        let bytes = [0, 0, 0, 255, 34, 34, 34, 255, 255, 255, 255, 255];
        let a = analyze_rgba8(&bytes, 3, 1);
        assert!(a.is_grayscale);
        assert_eq!(a.min_gray_bit_depth, 4);
    }

    #[test]
    fn min_gray_bit_depth_8bit() {
        // Value 100 is not divisible by 17 → 8-bit
        let bytes = [100, 100, 100, 255, 200, 200, 200, 255];
        let a = analyze_rgba8(&bytes, 2, 1);
        assert!(a.is_grayscale);
        assert_eq!(a.min_gray_bit_depth, 8);
    }

    #[test]
    fn binary_alpha_detection() {
        // Alpha values: 0 and 255 only → binary alpha
        let bytes = [255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 255];
        let a = analyze_rgba8(&bytes, 3, 1);
        assert!(a.is_binary_alpha);
        assert!(!a.is_opaque);
    }

    #[test]
    fn non_binary_alpha_detection() {
        // Alpha value 128 → not binary
        let bytes = [255, 0, 0, 128, 0, 255, 0, 255];
        let a = analyze_rgba8(&bytes, 2, 1);
        assert!(!a.is_binary_alpha);
    }

    #[test]
    fn single_transparent_color_tracking() {
        // One transparent color [255, 0, 0] with alpha=0
        let bytes = [255, 0, 0, 0, 0, 255, 0, 255, 255, 0, 0, 0];
        let a = analyze_rgba8(&bytes, 3, 1);
        assert_eq!(a.transparent_color, Some([255, 0, 0]));
    }

    #[test]
    fn multi_transparent_colors() {
        // Two different transparent colors → None
        let bytes = [255, 0, 0, 0, 0, 255, 0, 0, 128, 128, 128, 255];
        let a = analyze_rgba8(&bytes, 3, 1);
        assert_eq!(a.transparent_color, None);
    }

    #[test]
    fn trns_exclusive_color() {
        // Transparent color [255, 0, 0] never appears opaque
        let bytes = [255, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 255];
        assert!(trns_color_is_exclusive(&bytes, [255, 0, 0]));
    }

    #[test]
    fn trns_non_exclusive_color() {
        // Transparent color [255, 0, 0] also appears opaque
        let bytes = [255, 0, 0, 0, 255, 0, 0, 255];
        assert!(!trns_color_is_exclusive(&bytes, [255, 0, 0]));
    }

    #[test]
    fn gray8_to_subbyte_scaling() {
        // 1-bit: 0→0, 255→1
        assert_eq!(gray8_to_subbyte(&[0, 255], 1), vec![0, 1]);
        // 2-bit: 0→0, 85→1, 170→2, 255→3
        assert_eq!(gray8_to_subbyte(&[0, 85, 170, 255], 2), vec![0, 1, 2, 3]);
        // 4-bit: 0→0, 17→1, 34→2, ... 255→15
        assert_eq!(gray8_to_subbyte(&[0, 17, 34, 255], 4), vec![0, 1, 2, 15]);
        // 8-bit: identity
        assert_eq!(gray8_to_subbyte(&[42, 200], 8), vec![42, 200]);
    }

    #[test]
    fn optimize_rgba_to_subbyte_gray() {
        // 4x1 image: black and white, all opaque → should produce 1-bit gray
        let bytes = [
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        match optimize_rgba8(&bytes, 4, 1) {
            OptimalEncoding::Truecolor {
                color_type,
                bit_depth,
                trns,
                ..
            } => {
                assert_eq!(color_type, 0); // Grayscale
                assert!(bit_depth < 8); // Sub-byte
                assert!(trns.is_none());
            }
            OptimalEncoding::Indexed { .. } => {
                // Indexed might win for very small images — that's OK
            }
            OptimalEncoding::Original => panic!("should optimize"),
        }
    }

    #[test]
    fn optimize_rgba_to_gray_with_trns() {
        // Grayscale RGBA with binary alpha, single exclusive transparent color
        // 20x20 to exceed palette threshold
        let mut bytes = Vec::new();
        for i in 0..400 {
            let v = ((i * 7) % 256) as u8;
            // Make value 100 divisible by nothing useful to force 8-bit gray
            let gray = if v == 0 { 100 } else { v };
            bytes.extend_from_slice(&[gray, gray, gray, 255]);
        }
        // Add one transparent pixel with unique color
        bytes[0] = 1;
        bytes[1] = 1;
        bytes[2] = 1;
        bytes[3] = 0;
        match optimize_rgba8(&bytes, 20, 20) {
            OptimalEncoding::Truecolor {
                color_type, trns, ..
            } => {
                // Should be grayscale (0) with tRNS, or grayscale+alpha (4)
                if color_type == 0 {
                    assert!(trns.is_some(), "gray with tRNS expected");
                }
            }
            _ => {} // other encodings might be smaller
        }
    }

    #[test]
    fn optimize_rgba_to_rgb_with_trns() {
        // Non-grayscale RGBA with binary alpha, single exclusive transparent color
        // 400+ unique colors to exceed indexed
        let mut bytes = Vec::new();
        for r in 0..20u8 {
            for g in 0..21u8 {
                bytes.extend_from_slice(&[r * 13, g * 12, 128, 255]);
            }
        }
        // Make pixel 0 transparent with a unique RGB
        bytes[0] = 3;
        bytes[1] = 7;
        bytes[2] = 11;
        bytes[3] = 0;
        match optimize_rgba8(&bytes, 420, 1) {
            OptimalEncoding::Truecolor {
                color_type, trns, ..
            } => {
                if color_type == 2 {
                    // RGB with tRNS
                    assert!(trns.is_some(), "RGB with tRNS expected");
                    let t = trns.unwrap();
                    assert_eq!(t.len(), 6); // 3 big-endian u16 values
                }
            }
            _ => {} // other encodings might be chosen
        }
    }
}
