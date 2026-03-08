#![no_main]

use libfuzzer_sys::fuzz_target;

/// Primary PNG decode fuzzer: arbitrary bytes through the full decode pipeline.
/// Uses default (lenient) settings to maximize code coverage.
fuzz_target!(|data: &[u8]| {
    let config = zenpng::PngDecodeConfig::default();
    let _ = zenpng::decode(data, &config, &enough::Unstoppable);
});
