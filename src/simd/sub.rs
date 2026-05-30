//! Sub filter: `row[i] += row[i - bpp]`
//!
//! Sequential dependency — each pixel depends on the previous output.
//! For bpp=4, process one 4-byte pixel per iteration using 128-bit registers.

use archmage::prelude::*;
#[cfg(target_arch = "aarch64")]
use safe_unaligned_simd::aarch64::{vld1_u8, vst1_u8};
#[cfg(target_arch = "wasm32")]
use safe_unaligned_simd::wasm32::v128_load32_zero;
#[cfg(target_arch = "x86_64")]
use safe_unaligned_simd::x86_64::{_mm_loadu_si32, _mm_storeu_si32};

pub(crate) fn unfilter_sub(row: &mut [u8], bpp: usize) {
    match bpp {
        3 => incant!(unfilter_sub_bpp3_impl(row), [v1, neon, wasm128, scalar]),
        4 => incant!(unfilter_sub_bpp4_impl(row), [v1, neon, wasm128, scalar]),
        _ => unfilter_sub_scalar_any(row, bpp),
    }
}

// ── Scalar implementation ────────────────────────────────────────────

fn unfilter_sub_scalar_any(row: &mut [u8], bpp: usize) {
    let len = row.len();
    for i in bpp..len {
        row[i] = row[i].wrapping_add(row[i - bpp]);
    }
}

