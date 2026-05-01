//! SIMD-accelerated boolean reductions for encode-time downcast decisions.
//!
//! Every predicate is a single early-exit pass. On photographic content the
//! "is grayscale" / "is opaque" / "16-bit replicates" tests bail in row 1;
//! on screenshot/UI content they walk the buffer, and the SIMD width pays.
//!
//! Generic over magetypes 5-tier dispatch (`_v4x` AVX-512, `v4` AVX2,
//! `v3` SSE4.2, `neon`, `wasm128`) via `#[magetypes]`. The 512-bit-wide
//! body polyfills two-256-bit on V4 / four-128-bit on V3+NEON+WASM.
//!
//! Buffer-layout assumptions (PNG truecolor):
//! - RGBA8: `[R, G, B, A, R, G, B, A, …]` — 4 bytes/pixel
//! - RGB8:  `[R, G, B, R, G, B, …]` — 3 bytes/pixel
//! - 16-bit: big-endian pairs `[hi, lo, hi, lo, …]`. Lossless 16→8 condition
//!   is `hi == lo` (PNG bit-replication: `u16 = u8 * 0x0101`).

use archmage::prelude::*;
use magetypes::simd::generic::u8x64 as GenericU8x64;

// ── Repeating-pattern masks ────────────────────────────────────────────
//
// Built at compile time. The byte values are 0xFF where we want "this lane
// is one we care about" and 0x00 where we want to ignore. Used to AND
// down a `simd_ne`/`simd_eq` mask before calling `any_true`.

/// `[0,0,0,0xFF, 0,0,0,0xFF, …]` — alpha lane in RGBA8.
const ALPHA_MASK_RGBA8: [u8; 64] = {
    let mut a = [0u8; 64];
    let mut i = 3;
    while i < 64 {
        a[i] = 0xFF;
        i += 4;
    }
    a
};

/// `[0xFF,0xFF,0,0, 0xFF,0xFF,0,0, …]` — `R^G` and `G^B` byte positions
/// when XORing RGBA8 against itself shifted by one byte.
const RGB_DELTA_MASK_RGBA8: [u8; 64] = {
    let mut a = [0u8; 64];
    let mut i = 0;
    while i < 64 {
        a[i] = 0xFF;
        a[i + 1] = 0xFF;
        i += 4;
    }
    a
};

/// `[0xFF, 0, 0xFF, 0, …]` — even-byte positions, used to compare a
/// big-endian 16-bit buffer against itself shifted by one byte (low byte
/// equals high byte).
const EVEN_BYTE_MASK: [u8; 64] = {
    let mut a = [0u8; 64];
    let mut i = 0;
    while i < 64 {
        a[i] = 0xFF;
        i += 2;
    }
    a
};

const fn rgb8_phase_mask(start_phase: usize) -> [u8; 64] {
    let mut a = [0u8; 64];
    let mut k = 0;
    while k < 64 {
        let phase = (start_phase + k) % 3;
        if phase == 0 || phase == 1 {
            a[k] = 0xFF;
        }
        k += 1;
    }
    a
}

// ── is_opaque_rgba8 ────────────────────────────────────────────────────

/// Returns true iff every alpha byte equals 255. Early-exit on first
/// non-opaque pixel.
pub(crate) fn is_opaque_rgba8(rgba: &[u8]) -> bool {
    incant!(
        is_opaque_rgba8_impl(rgba),
        [v4x, v4, v3, neon, wasm128, scalar]
    )
}

#[magetypes(v4x, v4, v3, neon, wasm128, scalar)]
fn is_opaque_rgba8_impl(token: Token, rgba: &[u8]) -> bool {
    #[allow(non_camel_case_types)]
    type u8x64 = GenericU8x64<Token>;

    let alpha_mask = u8x64::from_array(token, ALPHA_MASK_RGBA8);
    let opaque = u8x64::splat(token, 0xFF);
    let mut i = 0;
    while i + 64 <= rgba.len() {
        let chunk: &[u8; 64] = (&rgba[i..i + 64]).try_into().unwrap();
        let v = u8x64::load(token, chunk);
        // (v != 0xFF) at every byte; mask down to alpha lanes.
        let bad = v.simd_ne(opaque) & alpha_mask;
        if bad.any_true() {
            return false;
        }
        i += 64;
    }
    while i + 4 <= rgba.len() {
        if rgba[i + 3] != 255 {
            return false;
        }
        i += 4;
    }
    true
}

// ── is_grayscale_rgba8 ─────────────────────────────────────────────────

