#![no_main]

use libfuzzer_sys::fuzz_target;

/// Probe fuzzer: test lightweight metadata extraction without pixel decode.
/// Exercises chunk parsing, IHDR validation, and metadata extraction.
fuzz_target!(|data: &[u8]| {
    let _ = zenpng::probe(data);
});