// ── SIMD bpp=4 (SSE2 / V1) ──────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
#[arcane]
fn unfilter_sub_bpp4_impl_v1(_token: X64V1Token, row: &mut [u8]) {
    let len = row.len();
    if len < 8 {
        // Need at least 2 pixels (first pixel is identity, second pixel is first addition)
        unfilter_sub_scalar_any(row, 4);
        return;
    }

    // First 4 bytes are unchanged (left neighbor is implicitly 0).
    // Initialize `a` with the first pixel.
    let mut a = _mm_loadu_si32(<&[u8; 4]>::try_from(&row[0..4]).unwrap());

    let mut i = 4;
    while i + 4 <= len {
        let filt = _mm_loadu_si32(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = _mm_add_epi8(filt, a);
        _mm_storeu_si32(
            <&mut [u8; 4]>::try_from(&mut row[i..i + 4]).unwrap(),
            result,
        );
        a = result;
        i += 4;
    }
}

// ── SIMD bpp=3 (SSE2 / V1) ──────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
#[arcane]
fn unfilter_sub_bpp3_impl_v1(_token: X64V1Token, row: &mut [u8]) {
    let len = row.len();
    if len < 6 {
        unfilter_sub_scalar_any(row, 3);
        return;
    }

    // First 3 bytes unchanged. Initialize `a` with first pixel.
    let mut a = _mm_loadu_si32(<&[u8; 4]>::try_from(&row[0..4]).unwrap());

    let mut i = 3;
    // Need i + 4 <= len to safely load 4 bytes (3 pixel + 1 overlap)
    while i + 4 <= len {
        let filt = _mm_loadu_si32(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = _mm_add_epi8(filt, a);
        // Store only 3 bytes (lane 3 is garbage from the 4-byte load)
        let val = (_mm_cvtsi128_si32(result) as u32).to_le_bytes();
        row[i..i + 3].copy_from_slice(&val[..3]);
        a = result;
        i += 3;
    }

    // Scalar tail for last pixel if 4-byte load would overrun
    for j in i..len {
        row[j] = row[j].wrapping_add(row[j - 3]);
    }
}

// ── NEON bpp=4 (aarch64) ─────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
#[arcane]
fn unfilter_sub_bpp4_impl_neon(_token: NeonToken, row: &mut [u8]) {
    let len = row.len();
    if len < 8 {
        unfilter_sub_scalar_any(row, 4);
        return;
    }

    // The recurrence `recon[x] = filt[x] + recon[x-4]` is sequential, but two
    // consecutive 4-byte pixels fit in one 64-bit NEON register, so we resolve
    // a 2-pixel batch entirely in-register and only carry the final pixel
    // forward — avoiding the per-pixel scalar reload of `a` that capped the old
    // loop at ~1x vs scalar on Neoverse-N1.
    //
    // For pixels [f0, f1] in one uint8x8_t and carry `a` (= recon[x-4]):
    //   out0 = f0 + a
    //   out1 = f1 + out0 = f1 + f0 + a
    // Computed as:
    //   t   = v + [a, 0]            -> [f0+a, f1]
    //   out = t + [0, low4(t)]      -> [f0+a, f1 + (f0+a)] = [out0, out1]
    // `a` carries the low 4 lanes of the previous result (out1) for the next batch.

    // Carry starts as the first reconstructed pixel (row[0..4], identity).
    // Broadcast it into the low 4 lanes; high lanes are zero.
    let first = u32::from_le_bytes(<[u8; 4]>::try_from(&row[0..4]).unwrap());
    let mut a = vreinterpret_u8_u32(vdup_n_u32(first)); // lanes 0-3 = pixel, 4-7 = copy

    let mut i = 4;
    // 2-pixel batched main loop.
    while i + 8 <= len {
        let v = vld1_u8(<&[u8; 8]>::try_from(&row[i..i + 8]).unwrap());

        // [a, 0]: keep the carry in lanes 0-3, zero lanes 4-7.
        let a_lo = vreinterpret_u8_u32(vset_lane_u32::<1>(0, vreinterpret_u32_u8(a)));
        // t = [f0 + a, f1]
        let t = vadd_u8(v, a_lo);

        // Move (f0 + a) from lanes 0-3 into lanes 4-7, zero the low half.
        // vext shifts the concatenation (zero:t) left by 4 bytes -> low4(t) lands high.
        let zero = vdup_n_u8(0);
        let t_shifted = vext_u8::<4>(zero, t);
        // out = t + [0, f0+a] = [out0, out1]
        let out = vadd_u8(t, t_shifted);

        vst1_u8(<&mut [u8; 8]>::try_from(&mut row[i..i + 8]).unwrap(), out);

        // Carry the high pixel (out1) down into lanes 0-3 for the next batch.
        a = vext_u8::<4>(out, zero);
        i += 8;
    }

    // Single-pixel remainder (at most one 4-byte pixel left after the 2-pixel
    // batch loop). `len - 4` is a multiple of 4, so 0 or 4 bytes remain here.
    // Resolve it with the scalar recurrence reading the already-reconstructed
    // left neighbor from `row` — cheap and avoids a partial NEON store.
    while i < len {
        row[i] = row[i].wrapping_add(row[i - 4]);
        i += 1;
    }
}

// ── NEON bpp=3 (aarch64) ─────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
#[arcane]
fn unfilter_sub_bpp3_impl_neon(_token: NeonToken, row: &mut [u8]) {
    let len = row.len();
    if len < 6 {
        unfilter_sub_scalar_any(row, 3);
        return;
    }

    // Keep the running reconstructed pixel in a NEON register (low 3 lanes carry
    // the previous 3-byte pixel) rather than reloading a scalar u32 every step.
    // Each iteration still extracts a u32 for the 3-byte store (a 3-byte SIMD
    // store would overrun), but the load is folded into the carry, so the
    // recurrence chain is one `vadd_u8` per pixel instead of load+create+add+extract.
    //
    // `a` starts as the first reconstructed pixel (row[0..3], identity); load 4
    // bytes (3 pixel + 1 overlap) — the overlap lane is never stored.
    let mut a = vcreate_u8(u32::from_le_bytes(<[u8; 4]>::try_from(&row[0..4]).unwrap()) as u64);

    let mut i = 3;
    while i + 4 <= len {
        // 4-byte load (3-pixel + 1 overlap) into low lanes of a uint8x8_t.
        let filt = u32::from_le_bytes(<[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let f = vcreate_u8(filt as u64);
        // recon = filt + a (byte-wise wrapping). Lanes 0-2 are the real pixel.
        let result_v = vadd_u8(f, a);
        // Store only 3 bytes (lane 3 is garbage from the 4-byte overlap load).
        let result = vget_lane_u32::<0>(vreinterpret_u32_u8(result_v));
        row[i..i + 3].copy_from_slice(&result.to_le_bytes()[..3]);
        // Carry the reconstructed pixel forward in-register (no scalar reload of `a`).
        a = result_v;
        i += 3;
    }

    // Scalar tail
    for j in i..len {
        row[j] = row[j].wrapping_add(row[j - 3]);
    }
}

// ── WASM SIMD128 bpp=4 ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[arcane]
fn unfilter_sub_bpp4_impl_wasm128(_token: Wasm128Token, row: &mut [u8]) {
    let len = row.len();
    if len < 8 {
        unfilter_sub_scalar_any(row, 4);
        return;
    }

    // First 4 bytes are unchanged (left neighbor is implicitly 0).
    // Initialize `a` with the first pixel loaded into a v128.
    let mut a = v128_load32_zero(<&[u8; 4]>::try_from(&row[0..4]).unwrap());

    let mut i = 4;
    while i + 4 <= len {
        let filt = v128_load32_zero(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = i8x16_add(filt, a);
        let val = (i32x4_extract_lane::<0>(result) as u32).to_le_bytes();
        row[i..i + 4].copy_from_slice(&val);
        a = result;
        i += 4;
    }
}

// ── WASM SIMD128 bpp=3 ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[arcane]
fn unfilter_sub_bpp3_impl_wasm128(_token: Wasm128Token, row: &mut [u8]) {
    let len = row.len();
    if len < 6 {
        unfilter_sub_scalar_any(row, 3);
        return;
    }

    // First 3 bytes unchanged. Load first 4 bytes (3 pixel + 1 overlap).
    let mut a = v128_load32_zero(<&[u8; 4]>::try_from(&row[0..4]).unwrap());

    let mut i = 3;
    while i + 4 <= len {
        let filt = v128_load32_zero(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = i8x16_add(filt, a);
        // Store only 3 bytes (lane 3 is garbage from the 4-byte load)
        let val = (i32x4_extract_lane::<0>(result) as u32).to_le_bytes();
        row[i..i + 3].copy_from_slice(&val[..3]);
        a = result;
        i += 3;
    }

    // Scalar tail for last pixel if 4-byte load would overrun
    for j in i..len {
        row[j] = row[j].wrapping_add(row[j - 3]);
    }
}

fn unfilter_sub_bpp3_impl_scalar(_token: ScalarToken, row: &mut [u8]) {
    unfilter_sub_scalar_any(row, 3);
}

// Scalar fallback for incant! dispatch
fn unfilter_sub_bpp4_impl_scalar(_token: ScalarToken, row: &mut [u8]) {
    unfilter_sub_scalar_any(row, 4);
}

#[cfg(test)]
mod tests {
    use archmage::testing::{CompileTimePolicy, for_each_token_permutation};

    fn scalar_sub(row: &mut [u8], bpp: usize) {
        let len = row.len();
        for i in bpp..len {
            row[i] = row[i].wrapping_add(row[i - bpp]);
        }
    }

    #[test]
    fn sub_bpp4_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            // Sizes cover the 2-pixel batch loop and single-pixel remainder:
            // 16 = 4 + one batch + remainder, 20 = 4 + 2 batches + remainder,
            // 28 = 4 + 3 batches, etc.
            for &len in &[0, 4, 8, 12, 16, 20, 24, 28, 100, 4096, 65536] {
                let filtered: Vec<u8> = (0..len).map(|i| (i * 3 + 5) as u8).collect();

                let mut expected = filtered.clone();
                scalar_sub(&mut expected, 4);

                let mut actual = filtered.clone();
                super::unfilter_sub(&mut actual, 4);

                assert_eq!(actual, expected, "sub bpp=4 mismatch at len={len}");
            }
        });
        eprintln!("sub bpp=4: {report}");
    }

    #[test]
    fn sub_bpp3_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            // Cover varied bpp=3 alignments against the 4-byte overlap loop.
            for &len in &[0, 3, 6, 9, 12, 15, 18, 21, 99, 4095, 65535] {
                let filtered: Vec<u8> = (0..len).map(|i| (i * 3 + 5) as u8).collect();

                let mut expected = filtered.clone();
                scalar_sub(&mut expected, 3);

                let mut actual = filtered.clone();
                super::unfilter_sub(&mut actual, 3);

                assert_eq!(actual, expected, "sub bpp=3 mismatch at len={len}");
            }
        });
        eprintln!("sub bpp=3: {report}");
    }

    #[test]
    fn sub_other_bpp_unchanged() {
        for &bpp in &[1, 2, 6, 8] {
            for &len in &[0, bpp, bpp * 4, bpp * 100] {
                let filtered: Vec<u8> = (0..len).map(|i| (i * 5 + 7) as u8).collect();

                let mut expected = filtered.clone();
                scalar_sub(&mut expected, bpp);

                let mut actual = filtered.clone();
                super::unfilter_sub(&mut actual, bpp);

                assert_eq!(actual, expected, "sub bpp={bpp} mismatch at len={len}");
            }
        }
    }
}
