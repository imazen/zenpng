//! Preserve PNG's full cLLI range through the native API and check it independently.
use enough::Unstoppable;
use imgref::ImgRef;
use zenpng::{ApngEncodeConfig, ApngFrameInput, Compression, EncodeConfig, PngDecodeConfig};

#[test]
fn exact_content_light_static_and_animation() {
    let pixels = [rgb::Rgb::new(7u8, 19, 43); 4];
    let rgba = [7u8, 19, 43, 255].repeat(4);
    for raw in [
        [0, 0],
        [1, 1],
        [10_000_000, 2_500_000],
        [u32::MAX, 655_350_001],
    ] {
        let config = EncodeConfig::default()
            .with_compression(Compression::None)
            .with_content_light_level(Some(zencodec::ContentLightLevel::new(42, 7)))
            .with_content_light_level_raw(Some(raw));
        let still = zenpng::encode_rgb8(
            ImgRef::new(&pixels, 2, 2),
            None,
            &config,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let anim = zenpng::encode_apng(
            &[
                ApngFrameInput::new(&rgba, 1, 10),
                ApngFrameInput::new(&rgba, 2, 10),
            ],
            2,
            2,
            &ApngEncodeConfig::default().with_encode(config.clone()),
            None,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        for bytes in [&still, &anim] {
            let info = zenpng::probe(bytes).unwrap();
            assert_eq!(info.content_light_level_raw, Some(raw));
            let decoded = zenpng::decode(bytes, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
            assert_eq!(decoded.info.content_light_level_raw, Some(raw));
            let decoded =
                zenpng::decode_apng(bytes, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
            assert_eq!(decoded.info.content_light_level_raw, Some(raw));
            let mut oracle = png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .unwrap();
            let got = oracle.info().content_light_level.as_ref().unwrap();
            assert_eq!(
                [
                    got.max_content_light_level,
                    got.max_frame_average_light_level
                ],
                raw
            );
            let mut out = vec![0; oracle.output_buffer_size().unwrap()];
            oracle.next_frame(&mut out).unwrap();
        }
        let expected = zencodec::ContentLightLevel::new(
            (raw[0] / 10_000).min(65535) as u16,
            (raw[1] / 10_000).min(65535) as u16,
        );
        assert_eq!(
            zenpng::probe(&still).unwrap().content_light_level,
            Some(expected)
        );
        // Existing setter explicitly returns to whole-nit input.
        let reset =
            config.with_content_light_level(Some(zencodec::ContentLightLevel::new(1000, 250)));
        let bytes = zenpng::encode_rgb8(
            ImgRef::new(&pixels, 2, 2),
            None,
            &reset,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        assert_eq!(
            zenpng::probe(&bytes).unwrap().content_light_level_raw,
            Some([10_000_000, 2_500_000])
        );
    }
}