/// Returns true iff every pixel has `R == G == B`. Alpha is ignored.
/// Early-exits on first colorful pixel.
pub(crate) fn is_grayscale_rgba8(rgba: &[u8]) -> bool {
    incant!(
        is_grayscale_rgba8_impl(rgba),
        [v4x, v4, v3, neon, wasm128, scalar]
    )
}

#[magetypes(v4x, v4, v3, neon, wasm128, scalar)]
fn is_grayscale_rgba8_impl(token: Token, rgba: &[u8]) -> bool {
    #[allow(non_camel_case_types)]
    type u8x64 = GenericU8x64<Token>;

    let mask = u8x64::from_array(token, RGB_DELTA_MASK_RGBA8);
    let mut i = 0;
    // Need 65 bytes per chunk: a load at i and a load at i+1.
    while i + 65 <= rgba.len() {
        let chunk0: &[u8; 64] = (&rgba[i..i + 64]).try_into().unwrap();
        let chunk1: &[u8; 64] = (&rgba[i + 1..i + 65]).try_into().unwrap();
        let v0 = u8x64::load(token, chunk0);
        let v1 = u8x64::load(token, chunk1);
        // simd_ne yields 0xFF where bytes differ; mask keeps only the
        // R^G and G^B byte positions that matter.
        let masked = v0.simd_ne(v1) & mask;
        if masked.any_true() {
            return false;
        }
        i += 64;
    }
    while i + 4 <= rgba.len() {
        if rgba[i] != rgba[i + 1] || rgba[i + 1] != rgba[i + 2] {
            return false;
        }
        i += 4;
    }
    true
}

// ── alpha_is_binary_rgba8 ──────────────────────────────────────────────

/// Returns true iff every alpha byte is exactly 0 or 255. Useful for
/// choosing tRNS encoding over a full alpha channel. Early-exit.
pub(crate) fn alpha_is_binary_rgba8(rgba: &[u8]) -> bool {
    incant!(
        alpha_is_binary_rgba8_impl(rgba),
        [v4x, v4, v3, neon, wasm128, scalar]
    )
}

#[magetypes(v4x, v4, v3, neon, wasm128, scalar)]
fn alpha_is_binary_rgba8_impl(token: Token, rgba: &[u8]) -> bool {
    #[allow(non_camel_case_types)]
    type u8x64 = GenericU8x64<Token>;

    let alpha_mask = u8x64::from_array(token, ALPHA_MASK_RGBA8);
    let zero = u8x64::splat(token, 0);
    let opaque = u8x64::splat(token, 0xFF);
    let mut i = 0;
    while i + 64 <= rgba.len() {
        let chunk: &[u8; 64] = (&rgba[i..i + 64]).try_into().unwrap();
        let v = u8x64::load(token, chunk);
        // Bad if (alpha != 0) AND (alpha != 255). Both compares produce
        // 0xFF/0 masks; AND them together, mask to alpha lanes only.
        let bad = v.simd_ne(zero) & v.simd_ne(opaque) & alpha_mask;
        if bad.any_true() {
            return false;
        }
        i += 64;
    }
    while i + 4 <= rgba.len() {
        let a = rgba[i + 3];
        if a != 0 && a != 255 {
            return false;
        }
        i += 4;
    }
    true
}

// ── is_grayscale_rgb8 ──────────────────────────────────────────────────

/// Returns true iff every RGB pixel has `R == G == B`. Early-exit.
///
/// 3-byte pixels don't tile evenly into 64-byte SIMD chunks (gcd(3,64)=1),
/// so we process 192-byte super-chunks (64 RGB pixels). Within each super-
/// chunk three masks handle the three byte-phase rotations.
pub(crate) fn is_grayscale_rgb8(rgb: &[u8]) -> bool {
    incant!(
        is_grayscale_rgb8_impl(rgb),
        [v4x, v4, v3, neon, wasm128, scalar]
    )
}

