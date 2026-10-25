use std::borrow::Cow;
use zencodec::{
    CodecSet, ImageFormat, Resolution, ResolutionError, ResolutionUnit,
    animation::FrameDuration,
    decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig},
    encode::{AnimationFrameEncoder, DynEncoderConfig, EncodeJob, Encoder, EncoderConfig},
    operation::{Decision, EncodeRequest, Question},
};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{Compression, PngDecoderConfig, PngEncoderConfig};

const BYTES: [u8; 24] = [
    19, 83, 201, 23, 151, 43, 65, 97, 111, 0xff, 0xff, 0xff, 39, 63, 181, 43, 131, 63, 85, 77, 131,
    0xff, 0xff, 0xff,
];
fn pixels() -> PixelSlice<'static> {
    PixelSlice::new(&BYTES, 3, 2, 12, PixelDescriptor::RGB8_SRGB).unwrap()
}
fn resolution(x: f64, y: f64, unit: ResolutionUnit) -> Resolution {
    Resolution { x, y, unit }
}
fn assert_output(bytes: &[u8], expected: Resolution) {
    let info = PngDecoderConfig::new().probe(bytes).unwrap();
    assert!(expected.is_equivalent_to(&info.resolution.unwrap()));
    let output = PngDecoderConfig::new().decode(bytes).unwrap();
    for y in 0..2 {
        assert_eq!(output.pixels().row(y), pixels().row(y));
    }
}

#[test]
fn exact_density_survives_generic_erased_registered_and_row_paths() {
    let cases = [
        resolution(254.0, 127.0, ResolutionUnit::Inch),
        resolution(100.0, 50.0, ResolutionUnit::Centimeter),
        resolution(3780.0, 7559.0, ResolutionUnit::Meter),
        resolution(0.5, 0.25, ResolutionUnit::Unknown),
        resolution(f64::from(i32::MAX), 1.0, ResolutionUnit::Meter),
    ];
    for compression in [
        Compression::None,
        Compression::Fastest,
        Compression::Balanced,
    ] {
        let config = PngEncoderConfig::new().with_compression(compression);
        let set = CodecSet::new().with_send_encoder(config.clone());
        for expected in cases {
            let output = config
                .clone()
                .job()
                .try_with_resolution(expected)
                .unwrap()
                .encoder()
                .unwrap()
                .encode(pixels())
                .unwrap();
            assert_output(output.data(), expected);

            let mut job = config.dyn_job();
            job.try_set_resolution(expected).unwrap();
            let erased = job.encode(pixels()).unwrap();
            assert_eq!(output.data(), erased.data());

            let request = EncodeRequest::new(ImageFormat::Png).with_resolution(expected);
            let mut calls = 0;
            let registered = request
                .encode_with(&set, pixels(), &mut |question: Question<'_>| -> Result<
                    Decision,
                    zencodec::At<core::convert::Infallible>,
                > {
                    let Question::Encode { choices, .. } = question else {
                        panic!("wrong policy domain")
                    };
                    calls += 1;
                    let choice = choices.first().unwrap();
                    assert_eq!(choice.value().resolution, Some(expected));
                    Ok(Decision::choose(choice))
                })
                .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(output.data(), registered.data());
            let sent = request
                .prepare_send(&set)
                .unwrap()
                .pop()
                .unwrap()
                .into_job()
                .encode(pixels())
                .unwrap();
            assert_eq!(output.data(), sent.data());

            let mut rows = config
                .clone()
                .job()
                .try_with_resolution(expected)
                .unwrap()
                .encoder()
                .unwrap();
            rows.begin(3, 2, PixelDescriptor::RGB8_SRGB).unwrap();
            for y in 0..2 {
                rows.push_rows(
                    PixelSlice::new(
                        &BYTES[y * 12..y * 12 + 9],
                        3,
                        1,
                        9,
                        PixelDescriptor::RGB8_SRGB,
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            assert_output(rows.finish().unwrap().data(), expected);
        }
    }
}

#[test]
fn unrepresentable_density_refusal_preserves_the_previous_job_requirement() {
    let expected = resolution(254.0, 127.0, ResolutionUnit::Inch);
    for rejected in [
        resolution(300.0, 150.0, ResolutionUnit::Inch),
        resolution(f64::from(i32::MAX) + 1.0, 1.0, ResolutionUnit::Meter),
        resolution(1.0, 0.0, ResolutionUnit::Meter),
        resolution(f64::NAN, 1.0, ResolutionUnit::Meter),
    ] {
        let mut job = PngEncoderConfig::new()
            .job()
            .try_with_resolution(expected)
            .unwrap();
        let reason = job.try_set_resolution(rejected).unwrap_err();
        assert!(matches!(
            reason,
            ResolutionError::NotRepresentable | ResolutionError::InvalidValue
        ));
        assert_output(
            job.encoder().unwrap().encode(pixels()).unwrap().data(),
            expected,
        );
        let mut erased = PngEncoderConfig::new().dyn_job();
        erased.try_set_resolution(expected).unwrap();
        let failure = erased.try_set_resolution(rejected).unwrap_err();
        assert_eq!(
            zencodec::find_cause::<ResolutionError>(&*failure),
            Some(&reason)
        );
        assert_output(erased.encode(pixels()).unwrap().data(), expected);
    }
}

#[test]
fn animation_canvas_keeps_the_same_required_density_and_frame_pixels() {
    let expected = resolution(254.0, 127.0, ResolutionUnit::Inch);
    for streaming in [false, true] {
        let mut encoder = PngEncoderConfig::new()
            .job()
            .try_with_resolution(expected)
            .unwrap()
            .with_canvas_size(3, 2)
            .animation_frame_encoder()
            .unwrap();
        if streaming {
            encoder
                .begin(3, 2, PixelDescriptor::RGB8_SRGB, Some(2))
                .unwrap();
        }
        for _ in 0..2 {
            encoder
                .push_frame_timed(pixels(), FrameDuration::new(1, 10).unwrap(), None)
                .unwrap();
        }
        let output = encoder.finish(None).unwrap();
        let info = PngDecoderConfig::new().probe(output.data()).unwrap();
        assert!(expected.is_equivalent_to(&info.resolution.unwrap()));
        let mut decoder = PngDecoderConfig::new()
            .job()
            .animation_frame_decoder(Cow::Borrowed(output.data()), &[PixelDescriptor::RGB8_SRGB])
            .unwrap();
        for _ in 0..2 {
            let frame = decoder.render_next_frame(None).unwrap().unwrap();
            for y in 0..2 {
                assert_eq!(frame.pixels().row(y), pixels().row(y));
            }
        }
        assert!(decoder.render_next_frame(None).unwrap().is_none());
    }
}
