#![no_main]

use libfuzzer_sys::fuzz_target;

// Strict-mode decode fuzzer: enables checksum verification (Adler-32 + CRC-32).
// Tests that checksum validation itself doesn't panic.
//
// Starts from `default()` (120 MP / 4 GiB caps) and turns the checksums on,
// rather than from `strict()`, which carries **no** resource limits: with no
// cap, an IHDR advertising 2^31-1 × 2^31-1 asks the allocator for exabytes and
// ASan aborts with `allocation-size-too-big` before the decoder's fallible
// `try_reserve` can return `Err` (issue #19). The mirror of this config lives
// in `tests/fuzz_regression.rs` — keep the two in sync.
fuzz_target!(|data: &[u8]| {
    let config = zenpng::PngDecodeConfig::default()
        .with_skip_decompression_checksum(false)
        .with_skip_critical_chunk_crc(false);
    let _ = zenpng::decode(data, &config, &enough::Unstoppable);
});
