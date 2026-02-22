//! SIMD-accelerated PNG unfiltering.
//!
//! Dispatches to AVX2/SSE2/NEON implementations via `archmage::incant!`,
//! with scalar fallback for all filter types and bpp values.

mod avg;
mod paeth;
mod sub;
mod up;

use crate::error::PngError;

/// Expose the raw unfilter dispatch for benchmarking.
#[cfg(feature = "_dev")]
pub fn bench_unfilter_row(filter_type: u8, row: &mut [u8], prev: &[u8], bpp: usize) {
    match filter_type {
        1 => sub::unfilter_sub(row, bpp),
        2 => up::unfilter_up(row, prev),
        3 => avg::unfilter_avg(row, prev, bpp),
        4 => paeth::unfilter_paeth(row, prev, bpp),
        _ => {}
    }
}

/// SIMD-accelerated inverse filter. Replaces the scalar `unfilter_row` in `png_reader.rs`.
pub(crate) fn unfilter_row(
    filter_type: u8,
    row: &mut [u8],
    prev: &[u8],
    bpp: usize,
) -> Result<(), PngError> {
    match filter_type {
        0 => Ok(()),
        1 => {
            sub::unfilter_sub(row, bpp);
            Ok(())
        }
        2 => {
            up::unfilter_up(row, prev);
            Ok(())
        }
        3 => {
            avg::unfilter_avg(row, prev, bpp);
            Ok(())
        }
        4 => {
            paeth::unfilter_paeth(row, prev, bpp);
            Ok(())
        }
        _ => Err(PngError::Decode(alloc::format!(
            "unknown filter type {}",
            filter_type
        ))),
    }
}
