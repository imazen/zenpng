use std::borrow::Cow;
use std::io::{Cursor, Write as _};

use enough::Unstoppable;
use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpixels::{Cicp, ColorSpace, OriginKind, Primaries, Transfer};
use zenpixels_convert::icc_profiles::DISPLAY_P3_V4;
use zenpng::{Compression, DowncastFlags, PngDecodeConfig, PngDecoderConfig, PngEncoderConfig};

fn chunk(file: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    file.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let start = file.len();
    file.extend_from_slice(kind);
    file.extend_from_slice(payload);
    let mut crc = !0u32;
    for &byte in &file[start..] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    file.extend_from_slice(&(!crc).to_be_bytes());
}

fn compress(data: &[u8]) -> Vec<u8> {
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    zlib.write_all(data).unwrap();
    zlib.finish().unwrap()
}

fn fixture(kind: u8, bits: u8, row: &[u8], extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut hdr = [0; 13];
    hdr[3] = 1;
    hdr[7] = 1;
    hdr[8] = bits;
    hdr[9] = kind;
    chunk(&mut file, b"IHDR", &hdr);
    for &(tag, bytes) in extra {
        chunk(&mut file, tag, bytes);
    }
    let mut scan = vec![0];
    scan.extend_from_slice(row);
    chunk(&mut file, b"IDAT", &compress(&scan));
    chunk(&mut file, b"IEND", &[]);
    file
}

