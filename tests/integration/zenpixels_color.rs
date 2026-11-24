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
    raster_fixture(kind, bits, 1, &[row], extra)
}

fn raster_fixture(
    kind: u8,
    bits: u8,
    width: u32,
    rows: &[&[u8]],
    extra: &[(&[u8; 4], &[u8])],
) -> Vec<u8> {
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut hdr = [0; 13];
    hdr[..4].copy_from_slice(&width.to_be_bytes());
    hdr[4..8].copy_from_slice(&(rows.len() as u32).to_be_bytes());
    hdr[8] = bits;
    hdr[9] = kind;
    chunk(&mut file, b"IHDR", &hdr);
    for &(tag, bytes) in extra {
        chunk(&mut file, tag, bytes);
    }
    let mut scan = Vec::new();
    for row in rows {
        scan.push(0);
        scan.extend_from_slice(row);
    }
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

#[test]
fn baking_hdr_orientation_preserves_color_and_raw_signaling() {
    use zencodec::Orientation;
    use zenpixels::{AlphaMode, ImageSpec, SampleLayout};

    let cicp = [9, 16, 0, 1];
    let cll = [9_999_999u32, 123_456];
    let cll_wire: Vec<_> = cll.into_iter().flat_map(u32::to_be_bytes).collect();
    let mdcv = [
        0x8a, 0x48, 0x39, 0x08, 0x21, 0x34, 0x9b, 0xaa, 0x19, 0x96, 0x08, 0xfc, 0x3d, 0x13, 0x40,
        0x42, 0, 0x98, 0x96, 0x80, 0, 0, 0, 0x32,
    ];
    // Big-endian TIFF, one SHORT orientation entry with value 6 (Rotate90).
    let exif = b"MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    let a = [0x0001u16, 0x0002, 0x0003];
    let b = [0x1234, 0x2345, 0x3456];
    let c = [0x8000, 0x0004, 0x5432];
    let d = [0x0034, 0x7676, 0xff00];
    let e = [0xfffe, 0x6000, 0x0002];
    let f = [0x0067, 0x7878, 0x9a0b];
    let wire = |pixels: &[[u16; 3]]| {
        pixels
            .iter()
            .flatten()
            .flat_map(|value| value.to_be_bytes())
            .collect::<Vec<_>>()
    };
    let file = raster_fixture(
        2,
        16,
        2,
        &[&wire(&[a, b]), &wire(&[c, d]), &wire(&[e, f])],
        &[
            (b"cICP", &cicp),
            (b"cLLI", &cll_wire),
            (b"mDCV", &mdcv),
            (b"eXIf", exif),
        ],
    );
    assert_eq!(pixels_from_oracle(&file), wire(&[a, b, c, d, e, f]));
    let decode = |file: &[u8]| {
        PngDecoderConfig::new()
            .job()
            .decoder(Cow::Borrowed(file), &[])
            .unwrap()
            .decode()
            .unwrap()
    };
    let decoded = decode(&file);
    assert_eq!(decoded.info().orientation, Orientation::Rotate90);
    let spec = ImageSpec::new(
        SampleLayout::RGB16,
        Cicp::BT2100_PQ_RGB.to_color_space(AlphaMode::None).unwrap(),
    );
    let pixels = decoded.pixels();
    assert_eq!(pixels.spec(), &spec);
    let origin = pixels.origin().unwrap().clone();
    assert_eq!(origin.kind, OriginKind::Cicp);
    assert_eq!(origin.cicp, Some(Cicp::BT2100_PQ_RGB));
    let hdr = pixels.hdr();
    assert!(hdr.cll.is_some() && hdr.mastering.is_some());
    let facts = pixels.facts();
    assert!(facts.depth.is_some());

    let oriented =
        zenpixels_convert::apply_orientation(&pixels, decoded.info().orientation).unwrap();
    assert_eq!((oriented.width(), oriented.height()), (3, 2));
    assert_eq!(oriented.spec(), &spec);
    assert_eq!(oriented.origin(), &origin);
    assert_eq!(oriented.hdr(), hdr);
    assert_eq!(oriented.facts(), facts);
    let row_words = |row: &[u8]| {
        row.as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_ne_bytes)
            .collect::<Vec<_>>()
    };
    assert_eq!(row_words(oriented.as_view().row(0)), [e, c, a].concat());
    assert_eq!(row_words(oriented.as_view().row(1)), [f, d, b].concat());

    let raw = zenpng::probe(&file).unwrap();
    assert_eq!(raw.content_light_level_raw, Some(cll));
    assert_eq!(raw.mastering_display_raw, Some(mdcv));
    // Native builders carry the exact PNG integers. The already baked
    // orientation is represented by the absence of eXIf in the new file.
    let encoded = PngEncoderConfig::new()
        .with_compression(Compression::None)
        .with_downcast(DowncastFlags::none())
        .with_content_light_level_raw(raw.content_light_level_raw)
        .with_mastering_display_raw(raw.mastering_display_raw)
        .job()
        .encoder()
        .unwrap()
        .encode(oriented.as_view())
        .unwrap();
    let output = encoded.data();
    assert_eq!(pixels_from_oracle(output), wire(&[e, c, a, f, d, b]));
    let oracle = png::Decoder::new(Cursor::new(output)).read_info().unwrap();
    assert_eq!((oracle.info().width, oracle.info().height), (3, 2));
    assert!(oracle.info().exif_metadata.is_none());
    let mut chunks = &output[8..];
    let mut found = 0;
    while !chunks.is_empty() {
        let n = u32::from_be_bytes(chunks[..4].try_into().unwrap()) as usize;
        let payload = &chunks[8..8 + n];
        match &chunks[4..8] {
            b"cICP" => {
                assert_eq!(payload, cicp);
                found |= 1;
            }
            b"cLLI" => {
                assert_eq!(payload, cll_wire);
                found |= 2;
            }
            b"mDCV" => {
                assert_eq!(payload, mdcv);
                found |= 4;
            }
            _ => {}
        }
        chunks = &chunks[12 + n..];
    }
    assert_eq!(found, 7);
    let roundtrip = decode(output);
    assert_eq!(roundtrip.info().orientation, Orientation::Identity);
    assert_eq!(roundtrip.pixels().spec(), &spec);
    assert_eq!(roundtrip.pixels().origin(), Some(&origin));
    assert_eq!(roundtrip.pixels().hdr(), hdr);
}

