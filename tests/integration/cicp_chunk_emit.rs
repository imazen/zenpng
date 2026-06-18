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
