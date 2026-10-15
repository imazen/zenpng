//! `PngEncoderConfig::with_cicp` / `with_content_light_level` emit the PNG
//! `cICP` / `cLLI` chunks directly (cICP-only, no ICC synthesis) through the
//! trait `Encoder::encode(PixelSlice)` path — the byte-faithful,
//! PixelBuffer-native route for HDR renditions (`RGB16_BT2100_PQ`/HLG)
//! where cICP is the canonical color signal.

use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::{Cicp, ContentLightLevel};
use zenpixels::{PixelDescriptor, PixelSlice};
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
    let slice = PixelSlice::new(&bytes, w, h, (w * 6) as usize, PixelDescriptor::RGB16_SRGB)
        .expect("rgb16 slice");

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
fn a_linear_cicp_on_f32_input_is_written_as_srgb() {
    let (w, h) = (3u32, 2u32);
    let floats: Vec<f32> = (0..w * h * 3).map(|i| (i as f32) / 20.0).collect();
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_ne_bytes()).collect();
    let desc = PixelDescriptor::RGBF32_LINEAR;
    let slice = || PixelSlice::new(&bytes, w, h, (w * 12) as usize, desc).unwrap();
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
fn grey_pixels_with_an_rgb_icc_stay_rgb() {
    let (w, h) = (4u32, 4u32);
    let bytes: Vec<u8> = (0..w * h).flat_map(|i| [(i * 9) as u8; 3]).collect();
    let slice =
        || PixelSlice::new(&bytes, w, h, (w * 3) as usize, PixelDescriptor::RGB8_SRGB).unwrap();
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
        .encode(slice())
        .unwrap();
    assert_eq!(colour_type(with_icc.data()), 2, "RGB profile: stays RGB");
    assert!(
        find_chunk(with_icc.data(), b"iCCP").is_some(),
        "profile kept"
    );
}
