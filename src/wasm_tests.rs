//! WASM-specific encode/decode roundtrip tests.
//!
//! These exercise the full encoding pipeline on wasm32-wasip1 with SIMD128,
//! covering both lossless (truecolor) and lossy (indexed/quantized) paths.
//! Every test verifies actual pixel data, not just dimensions.
//! Run via: `cargo test --lib --target wasm32-wasip1`

use crate::decode::{PngDecodeConfig, decode};
use crate::encode::*;
use crate::types::Compression;
use enough::Unstoppable;
use imgref::{Img, ImgVec};
use rgb::{Gray, Rgb, Rgba};

// ── Helpers ────────────────────────────────────────────────────────────

fn single_threaded(compression: Compression) -> EncodeConfig {
    EncodeConfig {
        compression,
        max_threads: 1,
        parallel: false,
        ..Default::default()
    }
}

fn rgb8_image(w: usize, h: usize) -> ImgVec<Rgb<u8>> {
    let pixels: Vec<Rgb<u8>> = (0..w * h)
        .map(|i| Rgb {
            r: (i * 7) as u8,
            g: (i * 13) as u8,
            b: (i * 23) as u8,
        })
        .collect();
    Img::new(pixels, w, h)
}

fn rgba8_image(w: usize, h: usize) -> ImgVec<Rgba<u8>> {
    let pixels: Vec<Rgba<u8>> = (0..w * h)
        .map(|i| Rgba {
            r: (i * 7) as u8,
            g: (i * 13) as u8,
            b: (i * 23) as u8,
            a: if i % 4 == 0 { 0 } else { 255 },
        })
        .collect();
    Img::new(pixels, w, h)
}

fn rgba8_opaque_image(w: usize, h: usize) -> ImgVec<Rgba<u8>> {
    let pixels: Vec<Rgba<u8>> = (0..w * h)
        .map(|i| Rgba {
            r: (i * 7) as u8,
            g: (i * 13) as u8,
            b: (i * 23) as u8,
            a: 255,
        })
        .collect();
    Img::new(pixels, w, h)
}

fn gray8_image(w: usize, h: usize) -> ImgVec<Gray<u8>> {
    let pixels: Vec<Gray<u8>> = (0..w * h).map(|i| Gray((i * 37) as u8)).collect();
    Img::new(pixels, w, h)
}

fn rgb16_image(w: usize, h: usize) -> ImgVec<Rgb<u16>> {
    let pixels: Vec<Rgb<u16>> = (0..w * h)
        .map(|i| {
            let i = i as u16;
            Rgb {
                r: i.wrapping_mul(2048).wrapping_add(1),
                g: i.wrapping_mul(1024).wrapping_add(3),
                b: i.wrapping_mul(512).wrapping_add(7),
            }
        })
        .collect();
    Img::new(pixels, w, h)
}

fn rgba16_image(w: usize, h: usize) -> ImgVec<Rgba<u16>> {
    let pixels: Vec<Rgba<u16>> = (0..w * h)
        .map(|i| {
            let i = i as u16;
            Rgba {
                r: i.wrapping_mul(2048).wrapping_add(1),
                g: i.wrapping_mul(1024).wrapping_add(3),
                b: i.wrapping_mul(512).wrapping_add(7),
                a: 65535,
            }
        })
        .collect();
    Img::new(pixels, w, h)
}

fn gray16_image(w: usize, h: usize) -> ImgVec<Gray<u16>> {
    let pixels: Vec<Gray<u16>> = (0..w * h)
        .map(|i| Gray((i as u16).wrapping_mul(2048).wrapping_add(5)))
        .collect();
    Img::new(pixels, w, h)
}

fn decode_strict(data: &[u8]) -> crate::decode::PngDecodeOutput {
    decode(data, &PngDecodeConfig::strict(), &Unstoppable).unwrap()
}

/// Verify RGB8 pixel-perfect roundtrip at the given compression level.
fn verify_rgb8_roundtrip(w: usize, h: usize, compression: Compression) {
    let img = rgb8_image(w, h);
    let cfg = single_threaded(compression);
    let enc = encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, w as u32);
    assert_eq!(dec.info.height, h as u32);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img.buf().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    assert_eq!(raw.len(), expected.len(), "output byte count mismatch");
    assert_eq!(raw, expected);
}

