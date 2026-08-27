//! Allocation helpers honoring the [`AllocPreference`] policy per call site.
//!
//! A PNG decode mixes two allocation regimes:
//!
//! * **Big, untrusted-sized buffers** (the full-image pixel buffer) default to
//!   the *fallible* `try_reserve` path — a malicious IHDR can demand gigabytes,
//!   so we want a graceful [`PngError::LimitExceeded`] rather than an abort.
//! * **Small, bounded scratch** (one row of zeros, one raw-row copy) defaults
//!   to the *infallible* `vec!` path — a single `calloc` is faster and the size
//!   is bounded by the image width, not attacker-controlled in any unbounded
//!   way.
//!
//! [`AllocPreference`] is a **3-mode, per-site override** of that default:
//! `Fallible` / `Infallible` force one path everywhere; `CodecDefault` (and any
//! future `#[non_exhaustive]` variant) keeps each site's own default. The helper
//! signatures therefore take the caller's preference *and* the site default, and
//! resolve them together.
//!
//! [`AllocPreference`]: zencodec::AllocPreference

use alloc::vec;
use alloc::vec::Vec;
use whereat::{At, at};

use crate::error::PngError;

/// Largest byte count a single Rust allocation can hold: `isize::MAX`.
///
/// `Vec` panics with "capacity overflow" (even on the `try_reserve` path it
/// returns `CapacityOverflow`, but `vec![0; n]` panics) for anything larger, so
/// a size that fits `usize` but not `isize` must be rejected *before* it reaches
/// an allocator. On 64-bit this is unreachable for PNG geometry; on 32-bit
/// targets a single row can exceed it (gray8 at the PNG max width is exactly
/// `isize::MAX` bytes, and any wider pixel format overshoots).
pub(crate) const MAX_ALLOC_BYTES: u64 = isize::MAX as u64;

/// Narrow a `u64` byte count to `usize` if a single allocation of that many
/// bytes is representable on this target (`<= isize::MAX`), else `None`.
#[inline]
#[must_use]
pub(crate) fn alloc_len(bytes: u64) -> Option<usize> {
    if bytes > MAX_ALLOC_BYTES {
        return None;
    }
    usize::try_from(bytes).ok()
}

/// Headroom reserved for the fixed-size buffers zenflate's streaming
/// decompressor allocates alongside the caller's capacity (a 32 KiB lookback
/// window plus a small input buffer, allocated as one `vec![0u8; lookback +
/// capacity]`). Kept generously above the real figure so a zenflate bump cannot
/// silently turn a capacity we accepted into a "capacity overflow" panic.
const STREAM_DECOMPRESSOR_OVERHEAD: u64 = 64 * 1024;

/// Capacity for a [`zenflate::StreamDecompressor`] that must buffer two rows of
/// `stride` bytes, computed with checked arithmetic and bounded by what the
/// allocator can represent.
///
/// The previous `stride * 2` wrapped on 32-bit targets for a gray8 IHDR at the
/// PNG maximum width (`stride == 2^31`): a debug-build panic, a wrapped `0`
/// capacity in release. Returns [`PngError::OutOfMemory`] when two rows plus
/// zenflate's own overhead cannot fit in one allocation on this target.
pub(crate) fn stream_capacity(stride: usize) -> Result<usize, At<PngError>> {
    let too_large = || {
        at!(PngError::OutOfMemory(alloc::format!(
            "row stride {stride} too large: two rows exceed the platform address space"
        )))
    };
    let capacity = (stride as u64).checked_mul(2).ok_or_else(too_large)?;
    // Two rows plus zenflate's fixed buffers must be one representable allocation.
    capacity
        .checked_add(STREAM_DECOMPRESSOR_OVERHEAD)
        .and_then(alloc_len)
        .ok_or_else(too_large)?;
    alloc_len(capacity).ok_or_else(too_large)
}

