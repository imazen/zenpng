use std::borrow::Cow;
use zencodec::{
    animation::FrameDuration,
    decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig},
    encode::{AnimationFrameEncoder, EncodeJob, EncoderConfig},
};
use zenpixels::{Cicp, ColorSpace, PixelBuf, PixelFormat, Primaries, Transfer};

#[test]
fn exact_native_color_preference_never_converts_the_animation() {
    for (primaries, transfer) in [
        (Primaries::Bt709, Transfer::Linear),
        (Primaries::Bt2020, Transfer::Pq),
        (Primaries::Bt2020, Transfer::Hlg(zenpixels::HlgMode::Scene)),
    ] {
        let descriptor = PixelFormat::RGBA16_SRGB
            .with_primaries(primaries)
            .with_transfer(transfer);
        let color = Cicp::new(
            primaries.to_cicp().unwrap(),
            transfer.to_cicp().unwrap(),
            0,
            true,
        );
        let input = PixelBuf::from_bytes(
            (0..17 * 13 * 4)
                .flat_map(|i| ((i * 173 + 32769) as u16).to_ne_bytes())
                .collect(),
            17,
            13,
            descriptor.clone(),
        )
        .unwrap();
        let mut encoder = zenpng::PngEncoderConfig::new()
            .job()
            .animation_frame_encoder()
            .unwrap();
        for _ in 0..3 {
            encoder
                .push_frame_timed(
                    input.as_view(),
                    FrameDuration::new(1001, 30000).unwrap(),
                    None,
                )
                .unwrap();
        }
        let bytes = encoder.finish(None).unwrap();
        for preference in [vec![], vec![descriptor.clone()]] {
            let mut decoder = zenpng::PngDecoderConfig::new()
                .job()
                .animation_frame_decoder(Cow::Borrowed(bytes.data()), &preference)
                .unwrap();
            for _ in 0..3 {
                let frame = decoder.render_next_frame(None).unwrap().unwrap();
                let pixels = frame.pixels();
                assert_eq!(pixels.format(), &descriptor, "{transfer:?} {preference:?}");
                assert_eq!(pixels.format().color.to_cicp(), Some(color));
                for y in 0..13 {
                    assert_eq!(pixels.row(y), input.as_view().row(y));
                }
            }
        }
    }
}

#[test]
fn unrecognized_cicp_does_not_claim_srgb() {
    let input =
        PixelBuf::from_bytes(vec![127, 37, 83, 255], 1, 1, PixelFormat::RGBA8_SRGB).unwrap();
    let mut encoder = zenpng::PngEncoderConfig::new()
        .with_cicp(Some(Cicp::SRGB))
        .job()
        .animation_frame_encoder()
        .unwrap();
    for _ in 0..2 {
        encoder
            .push_frame_timed(input.as_view(), FrameDuration::new(1, 100).unwrap(), None)
            .unwrap();
    }
    let mut bytes = encoder.finish(None).unwrap().into_vec();
    let at = bytes.windows(4).position(|b| b == b"cICP").unwrap();
    bytes[at + 4] = 222;
    bytes[at + 5] = 223;
    // Decoder defaults skip ancillary CRCs; retain a valid chunk CRC anyway.
    let mut crc = !0u32;
    for byte in &bytes[at..at + 8] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bytes[at + 8..at + 12].copy_from_slice(&(!crc).to_be_bytes());
    let mut decoder = zenpng::PngDecoderConfig::new()
        .job()
        .animation_frame_decoder(Cow::Owned(bytes), &[])
        .unwrap();
    let frame = decoder.render_next_frame(None).unwrap().unwrap();
    assert!(matches!(
        frame.pixels().format().color,
        ColorSpace::Rgb {
            transfer: Transfer::Unknown,
            primaries: Primaries::Unknown,
            ..
        },
    ));
    assert_eq!(frame.pixels().row(0), &[127, 37, 83, 255]);
    assert_eq!(
        decoder.info().source_color.cicp,
        Some(Cicp::new(222, 223, 0, true))
    );
}
