//! Apple ImageIO's iDOT path mis-decodes 1/2/4-bit grayscale (its serial
//! path is fine; measured on macOS 27, `tests/fixtures/idot/mac/`), so the
//! encoder never segments those. Separate test binary: it lowers the
//! process-wide iDOT size threshold so a small image qualifies.

#![cfg(all(feature = "_dev", not(target_arch = "wasm32")))]

use imgref::ImgVec;
use rgb::Rgba;
use zenflate::Unstoppable;
use zenpng::{DowncastFlags, EncodeConfig, encode_rgba8};

fn has_idot(png: &[u8]) -> bool {
    png.windows(4).any(|w| w == b"iDOT")
}

fn encode_levels(levels: u32) -> Vec<u8> {
    let (w, h) = (256usize, 256usize);
    let step = 255 / (levels - 1);
    let px: Vec<Rgba<u8>> = (0..w * h)
        .map(|i| {
            let v = (((i * 7 + i / w) as u32 % levels) * step) as u8;
            Rgba {
                r: v,
                g: v,
                b: v,
                a: 255,
            }
        })
        .collect();
    let mut flags = DowncastFlags::default();
    flags.indexed = false; // force the grayscale (not palette) encoding
    let cfg = EncodeConfig::default()
        .with_downcast(flags)
        .with_decode_segments(4);
    encode_rgba8(
        ImgVec::new(px, w, h).as_ref(),
        None,
        &cfg,
        &Unstoppable,
        &Unstoppable,
    )
    .unwrap()
}

#[test]
fn sub_byte_gray_is_never_segmented() {
    zenpng::__set_idot_min_bytes(1);
    for (levels, depth) in [(2u32, 1u8), (4, 2), (16, 4)] {
        let png = encode_levels(levels);
        assert_eq!(
            (png[24], png[25]),
            (depth, 0),
            "{levels} levels: expected {depth}-bit gray"
        );
        assert!(!has_idot(&png), "{depth}-bit gray got an iDOT");
    }
    // 8-bit gray is still segmented.
    let png = encode_levels(256);
    assert_eq!((png[24], png[25]), (8, 0));
    assert!(has_idot(&png), "8-bit gray should be segmented");
}
