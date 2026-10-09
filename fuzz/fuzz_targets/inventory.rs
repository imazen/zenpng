#![no_main]

use libfuzzer_sys::fuzz_target;
use zencodec::decode::{DecodeJob, DecodePolicy, DecoderConfig};

// Inventory fuzzer: the walker must not panic on any input, must return Ok, and
// its result must validate (full coverage, no overlaps, nesting).
//
// The first input byte picks the policy (strict CRC, ICC/EXIF/XMP bans, animation and
// progressive bans), so the policy-dependent dispositions run under the fuzzer too.
fuzz_target!(|input: &[u8]| {
    let (bits, data) = input.split_first().map_or((0, input), |(b, r)| (*b, r));
    let mut job = zenpng::PngDecoderConfig::new().job();
    if bits & 0x80 != 0 {
        job = job.with_policy(
            DecodePolicy::none()
                .with_strict(bits & 1 != 0)
                .with_allow_icc(bits & 2 == 0)
                .with_allow_exif(bits & 4 == 0)
                .with_allow_xmp(bits & 8 == 0)
                .with_allow_animation(bits & 16 == 0)
                .with_allow_progressive(bits & 32 == 0),
        );
    }
    let inv = job
        .inventory(data)
        .expect("inventory never errors without a stop token or part overflow")
        .expect("zenpng declares the inventory capability");
    assert_eq!(inv.input_len(), data.len() as u64);
    inv.validate().expect("inventory must validate");
});
