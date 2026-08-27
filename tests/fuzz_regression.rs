//! Fuzz regression gate: every seed under `fuzz/regression/` is run through
//! each fuzz-target entry point with the same config the harness uses, on the
//! stable toolchain (no nightly / sanitizer needed).
//!
//! This is a separate test binary (not a `tests/integration.rs` submodule)
//! because `.github/workflows/fuzz.yml` invokes it by name:
//! `cargo test --test fuzz_regression`.

use std::path::PathBuf;

use zenpng::{PngDecodeConfig, PngError};

fn regression_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/regression")
}

fn regression_seeds() -> Vec<(String, Vec<u8>)> {
    let mut seeds: Vec<(String, Vec<u8>)> = std::fs::read_dir(regression_dir())
        .expect("fuzz/regression/ must exist")
        .map(|e| e.expect("readable dir entry").path())
        .filter(|p| p.is_file())
        .map(|p| {
            let bytes = std::fs::read(&p).expect("readable seed");
            (p.file_name().unwrap().to_string_lossy().into_owned(), bytes)
        })
        .collect();
    seeds.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!seeds.is_empty(), "fuzz/regression/ has no seeds");
    seeds
}

/// Mirror of `fuzz/fuzz_targets/fuzz_decode_strict.rs`: default caps + both
/// checksums verified. Keep in sync with the harness.
fn strict_harness_config() -> PngDecodeConfig {
    PngDecodeConfig::default()
        .with_skip_decompression_checksum(false)
        .with_skip_critical_chunk_crc(false)
}

/// Every seed, through every harness entry point, must return (Ok or Err)
/// without panicking or aborting.
#[test]
fn regression_seeds_survive_every_fuzz_entry_point() {
    for (name, data) in regression_seeds() {
        // fuzz_decode
        let _ = zenpng::decode(&data, &PngDecodeConfig::default(), &enough::Unstoppable);
        // fuzz_decode_strict
        let _ = zenpng::decode(&data, &strict_harness_config(), &enough::Unstoppable);
        // fuzz_decode_apng
        let _ = zenpng::decode_apng(&data, &PngDecodeConfig::default(), &enough::Unstoppable);
        // fuzz_probe
        let _ = zenpng::probe(&data);
        eprintln!("seed {name}: ok");
    }
}

/// Issue #19: `fuzz_decode_strict` used `PngDecodeConfig::strict()`, which has
/// no `max_pixels` / `max_memory_bytes`. A 2^31-1 × 2^31-1 IHDR then reached
/// the allocator with a multi-exabyte request; under ASan that is an
/// `allocation-size-too-big` abort, which fires even on the fallible
/// `try_reserve` path. The bounded harness config must reject the same input
/// at the pixel cap — `LimitExceeded`, never an allocation attempt.
#[test]
fn strict_harness_config_rejects_huge_ihdr_at_the_limit() {
    let seed = regression_dir().join("huge-ihdr-gray8-2147483647sq.png");
    let data = std::fs::read(&seed).expect("seed file present");

    let err = zenpng::decode(&data, &strict_harness_config(), &enough::Unstoppable)
        .expect_err("2^31-1 square image must be rejected");
    assert!(
        matches!(err.error(), PngError::LimitExceeded(_)),
        "expected LimitExceeded (rejected before allocation), got {:?}",
        err.error()
    );
}

/// Documents *why* the harness must not start from `strict()`: with no caps,
/// nothing short of the allocation machinery stops the same seed — a
/// graceful `OutOfMemory` here, an ASan abort under the fuzzer.
///
/// The expectation is the same on every pointer width, for different reasons:
/// on 64-bit the row buffers (2 GiB each) and the 4 GiB inflate buffer are
/// allocated and the ~4.6 EB full-image `try_reserve` fails; on 32-bit the
/// gray8 row (`2^31 - 1` bytes) is `isize::MAX` exactly, so the checked
/// two-row inflate capacity (`2^32`) is rejected before any allocation is
/// attempted. Either way it must be `OutOfMemory`, never a panic (this seed
/// previously tripped `attempt to multiply with overflow` on i686).
#[test]
fn unbounded_strict_config_reaches_the_allocator_for_huge_ihdr() {
    let seed = regression_dir().join("huge-ihdr-gray8-2147483647sq.png");
    let data = std::fs::read(&seed).expect("seed file present");

    let err = zenpng::decode(&data, &PngDecodeConfig::strict(), &enough::Unstoppable)
        .expect_err("exabyte allocation cannot succeed");
    assert!(
        matches!(err.error(), PngError::OutOfMemory(_)),
        "expected OutOfMemory (allocation attempted, no limit hit), got {:?}",
        err.error()
    );
}
