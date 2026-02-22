//! Paeth filter: `row[i] += paeth_predictor(left, above, upper_left)`
//!
//! Sequential dependency per pixel, but SIMD computes all 4 channels
//! of a bpp=4 pixel in parallel using a branchless i16 predictor.

use archmage::prelude::*;
#[cfg(target_arch = "x86_64")]
use safe_unaligned_simd::x86_64::{_mm_loadu_si32, _mm_storeu_si32};

pub(crate) fn unfilter_paeth(row: &mut [u8], prev: &[u8], bpp: usize) {
    match bpp {
        4 => incant!(unfilter_paeth_bpp4_impl(row, prev), [v2]),
        _ => unfilter_paeth_scalar_any(row, prev, bpp),
    }
}

// ── Scalar reference implementation ──────────────────────────────────

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i16;
    let b = b as i16;
    let c = c as i16;
    let p = a + b - c;
    let pa = (p - a).unsigned_abs();
    let pb = (p - b).unsigned_abs();
    let pc = (p - c).unsigned_abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn unfilter_paeth_scalar_any(row: &mut [u8], prev: &[u8], bpp: usize) {
    let len = row.len();
    for i in 0..bpp.min(len) {
        row[i] = row[i].wrapping_add(paeth_predictor(0, prev[i], 0));
    }
    for i in bpp..len {
        let pred = paeth_predictor(row[i - bpp], prev[i], prev[i - bpp]);
        row[i] = row[i].wrapping_add(pred);
    }
}

// ── SIMD bpp=4 (SSE4.2 / V2) ────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
#[arcane]
fn unfilter_paeth_bpp4_impl_v2(token: X64V2Token, row: &mut [u8], prev: &[u8]) {
    let len = row.len();
    if len < 4 {
        return;
    }

    let zero = _mm_setzero_si128();
    let mut a_wide = zero; // left pixel, widened to i16
    let mut c_wide = zero; // upper-left pixel, widened to i16

    let mut i = 0;
    while i + 4 <= len {
        // b = above pixel, widened to i16
        let b_raw = _mm_loadu_si32(<&[u8; 4]>::try_from(&prev[i..i + 4]).unwrap());
        let b_wide = _mm_unpacklo_epi8(b_raw, zero);

        // Branchless Paeth predictor in i16
        let pred_wide = paeth_simd_v2(token, a_wide, b_wide, c_wide);

        // Narrow predictor to u8 (values are 0-255, packus won't clamp)
        let pred_narrow = _mm_packus_epi16(pred_wide, zero);

        // Load filtered bytes and add predictor (wrapping u8 add)
        let filt = _mm_loadu_si32(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = _mm_add_epi8(filt, pred_narrow);

        // Store 4-byte result
        _mm_storeu_si32(
            <&mut [u8; 4]>::try_from(&mut row[i..i + 4]).unwrap(),
            result,
        );

        // Feedback: a = result widened, c = b
        a_wide = _mm_unpacklo_epi8(result, zero);
        c_wide = b_wide;

        i += 4;
    }
}

/// Branchless Paeth predictor for 4 channels in parallel (i16 lanes).
///
/// Selects one of a, b, c per lane based on which is closest to `p = a + b - c`.
#[cfg(target_arch = "x86_64")]
#[rite]
fn paeth_simd_v2(_token: X64V2Token, a: __m128i, b: __m128i, c: __m128i) -> __m128i {
    // p = a + b - c
    let p = _mm_sub_epi16(_mm_add_epi16(a, b), c);

    // Absolute differences
    let pa = _mm_abs_epi16(_mm_sub_epi16(p, a)); // |p - a| = |b - c|
    let pb = _mm_abs_epi16(_mm_sub_epi16(p, b)); // |p - b| = |a - c|
    let pc = _mm_abs_epi16(_mm_sub_epi16(p, c)); // |p - c| = |a + b - 2c|

    // Branchless select: PNG spec tie-breaking is pa <= pb && pa <= pc → a; pb <= pc → b; else c
    // pa <= pb ↔ max(pa, pb) == pb ↔ cmpeq(max(pa, pb), pb)
    // Note: values are non-negative (abs results), so signed max/compare works.
    let mask_ab = _mm_cmpeq_epi16(_mm_max_epi16(pa, pb), pb); // pa <= pb
    let mask_ac = _mm_cmpeq_epi16(_mm_max_epi16(pa, pc), pc); // pa <= pc
    let mask_bc = _mm_cmpeq_epi16(_mm_max_epi16(pb, pc), pc); // pb <= pc

    // Start with c, blend in b where pb <= pc, then a where pa <= pb AND pa <= pc
    let result = c;
    let result = _mm_blendv_epi8(result, b, mask_bc);
    _mm_blendv_epi8(result, a, _mm_and_si128(mask_ab, mask_ac))
}

// Scalar fallback for incant! dispatch
fn unfilter_paeth_bpp4_impl_scalar(_token: ScalarToken, row: &mut [u8], prev: &[u8]) {
    unfilter_paeth_scalar_any(row, prev, 4);
}

#[cfg(test)]
mod tests {
    use archmage::testing::{CompileTimePolicy, for_each_token_permutation};

    use super::paeth_predictor;

    fn scalar_paeth(row: &mut [u8], prev: &[u8], bpp: usize) {
        let len = row.len();
        for i in 0..bpp.min(len) {
            row[i] = row[i].wrapping_add(paeth_predictor(0, prev[i], 0));
        }
        for i in bpp..len {
            let pred = paeth_predictor(row[i - bpp], prev[i], prev[i - bpp]);
            row[i] = row[i].wrapping_add(pred);
        }
    }

    #[test]
    fn paeth_bpp4_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &len in &[0, 4, 8, 12, 100, 4096, 65536] {
                let prev: Vec<u8> = (0..len).map(|i| (i * 7 + 13) as u8).collect();
                let filtered: Vec<u8> = (0..len).map(|i| (i * 3 + 5) as u8).collect();

                let mut expected = filtered.clone();
                scalar_paeth(&mut expected, &prev, 4);

                let mut actual = filtered.clone();
                super::unfilter_paeth(&mut actual, &prev, 4);

                assert_eq!(actual, expected, "paeth bpp=4 mismatch at len={len}");
            }
        });
        eprintln!("paeth bpp=4: {report}");
    }

    #[test]
    fn paeth_other_bpp_unchanged() {
        // Non-bpp=4 paths use scalar; verify they still work.
        for &bpp in &[1, 2, 3, 6, 8] {
            for &len in &[0, bpp, bpp * 4, bpp * 100] {
                let prev: Vec<u8> = (0..len).map(|i| (i * 11 + 3) as u8).collect();
                let filtered: Vec<u8> = (0..len).map(|i| (i * 5 + 7) as u8).collect();

                let mut expected = filtered.clone();
                scalar_paeth(&mut expected, &prev, bpp);

                let mut actual = filtered.clone();
                super::unfilter_paeth(&mut actual, &prev, bpp);

                assert_eq!(actual, expected, "paeth bpp={bpp} mismatch at len={len}");
            }
        }
    }
}
