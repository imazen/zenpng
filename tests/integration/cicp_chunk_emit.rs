//! `PngEncoderConfig::with_cicp` / `with_content_light_level` emit the PNG
//! `cICP` / `cLLI` chunks directly (cICP-only, no ICC synthesis) through the
//! trait `Encoder::encode(PixelView)` path — the byte-faithful,
//! PixelBuf-native route for HDR renditions (`RGB16_BT2100_PQ`/HLG)
//! where cICP is the canonical color signal.

use zencodec::Cicp;
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpixels::hdr::ContentLightLevel;
use zenpixels::{ImageSpec, PixelView};
use zenpng::{Compression, PngEncoderConfig};

/// Find a PNG chunk's payload by 4-byte type (IHDR is the first chunk after
/// the 8-byte signature; chunks are [len:4][type:4][data:len][crc:4]).
fn find_chunk<'a>(png: &'a [u8], typ: &[u8; 4]) -> Option<&'a [u8]> {
    let mut p = 8;
    while p + 12 <= png.len() {
        let len = u32::from_be_bytes(png[p..p + 4].try_into().unwrap()) as usize;
        if &png[p + 4..p + 8] == typ {
            return Some(&png[p + 8..p + 8 + len]);
        }
        p += 12 + len;
    }
    None
}

#[test]
fn with_cicp_and_cll_emit_chunks_through_trait_encode() {
    let (w, h) = (4u32, 2u32);
    // RGB16: 6 bytes/pixel, tightly packed. Content is irrelevant to chunks.
    let bytes = vec![0x11u8; (w * h * 6) as usize];
    let spec = ImageSpec::RGB16_SRGB
        .with_primaries(zenpixels::Primaries::DisplayP3)
        .with_transfer(zenpixels::Transfer::Pq);
    let slice = PixelView::new(&bytes, w, h, (w * 6) as usize, &spec).expect("rgb16 slice");

    let out = PngEncoderConfig::new()
        .with_compression(Compression::Fast)
        .with_cicp(Some(Cicp::new(12, 16, 0, true))) // Display P3 primaries, PQ transfer
        .with_content_light_level(Some(ContentLightLevel::new(500, 100)))
        .job()
        .encoder()
        .expect("encoder")
        .encode(slice)
        .expect("encode");
    let png = out.data();

    let cicp = find_chunk(png, b"cICP").expect("cICP chunk emitted from with_cicp");
    assert_eq!(
        cicp,
        &[12u8, 16, 0, 1],
        "cICP = primaries 12 / transfer 16 / matrix 0 / full-range 1"
    );
    assert!(
        find_chunk(png, b"cLLI").is_some(),
        "cLLI chunk emitted from with_content_light_level"
    );
    // cICP-only: no ICC synthesized onto this HDR rendition.
    assert!(
        find_chunk(png, b"iCCP").is_none(),
        "the builder cICP path must not synthesize an ICC"
    );
}

/// f32 input is stored as 8-bit sRGB, so a cICP that called the input linear
/// must leave with the sRGB transfer (keeping its primaries), whichever way
/// it was given and whichever path encodes.
#[test]
#[allow(deprecated)] // Exercises the published best-effort job setters.
fn a_linear_cicp_on_f32_input_is_written_as_srgb() {
    let (w, h) = (3u32, 2u32);
    let floats: Vec<f32> = (0..w * h * 3).map(|i| (i as f32) / 20.0).collect();
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_ne_bytes()).collect();
    let desc = ImageSpec::RGBF32_LINEAR.with_primaries(zenpixels::Primaries::DisplayP3);
    let slice = || PixelView::new(&bytes, w, h, (w * 12) as usize, &desc).unwrap();
    let p3_linear = Cicp::new(12, 8, 0, true);

    let via_config = PngEncoderConfig::new()
        .with_cicp(Some(p3_linear))
        .job()
        .encoder()
        .unwrap()
        .encode(slice())
        .unwrap();
    let via_metadata = PngEncoderConfig::new()
        .job()
        .with_metadata_policy(
            zencodec::Metadata::none().with_cicp(p3_linear),
            zencodec::MetadataPolicy::PreserveExact,
        )
        .encoder()
        .unwrap()
        .encode(slice())
        .unwrap();
    let mut streaming = PngEncoderConfig::new()
        .job()
        .with_metadata_policy(
            zencodec::Metadata::none().with_cicp(p3_linear),
            zencodec::MetadataPolicy::PreserveExact,
        )
        .encoder()
        .unwrap();
    streaming.push_rows(slice()).unwrap();
    let streamed = streaming.finish().unwrap();

    for (how, out) in [
        ("with_cicp", &via_config),
        ("metadata", &via_metadata),
        ("push_rows", &streamed),
    ] {
        assert_eq!(
            find_chunk(out.data(), b"cICP"),
            Some(&[12u8, 13, 0, 1][..]),
            "{how}: P3 primaries kept, transfer moved from linear to sRGB"
        );
    }
}

