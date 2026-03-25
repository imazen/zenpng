//! Up filter: `row[i] = row[i].wrapping_add(prev[i])`
//!
//! Trivially parallel — no inter-pixel dependency. Process 32 bytes (AVX2)
//! or 16 bytes (SSE2) at a time.

use archmage::prelude::*;
#[cfg(target_arch = "aarch64")]
use safe_unaligned_simd::aarch64::{vld1q_u8, vst1q_u8};
#[cfg(target_arch = "wasm32")]
use safe_unaligned_simd::wasm32::{v128_load, v128_store};
#[cfg(target_arch = "x86_64")]
use safe_unaligned_simd::x86_64::{
    _mm_loadu_si128, _mm_storeu_si128, _mm256_loadu_si256, _mm256_storeu_si256,
};

pub(crate) fn unfilter_up(row: &mut [u8], prev: &[u8]) {
    incant!(unfilter_up_impl(row, prev), [v3, v1, neon, wasm128, scalar])
}

#[cfg(target_arch = "x86_64")]
#[arcane]
fn unfilter_up_impl_v3(_token: Desktop64, row: &mut [u8], prev: &[u8]) {
    let len = row.len().min(prev.len());
    let mut i = 0;

    while i + 32 <= len {
        let vr = _mm256_loadu_si256(<&[u8; 32]>::try_from(&row[i..i + 32]).unwrap());
        let vp = _mm256_loadu_si256(<&[u8; 32]>::try_from(&prev[i..i + 32]).unwrap());
        let sum = _mm256_add_epi8(vr, vp);
        _mm256_storeu_si256(<&mut [u8; 32]>::try_from(&mut row[i..i + 32]).unwrap(), sum);
        i += 32;
    }

    // SSE2 pass for 16–31 remaining bytes (AVX2 implies SSE2).
    while i + 16 <= len {
        let vr = _mm_loadu_si128(<&[u8; 16]>::try_from(&row[i..i + 16]).unwrap());
        let vp = _mm_loadu_si128(<&[u8; 16]>::try_from(&prev[i..i + 16]).unwrap());
        let sum = _mm_add_epi8(vr, vp);
        _mm_storeu_si128(<&mut [u8; 16]>::try_from(&mut row[i..i + 16]).unwrap(), sum);
        i += 16;
    }

    while i < len {
        row[i] = row[i].wrapping_add(prev[i]);
        i += 1;
    }
}

#[cfg(target_arch = "x86_64")]
#[arcane]
fn unfilter_up_impl_v1(_token: Sse2Token, row: &mut [u8], prev: &[u8]) {
    let len = row.len().min(prev.len());
    let mut i = 0;

    while i + 16 <= len {
        let vr = _mm_loadu_si128(<&[u8; 16]>::try_from(&row[i..i + 16]).unwrap());
        let vp = _mm_loadu_si128(<&[u8; 16]>::try_from(&prev[i..i + 16]).unwrap());
        let sum = _mm_add_epi8(vr, vp);
        _mm_storeu_si128(<&mut [u8; 16]>::try_from(&mut row[i..i + 16]).unwrap(), sum);
        i += 16;
    }

    while i < len {
        row[i] = row[i].wrapping_add(prev[i]);
        i += 1;
    }
}

// ── NEON (aarch64) ──────────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
#[arcane]
fn unfilter_up_impl_neon(_token: NeonToken, row: &mut [u8], prev: &[u8]) {
    let len = row.len().min(prev.len());
    let mut i = 0;

    while i + 16 <= len {
        let vr = vld1q_u8(<&[u8; 16]>::try_from(&row[i..i + 16]).unwrap());
        let vp = vld1q_u8(<&[u8; 16]>::try_from(&prev[i..i + 16]).unwrap());
        let sum = vaddq_u8(vr, vp);
        vst1q_u8(<&mut [u8; 16]>::try_from(&mut row[i..i + 16]).unwrap(), sum);
        i += 16;
    }

    while i < len {
        row[i] = row[i].wrapping_add(prev[i]);
        i += 1;
    }
}

// ── WASM SIMD128 ───────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[arcane]
fn unfilter_up_impl_wasm128(_token: Wasm128Token, row: &mut [u8], prev: &[u8]) {
    let len = row.len().min(prev.len());
    let mut i = 0;

    while i + 16 <= len {
        let vr = v128_load(<&[u8; 16]>::try_from(&row[i..i + 16]).unwrap());
        let vp = v128_load(<&[u8; 16]>::try_from(&prev[i..i + 16]).unwrap());
        let sum = i8x16_add(vr, vp);
        v128_store(<&mut [u8; 16]>::try_from(&mut row[i..i + 16]).unwrap(), sum);
        i += 16;
    }

    while i < len {
        row[i] = row[i].wrapping_add(prev[i]);
        i += 1;
    }
}

fn unfilter_up_impl_scalar(_token: ScalarToken, row: &mut [u8], prev: &[u8]) {
    let len = row.len().min(prev.len());
    for i in 0..len {
        row[i] = row[i].wrapping_add(prev[i]);
    }
}

#[cfg(test)]
mod tests {
    use archmage::testing::{CompileTimePolicy, for_each_token_permutation};

    fn scalar_up(row: &mut [u8], prev: &[u8]) {
        for i in 0..row.len().min(prev.len()) {
            row[i] = row[i].wrapping_add(prev[i]);
        }
    }

    #[test]
    fn up_all_tiers() {
        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            for &len in &[0, 1, 3, 4, 15, 16, 17, 31, 32, 33, 100, 4096, 65536] {
                let prev: Vec<u8> = (0..len).map(|i| (i * 7 + 13) as u8).collect();
                let filtered: Vec<u8> = (0..len).map(|i| (i * 3 + 5) as u8).collect();

                let mut expected = filtered.clone();
                scalar_up(&mut expected, &prev);

                let mut actual = filtered.clone();
                super::unfilter_up(&mut actual, &prev);

                assert_eq!(actual, expected, "up mismatch at len={len}");
            }
        });
        eprintln!("up filter: {report}");
    }
}
