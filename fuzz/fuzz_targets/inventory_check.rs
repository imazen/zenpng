use zencodec::decode::{DecodeJob, DecodePolicy, DecoderConfig};

#[allow(deprecated)] // Exercises the legacy best-effort policy setter on purpose.
pub fn check(input: &[u8]) {
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
        .expect("inventory never errors without a stop token or part overflow");
    assert_eq!(inv.input_len(), data.len() as u64);
    inv.validate().expect("inventory must validate");
}