/// IHDR colour type (byte 9 of the IHDR payload): 0 grey, 2 RGB.
fn colour_type(png: &[u8]) -> u8 {
    find_chunk(png, b"IHDR").expect("IHDR")[9]
}

/// An RGB ICC profile can't describe a greyscale PNG, so grey-valued RGB
/// pixels written with one stay RGB and keep the profile.
#[test]
#[allow(deprecated)] // Exercises the published best-effort job setters.
fn grey_pixels_with_an_rgb_icc_stay_rgb() {
    let (w, h) = (4u32, 4u32);
    let bytes: Vec<u8> = (0..w * h).flat_map(|i| [(i * 9) as u8; 3]).collect();
    let slice =
        || PixelView::new(&bytes, w, h, (w * 3) as usize, &zenpixels::specs::RGB8_SRGB).unwrap();
    let icc = zenpixels_convert::icc_profiles::DISPLAY_P3_V4.to_vec();
    assert_eq!(&icc[16..20], b"RGB ");

    let plain = PngEncoderConfig::new()
        .job()
        .encoder()
        .unwrap()
        .encode(slice())
        .unwrap();
    assert_eq!(colour_type(plain.data()), 0, "no profile: reduced to grey");

    let with_icc = PngEncoderConfig::new()
        .job()
        .with_metadata_policy(
            zencodec::Metadata::none().with_icc(icc),
            zencodec::MetadataPolicy::PreserveExact,
        )
        .encoder()
        .unwrap()
        .encode(
            PixelView::from_bytes(
                &bytes,
                w,
                h,
                &ImageSpec::new(
                    zenpixels::SampleLayout::RGB8,
                    zenpixels::ColorSpace::DISPLAY_P3,
                ),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(colour_type(with_icc.data()), 2, "RGB profile: stays RGB");
    assert!(
        find_chunk(with_icc.data(), b"iCCP").is_some(),
        "profile kept"
    );
}

/// Translucent HDR through the permissive encode negotiator that `zencodecs`
/// uses: alpha, depth and color signal all survive, as an independent PNG
/// decoder reads them (issue #53).
#[test]
fn translucent_hdr_keeps_alpha_through_negotiation() {
    use zenpixels::{AlphaMode, ChannelOrder, Primaries, SampleType, Transfer};

    for (transfer, primaries, cicp) in [
        (Transfer::Pq, Primaries::Bt2020, [9u8, 16, 0, 1]),
        (
            Transfer::Hlg(zenpixels::HlgMode::Scene),
            Primaries::Bt2020,
            [9, 18, 0, 1],
        ),
        (Transfer::Pq, Primaries::DisplayP3, [12, 16, 0, 1]),
        (
            Transfer::Hlg(zenpixels::HlgMode::Scene),
            Primaries::DisplayP3,
            [12, 18, 0, 1],
        ),
    ] {
        let descriptor = ImageSpec::new(
            zenpixels::SampleLayout::new(SampleType::U16, ChannelOrder::Rgba),
            zenpixels::Cicp::new(cicp[0], cicp[1], cicp[2], cicp[3] == 1)
                .to_color_space(AlphaMode::Straight)
                .unwrap(),
        );
        assert!(matches!(descriptor.color, zenpixels::ColorSpace::Rgb {
            primaries: actual_primaries, transfer: actual_transfer, ..
        } if actual_primaries == primaries && actual_transfer == transfer));
        let (w, h) = (5u32, 3u32);
        let samples: Vec<u16> = (0..w * h)
            .flat_map(|i| {
                let v = i as u16;
                [
                    v * 1009,
                    60000 - v * 701,
                    v * 3,
                    if i % 2 == 0 { 32768 } else { 1 + v },
                ]
            })
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&samples);
        let stride = w as usize * 8;
        let supported = PngEncoderConfig::supported_descriptors();
        let policy = zenpixels_convert::ConvertPolicy::permissive()
            .with_alpha(zenpixels_convert::AlphaPolicy::DiscardUnchecked);
        let selected =
            zenpixels_convert::Negotiate::select(&descriptor, supported, &policy).unwrap();
        assert_eq!(selected.spec, descriptor, "{cicp:?}: negotiated away");
        let slice = PixelView::new(bytes, w, h, stride, &selected.spec).unwrap();

        let out = PngEncoderConfig::new()
            .with_compression(Compression::Fast)
            .job()
            .encoder()
            .unwrap()
            .encode(slice)
            .unwrap();
        let png_bytes = out.data();
        let ihdr = find_chunk(png_bytes, b"IHDR").unwrap();
        assert_eq!(
            (ihdr[8], ihdr[9]),
            (16, 6),
            "{cicp:?}: IHDR depth and color"
        );
        assert_eq!(find_chunk(png_bytes, b"cICP").unwrap(), &cicp[..]);

        let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
            .read_info()
            .unwrap();
        let mut decoded = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Sixteen);
        let read_back: Vec<u16> = decoded[..info.buffer_size()]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_be_bytes(*b))
            .collect();
        assert_eq!(read_back, samples, "{cicp:?}: samples");
    }
}
