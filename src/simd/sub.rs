//! Sub filter: `row[i] += row[i - bpp]`
//!
//! Sequential dependency — each pixel depends on the previous output.
//! For bpp=4, process one 4-byte pixel per iteration using 128-bit registers.

use archmage::prelude::*;
#[cfg(target_arch = "x86_64")]
use safe_unaligned_simd::x86_64::{_mm_loadu_si32, _mm_storeu_si32};

pub(crate) fn unfilter_sub(row: &mut [u8], bpp: usize) {
    match bpp {
        4 => incant!(unfilter_sub_bpp4_impl(row), [v1]),
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
    fn sub_other_bpp_unchanged() {
        for &bpp in &[1, 2, 3, 6, 8] {
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
