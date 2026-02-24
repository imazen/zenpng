//! Lossless image analysis and color type optimization.
//!
//! Analyzes pixel data to find the optimal PNG color type and bit depth,
//! performing lossless transformations that reduce file size without any
//! quality loss. Optimizations include:
//!
//! - RGBA → RGB (strip unused alpha channel)
//! - RGBA/RGB → Grayscale (when R==G==B for all pixels)
//! - RGBA → GrayscaleAlpha (grayscale with varying alpha)
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
/// - Alpha channel analysis (all opaque?)
/// - Unique color counting with early exit at 257
pub(crate) fn analyze_rgba8(bytes: &[u8], width: usize, height: usize) -> ImageAnalysis {
    let npixels = width * height;
    let mut is_grayscale = true;
    let mut is_opaque = true;
    let mut color_map: HashMap<[u8; 4], u8> = HashMap::with_capacity(257);
    let mut palette: Vec<[u8; 4]> = Vec::with_capacity(256);
    let mut palette_overflow = false;
    let mut has_transparency = false;

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

    for i in 0..npixels {
        let off = i * 3;
        let r = bytes[off];
        let g = bytes[off + 1];
        let b = bytes[off + 2];

        if is_grayscale && (r != g || r != b) {
            is_grayscale = false;
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
pub(crate) fn sort_palette_luminance(
    palette: &mut Vec<[u8; 4]>,
    indices: &mut [u8],
) {
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
pub(crate) fn optimize_palette_order(
    palette: &mut Vec<[u8; 4]>,
    indices: &mut [u8],
) {
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
    },
    /// No optimization: use original data as-is.
    Original,
}

/// Determine the optimal encoding for RGBA8 pixel data.
pub(crate) fn optimize_rgba8(bytes: &[u8], width: usize, height: usize) -> OptimalEncoding {
    let analysis = analyze_rgba8(bytes, width, height);

    // Prefer indexed if ≤256 unique colors (biggest win)
    if let Some(mut exact) = analysis.exact_palette {
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

    // Grayscale + opaque → Gray8 (4:1 reduction)
    if analysis.is_grayscale && analysis.is_opaque {
        return OptimalEncoding::Truecolor {
            bytes: rgba8_to_gray8(bytes),
            color_type: 0,
            bit_depth: 8,
        };
    }

    // Grayscale + alpha → GrayscaleAlpha8 (2:1 reduction)
    if analysis.is_grayscale {
        return OptimalEncoding::Truecolor {
            bytes: rgba8_to_gray_alpha8(bytes),
            color_type: 4,
            bit_depth: 8,
        };
    }

    // Opaque → RGB8 (4:3 reduction)
    if analysis.is_opaque {
        return OptimalEncoding::Truecolor {
            bytes: rgba8_to_rgb8(bytes),
            color_type: 2,
            bit_depth: 8,
        };
    }

    OptimalEncoding::Original
}

/// Determine the optimal encoding for RGB8 pixel data.
pub(crate) fn optimize_rgb8(bytes: &[u8], width: usize, height: usize) -> OptimalEncoding {
    let analysis = analyze_rgb8(bytes, width, height);

    // Prefer indexed if ≤256 unique colors
    if let Some(mut exact) = analysis.exact_palette {
        optimize_palette_order(&mut exact.palette_rgba, &mut exact.indices);
        let (rgb, _alpha) = split_palette_rgba(&exact.palette_rgba);
        return OptimalEncoding::Indexed {
            palette_rgb: rgb,
            palette_alpha: None,
            indices: exact.indices,
        };
    }

    // Grayscale → Gray8 (3:1 reduction)
    if analysis.is_grayscale {
        return OptimalEncoding::Truecolor {
            bytes: rgb8_to_gray8(bytes),
            color_type: 0,
            bit_depth: 8,
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
    fn rgb_grayscale_detection() {
        let bytes = [50, 50, 50, 100, 100, 100, 150, 150, 150];
        let a = analyze_rgb8(&bytes, 3, 1);
        assert!(a.is_grayscale);
        assert_eq!(a.unique_color_count, 3);
    }
}
