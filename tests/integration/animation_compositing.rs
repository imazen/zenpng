//! Authored PNG containers with native zenpng-compressed pixels and hand-derived
//! displayed canvases. No decoder's output is used as the expected image.
use enough::Unstoppable;
use std::borrow::Cow;
use zencodec::decode::{AnimationFrameDecoder, DecodeJob, DecoderConfig};
use zenpixels::PixelDescriptor;
use zenpng::{DowncastFlags, EncodeConfig, PngDecoderConfig};

fn chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc = !0_u32;
    for &byte in kind.iter().chain(data) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    output.extend_from_slice(&(!crc).to_be_bytes());
}

fn png(width: usize, height: usize, color: [u16; 4], depth: u8) -> Vec<u8> {
    let config = EncodeConfig::default()
        .with_compression(zenpng::Compression::None)
        .with_downcast(DowncastFlags::none());
    if depth == 16 {
        let pixels = vec![rgb::Rgba::new(color[0], color[1], color[2], color[3]); width * height];
        zenpng::encode_rgba16(
            imgref::ImgRef::new(&pixels, width, height),
            None,
            &config,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap()
    } else {
        let pixels = vec![
            rgb::Rgba::new(
                color[0] as u8,
                color[1] as u8,
                color[2] as u8,
                color[3] as u8
            );
            width * height
        ];
        zenpng::encode_rgba8(
            imgref::ImgRef::new(&pixels, width, height),
            None,
            &config,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap()
    }
}

fn idat(png: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut offset = 8;
    while offset < png.len() {
        let len = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
        if &png[offset + 4..offset + 8] == b"IDAT" {
            bytes.extend_from_slice(&png[offset + 8..offset + 8 + len]);
        }
        offset += len + 12;
    }
    bytes
}

fn fixture(depth: u8, disposal: u8, excluded_default: bool) -> (Vec<u8>, Vec<Vec<u16>>) {
    let m = if depth == 16 { 65535 } else { 255 };
    let foreground_alpha = if depth == 16 { 32768 } else { 128 };
    let background = [m, 0, 0, m];
    let first = png(3, 2, background, depth);
    let mut bytes = first[..8].to_vec();
    chunk(&mut bytes, b"IHDR", &first[16..29]);
    let mut actl = 4_u32.to_be_bytes().to_vec();
    actl.extend_from_slice(&2_u32.to_be_bytes());
    chunk(&mut bytes, b"acTL", &actl);
    if excluded_default {
        chunk(&mut bytes, b"IDAT", &idat(&first));
    }
    let mut sequence = 0_u32;
    let frames = [
        (3, 2, 0, 0, background, 0, 0),
        (1, 1, 1, 0, [0, 0, m, foreground_alpha], disposal, 1),
        (1, 1, 0, 1, [0, m, 0, m], 0, 0),
        (1, 1, 0, 0, [0, 0, 0, 0], 0, 0),
    ];
    for (i, (w, h, x, y, color, dispose, blend)) in frames.into_iter().enumerate() {
        let mut fctl = Vec::new();
        for value in [sequence, w, h, x, y] {
            fctl.extend_from_slice(&value.to_be_bytes());
        }
        sequence += 1;
        fctl.extend_from_slice(&1001_u16.to_be_bytes());
        fctl.extend_from_slice(&30000_u16.to_be_bytes());
        fctl.extend_from_slice(&[dispose, blend]);
        chunk(&mut bytes, b"fcTL", &fctl);
        let encoded = idat(&png(w as usize, h as usize, color, depth));
        if i == 0 && !excluded_default {
            chunk(&mut bytes, b"IDAT", &encoded);
        } else {
            let mut data = sequence.to_be_bytes().to_vec();
            sequence += 1;
            data.extend_from_slice(&encoded);
            chunk(&mut bytes, b"fdAT", &data);
        }
    }
    chunk(&mut bytes, b"IEND", &[]);
    let mut canvas = background.repeat(6);
    let mut expected = vec![canvas.clone()];
    canvas[4..8].copy_from_slice(&[m - foreground_alpha, 0, foreground_alpha, m]);
    expected.push(canvas.clone());
    match disposal {
        0 => (),
        1 => canvas[4..8].fill(0),
        2 => canvas[4..8].copy_from_slice(&background),
        _ => unreachable!(),
    }
    canvas[12..16].copy_from_slice(&[0, m, 0, m]);
    expected.push(canvas.clone());
    canvas[..4].fill(0);
    expected.push(canvas);
    (bytes, expected)
}

#[test]
fn subframes_blend_dispose_and_skip_at_both_precisions() {
    for depth in [8, 16] {
        for disposal in [0, 1, 2] {
            for excluded_default in [false, true] {
                let (bytes, expected) = fixture(depth, disposal, excluded_default);
                let whole =
                    zenpng::decode_apng(&bytes, &zenpng::PngDecodeConfig::strict(), &Unstoppable)
                        .unwrap();
                for start in 0..4 {
                    let mut decoder = PngDecoderConfig::new()
                        .job()
                        .with_start_frame_index(start)
                        .animation_frame_decoder(Cow::Borrowed(&bytes), &[])
                        .unwrap();
                    for (index, expected_frame) in expected.iter().enumerate().skip(start as usize)
                    {
                        let frame = decoder.render_next_frame(None).unwrap().unwrap();
                        let pixels = frame.pixels();
                        assert_eq!((pixels.width(), pixels.rows()), (3, 2));
                        assert_eq!(
                            pixels.descriptor(),
                            if depth == 16 {
                                PixelDescriptor::RGBA16_SRGB
                            } else {
                                PixelDescriptor::RGBA8_SRGB
                            }
                        );
                        let actual: Vec<u16> = (0..2)
                            .flat_map(|y| {
                                if depth == 16 {
                                    pixels
                                        .row(y)
                                        .as_chunks::<2>()
                                        .0
                                        .iter()
                                        .map(|&b| u16::from_ne_bytes(b))
                                        .collect::<Vec<_>>()
                                } else {
                                    pixels.row(y).iter().map(|&v| u16::from(v)).collect()
                                }
                            })
                            .collect();
                        assert_eq!(
                            &actual, expected_frame,
                            "depth={depth} disposal={disposal} excluded={excluded_default} start={start} index={index}"
                        );
                        for y in 0..2 {
                            assert_eq!(pixels.row(y), whole.frames[index].pixels.as_slice().row(y));
                        }
                    }
                    assert!(decoder.render_next_frame(None).unwrap().is_none());
                }
            }
        }
    }
}
