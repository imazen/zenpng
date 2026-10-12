use enough::Unstoppable;
use std::borrow::Cow;
use zencodec::animation::FrameDuration;
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, EncoderConfig};
use zenpixels::{PixelBuffer, PixelDescriptor};
use zenpng::{PngDecoderConfig, PngEncoderConfig};

fn pixels() -> PixelBuffer {
    PixelBuffer::from_vec(
        [10, 20, 30, 255].repeat(12),
        4,
        3,
        PixelDescriptor::RGBA8_SRGB,
    )
    .unwrap()
}
fn wire_delays(bytes: &[u8]) -> Vec<(u16, u16)> {
    let mut offset = 8;
    let mut result = Vec::new();
    while offset < bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if &bytes[offset + 4..offset + 8] == b"fcTL" {
            let data = &bytes[offset + 8..offset + 8 + length];
            result.push((
                u16::from_be_bytes(data[20..22].try_into().unwrap()),
                u16::from_be_bytes(data[22..24].try_into().unwrap()),
            ));
        }
        offset += length + 12;
    }
    result
}

#[test]
fn fractional_zero_and_long_delays_are_exact_on_wire_and_decode() {
    let pixels = pixels();
    let durations = [(1001, 30000), (1, 65535), (0, 1), (65535, 1), (70, 1)];
    let mut encoder = PngEncoderConfig::new()
        .with_compression(zenpng::Compression::None)
        .job()
        .with_loop_count(Some(2))
        .animation_frame_encoder()
        .unwrap();
    for (numerator, denominator) in durations {
        encoder
            .push_frame_timed(
                pixels.as_slice(),
                FrameDuration::new(numerator, denominator).unwrap(),
                None,
            )
            .unwrap();
    }
    let encoded = encoder.finish(None).unwrap();
    assert_eq!(
        wire_delays(encoded.data()),
        durations.map(|(n, d)| (n as u16, d as u16))
    );
    let mut decoder = PngDecoderConfig::new()
        .job()
        .animation_frame_decoder(
            Cow::Borrowed(encoded.data()),
            &[PixelDescriptor::RGBA8_SRGB],
        )
        .unwrap();
    assert_eq!(decoder.loop_count(), Some(2));
    for (n, d) in durations {
        let frame = decoder.render_next_frame(None).unwrap().unwrap();
        assert_eq!(frame.duration(), FrameDuration::new(n, d).unwrap());
        assert_eq!(frame.pixels().row(0), pixels.as_slice().row(0));
    }
    assert!(decoder.render_next_frame(None).unwrap().is_none());
}

#[test]
fn rejected_timing_and_dimensions_do_not_accept_frames() {
    let pixels = pixels();
    let mut encoder = PngEncoderConfig::new()
        .with_compression(zenpng::Compression::None)
        .job()
        .with_canvas_size(4, 3)
        .animation_frame_encoder()
        .unwrap();
    for duration in [
        FrameDuration::new(65536, 1).unwrap(),
        FrameDuration::new(1, 65536).unwrap(),
        FrameDuration::new(u64::MAX, 1).unwrap(),
    ] {
        assert!(
            encoder
                .push_frame_timed(pixels.as_slice(), duration, None)
                .is_err()
        );
    }
    let wrong = PixelBuffer::from_vec(vec![0; 4], 1, 1, PixelDescriptor::RGBA8_SRGB).unwrap();
    assert!(
        encoder
            .push_frame_timed(wrong.as_slice(), FrameDuration::from_millis(10), None)
            .is_err()
    );
    // Previously silently clipped to 65.535 seconds, though 70/1 fits APNG.
    encoder.push_frame(pixels.as_slice(), 70_000, None).unwrap();
    let encoded = encoder.finish(None).unwrap();
    assert_eq!(wire_delays(encoded.data()), vec![(70, 1)]);
}

#[test]
fn native_zero_denominator_means_centiseconds_without_rounding() {
    let pixels = pixels();
    let bytes = pixels.copy_to_contiguous_bytes();
    let frames = [
        zenpng::ApngFrameInput::new(&bytes, 7, 0),
        zenpng::ApngFrameInput::new(&bytes, 1, 30000),
    ];
    let config = zenpng::ApngEncodeConfig::default();
    let encoded =
        zenpng::encode_apng(&frames, 4, 3, &config, None, &Unstoppable, &Unstoppable).unwrap();
    let mut decoder = PngDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(&encoded), &[])
        .unwrap();
    assert_eq!(
        decoder.render_next_frame(None).unwrap().unwrap().duration(),
        FrameDuration::new(7, 100).unwrap()
    );
    assert_eq!(
        decoder.render_next_frame(None).unwrap().unwrap().duration(),
        FrameDuration::new(1, 30000).unwrap()
    );
}

