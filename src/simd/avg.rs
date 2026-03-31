//! Average filter: `row[i] += floor((left + above) / 2)`
//!
//! Sequential dependency per pixel. For bpp=4, widen left+above to u16,
//! average, narrow back, add. One pixel (4 bytes) per SIMD iteration.

use archmage::prelude::*;
#[cfg(target_arch = "wasm32")]
use safe_unaligned_simd::wasm32::v128_load32_zero;
#[cfg(target_arch = "x86_64")]
use safe_unaligned_simd::x86_64::{_mm_loadu_si32, _mm_storeu_si32};

pub(crate) fn unfilter_avg(row: &mut [u8], prev: &[u8], bpp: usize) {
    match bpp {
        4 => incant!(
            unfilter_avg_bpp4_impl(row, prev),
            [v1, neon, wasm128, scalar]
        ),
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
fn unfilter_avg_bpp4_impl_v1(_token: X64V1Token, row: &mut [u8], prev: &[u8]) {
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

// ── NEON bpp=4 (aarch64) ─────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
#[arcane]
fn unfilter_avg_bpp4_impl_neon(_token: NeonToken, row: &mut [u8], prev: &[u8]) {
    let len = row.len();
    if len < 4 {
        return;
    }

    // a_wide = left pixel widened to u16 (starts as zero)
    let mut a_wide = vdup_n_u16(0);

    let mut i = 0;
    while i + 4 <= len {
        // b = above pixel, widened to u16
        let b_bytes = u32::from_le_bytes(<[u8; 4]>::try_from(&prev[i..i + 4]).unwrap());
        let b_raw = vcreate_u8(b_bytes as u64);
        let b_wide = vget_low_u16(vmovl_u8(b_raw));

        // avg = (a + b) >> 1  (u16 arithmetic, no overflow: max 255+255=510)
        let sum = vadd_u16(a_wide, b_wide);
        let avg_wide = vshr_n_u16::<1>(sum);

        // Narrow avg to u8 (values 0-254, vmovn won't saturate)
        let avg_narrow = vmovn_u16(vcombine_u16(avg_wide, vdup_n_u16(0)));

        // Load filtered bytes and add average (wrapping u8 add)
        let filt_bytes = u32::from_le_bytes(<[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let filt = vcreate_u8(filt_bytes as u64);
        let result = vadd_u8(filt, avg_narrow);

        // Store 4-byte result
        let result_u32 = vget_lane_u32::<0>(vreinterpret_u32_u8(result));
        row[i..i + 4].copy_from_slice(&result_u32.to_le_bytes());

        // Feedback: a = result widened
        a_wide = vget_low_u16(vmovl_u8(result));

        i += 4;
    }
}

// ── WASM SIMD128 bpp=4 ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[arcane]
fn unfilter_avg_bpp4_impl_wasm128(_token: Wasm128Token, row: &mut [u8], prev: &[u8]) {
    let len = row.len();
    if len < 4 {
        return;
    }

    let zero = i16x8_splat(0);
    let mut a_wide = zero; // left pixel widened to u16

    let mut i = 0;
    while i + 4 <= len {
        // b = above pixel, widened to u16
        let b_raw = v128_load32_zero(<&[u8; 4]>::try_from(&prev[i..i + 4]).unwrap());
        let b_wide = i16x8_extend_low_u8x16(b_raw);

        // avg = (a + b) >> 1  (u16 arithmetic, no overflow: max 255+255=510)
        let sum = i16x8_add(a_wide, b_wide);
        let avg_wide = u16x8_shr(sum, 1);

        // Narrow avg to u8 (values 0-254, saturating narrow won't clamp)
        let avg_narrow = u8x16_narrow_i16x8(avg_wide, zero);

        // Load filtered bytes and add average (wrapping u8 add)
        let filt = v128_load32_zero(<&[u8; 4]>::try_from(&row[i..i + 4]).unwrap());
        let result = i8x16_add(filt, avg_narrow);

        // Store 4-byte result
        let val = (i32x4_extract_lane::<0>(result) as u32).to_le_bytes();
        row[i..i + 4].copy_from_slice(&val);

        // Feedback: a = result widened
        a_wide = i16x8_extend_low_u8x16(result);

        i += 4;
    }
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
    fn avg_other_bpp_unchanged() {
        for &bpp in &[1, 2, 3, 6, 8] {
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
