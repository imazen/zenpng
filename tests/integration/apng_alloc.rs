//! Issue #13: APNG decode buffers must fail gracefully, not abort the process.
//!
//! The canvas (and the per-frame / saved-region buffers) used infallible
//! `vec!` / `Vec::with_capacity`. An allocation the configured caps allow but
//! the machine cannot satisfy then aborted the whole process instead of
//! returning `Err` for that one decode.

use std::path::PathBuf;

use zenpng::{PngDecodeConfig, PngError};

fn seed(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/regression")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// A 2^31-1 × 2^31-1 RGBA8 APNG (one frame, tiny IDAT) with no resource
/// limits: the canvas is ~2^64 bytes. Before the fix, `vec![0u8; n]` with
/// `n > isize::MAX` panicked with "capacity overflow" (and a merely-too-large
/// size aborted on OOM); the fallible path must surface `OutOfMemory` instead.
///
/// On 32-bit targets the row-size check in `Ihdr::parse` rejects the image
/// earlier with the same `OutOfMemory` variant, so the assertion holds there
/// too (but does not exercise the canvas site).
#[test]
fn apng_canvas_allocation_failure_is_an_error_not_an_abort() {
    let data = seed("huge-ihdr-rgba8-apng-2147483647sq.png");

    let err = zenpng::decode_apng(&data, &PngDecodeConfig::none(), &enough::Unstoppable)
        .expect_err("a 2^64-byte canvas cannot be allocated");
    assert!(
        matches!(err.error(), PngError::OutOfMemory(_)),
        "expected OutOfMemory, got {:?}",
        err.error()
    );
}

/// With the default caps the same file is rejected at the pixel limit before
/// any allocation is attempted.
#[test]
fn apng_huge_canvas_is_rejected_by_default_limits() {
    let data = seed("huge-ihdr-rgba8-apng-2147483647sq.png");

    let err = zenpng::decode_apng(&data, &PngDecodeConfig::default(), &enough::Unstoppable)
        .expect_err("2^31-1 square exceeds the 120 MP default cap");
    assert!(
        matches!(err.error(), PngError::LimitExceeded(_)),
        "expected LimitExceeded, got {:?}",
        err.error()
    );
}
