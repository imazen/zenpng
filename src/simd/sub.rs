//! Sub filter: `row[i] += row[i - bpp]`
//!
//! Sequential dependency — each pixel depends on the previous output.
//! For bpp=4, process one 4-byte pixel per iteration using 128-bit registers.

use archmage::prelude::*;
#[cfg(target_arch = "x86_64")]
use safe_unaligned_simd::x86_64::{_mm_loadu_si32, _mm_storeu_si32};

pub(crate) fn unfilter_sub(row: &mut [u8], bpp: usize) {
    match bpp {
        3 => incant!(unfilter_sub_bpp3_impl(row), [v1, neon]),
        4 => incant!(unfilter_sub_bpp4_impl(row), [v1, neon]),
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
fn unfilter_sub_bpp4_impl_v1(_token: Sse2Token, row: &mut [u8]) {
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
fn unfilter_sub_bpp3_impl_v1(_token: Sse2Token, row: &mut [u8]) {
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

    // First 4 bytes are unchanged (left neighbor is implicitly 0).
    // Initialize `a` with the first pixel as a u32.
    let mut a = u32::from_le_bytes(<[u8; 4]>::try_from(&row[0..4]).unwrap());

    let mut i = 4;
    while i + 4 <= len {
        let filt = u32::from_le_bytes(<[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        // Wrapping byte-wise add via NEON: load both into uint8x8_t, add, extract
        let a_v = vcreate_u8(a as u64);
        let f_v = vcreate_u8(filt as u64);
        let result_v = vadd_u8(a_v, f_v);
        let result = vget_lane_u32::<0>(vreinterpret_u32_u8(result_v));
        row[i..i + 4].copy_from_slice(&result.to_le_bytes());
        a = result;
        i += 4;
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

    // First 3 bytes unchanged. Load first 4 bytes (3 pixel + 1 overlap) into a.
    let mut a = u32::from_le_bytes(<[u8; 4]>::try_from(&row[0..4]).unwrap());

    let mut i = 3;
    while i + 4 <= len {
        let filt = u32::from_le_bytes(<[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let a_v = vcreate_u8(a as u64);
        let f_v = vcreate_u8(filt as u64);
        let result_v = vadd_u8(a_v, f_v);
        let result = vget_lane_u32::<0>(vreinterpret_u32_u8(result_v));
        // Store only 3 bytes (byte 3 is garbage from the 4-byte load)
        row[i..i + 3].copy_from_slice(&result.to_le_bytes()[..3]);
        a = result;
        i += 3;
    }

    // Scalar tail
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
            for &len in &[0, 4, 8, 12, 100, 4096, 65536] {
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
            for &len in &[0, 3, 6, 9, 99, 4095, 65535] {
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