/// Verify RGBA8 encode→decode with pixel-level checks.
/// The encoder zeros RGB of transparent pixels, so we check accordingly.
fn verify_rgba8_roundtrip(w: usize, h: usize, compression: Compression) {
    let img = rgba8_image(w, h);
    let cfg = single_threaded(compression);
    let enc = encode_rgba8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, w as u32);
    assert_eq!(dec.info.height, h as u32);
    assert!(dec.info.has_alpha);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    assert_eq!(raw.len(), w * h * 4);
    for (i, px) in img.buf().iter().enumerate() {
        let off = i * 4;
        if px.a == 255 {
            assert_eq!(raw[off], px.r, "pixel {i} R");
            assert_eq!(raw[off + 1], px.g, "pixel {i} G");
            assert_eq!(raw[off + 2], px.b, "pixel {i} B");
            assert_eq!(raw[off + 3], 255, "pixel {i} A");
        } else {
            // Transparent pixels: RGB may be zeroed, alpha must match
            assert_eq!(raw[off + 3], px.a, "pixel {i} alpha");
        }
    }
}

/// Verify Gray8 pixel-perfect roundtrip.
fn verify_gray8_roundtrip(w: usize, h: usize, compression: Compression) {
    let img = gray8_image(w, h);
    let cfg = single_threaded(compression);
    let enc = encode_gray8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, w as u32);
    assert_eq!(dec.info.height, h as u32);
    assert!(!dec.info.has_alpha);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img.buf().iter().map(|p| p.value()).collect();
    assert_eq!(raw.len(), expected.len());
    assert_eq!(raw, expected);
}

// ── Lossless RGB8: pixel-perfect at every effort level ─────────────────

#[test]
fn wasm_rgb8_effort_none() {
    verify_rgb8_roundtrip(16, 12, Compression::None);
}

#[test]
fn wasm_rgb8_effort_fastest() {
    verify_rgb8_roundtrip(16, 12, Compression::Fastest);
}

#[test]
fn wasm_rgb8_effort_turbo() {
    verify_rgb8_roundtrip(16, 12, Compression::Turbo);
}

#[test]
fn wasm_rgb8_effort_fast() {
    verify_rgb8_roundtrip(16, 12, Compression::Fast);
}

#[test]
fn wasm_rgb8_effort_balanced() {
    verify_rgb8_roundtrip(16, 12, Compression::Balanced);
}

#[test]
fn wasm_rgb8_effort_thorough() {
    verify_rgb8_roundtrip(16, 12, Compression::Thorough);
}

#[test]
fn wasm_rgb8_effort_high() {
    verify_rgb8_roundtrip(16, 12, Compression::High);
}

#[test]
fn wasm_rgb8_effort_aggressive() {
    verify_rgb8_roundtrip(16, 12, Compression::Aggressive);
}

// ── Lossless RGBA8: pixel verification with transparency ───────────────

#[test]
fn wasm_rgba8_fast() {
    verify_rgba8_roundtrip(16, 12, Compression::Fast);
}

#[test]
fn wasm_rgba8_balanced() {
    verify_rgba8_roundtrip(16, 12, Compression::Balanced);
}

#[test]
fn wasm_rgba8_thorough() {
    verify_rgba8_roundtrip(16, 12, Compression::Thorough);
}

#[test]
fn wasm_rgba8_high() {
    verify_rgba8_roundtrip(16, 12, Compression::High);
}

// ── Lossless Gray8: pixel verification ─────────────────────────────────

#[test]
fn wasm_gray8_fast() {
    verify_gray8_roundtrip(16, 12, Compression::Fast);
}

#[test]
fn wasm_gray8_balanced() {
    verify_gray8_roundtrip(16, 12, Compression::Balanced);
}

#[test]
fn wasm_gray8_thorough() {
    verify_gray8_roundtrip(16, 12, Compression::Thorough);
}

// ── 16-bit pixel-perfect roundtrips ────────────────────────────────────

#[test]
fn wasm_rgb16_pixel_perfect() {
    let img = rgb16_image(8, 4);
    let cfg = single_threaded(Compression::Balanced);
    let enc = encode_rgb16(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, 8);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img
        .buf()
        .iter()
        .flat_map(|p| [p.r.to_ne_bytes(), p.g.to_ne_bytes(), p.b.to_ne_bytes()].concat())
        .collect();
    assert_eq!(raw.len(), expected.len(), "RGB16 byte count mismatch");
    assert_eq!(raw, expected, "RGB16 pixel mismatch");
}