#[magetypes(v4x, v4, v3, neon, wasm128, scalar)]
fn is_grayscale_rgb8_impl(token: Token, rgb: &[u8]) -> bool {
    #[allow(non_camel_case_types)]
    type u8x64 = GenericU8x64<Token>;

    // Within bytes [0..64), [64..128), [128..192) the phase k%3 starts at
    // 0, 1, 2 respectively (because 64 % 3 == 1).
    let m0 = u8x64::from_array(token, rgb8_phase_mask(0));
    let m1 = u8x64::from_array(token, rgb8_phase_mask(1));
    let m2 = u8x64::from_array(token, rgb8_phase_mask(2));

    let mut i = 0;
    // Need 193 bytes per super-chunk (64+64+64 plus the final +1 shifted load).
    while i + 193 <= rgb.len() {
        for (off, mask) in [(0usize, m0), (64, m1), (128, m2)] {
            let c0: &[u8; 64] = (&rgb[i + off..i + off + 64]).try_into().unwrap();
            let c1: &[u8; 64] = (&rgb[i + off + 1..i + off + 65]).try_into().unwrap();
            let v0 = u8x64::load(token, c0);
            let v1 = u8x64::load(token, c1);
            let masked = v0.simd_ne(v1) & mask;
            if masked.any_true() {
                return false;
            }
        }
        i += 192;
    }
    while i + 3 <= rgb.len() {
        if rgb[i] != rgb[i + 1] || rgb[i + 1] != rgb[i + 2] {
            return false;
        }
        i += 3;
    }
    true
}

// ── 16→8 bit-replication check ─────────────────────────────────────────

/// Returns true iff every big-endian 16-bit pair satisfies `hi == lo`,
/// i.e. the value can be losslessly downcast to 8-bit and a PNG decoder
/// will reconstruct the original via bit-replication (`u16 = u8 * 0x0101`).
///
/// `be_bytes.len()` must be a multiple of 2.
pub(crate) fn bit_replication_lossless_be16(be_bytes: &[u8]) -> bool {
    incant!(
        bit_replication_lossless_be16_impl(be_bytes),
        [v4x, v4, v3, neon, wasm128, scalar]
    )
}

