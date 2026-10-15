//! Text chunks are written in the encoding their chunk type requires.
use enough::Unstoppable;
use zenpng::{EncodeConfig, encode_rgb8};

fn chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 8;
    while pos + 12 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        let ty: [u8; 4] = png[pos + 4..pos + 8].try_into().unwrap();
        out.push((ty, png[pos + 8..pos + 8 + len].to_vec()));
        pos += 12 + len;
    }
    out
}

#[test]
fn latin1_text_is_text_and_wider_text_is_itxt() {
    let pixels = vec![rgb::Rgb { r: 1u8, g: 2, b: 3 }; 4];
    let img = imgref::ImgVec::new(pixels, 2, 2);
    let config = EncodeConfig::default()
        .with_text("Title", "Bj\u{f8}rn")
        .with_text("Author", "Sog\u{14d} Gor\u{14d}");
    let png = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
    let all = chunks(&png);
    let text: Vec<_> = all.iter().filter(|(t, _)| t == b"tEXt").collect();
    let itxt: Vec<_> = all.iter().filter(|(t, _)| t == b"iTXt").collect();
    // "Bj\u{f8}rn" in Latin-1: one byte 0xF8, not UTF-8's two.
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].1, b"Title\0Bj\xf8rn");
    // \u{14d} (o macron) is outside Latin-1: an uncompressed iTXt with UTF-8 text.
    assert_eq!(itxt.len(), 1);
    assert_eq!(
        itxt[0].1,
        [
            b"Author\0\0\0\0\0".as_slice(),
            "Sog\u{14d} Gor\u{14d}".as_bytes()
        ]
        .concat()
    );
    // And the file still decodes.
    zenpng::decode(&png, &zenpng::PngDecodeConfig::default(), &Unstoppable).unwrap();
}