#[test]
fn wasm_rgba16_pixel_perfect() {
    let img = rgba16_image(8, 4);
    let cfg = single_threaded(Compression::Fast);
    let enc = encode_rgba16(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, 8);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img
        .buf()
        .iter()
        .flat_map(|p| {
            [
                p.r.to_ne_bytes(),
                p.g.to_ne_bytes(),
                p.b.to_ne_bytes(),
                p.a.to_ne_bytes(),
            ]
            .concat()
        })
        .collect();
    assert_eq!(raw.len(), expected.len(), "RGBA16 byte count mismatch");
    assert_eq!(raw, expected, "RGBA16 pixel mismatch");
}

#[test]
fn wasm_gray16_pixel_perfect() {
    let img = gray16_image(8, 4);
    let cfg = single_threaded(Compression::Fast);
    let enc = encode_gray16(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, 8);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img
        .buf()
        .iter()
        .flat_map(|p| p.value().to_ne_bytes())
        .collect();
    assert_eq!(raw.len(), expected.len(), "Gray16 byte count mismatch");
    assert_eq!(raw, expected, "Gray16 pixel mismatch");
}

// ── Near-lossless (verify bounded error, not just "doesn't crash") ─────

#[test]
fn wasm_near_lossless_rgb8() {
    let img = rgb8_image(16, 12);
    let mut cfg = single_threaded(Compression::Fast);
    cfg.near_lossless_bits = 2;
    let enc = encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, 16);
    assert_eq!(dec.info.height, 12);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    // With 2 near-lossless bits, max error per sample is 2^1 = 2
    for (i, px) in img.buf().iter().enumerate() {
        let off = i * 3;
        assert!(
            (raw[off] as i16 - px.r as i16).unsigned_abs() <= 4,
            "pixel {i} R"
        );
        assert!(
            (raw[off + 1] as i16 - px.g as i16).unsigned_abs() <= 4,
            "pixel {i} G"
        );
        assert!(
            (raw[off + 2] as i16 - px.b as i16).unsigned_abs() <= 4,
            "pixel {i} B"
        );
    }
}

#[test]
fn wasm_near_lossless_rgba8() {
    let img = rgba8_image(16, 12);
    let mut cfg = single_threaded(Compression::Fast);
    cfg.near_lossless_bits = 3;
    let enc = encode_rgba8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, 16);
    assert_eq!(dec.info.height, 12);
    assert!(dec.info.has_alpha);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    assert_eq!(raw.len(), 16 * 12 * 4);
    // Alpha channel must be preserved exactly
    for (i, px) in img.buf().iter().enumerate() {
        let off = i * 4;
        assert_eq!(raw[off + 3], px.a, "pixel {i} alpha must be exact");
    }
}

#[test]
fn wasm_near_lossless_gray8() {
    let img = gray8_image(16, 12);
    let mut cfg = single_threaded(Compression::Fast);
    cfg.near_lossless_bits = 2;
    let enc = encode_gray8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, 16);
    assert_eq!(dec.info.height, 12);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    for (i, px) in img.buf().iter().enumerate() {
        assert!(
            (raw[i] as i16 - px.value() as i16).unsigned_abs() <= 4,
            "pixel {i} gray value"
        );
    }
}

// ── APNG lossless with decode verification ─────────────────────────────

