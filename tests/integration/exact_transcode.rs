//! Exact transparent samples and explicit ICC authority for codec consumers.
use std::io::Cursor;
use zencodec::encode::{AnimationFrameEncoder, EncodeJob, Encoder, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{Compression, DowncastFlags, PngEncoderConfig};

fn config(effort: u32) -> PngEncoderConfig {
    PngEncoderConfig::new()
        .with_compression(Compression::Effort(effort))
        .with_downcast(DowncastFlags::none())
        .with_preserve_transparent_rgb(true)
}

fn independent(bytes: &[u8]) -> Vec<u8> {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut out = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut out).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    out.truncate(info.buffer_size());
    out
}

#[test]
fn hidden_rgb_survives_buffered_strip_and_segmented_output() {
    let (w, h) = (1024u32, 600u32); // Multiple strips and eligible iDOT output.
    let stride = w as usize * 4 + 12;
    let mut padded = vec![0xee; stride * h as usize];
    let mut expected = Vec::new();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let pixel = [
                (x % 251) as u8,
                (y % 253) as u8,
                79,
                if x % 3 == 0 { 0 } else { 255 },
            ];
            padded[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&pixel);
            expected.extend_from_slice(&pixel);
        }
    }
    for effort in [1, 7] {
        let pixels = PixelSlice::new(&padded, w, h, stride, PixelDescriptor::RGBA8_SRGB).unwrap();
        let out = config(effort)
            .job()
            .encoder()
            .unwrap()
            .encode(pixels)
            .unwrap();
        assert_eq!(independent(out.data()), expected);
        let mut rows = config(effort)
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for row in padded.chunks_exact(stride) {
            rows.push_rows(
                PixelSlice::new(row, w, 1, stride, PixelDescriptor::RGBA8_SRGB).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(independent(rows.finish().unwrap().data()), expected);
    }
    // Native segmented writer shares the same exact compression flag.
    let typed: Vec<rgb::Rgba<u8>> = expected
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| rgb::Rgba::new(p[0], p[1], p[2], p[3]))
        .collect();
    let bytes = zenpng::encode_rgba8(
        imgref::ImgRef::new(&typed, w as usize, h as usize),
        None,
        &zenpng::EncodeConfig::default()
            .with_compression(Compression::Fastest)
            .with_downcast(DowncastFlags::none())
            .with_decode_segments(4)
            .with_preserve_transparent_rgb(true),
        &enough::Unstoppable,
        &enough::Unstoppable,
    )
    .unwrap();
    let mut offset = 8;
    let mut idot = false;
    while offset < bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        idot |= &bytes[offset + 4..offset + 8] == b"iDOT";
        offset += length + 12;
    }
    assert!(idot);
    assert_eq!(independent(&bytes), expected);
}

#[test]
fn hidden_rgb_animation_uses_exact_source_deltas() {
    let (w, h) = (9u32, 5u32);
    let first = [17, 29, 43, 0].repeat((w * h) as usize);
    let mut second = first.clone();
    second[4 * 13] = 99; // Hidden RGB is a real byte difference.
    second[4 * 21 + 3] = 128;
    let frames = [first, second.clone(), second];
    let mut encoder = config(7).job().animation_frame_encoder().unwrap();
    for frame in &frames {
        encoder
            .push_frame(
                PixelSlice::new(frame, w, h, w as usize * 4, PixelDescriptor::RGBA8_SRGB).unwrap(),
                17,
                None,
            )
            .unwrap();
    }
    let out = encoder.finish(None).unwrap();
    let mut oracle = png::Decoder::new(Cursor::new(out.data()))
        .read_info()
        .unwrap();
    let mut canvas = vec![0; (w * h * 4) as usize];
    for expected in frames {
        let mut raw = vec![0; oracle.output_buffer_size().unwrap()];
        let frame = oracle.next_frame(&mut raw).unwrap();
        assert_eq!(frame.color_type, png::ColorType::Rgba);
        let ctrl = oracle.info().frame_control.unwrap();
        assert_eq!(ctrl.blend_op, png::BlendOp::Source);
        assert_eq!(ctrl.dispose_op, png::DisposeOp::None);
        for y in 0..ctrl.height as usize {
            let dst = ((ctrl.y_offset as usize + y) * w as usize + ctrl.x_offset as usize) * 4;
            let src = y * ctrl.width as usize * 4;
            canvas[dst..dst + ctrl.width as usize * 4]
                .copy_from_slice(&raw[src..src + ctrl.width as usize * 4]);
        }
        assert_eq!(canvas, expected);
    }
}