#[test]
fn animation_observes_job_and_call_cancellation_during_frame_copy_and_finish() {
    use enough::{Stop, StopReason};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use zencodec::{CategorizedError, ErrorCategory, StopToken};
    struct Counter(Arc<AtomicUsize>, usize);
    impl Stop for Counter {
        fn check(&self) -> Result<(), StopReason> {
            if self.0.fetch_add(1, Ordering::Relaxed) >= self.1 {
                Err(StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }
    let pixels = pixels();
    for job_token in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let call = Counter(polls.clone(), 2);
        let mut job = PngEncoderConfig::new().job();
        if job_token {
            job = job.with_stop(StopToken::new(Counter(polls.clone(), 2)));
        }
        let mut enc = job.animation_frame_encoder().unwrap();
        let err = enc
            .push_frame(
                pixels.as_slice(),
                10,
                (!job_token).then_some(&call as &dyn Stop),
            )
            .unwrap_err();
        assert_eq!(
            err.category(),
            ErrorCategory::Stopped(StopReason::Cancelled)
        );
        assert!(
            polls.load(Ordering::Relaxed) >= 3,
            "must reach row-copy polling"
        );
    }
    let polls = Arc::new(AtomicUsize::new(0));
    let mut enc = PngEncoderConfig::new()
        .job()
        .with_stop(StopToken::new(Counter(polls.clone(), 20)))
        .animation_frame_encoder()
        .unwrap();
    enc.push_frame(pixels.as_slice(), 10, None).unwrap();
    polls.store(20, Ordering::Relaxed);
    assert_eq!(
        enc.finish(None).unwrap_err().category(),
        ErrorCategory::Stopped(StopReason::Cancelled)
    );
}

#[test]
fn limit_or_alpha_rejection_leaves_the_encoder_usable() {
    use zencodec::ResourceLimits;
    let source = pixels();
    let mut enc = PngEncoderConfig::new()
        .job()
        .with_limits(
            ResourceLimits::none()
                .with_max_pixels(12)
                .with_max_memory(48)
                .with_max_frames(1),
        )
        .animation_frame_encoder()
        .unwrap();
    let large = PixelBuffer::from_vec(vec![0; 52], 13, 1, PixelDescriptor::RGBA8_SRGB).unwrap();
    assert!(enc.push_frame(large.as_slice(), 10, None).is_err());
    let premultiplied = PixelBuffer::from_vec(
        vec![0; 48],
        4,
        3,
        PixelDescriptor::RGBA8_SRGB.with_alpha(Some(zenpixels::AlphaMode::Premultiplied)),
    )
    .unwrap();
    assert!(enc.push_frame(premultiplied.as_slice(), 10, None).is_err());
    enc.push_frame(source.as_slice(), 10, None).unwrap();
    assert!(enc.push_frame(source.as_slice(), 10, None).is_err());
    assert_eq!(
        wire_delays(enc.finish(None).unwrap().data()),
        vec![(1, 100)]
    );
}

#[test]
fn sixteen_bit_animation_preserves_all_sample_bits_and_color_signaling() {
    use zenpixels::{ColorPrimaries, TransferFunction};
    for transfer in [
        TransferFunction::Pq,
        TransferFunction::Hlg,
        TransferFunction::Linear,
    ] {
        let descriptor = PixelDescriptor::RGBA16_SRGB
            .with_transfer(transfer)
            .with_primaries(ColorPrimaries::Bt2020);
        let mut frames = Vec::new();
        for index in 0..3 {
            let values: Vec<u16> = (0..17 * 13 * 4)
                .map(|i| {
                    if i % 4 == 3 {
                        if i % 12 == 3 { 0 } else { 32767 }
                    } else {
                        (i * 271 + if index > 0 && i < 3 { 19 } else { 0 }) as u16
                    }
                })
                .collect();
            let bytes: Vec<_> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
            frames.push(PixelBuffer::from_vec(bytes, 17, 13, descriptor).unwrap());
        }
        let mut encoder = PngEncoderConfig::new()
            .job()
            .animation_frame_encoder()
            .unwrap();
        for pixels in &frames {
            encoder
                .push_frame_timed(
                    pixels.as_slice(),
                    FrameDuration::new(1001, 30000).unwrap(),
                    None,
                )
                .unwrap();
        }
        let output = encoder.finish(None).unwrap();
        assert_eq!(output.data()[24], 16, "IHDR sample depth");
        let mut decoder = PngDecoderConfig::new()
            .job()
            .animation_frame_decoder(Cow::Borrowed(output.data()), &[])
            .unwrap();
        for source in &frames {
            let frame = decoder.render_next_frame(None).unwrap().unwrap();
            assert_eq!(frame.pixels().descriptor(), descriptor);
            for y in 0..13 {
                assert_eq!(frame.pixels().row(y), source.as_slice().row(y));
            }
            assert_eq!(frame.duration(), FrameDuration::new(1001, 30000).unwrap());
        }
    }
}

#[test]
fn animation_inherits_frame_icc_and_enforces_decode_job_limits() {
    use std::sync::Arc;
    use zencodec::ResourceLimits;
    use zenpixels::ColorContext;
    let icc = zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
    let source = pixels().with_color_context(Arc::new(ColorContext::from_icc(icc)));
    let mut encoder = PngEncoderConfig::new()
        .job()
        .with_metadata_policy(
            zencodec::Metadata::none(),
            zencodec::MetadataPolicy::PreserveExact,
        )
        .animation_frame_encoder()
        .unwrap();
    encoder.push_frame(source.as_slice(), 10, None).unwrap();
    // A different source color context is not silently relabeled as the first.
    assert!(encoder.push_frame(pixels().as_slice(), 10, None).is_err());
    encoder.push_frame(source.as_slice(), 20, None).unwrap();
    let output = encoder.finish(None).unwrap();
    for limits in [
        ResourceLimits::none().with_max_frames(1),
        ResourceLimits::none().with_max_pixels(11),
        ResourceLimits::none().with_max_memory(47),
    ] {
        assert!(
            PngDecoderConfig::new()
                .job()
                .with_limits(limits)
                .animation_frame_decoder(Cow::Borrowed(output.data()), &[])
                .is_err()
        );
    }
    let mut decoder = PngDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Borrowed(output.data()), &[])
        .unwrap();
    let frame = decoder.render_next_frame(None).unwrap().unwrap();
    assert_eq!(
        frame.pixels().color_context().unwrap().icc.as_deref(),
        Some(icc)
    );
}