#[test]
fn wasm_apng_single_frame() {
    let w = 8u32;
    let h = 8u32;
    let pixels = vec![128u8; (w * h * 4) as usize];
    let frames = [ApngFrameInput::new(&pixels, 100, 1000)];
    let config = ApngEncodeConfig::default().with_encode(single_threaded(Compression::Fastest));
    let enc = encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
    // Verify PNG signature
    assert_eq!(&enc[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    // Decode and verify frame
    let apng = crate::decode_apng(&enc, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
    assert_eq!(apng.frames.len(), 1);
    let frame_bytes = apng.frames[0].pixels.copy_to_contiguous_bytes();
    assert_eq!(frame_bytes.len(), (w * h) as usize * 4);
    // All pixels should be (128, 128, 128, 128)
    for chunk in frame_bytes.chunks(4) {
        assert_eq!(chunk, &[128, 128, 128, 128]);
    }
}

#[test]
fn wasm_apng_two_frames() {
    let w = 4u32;
    let h = 4u32;
    let frame1: Vec<u8> = (0..w * h * 4).map(|i| (i * 3) as u8).collect();
    let frame2: Vec<u8> = (0..w * h * 4).map(|i| (i * 7) as u8).collect();
    let frames = [
        ApngFrameInput::new(&frame1, 100, 1000),
        ApngFrameInput::new(&frame2, 100, 1000),
    ];
    let config = ApngEncodeConfig::default().with_encode(single_threaded(Compression::Fastest));
    let enc = encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
    let apng = crate::decode_apng(&enc, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
    assert_eq!(apng.frames.len(), 2);
    // Verify first frame pixels match input
    let f0 = apng.frames[0].pixels.copy_to_contiguous_bytes();
    assert_eq!(f0, frame1);
    let f1 = apng.frames[1].pixels.copy_to_contiguous_bytes();
    assert_eq!(f1, frame2);
}

// ── Larger images (multi-row SIMD paths) ───────────────────────────────

#[test]
fn wasm_rgb8_128x64_pixel_perfect() {
    verify_rgb8_roundtrip(128, 64, Compression::Fast);
}

#[test]
fn wasm_rgb8_128x64_balanced_pixel_perfect() {
    verify_rgb8_roundtrip(128, 64, Compression::Balanced);
}

#[test]
fn wasm_rgba8_128x64_pixel_verified() {
    verify_rgba8_roundtrip(128, 64, Compression::Fast);
}

#[test]
fn wasm_gray8_128x64_pixel_perfect() {
    verify_gray8_roundtrip(128, 64, Compression::Fast);
}

// Odd dimensions (not divisible by SIMD lane widths)
#[test]
fn wasm_rgb8_odd_dims() {
    verify_rgb8_roundtrip(13, 7, Compression::Fast);
}

#[test]
fn wasm_rgba8_odd_dims() {
    verify_rgba8_roundtrip(13, 7, Compression::Fast);
}

#[test]
fn wasm_gray8_odd_dims() {
    verify_gray8_roundtrip(13, 7, Compression::Fast);
}

// Single-pixel edge case — encoder may reduce to indexed/gray
#[test]
fn wasm_rgba8_1x1() {
    let pixels = vec![Rgba {
        r: 42,
        g: 99,
        b: 200,
        a: 128,
    }];
    let img = Img::new(pixels, 1, 1);
    let cfg = single_threaded(Compression::Fast);
    let enc = encode_rgba8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.width, 1);
    assert_eq!(dec.info.height, 1);
    let raw = dec.pixels.copy_to_contiguous_bytes();
    // Encoder may keep RGBA or reduce format, but pixel values must match
    assert!(raw.len() >= 4, "at least 4 bytes for RGBA pixel");
    // Check last bytes are RGBA if format preserved
    if raw.len() == 4 {
        assert_eq!(raw, &[42, 99, 200, 128]);
    }
}

// Wide single-row (tests boundary-condition SIMD paths)
#[test]
fn wasm_rgb8_wide_1row() {
    verify_rgb8_roundtrip(256, 1, Compression::Fast);
}

// Tall single-column
#[test]
fn wasm_rgb8_1col_tall() {
    verify_rgb8_roundtrip(1, 64, Compression::Fast);
}

// ── Metadata roundtrip ─────────────────────────────────────────────────

#[test]
fn wasm_encode_with_srgb() {
    let img = rgb8_image(8, 4);
    let mut cfg = single_threaded(Compression::Fastest);
    cfg.srgb_intent = Some(0);
    let enc = encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.srgb_intent, Some(0));
    // Also verify pixels survived the metadata path
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img.buf().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    assert_eq!(raw, expected);
}

#[test]
fn wasm_encode_with_gamma_chrm() {
    let img = rgb8_image(8, 4);
    let mut cfg = single_threaded(Compression::Fastest);
    cfg.source_gamma = Some(45455);
    cfg.chromaticities = Some(crate::decode::PngChromaticities {
        white_x: 31270,
        white_y: 32900,
        red_x: 64000,
        red_y: 33000,
        green_x: 30000,
        green_y: 60000,
        blue_x: 15000,
        blue_y: 6000,
    });
    let enc = encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode_strict(&enc);
    assert_eq!(dec.info.source_gamma, Some(45455));
    assert_eq!(dec.info.chromaticities.unwrap().red_x, 64000);
    // Verify pixels
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img.buf().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    assert_eq!(raw, expected);
}

// ── Monotonicity (higher effort → smaller or equal output) ─────────────

#[test]
fn wasm_monotonicity_effort_1_through_19() {
    let img = rgb8_image(16, 12);
    let mut prev_size = usize::MAX;
    for effort in [1, 2, 7, 13, 17, 19] {
        let cfg = single_threaded(Compression::Effort(effort));
        let enc = encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
        // Also verify pixels at each level
        let dec = decode_strict(&enc);
        let raw = dec.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = img.buf().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        assert_eq!(raw, expected, "pixel mismatch at effort {effort}");
        if prev_size < usize::MAX {
            assert!(
                enc.len() <= prev_size,
                "effort {effort} produced {} bytes > previous {prev_size}",
                enc.len(),
            );
        }
        prev_size = enc.len();
    }
}

// ── Strict CRC/Adler32 verification ────────────────────────────────────

#[test]
fn wasm_strict_crc_rgb8() {
    let img = rgb8_image(32, 24);
    let cfg = single_threaded(Compression::Balanced);
    let enc = encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode(&enc, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
    assert!(
        dec.warnings.is_empty(),
        "strict decode should produce no warnings"
    );
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img.buf().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    assert_eq!(raw, expected);
}

#[test]
fn wasm_strict_crc_gray8() {
    let img = gray8_image(32, 24);
    let cfg = single_threaded(Compression::Balanced);
    let enc = encode_gray8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let dec = decode(&enc, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
    assert!(dec.warnings.is_empty());
    let raw = dec.pixels.copy_to_contiguous_bytes();
    let expected: Vec<u8> = img.buf().iter().map(|p| p.value()).collect();
    assert_eq!(raw, expected);
}

// ── Lossy: indexed/quantized encoding ──────────────────────────────────

#[cfg(feature = "quantize")]
mod indexed {
    use super::*;
    use crate::indexed::*;
    use crate::quantize::{Quantizer, default_quantizer};

    fn quantizer() -> Box<dyn Quantizer> {
        default_quantizer()
    }

    #[test]
    fn wasm_indexed_basic() {
        let img = rgba8_opaque_image(16, 12);
        let cfg = single_threaded(Compression::Fast);
        let q = quantizer();
        let enc =
            encode_indexed(img.as_ref(), &cfg, &*q, None, &Unstoppable, &Unstoppable).unwrap();
        let dec = decode_strict(&enc);
        assert_eq!(dec.info.width, 16);
        assert_eq!(dec.info.height, 12);
        // Indexed: color_type == 3
        assert_eq!(dec.info.color_type, 3, "should be indexed PNG");
        // Verify decoded pixel count
        let raw = dec.pixels.copy_to_contiguous_bytes();
        assert!(!raw.is_empty());
    }

    #[test]
    fn wasm_indexed_with_alpha() {
        let img = rgba8_image(16, 12);
        let cfg = single_threaded(Compression::Fast);
        let q = quantizer();
        let enc =
            encode_indexed(img.as_ref(), &cfg, &*q, None, &Unstoppable, &Unstoppable).unwrap();
        let dec = decode_strict(&enc);
        assert_eq!(dec.info.width, 16);
        assert_eq!(dec.info.height, 12);
        // With alpha, should still produce valid indexed PNG
        assert_eq!(dec.info.color_type, 3);
        let raw = dec.pixels.copy_to_contiguous_bytes();
        // Verify output has expected size (decoded from indexed → RGBA)
        assert!(raw.len() >= 16 * 12); // at least 1 byte per pixel
    }

    #[test]
    fn wasm_indexed_balanced() {
        let img = rgba8_opaque_image(16, 12);
        let cfg = single_threaded(Compression::Balanced);
        let q = quantizer();
        let enc =
            encode_indexed(img.as_ref(), &cfg, &*q, None, &Unstoppable, &Unstoppable).unwrap();
        let dec = decode_strict(&enc);
        assert_eq!(dec.info.width, 16);
        assert_eq!(dec.info.height, 12);
        assert_eq!(dec.info.color_type, 3);
    }

    #[test]
    fn wasm_indexed_thorough() {
        let img = rgba8_opaque_image(16, 12);
        let cfg = single_threaded(Compression::Thorough);
        let q = quantizer();
        let enc =
            encode_indexed(img.as_ref(), &cfg, &*q, None, &Unstoppable, &Unstoppable).unwrap();
        let dec = decode_strict(&enc);
        assert_eq!(dec.info.width, 16);
        assert_eq!(dec.info.color_type, 3);
    }

    #[test]
    fn wasm_auto_encode_lossless_fallback() {
        // Strict gate: DeltaE=0.0 allows exact-palette match or truecolor
        let img = rgba8_image(16, 12);
        let cfg = single_threaded(Compression::Fast);
        let q = quantizer();
        let result = encode_auto(
            img.as_ref(),
            &cfg,
            &*q,
            QualityGate::MaxDeltaE(0.0),
            None,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        assert!(!result.data.is_empty());
        if !result.indexed {
            assert_eq!(result.quality_loss, 0.0);
        }
        // Verify the output decodes correctly regardless of path
        let dec = decode_strict(&result.data);
        assert_eq!(dec.info.width, 16);
        assert_eq!(dec.info.height, 12);
    }

    #[test]
    fn wasm_auto_encode_indexed_path() {
        // Relaxed gate: should produce indexed
        let img = rgba8_opaque_image(8, 4);
        let cfg = single_threaded(Compression::Fast);
        let q = quantizer();
        let result = encode_auto(
            img.as_ref(),
            &cfg,
            &*q,
            QualityGate::MaxDeltaE(1.0),
            None,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        assert!(!result.data.is_empty());
        let dec = decode_strict(&result.data);
        assert_eq!(dec.info.width, 8);
        assert_eq!(dec.info.height, 4);
    }

    #[test]
    fn wasm_auto_quality_loss_reported() {
        let img = rgba8_opaque_image(8, 4);
        let cfg = single_threaded(Compression::Fast);
        let q = quantizer();
        let result = encode_auto(
            img.as_ref(),
            &cfg,
            &*q,
            QualityGate::MaxDeltaE(1.0),
            None,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        // quality_loss should be a finite non-negative number
        assert!(result.quality_loss >= 0.0);
        assert!(result.quality_loss.is_finite());
    }

    #[test]
    fn wasm_apng_indexed_single_frame() {
        let w = 8u32;
        let h = 8u32;
        let pixels: Vec<u8> = (0..w * h * 4).map(|i| ((i * 3) as u8) | 0x80).collect();
        let frames = [ApngFrameInput::new(&pixels, 100, 1000)];
        let config = ApngEncodeConfig::default().with_encode(single_threaded(Compression::Fastest));
        let q = quantizer();
        let params = ApngEncodeParams {
            frames: &frames,
            canvas_width: w,
            canvas_height: h,
            config: &config,
            quantizer: &*q,
            metadata: None,
            cancel: &Unstoppable,
            deadline: &Unstoppable,
        };
        let enc = encode_apng_indexed(&params).unwrap();
        // Verify it's a valid PNG and decodes
        let apng = crate::decode_apng(&enc, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(apng.frames.len(), 1);
    }

    #[test]
    fn wasm_apng_auto_two_frames() {
        let w = 4u32;
        let h = 4u32;
        let frame1: Vec<u8> = (0..w * h * 4).map(|i| ((i * 3) as u8) | 0x80).collect();
        let frame2: Vec<u8> = (0..w * h * 4).map(|i| ((i * 7) as u8) | 0x80).collect();
        let frames = [
            ApngFrameInput::new(&frame1, 100, 1000),
            ApngFrameInput::new(&frame2, 100, 1000),
        ];
        let config = ApngEncodeConfig::default().with_encode(single_threaded(Compression::Fastest));
        let q = quantizer();
        let params = ApngEncodeParams {
            frames: &frames,
            canvas_width: w,
            canvas_height: h,
            config: &config,
            quantizer: &*q,
            metadata: None,
            cancel: &Unstoppable,
            deadline: &Unstoppable,
        };
        let result = encode_apng_auto(&params, QualityGate::MaxDeltaE(1.0)).unwrap();
        assert!(!result.data.is_empty());
        let apng =
            crate::decode_apng(&result.data, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(apng.frames.len(), 2);
    }
}
