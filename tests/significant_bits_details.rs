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

fn fixture(color: u8, bits: &[u8], words: &[u16]) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&[16, color, 0, 0, 0]);
    let mut row = vec![0];
    row.extend(words.iter().flat_map(|v| v.to_be_bytes()));
    let mut zlib = vec![0x78, 0x01, 0x01];
    zlib.extend_from_slice(&(row.len() as u16).to_le_bytes());
    zlib.extend_from_slice(&(!(row.len() as u16)).to_le_bytes());
    zlib.extend_from_slice(&row);
    zlib.extend_from_slice(&zenflate::adler32(1, &row).to_be_bytes());
    [b"\x89PNG\r\n\x1a\n".as_slice(), &chunk(b"IHDR", &header),
        &chunk(b"sBIT", bits), &chunk(b"IDAT", &zlib), &chunk(b"IEND", &[])].concat()
}

#[test]
fn native_sbit_survives_without_relabeling_coded_or_exact_depth() {
    for (color, bits, expected) in [
        (0, vec![10], SignificantBits::Gray(10)),
        (2, vec![10, 11, 12], SignificantBits::Rgb(10, 11, 12)),
        (4, vec![10, 12], SignificantBits::GrayAlpha(10, 12)),
        (6, vec![10, 11, 12, 8], SignificantBits::Rgba(10, 11, 12, 8)),
    ] {
        // Code 1 is off every coarser full-range U16 grid, whatever sBIT claims.
        let words: Vec<u16> = (1..=bits.len() as u16).collect();
        let png = fixture(color, &bits, &words);
        let output = PngDecoderConfig::new().job().decoder(Cow::Borrowed(&png), &[])
            .unwrap().decode().unwrap();
        assert_eq!(output.extras::<SignificantBits>(), Some(&expected));
        assert_eq!(output.info().source_color.bit_depth, Some(16));
        let pixels = output.pixels();
        let exact = pixels.exact_depth().expect("current samples are measured");
        assert_eq!(exact.color_bits(), 16);
        assert!(exact.holds_for(&pixels).unwrap());
        let decoded: Vec<u16> = pixels.row(0).as_chunks::<2>().0.iter()
            .map(|v| u16::from_ne_bytes(*v)).collect();
        assert_eq!(decoded, words);
    }
}
