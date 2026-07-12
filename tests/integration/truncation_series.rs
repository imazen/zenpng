//! EOF/truncation conformance: cutting a known-good PNG short must categorize as
//! *incomplete client input* — never panic, OOM, or surface as an internal (5xx)
//! error for what is a 4xx-class truncated request.
//!
//! Delegates to the zencodec-testkit [`check_decode_truncation_series`] check,
//! which builds a deterministic prefix series (header sizes + fractions) and runs
//! each through the dyn-erased full decode path, verifying the erased
//! [`ErrorCategory`] lands in the incomplete-input set.

use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{PngDecoderConfig, PngEncoderConfig};

/// Encode a tiny, known-good PNG through the zencodec trait encode path.
fn valid_png() -> Vec<u8> {
    let (w, h) = (8u32, 8u32);
    // RGB8, tightly packed. Content is irrelevant to the truncation check.
    let bytes = vec![0x77u8; (w * h * 3) as usize];
    let slice = PixelSlice::new(&bytes, w, h, (w * 3) as usize, PixelDescriptor::RGB8_SRGB)
        .expect("rgb8 slice");
    PngEncoderConfig::new()
        .job()
        .encoder()
        .expect("encoder")
        .encode(slice)
        .expect("encode")
        .into_vec()
}

#[test]
fn truncation_series_categorizes_as_incomplete_input() {
    let valid = valid_png();
    zencodec_testkit::check_decode_truncation_series(PngDecoderConfig::new(), &valid).expect(
        "truncated PNG must categorize as incomplete input (UnexpectedEof/MalformedImage), \
         never panic, OOM, or Internal",
    );
}
