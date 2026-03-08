#![no_main]

use libfuzzer_sys::fuzz_target;

/// APNG decode fuzzer: exercise the animated PNG decode path.
/// Tests frame compositing, disposal operations, and blending.
fuzz_target!(|data: &[u8]| {
    let config = zenpng::PngDecodeConfig::default();
    let _ = zenpng::decode_apng(data, &config, &enough::Unstoppable);
});
