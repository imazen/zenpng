//! Native lossless encoding must carry compatible color descriptions.
#![cfg(feature = "cms")]
use std::{io::Cursor, sync::Arc};
use zencodec::encode::{EncodeDeclaration, EncodeJob as _, Encoder as _, EncoderConfig as _};
use zenpixels::{
    Cicp, ColorContext, ColorPrimaries, PixelDescriptor, PixelSlice, TransferFunction,
};
use zenpixels_convert::icc_profiles::SynthesizedIcc;
use zenpng::{Compression, DowncastFlags, PngEncoderConfig};

#[test]
fn native_hdr_carriers_preserve_strided_pixels_and_context_in_both_encoder_modes() {
    let layouts = [
        (PixelDescriptor::RGB8_SRGB, PixelDescriptor::RGB16_SRGB, 3),
        (PixelDescriptor::RGBA8_SRGB, PixelDescriptor::RGBA16_SRGB, 4),
        (PixelDescriptor::GRAY8_SRGB, PixelDescriptor::GRAY16_SRGB, 1),
        (
            PixelDescriptor::GRAYA8_SRGB,
            PixelDescriptor::GRAYA16_SRGB,
            2,
        ),
    ];
    for (eight, sixteen, channels) in layouts {
        for (base, depth) in [(eight, 1), (sixteen, 2)] {
            for transfer in [TransferFunction::Pq, TransferFunction::Hlg] {
                let cicp = Cicp::new(9, transfer.to_cicp().unwrap(), 0, true);
                let descriptor = base
                    .with_transfer(transfer)
                    .with_primaries(ColorPrimaries::Bt2020);
                let gray = channels <= 2;
                let profile = if gray {
                    SynthesizedIcc::for_gray_cicp(cicp)
                } else {
                    SynthesizedIcc::for_cicp(cicp)
                };
                let expected_icc = match profile {
                    SynthesizedIcc::Profile(bytes) => Some(bytes.into_owned()),
                    _ if transfer == TransferFunction::Hlg => None,
                    other => panic!("missing PQ companion: {other:?}"),
                };
                // Aligned sample backing with padding that cannot be mistaken for pixels.
                let stride = (3 + 2) * channels * depth;
                let mut words = vec![0xa5a5u16; stride * 2 / 2];
                let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut words);
                let mut expected = Vec::new();
                for y in 0..2 {
                    for x in 0..3 {
                        for c in 0..channels {
                            let value = [1u16, 32768, 65535][x];
                            let at = y * stride + (x * channels + c) * depth;
                            if depth == 2 {
                                bytes[at..at + 2].copy_from_slice(&value.to_ne_bytes());
                                expected.extend_from_slice(&value.to_be_bytes());
                            } else {
                                bytes[at] = (value >> 8) as u8;
                                expected.push(bytes[at]);
                            }
                        }
                    }
                }
                for context_kind in 0..3 {
                    if context_kind == 2 && expected_icc.is_none() {
                        continue;
                    }
                    let context = match context_kind {
                        0 => None,
                        1 => Some(Arc::new(ColorContext::from_cicp(cicp))),
                        _ => {
                            let mut context = ColorContext::from_icc(expected_icc.clone().unwrap());
                            context.cicp = Some(cicp);
                            Some(Arc::new(context))
                        }
                    };
                    for streaming in [false, true] {
                        let mut pixels = PixelSlice::new(bytes, 3, 2, stride, descriptor).unwrap();
                        if let Some(context) = &context {
                            pixels = pixels.with_color_context(context.clone());
                        }
                        let config = PngEncoderConfig::new()
                            .with_compression(Compression::Effort(0))
                            .with_downcast(DowncastFlags::none());
                        let mut encoder = config.job().encoder().unwrap();
                        let file = if streaming {
                            let mut declaration = EncodeDeclaration::new(3, 2, descriptor);
                            if let Some(context) = &context {
                                declaration = declaration.with_color(context);
                            }
                            encoder.begin_declared(&declaration).unwrap();
                            let mut output = Vec::new();
                            for y in 0..2 {
                                encoder.push_rows(pixels.sub_rows(y, 1)).unwrap();
                                encoder.take_output(&mut output);
                            }
                            output.extend_from_slice(encoder.finish().unwrap().data());
                            output
                        } else {
                            encoder.encode(pixels).unwrap().into_vec()
                        };
                        let mut reader = png::Decoder::new(Cursor::new(file)).read_info().unwrap();
                        assert_eq!(
                            reader.info().icc_profile.as_deref(),
                            expected_icc.as_deref()
                        );
                        let actual = reader
                            .info()
                            .coding_independent_code_points
                            .expect("native cICP");
                        assert_eq!(
                            (
                                actual.color_primaries,
                                actual.transfer_function,
                                actual.matrix_coefficients,
                                actual.is_video_full_range_image
                            ),
                            (9, cicp.transfer_characteristics, 0, true)
                        );
                        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
                        let frame = reader.next_frame(&mut decoded).unwrap();
                        assert_eq!(
                            &decoded[..frame.buffer_size()],
                            expected,
                            "{descriptor:?} context {context_kind} streaming {streaming}"
                        );
                    }
                }
            }
        }
    }
}
