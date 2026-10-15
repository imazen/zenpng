//! Exact HDR mastering output through native and common writer routes.
use enough::Unstoppable;
use imgref::ImgRef;
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{ApngEncodeConfig, ApngFrameInput, Compression, DowncastFlags, EncodeConfig};

fn payload() -> [u8; 24] {
    let mut raw = [0; 24];
    for (dst, value) in raw[..16]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip([35401u16, 14599, 8501, 39849, 6549, 2301, 15636, 16449])
    {
        *dst = value.to_be_bytes();
    }
    raw[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    raw[20..].copy_from_slice(&1u32.to_be_bytes());
    raw
}

fn verify(bytes: &[u8], raw: [u8; 24]) {
    assert_eq!(
        zenpng::probe(bytes).unwrap().mastering_display_raw,
        Some(raw)
    );
    let mut oracle = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let m = oracle.info().mastering_display_color_volume.unwrap();
    assert_eq!(m.max_luminance, u32::MAX);
    assert_eq!(m.min_luminance, 1);
    let coordinates = [
        m.chromaticities.red,
        m.chromaticities.green,
        m.chromaticities.blue,
        m.chromaticities.white,
    ];
    for (pair, expected) in coordinates.iter().zip(raw[..16].as_chunks::<4>().0) {
        // image-png stores chromaticities in 1/100000 units, PNG mDCV uses 1/50000.
        assert_eq!(
            pair.0.into_scaled(),
            u32::from(u16::from_be_bytes([expected[0], expected[1]])) * 2
        );
        assert_eq!(
            pair.1.into_scaled(),
            u32::from(u16::from_be_bytes([expected[2], expected[3]])) * 2
        );
    }
    let mut pixels = vec![0; oracle.output_buffer_size().unwrap()];
    oracle.next_frame(&mut pixels).unwrap();
}

#[test]
fn raw_mastering_still_animation_and_row_routes() {
    let raw = payload();
    let cicp = zencodec::Cicp::BT2100_PQ;
    let pixels = [rgb::Rgb::new(7u8, 19, 43); 4];
    let rgba = [7u8, 19, 43, 255].repeat(4);
    let config = EncodeConfig::default()
        .with_compression(Compression::None)
        .with_cicp(Some(cicp))
        .with_mastering_display(Some(zenpixels::hdr::MasteringDisplay::HDR10_REFERENCE))
        .with_mastering_display_raw(Some(raw));
    let still = zenpng::encode_rgb8(
        ImgRef::new(&pixels, 2, 2),
        None,
        &config,
        &Unstoppable,
        &Unstoppable,
    )
    .unwrap();
    verify(&still, raw);
    let animation = zenpng::encode_apng(
        &[
            ApngFrameInput::new(&rgba, 1, 60),
            ApngFrameInput::new(&rgba, 1, 60),
        ],
        2,
        2,
        &ApngEncodeConfig::default().with_encode(config.clone()),
        None,
        &Unstoppable,
        &Unstoppable,
    )
    .unwrap();
    verify(&animation, raw);
    for effort in [0, 1, 7] {
        let cfg = zenpng::PngEncoderConfig::new()
            .with_compression(Compression::Effort(effort))
            .with_downcast(DowncastFlags::none())
            .with_cicp(Some(cicp))
            .with_mastering_display_raw(Some(raw));
        let mut encoder = cfg.job().with_canvas_size(2, 2).encoder().unwrap();
        for row in rgba.as_chunks::<8>().0 {
            encoder
                .push_rows(PixelSlice::new(row, 2, 1, 8, PixelDescriptor::RGBA8_SRGB).unwrap())
                .unwrap();
        }
        verify(encoder.finish().unwrap().data(), raw);
    }
    // Restoring the typed setter must explicitly remove the raw override.
    assert!(
        config
            .clone()
            .with_mastering_display(None)
            .mastering_display_raw
            .is_none()
    );
    let reset = config.clone().with_mastering_display_raw(None);
    let typed = zenpng::encode_rgb8(
        ImgRef::new(&pixels, 2, 2),
        None,
        &reset,
        &Unstoppable,
        &Unstoppable,
    )
    .unwrap();
    assert_ne!(
        zenpng::probe(&typed).unwrap().mastering_display_raw,
        Some(raw)
    );
    let no_cicp = zenpng::encode_rgb8(
        ImgRef::new(&pixels, 2, 2),
        None,
        &config.with_cicp(None),
        &Unstoppable,
        &Unstoppable,
    )
    .unwrap();
    assert!(
        zenpng::probe(&no_cicp)
            .unwrap()
            .mastering_display_raw
            .is_none()
    );
}
