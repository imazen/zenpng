//! Keep the source claim separate from proof about the decoded integer grid.
use std::borrow::Cow;
use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
use zenpng::{PngDecoderConfig, SignificantBits};

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(&zenflate::crc32(zenflate::crc32(0, kind), data).to_be_bytes());
    bytes
}

fn fixture(depth: u8, color: u8, bits: &[u8], words: &[u16]) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&[depth, color, 0, 0, 0]);
    let mut row = vec![0];
    if depth == 16 {
        row.extend(words.iter().flat_map(|v| v.to_be_bytes()));
    } else {
        row.extend(words.iter().map(|&v| v as u8));
    }
    let mut zlib = vec![0x78, 0x01, 0x01];
    zlib.extend_from_slice(&(row.len() as u16).to_le_bytes());
    zlib.extend_from_slice(&(!(row.len() as u16)).to_le_bytes());
    zlib.extend_from_slice(&row);
    zlib.extend_from_slice(&zenflate::adler32(1, &row).to_be_bytes());
    [
        b"\x89PNG\r\n\x1a\n".as_slice(),
        &chunk(b"IHDR", &header),
        &chunk(b"sBIT", bits),
        &chunk(b"IDAT", &zlib),
        &chunk(b"IEND", &[]),
    ]
    .concat()
}

#[test]
fn native_sbit_survives_without_relabeling_coded_or_exact_depth() {
    for depth in [8, 16] {
        let (r, g, b) = if depth == 8 { (5, 6, 7) } else { (10, 11, 12) };
        for (color, bits, expected) in [
            (0, vec![r], SignificantBits::Gray(r)),
            (2, vec![r, g, b], SignificantBits::Rgb(r, g, b)),
            (4, vec![r, b], SignificantBits::GrayAlpha(r, b)),
            (6, vec![r, g, b, 8], SignificantBits::Rgba(r, g, b, 8)),
        ] {
            // Code 1 is off every coarser full-range integer grid, whatever sBIT claims.
            let words: Vec<u16> = (1..=bits.len() as u16).collect();
            let png = fixture(depth, color, &bits, &words);
            let output = PngDecoderConfig::new()
                .job()
                .decoder(Cow::Borrowed(&png), &[])
                .unwrap()
                .decode()
                .unwrap();
            assert_eq!(output.extras::<SignificantBits>(), Some(&expected));
            assert_eq!(output.info().source_color.bit_depth, Some(depth));
            let pixels = output.pixels();
            let exact = pixels.exact_depth().expect("current samples are measured");
            assert_eq!(exact.color_bits(), depth);
            assert!(exact.holds_for(&pixels).unwrap());
            let decoded: Vec<u16> = if depth == 16 {
                pixels
                    .row(0)
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|v| u16::from_ne_bytes(*v))
                    .collect()
            } else {
                pixels.row(0).iter().map(|&v| u16::from(v)).collect()
            };
            let expected_words = if color == 4 && depth == 8 {
                vec![words[0], words[0], words[0], words[1]]
            } else {
                words
            };
            assert_eq!(decoded, expected_words);
        }
    }
}
