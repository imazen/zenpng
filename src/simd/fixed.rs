//! Portable unfilter kernels specialised per pixel size.
//!
//! Each kernel takes the pixel size `N` as a const generic and walks the row
//! as `[u8; N]` chunks, so the left / upper-left pixels live in registers and
//! LLVM vectorises across the `N` channel lanes. They serve every filter and
//! pixel size without a hand-written SIMD kernel: all of 1, 2, 6, 8 bytes,
//! bpp=3 Sub/Avg/Paeth on every ISA (they beat the old x86/NEON/SIMD128
//! kernels), and bpp=4 Avg/Paeth on wasm32. See the tier table in CLAUDE.md.
//!
//! Rows whose length is not a multiple of `N` cannot occur for PNG scanlines
//! (`len = width × N`); the dispatchers fall back to the byte-wise loops for
//! them anyway rather than assume it.

/// Branchless Paeth predictor (the stb_image formulation). Exactly equal to
/// the PNG spec predictor for all inputs; `paeth_matches_spec_exhaustively`
/// checks all 2^24 cases.
#[inline(always)]
pub(super) fn paeth_branchless(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i16, b as i16, c as i16);
    let thresh = c * 3 - (a + b);
    let lo = a.min(b);
    let hi = a.max(b);
    let t0 = if hi <= thresh { lo } else { c };
    (if thresh <= lo { hi } else { t0 }) as u8
}

/// Paeth predictor as pairwise minimum selection over `|b-c|`, `|a-c|`,
/// `|a+b-2c|` (the spec's distances). Exactly equal to the spec predictor;
/// `paeth_matches_spec_exhaustively` checks all 2^24 cases.
#[inline(always)]
pub(super) fn paeth_minselect(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i16, b as i16, c as i16);
    let pa = (ib - ic).abs();
    let pb = (ia - ic).abs();
    let pc = (ia + ib - 2 * ic).abs();
    let (mut out, mut min) = (a, pa);
    if pb < min {
        min = pb;
        out = b;
    }
    if pc < min {
        out = c;
    }
    out
}

#[inline(always)]
pub(super) fn sub<const N: usize>(row: &mut [u8]) {
    let mut left = [0u8; N];
    for px in row.as_chunks_mut::<N>().0 {
        for k in 0..N {
            px[k] = px[k].wrapping_add(left[k]);
        }
        left = *px;
    }
}

#[inline(always)]
pub(super) fn avg<const N: usize>(row: &mut [u8], prev: &[u8]) {
    let mut left = [0u8; N];
    for (px, up) in row
        .as_chunks_mut::<N>()
        .0
        .iter_mut()
        .zip(prev.as_chunks::<N>().0)
    {
        for k in 0..N {
            px[k] = px[k].wrapping_add(((left[k] as u16 + up[k] as u16) >> 1) as u8);
        }
        left = *px;
    }
}

/// The predictor the fixed Paeth kernel uses on this target. Measured with
/// `benches/unfilter_tiers.rs` and `examples/unfilter_bench.rs` (2026-10-06):
/// min-select is faster on aarch64 (Neoverse-N1, 1920 px RGB8 row 9.8 vs
/// 13.2 us; RGBA8 16.6 vs 19.8 us), the branchless stb form is faster on x86
/// (i265, 6.8 vs 7.1 us; 11.8 vs 14.1 us) and on wasm32 SIMD128 (wasmtime on
/// Apple Silicon, 827 vs 530 MB/s). Apple Silicon native is not measured.
#[inline(always)]
fn paeth_pred(a: u8, b: u8, c: u8) -> u8 {
    if cfg!(target_arch = "aarch64") {
        paeth_minselect(a, b, c)
    } else {
        paeth_branchless(a, b, c)
    }
}

#[inline(always)]
pub(super) fn paeth<const N: usize>(row: &mut [u8], prev: &[u8]) {
    let mut left = [0u8; N];
    let mut up_left = [0u8; N];
    for (px, up) in row
        .as_chunks_mut::<N>()
        .0
        .iter_mut()
        .zip(prev.as_chunks::<N>().0)
    {
        for k in 0..N {
            px[k] = px[k].wrapping_add(paeth_pred(left[k], up[k], up_left[k]));
        }
        left = *px;
        up_left = *up;
    }
}

/// Dispatch a const-generic kernel on a runtime pixel size; `None` when the
/// size has no specialisation or the row is not whole pixels. Invoke from a
/// sibling module of `fixed` (paths resolve at the call site).
macro_rules! by_bpp {
    ($bpp:expr, $len:expr, $f:ident ( $($arg:expr),* )) => {
        if $len % $bpp != 0 {
            None
        } else {
            match $bpp {
                1 => Some(super::fixed::$f::<1>($($arg),*)),
                2 => Some(super::fixed::$f::<2>($($arg),*)),
                3 => Some(super::fixed::$f::<3>($($arg),*)),
                4 => Some(super::fixed::$f::<4>($($arg),*)),
                6 => Some(super::fixed::$f::<6>($($arg),*)),
                8 => Some(super::fixed::$f::<8>($($arg),*)),
                _ => None,
            }
        }
    };
}
pub(super) use by_bpp;

// Lets the tests below invoke `by_bpp!`, whose paths resolve via `super::fixed`.
#[cfg(test)]
use super::fixed;

#[cfg(test)]
mod tests {
    use super::*;

    fn paeth_spec(a: u8, b: u8, c: u8) -> u8 {
        let (ia, ib, ic) = (a as i16, b as i16, c as i16);
        let p = ia + ib - ic;
        let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
        if pa <= pb && pa <= pc {
            a
        } else if pb <= pc {
            b
        } else {
            c
        }
    }

    #[test]
    fn paeth_matches_spec_exhaustively() {
        for a in 0..=255u8 {
            for b in 0..=255u8 {
                for c in 0..=255u8 {
                    let spec = paeth_spec(a, b, c);
                    assert_eq!(paeth_branchless(a, b, c), spec, "{a} {b} {c}");
                    assert_eq!(paeth_minselect(a, b, c), spec, "minselect {a} {b} {c}");
                }
            }
        }
    }

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 24) as u8
            })
            .collect()
    }

    /// Byte-wise references straight from the PNG spec.
    fn reference(ft: u8, row: &mut [u8], prev: &[u8], bpp: usize) {
        for i in 0..row.len() {
            let a = if i >= bpp { row[i - bpp] } else { 0 };
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let b = prev[i];
            let p = match ft {
                1 => a,
                3 => ((a as u16 + b as u16) >> 1) as u8,
                _ => paeth_spec(a, b, c),
            };
            row[i] = row[i].wrapping_add(p);
        }
    }

    #[test]
    fn fixed_kernels_match_reference() {
        for bpp in [1usize, 2, 3, 4, 6, 8] {
            for width in [0usize, 1, 2, 5, 17, 64, 257] {
                let len = width * bpp;
                let prev = noise(len, 7 + bpp as u32);
                let base = noise(len, 99 + width as u32);
                for ft in [1u8, 3, 4] {
                    let mut want = base.clone();
                    reference(ft, &mut want, &prev, bpp);
                    let mut got = base.clone();
                    let done = match ft {
                        1 => by_bpp!(bpp, len, sub(&mut got)),
                        3 => by_bpp!(bpp, len, avg(&mut got, &prev)),
                        _ => by_bpp!(bpp, len, paeth(&mut got, &prev)),
                    };
                    assert!(done.is_some());
                    assert_eq!(got, want, "filter {ft} bpp {bpp} width {width}");
                }
            }
        }
    }
}