#[magetypes(v4x, v4, v3, neon, wasm128, scalar)]
fn bit_replication_lossless_be16_impl(token: Token, be_bytes: &[u8]) -> bool {
    #[allow(non_camel_case_types)]
    type u8x64 = GenericU8x64<Token>;

    // Compare the buffer against a +1 shifted view: at even byte positions
    // the comparison checks pair[k]==pair[k+1] (the bit-replication test).
    // Odd positions compare pair[k+1]==pair[k+2] (across-pair, don't care)
    // and are masked out.
    let even_mask = u8x64::from_array(token, EVEN_BYTE_MASK);
    let mut i = 0;
    while i + 65 <= be_bytes.len() {
        let c0: &[u8; 64] = (&be_bytes[i..i + 64]).try_into().unwrap();
        let c1: &[u8; 64] = (&be_bytes[i + 1..i + 65]).try_into().unwrap();
        let v0 = u8x64::load(token, c0);
        let v1 = u8x64::load(token, c1);
        let masked = v0.simd_ne(v1) & even_mask;
        if masked.any_true() {
            return false;
        }
        i += 64;
    }
    while i + 2 <= be_bytes.len() {
        if be_bytes[i] != be_bytes[i + 1] {
            return false;
        }
        i += 2;
    }
    true
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use archmage::testing::{CompileTimePolicy, for_each_token_permutation};

    fn scalar_is_opaque(rgba: &[u8]) -> bool {
        rgba.chunks_exact(4).all(|p| p[3] == 255)
    }
    fn scalar_is_grayscale_rgba8(rgba: &[u8]) -> bool {
        rgba.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2])
    }
    fn scalar_alpha_binary(rgba: &[u8]) -> bool {
        rgba.chunks_exact(4).all(|p| p[3] == 0 || p[3] == 255)
    }
    fn scalar_is_grayscale_rgb8(rgb: &[u8]) -> bool {
        rgb.chunks_exact(3).all(|p| p[0] == p[1] && p[1] == p[2])
    }
    fn scalar_bit_replication(be: &[u8]) -> bool {
        be.chunks_exact(2).all(|p| p[0] == p[1])
    }

    fn rgba_pattern(n_pixels: usize, mutate: impl Fn(usize, &mut [u8; 4])) -> Vec<u8> {
        let mut v = Vec::with_capacity(n_pixels * 4);
        for i in 0..n_pixels {
            let mut p = [
                (i * 7 + 3) as u8,
                (i * 7 + 3) as u8,
                (i * 7 + 3) as u8,
                255,
            ];
            mutate(i, &mut p);
            v.extend_from_slice(&p);
        }
        v
    }

    #[test]
    fn opaque_predicate_matches_scalar_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &n in &[0usize, 1, 4, 15, 16, 17, 31, 64, 200, 1024, 4099] {
                let v = rgba_pattern(n, |_, _| {});
                assert_eq!(super::is_opaque_rgba8(&v), scalar_is_opaque(&v), "n={n} all-opaque");
                if n > 5 {
                    let mut v = rgba_pattern(n, |_, _| {});
                    v[5 * 4 + 3] = 128;
                    assert_eq!(super::is_opaque_rgba8(&v), scalar_is_opaque(&v), "n={n} pixel 5 alpha=128");
                }
                // Non-opaque at the very last pixel
                if n > 0 {
                    let mut v = rgba_pattern(n, |_, _| {});
                    v[(n - 1) * 4 + 3] = 200;
                    assert_eq!(super::is_opaque_rgba8(&v), scalar_is_opaque(&v), "n={n} last pixel alpha=200");
                }
            }
        });
        eprintln!("is_opaque_rgba8: {report}");
    }

    #[test]
    fn grayscale_rgba8_matches_scalar_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &n in &[0usize, 1, 4, 16, 17, 64, 200, 4099] {
                let v = rgba_pattern(n, |_, _| {});
                assert_eq!(super::is_grayscale_rgba8(&v), scalar_is_grayscale_rgba8(&v), "n={n} all-gray");
                if n > 5 {
                    let mut v = rgba_pattern(n, |_, _| {});
                    v[5 * 4] = 1;
                    v[5 * 4 + 1] = 2;
                    v[5 * 4 + 2] = 3;
                    assert_eq!(super::is_grayscale_rgba8(&v), scalar_is_grayscale_rgba8(&v), "n={n} colorful pixel 5");
                }
                // Off-by-one: only G differs from R by 1
                if n > 5 {
                    let mut v = rgba_pattern(n, |_, _| {});
                    v[5 * 4 + 1] = v[5 * 4].wrapping_add(1);
                    assert_eq!(super::is_grayscale_rgba8(&v), scalar_is_grayscale_rgba8(&v), "n={n} g=r+1 at pixel 5");
                }
            }
        });
        eprintln!("is_grayscale_rgba8: {report}");
    }

    #[test]
    fn alpha_binary_matches_scalar_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &n in &[0usize, 1, 4, 16, 64, 200, 4099] {
                let v = rgba_pattern(n, |i, p| p[3] = if i & 1 == 0 { 0 } else { 255 });
                assert_eq!(super::alpha_is_binary_rgba8(&v), scalar_alpha_binary(&v), "n={n} alternating 0/255");
                if n > 5 {
                    let mut v = rgba_pattern(n, |i, p| p[3] = if i & 1 == 0 { 0 } else { 255 });
                    v[5 * 4 + 3] = 128;
                    assert_eq!(super::alpha_is_binary_rgba8(&v), scalar_alpha_binary(&v), "n={n} alpha 128 at 5");
                }
                if n > 5 {
                    let mut v = rgba_pattern(n, |_, _| {});
                    v[5 * 4 + 3] = 1; // very small but nonzero
                    assert_eq!(super::alpha_is_binary_rgba8(&v), scalar_alpha_binary(&v), "n={n} alpha 1 at 5");
                }
            }
        });
        eprintln!("alpha_is_binary_rgba8: {report}");
    }

    #[test]
    fn grayscale_rgb8_matches_scalar_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &n in &[0usize, 1, 3, 16, 64, 200, 4099] {
                let mut v = Vec::with_capacity(n * 3);
                for i in 0..n {
                    let g = (i * 7 + 3) as u8;
                    v.extend_from_slice(&[g, g, g]);
                }
                assert_eq!(super::is_grayscale_rgb8(&v), scalar_is_grayscale_rgb8(&v), "n={n} all-gray");
                if n > 80 {
                    let mut v2 = v.clone();
                    v2[80 * 3 + 1] = v2[80 * 3].wrapping_add(1);
                    assert_eq!(super::is_grayscale_rgb8(&v2), scalar_is_grayscale_rgb8(&v2), "n={n} pixel 80 g+=1");
                }
            }
        });
        eprintln!("is_grayscale_rgb8: {report}");
    }

    #[test]
    fn bit_replication_matches_scalar_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &n in &[0usize, 2, 4, 16, 64, 200, 4096] {
                let mut v = Vec::with_capacity(n * 2);
                for i in 0..n {
                    let b = (i * 11 + 7) as u8;
                    v.extend_from_slice(&[b, b]);
                }
                assert_eq!(
                    super::bit_replication_lossless_be16(&v),
                    scalar_bit_replication(&v),
                    "n={n} replicated"
                );
                if n > 30 {
                    v[30 * 2 + 1] = v[30 * 2].wrapping_add(1);
                    assert_eq!(
                        super::bit_replication_lossless_be16(&v),
                        scalar_bit_replication(&v),
                        "n={n} broken at 30"
                    );
                }
            }
        });
        eprintln!("bit_replication_lossless_be16: {report}");
    }
}
