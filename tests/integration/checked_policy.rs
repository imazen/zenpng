//! Checked color handoff: supplied ICC stays authoritative through every encoder.
use std::io::Cursor;
use zencodec::{
    ColorEmitPolicy, Metadata, MetadataPolicy, UnsupportedOperation,
    encode::{AnimationFrameEncoder, EncodeJob, EncodePolicy, Encoder, EncoderConfig},
};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{Compression, DowncastFlags, PngEncoderConfig};

fn config(effort: u32) -> PngEncoderConfig {
    PngEncoderConfig::new()
        .with_compression(Compression::Effort(effort))
        .with_downcast(DowncastFlags::none())
        .with_preserve_transparent_rgb(true)
}
fn metadata() -> Metadata {
    Metadata::none().with_icc(zenpixels_convert::icc_profiles::DISPLAY_P3_V4)
}
fn assert_carriers_and_pixels(bytes: &[u8], expected: &[u8]) {
    let native = zenpng::probe(bytes).unwrap();
    let icc = zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
    assert_eq!(native.icc_profile.as_deref(), Some(icc));
    assert!(native.cicp.is_none(), "derived cICP changes ICC authority");
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    assert_eq!(reader.info().icc_profile.as_deref(), Some(icc));
    assert!(reader.info().coding_independent_code_points.is_none());
    let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut decoded).unwrap();
    assert_eq!(frame.color_type, png::ColorType::Rgba);
    assert_eq!(&decoded[..frame.buffer_size()], expected);
}

#[test]
#[allow(deprecated)] // Exercises the published best-effort job setters.
fn checked_verbatim_preserves_authority_for_still_rows_and_animation() {
    let row0 = [
        7, 19, 43, 255, 33, 22, 11, 0, 9, 8, 7, 255, 0xee, 0xee, 0xee, 0xee,
    ];
    let row1 = [
        13, 29, 51, 255, 90, 80, 70, 255, 1, 2, 3, 255, 0xee, 0xee, 0xee, 0xee,
    ];
    let data = [row0, row1].concat();
    let expected = [&row0[..12], &row1[..12]].concat();
    let desc = PixelDescriptor::RGBA8_SRGB;
    let pixels = || PixelSlice::new(&data, 3, 2, 16, desc).unwrap();
    let policy = EncodePolicy::none().with_color(ColorEmitPolicy::Verbatim);
    for effort in [0, 1, 7] {
        let job = || {
            config(effort)
                .job()
                .with_metadata_policy(metadata(), MetadataPolicy::PreserveExact)
                .try_with_policy(policy)
                .unwrap()
        };
        let encoded = job().encoder().unwrap().encode(pixels()).unwrap();
        assert_carriers_and_pixels(encoded.data(), &expected);
        let mut rows = job().with_canvas_size(3, 2).encoder().unwrap();
        rows.begin(3, 2, desc).unwrap();
        for row in [&row0, &row1] {
            rows.push_rows(PixelSlice::new(row, 3, 1, 16, desc).unwrap())
                .unwrap();
        }
        assert_carriers_and_pixels(rows.finish().unwrap().data(), &expected);
        let mut animation = job()
            .with_canvas_size(3, 2)
            .animation_frame_encoder()
            .unwrap();
        animation.push_frame(pixels(), 20, None).unwrap();
        assert_carriers_and_pixels(animation.finish(None).unwrap().data(), &expected);
    }
}

#[test]
#[allow(deprecated)] // Exercises the published best-effort job setters.
fn refused_checked_policy_does_not_replace_the_accepted_policy() {
    let policy = EncodePolicy::none().with_color(ColorEmitPolicy::Verbatim);
    let mut job = config(1)
        .job()
        .with_metadata_policy(metadata(), MetadataPolicy::PreserveExact)
        .try_with_policy(policy)
        .unwrap();
    for refused in [
        policy.with_strict(true),
        policy.with_embed_icc(false),
        policy.with_embed_exif(false),
        policy.with_embed_xmp(false),
        EncodePolicy::none().with_color(ColorEmitPolicy::Balanced),
    ] {
        assert_eq!(
            job.try_set_policy(refused),
            Err(UnsupportedOperation::EncodePolicy)
        );
    }
    job.try_set_policy(EncodePolicy::none()).unwrap();
    let pixels = [7, 19, 43, 255];
    let output = job
        .encoder()
        .unwrap()
        .encode(PixelSlice::new(&pixels, 1, 1, 4, PixelDescriptor::RGBA8_SRGB).unwrap())
        .unwrap();
    assert_carriers_and_pixels(output.data(), &pixels);
}