#[test]
fn exact_cicp_retention_keeps_the_original_transfer_code() {
    use zenpixels::{ImageSpec, PixelView};

    // Transfer::Bt709 projects back to code 1, including when the source
    // signaled code 14. Exact retention must carry the original code.
    let source = Cicp::new(9, 14, 0, true);
    let spec = ImageSpec::rgb8_srgb()
        .with_primaries(Primaries::Bt2020)
        .with_transfer(Transfer::Bt709);
    let samples = [7, 19, 43];
    let encoded = PngEncoderConfig::new()
        .with_compression(Compression::None)
        .with_downcast(DowncastFlags::none())
        .job()
        .try_with_metadata_policy(
            zencodec::Metadata::none().with_cicp(source),
            zencodec::MetadataPolicy::PreserveExact,
        )
        .unwrap()
        .encoder()
        .unwrap()
        .encode(PixelView::from_packed(&samples, 1, 1, spec).unwrap())
        .unwrap();
    let oracle = png::Decoder::new(Cursor::new(encoded.data()))
        .read_info()
        .unwrap();
    assert_eq!(
        oracle
            .info()
            .coding_independent_code_points
            .unwrap()
            .transfer_function,
        14
    );
    assert_eq!(pixels_from_oracle(encoded.data()), samples);
}

#[test]
fn gray_pq_metadata_gets_a_gray_companion_profile() {
    use zencodec::encode::EncodePolicy;
    use zenpixels::{ImageSpec, PixelView};

    let coded = [0x1234u16, 0x4321];
    let spec = ImageSpec::gray16_srgb().with_transfer(Transfer::Pq);
    #[allow(deprecated)] // Balanced ICC emission is a legacy policy option.
    let job = PngEncoderConfig::new()
        .with_compression(Compression::None)
        .with_downcast(DowncastFlags::none())
        .job()
        .with_policy(EncodePolicy::none().with_color(zencodec::ColorEmitPolicy::Balanced))
        .with_metadata(zencodec::Metadata::none().with_cicp(Cicp::new(1, 16, 0, true)));
    let output = job
        .encoder()
        .unwrap()
        .encode(PixelView::from_packed(bytemuck::cast_slice(&coded), 2, 1, spec).unwrap())
        .unwrap();
    let oracle = png::Decoder::new(Cursor::new(output.data()))
        .read_info()
        .unwrap();
    assert_eq!(oracle.info().color_type, png::ColorType::Grayscale);
    let icc = oracle.info().icc_profile.as_deref().unwrap();
    assert_eq!(&icc[12..20], b"mntrGRAY");
    assert_eq!(pixels_from_oracle(output.data()), [0x12, 0x34, 0x43, 0x21]);
}

#[test]
fn malformed_cicp_range_flag_does_not_become_active_color() {
    let row = [7, 19, 43];
    let file = fixture(2, 8, &row, &[(b"cICP", &[9, 16, 0, 2])]);
    let decoded = zenpng::decode(&file, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
    assert_eq!(decoded.info.cicp, None);
    assert_eq!(decoded.pixels.spec().color, ColorSpace::SRGB);
    assert_eq!(decoded.pixels.as_view().row(0), row);
}