#[test]
#[allow(deprecated)] // Exercises the legacy best-effort policy setter on purpose.
fn explicit_icc_is_not_overridden_by_descriptor_derived_cicp() {
    let icc = zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
    let meta = zencodec::Metadata::none().with_icc(icc);
    let mut desc = PixelDescriptor::RGB8_SRGB;
    desc.primaries = zenpixels::ColorPrimaries::Bt2020;
    let px = [7u8, 19, 43];
    let out = config(1)
        .job()
        .with_metadata_policy(meta, zencodec::MetadataPolicy::PreserveExact)
        .with_policy(
            zencodec::encode::EncodePolicy::none().with_color(zencodec::ColorEmitPolicy::Verbatim),
        )
        .encoder()
        .unwrap()
        .encode(PixelSlice::new(&px, 1, 1, 3, desc).unwrap())
        .unwrap();
    let info = zenpng::probe(out.data()).unwrap();
    assert_eq!(info.icc_profile.as_deref(), Some(icc));
    assert!(info.cicp.is_none());
    let oracle = png::Decoder::new(Cursor::new(out.data()))
        .read_info()
        .unwrap();
    assert_eq!(oracle.info().icc_profile.as_deref(), Some(icc));
    assert!(oracle.info().coding_independent_code_points.is_none());
}

#[test]
fn gray_alpha_native_depth_and_strided_rows() {
    for descriptor in [PixelDescriptor::GRAYA8_SRGB, PixelDescriptor::GRAYA16_SRGB] {
        assert!(PngEncoderConfig::supported_descriptors().contains(&descriptor));
        let (w, h) = (1024u32, 600u32);
        let bytes = descriptor.bytes_per_pixel();
        let row_bytes = w as usize * bytes;
        let stride = row_bytes + 12;
        let mut padded = vec![0xee; stride * h as usize];
        let mut expected = Vec::new();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let at = y * stride + x * bytes;
                if bytes == 2 {
                    let pair = [(x % 251) as u8, if x % 3 == 0 { 0 } else { 255 }];
                    padded[at..at + 2].copy_from_slice(&pair);
                    expected.extend_from_slice(&pair);
                } else {
                    // Alpha with a zero low byte must not be mistaken for RGBA8.
                    for (c, v) in [(x * 71 + y) as u16, 0xff00u16].into_iter().enumerate() {
                        padded[at + c * 2..at + c * 2 + 2].copy_from_slice(&v.to_ne_bytes());
                        expected.extend_from_slice(&v.to_be_bytes());
                    }
                }
            }
        }
        for effort in [0, 1, 7] {
            let pixels = PixelSlice::new(&padded, w, h, stride, descriptor).unwrap();
            let whole = config(effort)
                .job()
                .encoder()
                .unwrap()
                .encode(pixels)
                .unwrap();
            let mut rows = config(effort)
                .job()
                .with_canvas_size(w, h)
                .encoder()
                .unwrap();
            for row in padded.chunks_exact(stride) {
                rows.push_rows(PixelSlice::new(row, w, 1, stride, descriptor).unwrap())
                    .unwrap();
            }
            for output in [whole, rows.finish().unwrap()] {
                let mut reader = png::Decoder::new(Cursor::new(output.data()))
                    .read_info()
                    .unwrap();
                let mut raw = vec![0; reader.output_buffer_size().unwrap()];
                let info = reader.next_frame(&mut raw).unwrap();
                assert_eq!(info.color_type, png::ColorType::GrayscaleAlpha);
                assert_eq!(info.bit_depth as usize, bytes * 4);
                assert_eq!(&raw[..info.buffer_size()], expected);
            }
        }
    }
}