/// Resolve the 3-mode [`AllocPreference`](zencodec::AllocPreference) against
/// THIS site's default fallibility.
///
/// * [`Fallible`](zencodec::AllocPreference::Fallible) → always `true`.
/// * [`Infallible`](zencodec::AllocPreference::Infallible) → always `false`.
/// * [`CodecDefault`](zencodec::AllocPreference::CodecDefault) (and any future
///   `#[non_exhaustive]` variant) → the site default, unchanged.
#[inline]
#[must_use]
pub(crate) fn resolve_fallible(
    pref: zencodec::AllocPreference,
    site_default_fallible: bool,
) -> bool {
    match pref {
        zencodec::AllocPreference::Fallible => true,
        zencodec::AllocPreference::Infallible => false,
        _ => site_default_fallible,
    }
}

/// Allocate `n` zeroed bytes, honoring the per-site fallibility.
///
/// `pref` is the caller's [`AllocPreference`](zencodec::AllocPreference);
/// `site_default_fallible` is this site's default when `pref` is `CodecDefault`.
///
/// * fallible → `try_reserve_exact` then zero-fill, returning
///   [`PngError::LimitExceeded`] on allocation failure.
/// * infallible → `vec![0u8; n]` (single `calloc`, aborts on OOM).
pub(crate) fn alloc_zeroed(
    pref: zencodec::AllocPreference,
    site_default_fallible: bool,
    n: usize,
) -> Result<Vec<u8>, At<PngError>> {
    if resolve_fallible(pref, site_default_fallible) {
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| {
            at!(PngError::OutOfMemory(alloc::format!(
                "out of memory allocating {n} bytes"
            )))
        })?;
        v.resize(n, 0);
        Ok(v)
    } else {
        Ok(vec![0u8; n])
    }
}

