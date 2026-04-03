//! SIMD tier consistency tests for zenpng.
//!
//! Encodes and decodes PNG images under every archmage SIMD tier permutation
//! and verifies all produce identical output. The SIMD paths are in the PNG
//! filter/unfilter operations.

#![forbid(unsafe_code)]

use archmage::testing::{CompileTimePolicy, for_each_token_permutation};
use enough::Unstoppable;
use imgref::ImgVec;
use rgb::Rgb;
use zenpng::{EncodeConfig, PngDecodeConfig, decode, encode_rgb8};

/// FNV-1a hash of a byte slice.
fn hash_bytes(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Generate a deterministic test image with varied pixel values.
fn generate_test_image(width: usize, height: usize) -> ImgVec<Rgb<u8>> {
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 7 + y * 13 + 3) % 256) as u8;
            let g = ((x * 11 + y * 3 + 50) % 256) as u8;
            let b = ((x * 5 + y * 17 + 100) % 256) as u8;
            pixels.push(Rgb { r, g, b });
        }
    }
    ImgVec::new(pixels, width, height)
}

#[test]
fn png_encode_all_tiers_match() {
    let img = generate_test_image(64, 64);
    let config = EncodeConfig::default();
    let mut reference_hash: Option<u64> = None;

    let _ = for_each_token_permutation(CompileTimePolicy::Warn, |perm| {
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let h = hash_bytes(&encoded);

        if let Some(ref_h) = reference_hash {
            assert_eq!(
                h,
                ref_h,
                "PNG encode output differs under '{}' ({} bytes)",
                perm.label,
                encoded.len(),
            );
        } else {
            reference_hash = Some(h);
        }
    });
}

#[test]
fn png_roundtrip_all_tiers_match() {
    // Encode once with default tier to get a reference PNG.
    let img = generate_test_image(48, 48);
    let config = EncodeConfig::default();
    let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();

    let mut reference_hash: Option<u64> = None;

    let _ = for_each_token_permutation(CompileTimePolicy::Warn, |perm| {
        let decoded = decode(&encoded, &PngDecodeConfig::default(), &Unstoppable).unwrap();
        let pixel_bytes = decoded.pixels.copy_to_contiguous_bytes();
        let h = hash_bytes(&pixel_bytes);

        if let Some(ref_h) = reference_hash {
            assert_eq!(h, ref_h, "PNG decode output differs under '{}'", perm.label,);
        } else {
            reference_hash = Some(h);
        }
    });
}
