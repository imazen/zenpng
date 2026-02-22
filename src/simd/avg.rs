//! Average filter: `row[i] += floor((left + above) / 2)`
//!
//! Sequential dependency per pixel. For bpp=4, widen left+above to u16,
//! average, narrow back, add. One pixel (4 bytes) per SIMD iteration.

use archmage::prelude::*;
#[cfg(target_arch = "x86_64")]
use safe_unaligned_simd::x86_64::{_mm_loadu_si32, _mm_storeu_si32};

pub(crate) fn unfilter_avg(row: &mut [u8], prev: &[u8], bpp: usize) {
    match bpp {
        3 => incant!(unfilter_avg_bpp3_impl(row, prev), [v1]),
        4 => incant!(unfilter_avg_bpp4_impl(row, prev), [v1]),
        _ => unfilter_avg_scalar_any(row, prev, bpp),
    }
}

// ── Scalar implementation ────────────────────────────────────────────

fn unfilter_avg_scalar_any(row: &mut [u8], prev: &[u8], bpp: usize) {
    let len = row.len();
    for i in 0..bpp.min(len) {
        row[i] = row[i].wrapping_add(prev[i] >> 1);
    }
    for i in bpp..len {
        let avg = ((row[i - bpp] as u16 + prev[i] as u16) >> 1) as u8;
        row[i] = row[i].wrapping_add(avg);
    }
}

// ── SIMD bpp=4 (SSE2 / V1) ──────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
#[arcane]
fn unfilter_avg_bpp4_impl_v1(_token: Sse2Token, row: &mut [u8], prev: &[u8]) {
    let len = row.len();
    if len < 4 {
        return;
    }

    let zero = _mm_setzero_si128();
    let mut a_wide = zero; // left pixel widened to u16

    let mut i = 0;
    while i + 4 <= len {
        // b = above pixel, widened to u16
        let b_raw = _mm_loadu_si32(<&[u8; 4]>::try_from(&prev[i..i + 4]).unwrap());
        let b_wide = _mm_unpacklo_epi8(b_raw, zero);

        // avg = (a + b) >> 1  (u16 arithmetic, no overflow: max 255+255=510)
        let sum = _mm_add_epi16(a_wide, b_wide);
        let avg_wide = _mm_srli_epi16(sum, 1);

        // Narrow avg to u8 (values 0-254, packus won't clamp)
        let avg_narrow = _mm_packus_epi16(avg_wide, zero);

        // Load filtered bytes and add average (wrapping u8 add)
        let filt = _mm_loadu_si32(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = _mm_add_epi8(filt, avg_narrow);

        // Store 4-byte result
        _mm_storeu_si32(
            <&mut [u8; 4]>::try_from(&mut row[i..i + 4]).unwrap(),
            result,
        );

        // Feedback: a = result widened
        a_wide = _mm_unpacklo_epi8(result, zero);

        i += 4;
    }
}

// ── SIMD bpp=3 (SSE2 / V1) ──────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
#[arcane]
fn unfilter_avg_bpp3_impl_v1(_token: Sse2Token, row: &mut [u8], prev: &[u8]) {
    let len = row.len();
    if len < 3 {
        return;
    }

    let zero = _mm_setzero_si128();
    let mut a_wide = zero; // left pixel widened to u16

    let mut i = 0;
    while i + 4 <= len {
        let b_raw = _mm_loadu_si32(<&[u8; 4]>::try_from(&prev[i..i + 4]).unwrap());
        let b_wide = _mm_unpacklo_epi8(b_raw, zero);

        let sum = _mm_add_epi16(a_wide, b_wide);
        let avg_wide = _mm_srli_epi16(sum, 1);
        let avg_narrow = _mm_packus_epi16(avg_wide, zero);

        let filt = _mm_loadu_si32(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = _mm_add_epi8(filt, avg_narrow);

        // Store only 3 bytes
        let val = _mm_cvtsi128_si32(result) as u32;
        let bytes = val.to_le_bytes();
        row[i] = bytes[0];
        row[i + 1] = bytes[1];
        row[i + 2] = bytes[2];

        a_wide = _mm_unpacklo_epi8(result, zero);
        i += 3;
    }

    // Scalar tail
    for j in i..len {
        let left = if j >= 3 { row[j - 3] } else { 0 };
        let above = prev[j];
        let avg = ((left as u16 + above as u16) >> 1) as u8;
        row[j] = row[j].wrapping_add(avg);
    }
}

fn unfilter_avg_bpp3_impl_scalar(_token: ScalarToken, row: &mut [u8], prev: &[u8]) {
    unfilter_avg_scalar_any(row, prev, 3);
}

// Scalar fallback for incant! dispatch
fn unfilter_avg_bpp4_impl_scalar(_token: ScalarToken, row: &mut [u8], prev: &[u8]) {
    unfilter_avg_scalar_any(row, prev, 4);
}

#[cfg(test)]
mod tests {
    use archmage::testing::{CompileTimePolicy, for_each_token_permutation};

    fn scalar_avg(row: &mut [u8], prev: &[u8], bpp: usize) {
        let len = row.len();
        for i in 0..bpp.min(len) {
            row[i] = row[i].wrapping_add(prev[i] >> 1);
        }
        for i in bpp..len {
            let avg = ((row[i - bpp] as u16 + prev[i] as u16) >> 1) as u8;
            row[i] = row[i].wrapping_add(avg);
        }
    }

    #[test]
    fn avg_bpp4_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &len in &[0, 4, 8, 12, 100, 4096, 65536] {
                let prev: Vec<u8> = (0..len).map(|i| (i * 7 + 13) as u8).collect();
                let filtered: Vec<u8> = (0..len).map(|i| (i * 3 + 5) as u8).collect();

                let mut expected = filtered.clone();
                scalar_avg(&mut expected, &prev, 4);

                let mut actual = filtered.clone();
                super::unfilter_avg(&mut actual, &prev, 4);

                assert_eq!(actual, expected, "avg bpp=4 mismatch at len={len}");
            }
        });
        eprintln!("avg bpp=4: {report}");
    }

    #[test]
    fn avg_bpp3_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &len in &[0, 3, 6, 9, 99, 4095, 65535] {
                let prev: Vec<u8> = (0..len).map(|i| (i * 7 + 13) as u8).collect();
                let filtered: Vec<u8> = (0..len).map(|i| (i * 3 + 5) as u8).collect();

                let mut expected = filtered.clone();
                scalar_avg(&mut expected, &prev, 3);

                let mut actual = filtered.clone();
                super::unfilter_avg(&mut actual, &prev, 3);

                assert_eq!(actual, expected, "avg bpp=3 mismatch at len={len}");
            }
        });
        eprintln!("avg bpp=3: {report}");
    }

    #[test]
    fn avg_other_bpp_unchanged() {
        for &bpp in &[1, 2, 6, 8] {
            for &len in &[0, bpp, bpp * 4, bpp * 100] {
                let prev: Vec<u8> = (0..len).map(|i| (i * 11 + 3) as u8).collect();
                let filtered: Vec<u8> = (0..len).map(|i| (i * 5 + 7) as u8).collect();

                let mut expected = filtered.clone();
                scalar_avg(&mut expected, &prev, bpp);

                let mut actual = filtered.clone();
                super::unfilter_avg(&mut actual, &prev, bpp);

                assert_eq!(actual, expected, "avg bpp={bpp} mismatch at len={len}");
            }
        }
    }
}