/// Allocate an empty `Vec<u8>` with reserved capacity for `cap` bytes, honoring
/// the per-site fallibility (for the `Vec::with_capacity` + extend sites).
///
/// `pref` is the caller's [`AllocPreference`](zencodec::AllocPreference);
/// `site_default_fallible` is this site's default when `pref` is `CodecDefault`.
///
/// * fallible → `try_reserve_exact`, returning [`PngError::LimitExceeded`] on
///   allocation failure.
/// * infallible → `Vec::with_capacity(cap)` (aborts on OOM).
///
/// The returned `Vec` is empty (length 0); the caller fills it.
pub(crate) fn vec_with_capacity(
    pref: zencodec::AllocPreference,
    site_default_fallible: bool,
    cap: usize,
) -> Result<Vec<u8>, At<PngError>> {
    if resolve_fallible(pref, site_default_fallible) {
        let mut v = Vec::new();
        v.try_reserve_exact(cap).map_err(|_| {
            at!(PngError::OutOfMemory(alloc::format!(
                "out of memory allocating {cap} bytes"
            )))
        })?;
        Ok(v)
    } else {
        Ok(Vec::with_capacity(cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec::AllocPreference;

    // `CodecDefault` keeps each site's own default fallibility.

    #[test]
    fn codec_default_keeps_site_default_true() {
        // Big-buffer site (default fallible): CodecDefault stays fallible.
        assert!(resolve_fallible(AllocPreference::CodecDefault, true));
    }

    #[test]
    fn codec_default_keeps_site_default_false() {
        // Small-scratch site (default infallible): CodecDefault stays infallible.
        assert!(!resolve_fallible(AllocPreference::CodecDefault, false));
    }

    #[test]
    fn explicit_fallible_overrides_any_site_default() {
        assert!(resolve_fallible(AllocPreference::Fallible, false));
        assert!(resolve_fallible(AllocPreference::Fallible, true));
    }

    #[test]
    fn explicit_infallible_overrides_any_site_default() {
        assert!(!resolve_fallible(AllocPreference::Infallible, true));
        assert!(!resolve_fallible(AllocPreference::Infallible, false));
    }

    #[test]
    fn alloc_zeroed_all_modes_equal_bytes() {
        let a = alloc_zeroed(AllocPreference::CodecDefault, true, 4096).unwrap();
        let b = alloc_zeroed(AllocPreference::Infallible, true, 4096).unwrap();
        let c = alloc_zeroed(AllocPreference::Fallible, false, 4096).unwrap();
        assert_eq!(a.len(), 4096);
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert!(a.iter().all(|&x| x == 0));
    }

    #[test]
    fn vec_with_capacity_reserves_and_is_empty() {
        let a = vec_with_capacity(AllocPreference::Infallible, false, 1024).unwrap();
        let b = vec_with_capacity(AllocPreference::Fallible, false, 1024).unwrap();
        assert_eq!(a.len(), 0);
        assert_eq!(b.len(), 0);
        assert!(a.capacity() >= 1024);
        assert!(b.capacity() >= 1024);
    }

    #[test]
    fn alloc_zeroed_fallible_oom_returns_err() {
        // Request an impossibly large allocation; the fallible path must
        // return Err (mapped to LimitExceeded) rather than abort.
        let r = alloc_zeroed(AllocPreference::Fallible, true, usize::MAX);
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err().error(), PngError::OutOfMemory(_)));
    }

    #[test]
    fn vec_with_capacity_fallible_oom_returns_err() {
        let r = vec_with_capacity(AllocPreference::Fallible, true, usize::MAX);
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err().error(), PngError::OutOfMemory(_)));
    }

    #[test]
    fn alloc_len_bounds_at_isize_max_on_every_width() {
        assert_eq!(alloc_len(0), Some(0));
        assert_eq!(alloc_len(MAX_ALLOC_BYTES), Some(isize::MAX as usize));
        assert_eq!(alloc_len(MAX_ALLOC_BYTES + 1), None);
        assert_eq!(alloc_len(u64::MAX), None);
    }

    #[test]
    fn stream_capacity_small_stride_is_two_rows() {
        assert_eq!(stream_capacity(1).unwrap(), 2);
        assert_eq!(stream_capacity(4097).unwrap(), 8194);
    }

    /// Gray8 at the PNG maximum width (2^31 - 1): stride is 2^31. Two rows
    /// plus overhead fit a 64-bit allocation.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn stream_capacity_gray8_max_width_64bit() {
        let stride = 0x7FFF_FFFFusize + 1;
        assert_eq!(stream_capacity(stride).unwrap(), 0x1_0000_0000usize);
    }

    /// Same stride on 32-bit: `2 * stride` is not even representable in
    /// `usize`. This is the `huge-ihdr-gray8-2147483647sq.png` seed that
    /// panicked with `attempt to multiply with overflow` on i686.
    #[test]
    #[cfg(target_pointer_width = "32")]
    fn stream_capacity_gray8_max_width_32bit() {
        let stride = 0x7FFF_FFFFusize + 1;
        let r = stream_capacity(stride);
        assert!(matches!(r.unwrap_err().error(), PngError::OutOfMemory(_)));
    }

    /// A stride whose doubling fits `usize` but not `isize::MAX` on 32-bit:
    /// this is what previously reached zenflate's `vec![0u8; lookback +
    /// capacity]` and panicked with "capacity overflow".
    #[test]
    #[cfg(target_pointer_width = "32")]
    fn stream_capacity_rejects_two_rows_beyond_isize_max_32bit() {
        // 2 * 0x4000_0001 = 0x8000_0002 > isize::MAX (0x7FFF_FFFF).
        let r = stream_capacity(0x4000_0001);
        assert!(matches!(r.unwrap_err().error(), PngError::OutOfMemory(_)));
        // Exactly at the edge: 2 * stride + overhead must still fit.
        let edge = (isize::MAX as usize - 64 * 1024) / 2;
        assert_eq!(stream_capacity(edge).unwrap(), edge * 2);
        assert!(stream_capacity(edge + 1).is_err());
    }

    #[test]
    fn stream_capacity_rejects_usize_max() {
        let r = stream_capacity(usize::MAX);
        assert!(matches!(r.unwrap_err().error(), PngError::OutOfMemory(_)));
    }
}