fn pixels_from_oracle(file: &[u8]) -> Vec<u8> {
    let mut reader = png::Decoder::new(Cursor::new(file)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(frame.buffer_size());
    pixels
}

fn custom_chrm() -> Vec<u8> {
    [31270u32, 32900, 67300, 31500, 25100, 69100, 14700, 6100]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect()
}

#[test]
fn legacy_gamma_and_chromaticities_preserve_code_values_and_wire_units() {
    let chrm = custom_chrm();
    let row = [128, 64, 32];
    for gamma in [45455u32, 55556] {
        let file = fixture(
            2,
            8,
            &row,
            &[(b"gAMA", &gamma.to_be_bytes()), (b"cHRM", &chrm)],
        );
        assert_eq!(pixels_from_oracle(&file), row);
        let native = zenpng::decode(&file, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(native.pixels.origin().kind, OriginKind::GamaChrm);
        let spec = native.pixels.spec();
        assert!(matches!(spec.color, ColorSpace::Rgb {
            transfer: Transfer::Gamma(g), primaries: Primaries::Custom(_), ..
        } if g == zenpixels::Gamma::from_encoding_gamma(gamma as f32 / 100_000.0).unwrap()));
        let decoded = PngDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(&file), &[])
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.pixels().spec(), spec);
        let cfg = PngEncoderConfig::new()
            .with_compression(Compression::None)
            .with_downcast(DowncastFlags::none());
        let output = cfg
            .clone()
            .job()
            .encoder()
            .unwrap()
            .encode(native.pixels.as_view())
            .unwrap();
        let mut rows = cfg.job().with_canvas_size(1, 1).encoder().unwrap();
        rows.begin(1, 1, spec.clone()).unwrap();
        rows.push_rows(native.pixels.as_view()).unwrap();
        let mut streamed = rows.pending_output().to_vec();
        let written = streamed.len();
        rows.consume_output(written);
        streamed.extend_from_slice(rows.finish().unwrap().data());
        for output in [output.data(), streamed.as_slice()] {
            assert_eq!(pixels_from_oracle(output), row);
            let info = zenpng::probe(output).unwrap();
            assert_eq!(info.source_gamma, Some(gamma));
            assert_eq!(info.chromaticities, native.info.chromaticities);
        }
    }
    let file = fixture(2, 8, &row, &[]);
    let native = zenpng::decode(&file, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
    assert_eq!(native.pixels.origin().kind, OriginKind::Assumed);
    assert_eq!(native.pixels.spec().color, ColorSpace::SRGB);
}

#[test]
fn color_chunk_precedence_retains_raw_companion() {
    let row = [128, 64, 32];
    let mut iccp = b"Display P3\0\0".to_vec();
    iccp.extend_from_slice(&compress(DISPLAY_P3_V4));
    let mut carriers: Vec<(&[u8; 4], &[u8])> = vec![(b"gAMA", &[0, 0, 0xd9, 4]), (b"sRGB", &[0])];
    let decode = |carriers: &[(&[u8; 4], &[u8])]| {
        let file = fixture(2, 8, &row, carriers);
        assert_eq!(pixels_from_oracle(&file), row);
        zenpng::decode(&file, &PngDecodeConfig::strict(), &Unstoppable).unwrap()
    };
    assert_eq!(decode(&carriers).pixels.spec().color, ColorSpace::SRGB);
    carriers.push((b"iCCP", &iccp));
    let icc = decode(&carriers);
    assert!(
        matches!(&icc.pixels.spec().color, ColorSpace::Icc { profile, .. } if profile.as_ref() == DISPLAY_P3_V4)
    );
    carriers.remove(1);
    carriers.push((b"cICP", &[12, 13, 0, 1]));
    let dual = decode(&carriers);
    assert_eq!(dual.pixels.spec().color, ColorSpace::DISPLAY_P3);
    assert_eq!(dual.pixels.origin().kind, OriginKind::IccAndCicp);
    assert_eq!(dual.pixels.origin().icc.as_deref(), Some(DISPLAY_P3_V4));
}

#[test]
fn native_p3_pq_hlg_and_gray_keep_samples_without_icc_synthesis() {
    let cll = [9_999_999u32, 123_456];
    let cll_wire: Vec<u8> = cll.into_iter().flat_map(u32::to_be_bytes).collect();
    let mdcv = [
        0x8a, 0x48, 0x39, 0x08, 0x21, 0x34, 0x9b, 0xaa, 0x19, 0x96, 0x08, 0xfc, 0x3d, 0x13, 0x40,
        0x42, 0, 0x98, 0x96, 0x80, 0, 0, 0, 0x32,
    ];
    for (kind, coded, cicp) in [
        (2, &[0, 0x80, 0x12, 0x34, 0xff, 0][..], [12, 13, 0, 1]),
        (2, &[0, 0x80, 0x12, 0x34, 0xff, 0][..], [9, 16, 0, 1]),
        (2, &[0, 0x80, 0x12, 0x34, 0xff, 0][..], [9, 18, 0, 1]),
        (0, &[0x12, 0x34][..], [1, 16, 0, 1]),
    ] {
        let file = fixture(
            kind,
            16,
            coded,
            &[(b"cICP", &cicp), (b"cLLI", &cll_wire), (b"mDCV", &mdcv)],
        );
        assert_eq!(pixels_from_oracle(&file), coded);
        let decoded = zenpng::decode(&file, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
        let transfer = Transfer::from_cicp(cicp[1]).unwrap();
        match decoded.pixels.spec().color {
            ColorSpace::Rgb {
                transfer: actual, ..
            }
            | ColorSpace::Gray {
                transfer: actual, ..
            } => assert_eq!(actual, transfer),
            ref other => panic!("unexpected {other:?}"),
        }
        assert_eq!(decoded.info.content_light_level_raw, Some(cll));
        assert_eq!(decoded.info.mastering_display_raw, Some(mdcv));
        assert!(decoded.pixels.hdr().mastering.is_some());
        let cfg = PngEncoderConfig::new()
            .with_compression(Compression::None)
            .with_downcast(DowncastFlags::none())
            .with_cicp(decoded.info.cicp)
            .with_content_light_level_raw(Some(cll))
            .with_mastering_display_raw(Some(mdcv));
        let encoded = cfg
            .job()
            .encoder()
            .unwrap()
            .encode(decoded.pixels.as_view())
            .unwrap();
        let info = zenpng::probe(encoded.data()).unwrap();
        assert_eq!(info.cicp, Some(Cicp::new(cicp[0], cicp[1], 0, true)));
        assert!(info.icc_profile.is_none());
        assert_eq!(info.content_light_level_raw, Some(cll));
        assert_eq!(info.mastering_display_raw, Some(mdcv));
        assert_eq!(pixels_from_oracle(encoded.data()), coded);
    }
}

#[test]
fn grayscale_icc_keeps_gray_profile_and_samples() {
    use zenpixels_convert::icc_profiles::SynthesizedIcc;
    let result = SynthesizedIcc::for_gray_cicp(Cicp::BT2100_PQ_RGB);
    let SynthesizedIcc::Profile(profile) = result else {
        panic!("the profile database must supply a gray PQ profile");
    };
    assert_eq!(
        zenpixels::icc::profile_color_space(&profile),
        Some(zenpixels::IccColorSpace::Gray)
    );
    let mut iccp = b"Gray PQ\0\0".to_vec();
    iccp.extend_from_slice(&compress(&profile));
    let row = [0x12, 0x34];
    let file = fixture(0, 16, &row, &[(b"iCCP", &iccp)]);
    let output = zenpng::decode(&file, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
    assert!(
        matches!(&output.pixels.spec().color, ColorSpace::Icc { profile: active, channels: 1, .. } if active.as_ref() == profile.as_ref())
    );
    let encoded = PngEncoderConfig::new()
        .with_compression(Compression::None)
        .with_downcast(DowncastFlags::none())
        .job()
        .encoder()
        .unwrap()
        .encode(output.pixels.as_view())
        .unwrap();
    assert_eq!(pixels_from_oracle(encoded.data()), row);
    assert_eq!(
        zenpng::probe(encoded.data())
            .unwrap()
            .icc_profile
            .as_deref(),
        Some(profile.as_ref())
    );
}
