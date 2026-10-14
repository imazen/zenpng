use std::borrow::Cow;
use zencodec::Metadata;
#[cfg(feature = "metadata-rewrite")]
use zencodec::MetadataPolicy;
use zencodec::animation::FrameDuration as Duration;
use zencodec::decode::{Decode, DecodeJob, DecodePolicy, DecoderConfig};
use zencodec::encode::{EncodeJob, EncodePolicy, EncoderConfig};
use zencodec_testkit as tk;
use zenpng::{PngDecoderConfig as D, PngEncoderConfig as E};

#[test]
fn shared_timing_boundaries_and_dispatch() {
    let d = |n, den| Duration::new(n, den).unwrap();
    let img = tk::TestImage::rgba8_gradient(16, 16);
    let cases = [
        (d(1, 65536), None),
        (d(1001, 30000), Some(d(1001, 30000))),
        (d(u64::MAX, 1), None),
        (d(0, 1), Some(d(0, 1))),
        (d(65535, 1), Some(d(65535, 1))),
        (d(65536, 1), None),
        (d(1, 24), Some(d(1, 24))),
    ];
    for exact in [true, false] {
        tk::check_animation_timing(E::new(), D::new(), img.as_slice(), exact, &cases).unwrap();
    }
}

#[test]
fn policy_and_inventory_contracts() {
    let img = tk::TestImage::rgb8_gradient(16, 16);
    let enc = E::new();
    let bytes = enc.clone().job().encode(img.as_slice()).unwrap();
    let expected = D::new()
        .job()
        .decoder(Cow::Borrowed(bytes.data()), &[])
        .unwrap()
        .decode()
        .unwrap()
        .metadata();
    for (policy, accepted) in [
        (EncodePolicy::none(), true),
        (EncodePolicy::strip_all(), false),
    ] {
        tk::check_encode_policy_admission(
            enc.clone(),
            D::new(),
            img.as_slice(),
            Metadata::none(),
            EncodePolicy::none(),
            policy,
            accepted,
            &expected,
        )
        .unwrap();
    }
    for (policy, accepted) in [
        (DecodePolicy::none(), true),
        (DecodePolicy::strict(), false),
    ] {
        tk::check_decode_policy_admission(
            D::new(),
            bytes.data(),
            DecodePolicy::none(),
            policy,
            accepted,
            &expected,
        )
        .unwrap();
    }
    tk::check_inventory_dispatch(D::new(), bytes.data()).unwrap();
    #[cfg(feature = "inventory")]
    tk::check_inventory_input_limit(D::new(), bytes.data()).unwrap();
}

#[cfg(feature = "metadata-rewrite")]
#[test]
fn rewrite_preserves_pixels_with_independent_png_reader() {
    let img = tk::TestImage::rgb8_gradient(16, 16);
    let bytes = E::new()
        .job()
        .with_metadata_policy(
            Metadata::none().with_xmp(b"<private>SECRET</private>".as_slice()),
            MetadataPolicy::PreserveExact,
        )
        .encode(img.as_slice())
        .unwrap();
    for (policy, keep) in [
        (MetadataPolicy::ColorAndRotation, false),
        (MetadataPolicy::PreserveExact, true),
    ] {
        tk::check_metadata_rewrite(
            zencodec::metadata_rewrite::Request::new(bytes.data()).with_policy(policy),
            zenpng::metadata::rewrite,
            |out| {
                let mut reader = png_main::Decoder::new(std::io::Cursor::new(out))
                    .read_info()
                    .unwrap();
                let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
                let info = reader.next_frame(&mut pixels).unwrap();
                assert_eq!(info.width, 16);
                assert_eq!(info.height, 16);
                assert_eq!(info.color_type, png_main::ColorType::Rgb);
                for y in 0..16 {
                    assert_eq!(&pixels[y * 48..(y + 1) * 48], img.as_slice().row(y as u32));
                }
                assert_eq!(out.windows(6).any(|w| w == b"SECRET"), keep);
                Ok(())
            },
        )
        .unwrap();
    }
}

#[test]
fn exact_timing_refusal_survives_erasure() {
    use zencodec::encode::AnimationFrameEncoder;
    use zencodec::{CodecErrorExt, UnsupportedOperation};
    let img = tk::TestImage::rgba8_gradient(16, 16);
    let mut encoder = E::new().job().animation_frame_encoder().unwrap();
    let err = encoder
        .push_frame_exact(img.as_slice(), Duration::new(1, 65537).unwrap(), None)
        .unwrap_err();
    let erased: Box<dyn std::error::Error + Send + Sync> = Box::new(err);
    assert_eq!(
        erased.unsupported_operation(),
        Some(&UnsupportedOperation::AnimationTiming)
    );
}
