//! Exact HDR mastering output through native and common writer routes.
use enough::Unstoppable;
use imgref::ImgRef;
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpixels::{ImageSpec, PixelView};
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

fn verify(bytes: &[u8], raw: [u8; 24], color: png::ColorType, frames: u32) {
    let mut chunks = &bytes[8..];
    let mut found = 0;
    while !chunks.is_empty() {
        let n = u32::from_be_bytes(chunks[..4].try_into().unwrap()) as usize;
        let payload = &chunks[8..8 + n];
        match &chunks[4..8] {
            b"cICP" => {
                assert_eq!(payload, [9, 16, 0, 1]);
                found |= 1;
            }
            b"mDCV" => {
                assert_eq!(payload, raw);
                found |= 2;
            }
            _ => {}
        }
        chunks = &chunks[12 + n..];
    }
    assert_eq!(found, 3);
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
    assert_eq!(
        oracle.info().animation_control.map_or(1, |a| a.num_frames),
        frames
    );
    let expected: &[u8] = if color == png::ColorType::Rgba {
        &[7, 19, 43, 255]
    } else {
        &[7, 19, 43]
    };
    for _ in 0..frames {
        let frame = oracle.next_frame(&mut pixels).unwrap();
        assert_eq!(frame.color_type, color);
        assert_eq!(frame.bit_depth, png::BitDepth::Eight);
        assert_eq!(
            pixels[..frame.buffer_size()],
            expected.repeat((frame.width * frame.height) as usize)
        );
    }
}

#[test]
fn raw_mastering_still_animation_and_row_routes() {
    let raw = payload();
    let cicp = zencodec::Cicp::BT2100_PQ_RGB;
    let pixels = [rgb::Rgb::new(7u8, 19, 43); 4];
    let rgba = [7u8, 19, 43, 255].repeat(4);
    let config = EncodeConfig::default()
        .with_compression(Compression::None)
        .with_downcast(DowncastFlags::none())
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
    verify(&still, raw, png::ColorType::Rgb, 1);
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
    verify(&animation, raw, png::ColorType::Rgba, 2);
    for effort in [0, 1, 7] {
        let cfg = zenpng::PngEncoderConfig::new()
            .with_compression(Compression::Effort(effort))
            .with_downcast(DowncastFlags::none())
            .with_cicp(Some(cicp))
            .with_mastering_display_raw(Some(raw));
        let mut encoder = cfg.job().with_canvas_size(2, 2).encoder().unwrap();
        for row in rgba.as_chunks::<8>().0 {
            encoder
                .push_rows(
                    PixelView::new(
                        row,
                        2,
                        1,
                        8,
                        ImageSpec::new(
                            zenpixels::SampleLayout::RGBA8,
                            zenpixels::Cicp::BT2100_PQ_RGB
                                .to_color_space(zenpixels::AlphaMode::Straight)
                                .unwrap(),
                        ),
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        verify(
            encoder.finish().unwrap().data(),
            raw,
            png::ColorType::Rgba,
            1,
        );
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

#[test]
fn checked_matrix_nine_retention_refuses_before_output() {
    let cfg = zenpng::PngEncoderConfig::new().with_compression(Compression::None);
    let pixels = [7, 19, 43];
    let spec = ImageSpec::new(
        zenpixels::SampleLayout::RGB8,
        zenpixels::Cicp::BT2100_PQ_RGB
            .to_color_space(zenpixels::AlphaMode::None)
            .unwrap(),
    );
    let write = |cicp, output: &mut Vec<u8>| -> Result<(), zencodec::encode::BoxedError> {
        let mut job = cfg.clone().job().with_canvas_size(1, 1);
        job.try_set_metadata_policy(
            zencodec::Metadata::none().with_cicp(cicp),
            zencodec::MetadataPolicy::PreserveExact,
        )?;
        let bytes = job
            .encoder()?
            .encode(PixelView::new(&pixels, 1, 1, 3, spec.clone())?)?;
        output.extend_from_slice(bytes.data());
        Ok(())
    };
    let mut output = Vec::new();
    let error = write(zenpixels::Cicp::BT2100_PQ, &mut output).unwrap_err();
    assert_eq!(
        error.downcast_ref::<zencodec::UnsupportedOperation>(),
        Some(&zencodec::UnsupportedOperation::MetadataRetention)
    );
    assert!(output.is_empty());
    write(zenpixels::Cicp::BT2100_PQ_RGB, &mut output).unwrap();
    assert!(output.starts_with(b"\x89PNG"));
}
