#![no_main]

use libfuzzer_sys::fuzz_target;

/// Strict-mode decode fuzzer: enables checksum verification (Adler-32 + CRC-32).
/// Tests that checksum validation itself doesn't panic.
fuzz_target!(|data: &[u8]| {
    let config = zenpng::PngDecodeConfig::strict();
    let _ = zenpng::decode(data, &config, &enough::Unstoppable);
});
