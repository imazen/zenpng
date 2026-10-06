//! Multi-threaded strip encode (`with_parallel(true)` at screen-only efforts):
//! output is the same for any thread count and decodes to the exact input.

#![cfg(not(target_arch = "wasm32"))]

use imgref::ImgVec;
use rgb::Rgb;
use zenflate::Unstoppable;
use zenpng::{Compression, EncodeConfig, PngDecodeConfig, decode, encode_rgb8};

/// ~3.1 MB of filtered RGB data: six strips of 512 KiB.
fn image() -> ImgVec<Rgb<u8>> {
    let (w, h) = (1024usize, 1000usize);
    let mut x32: u32 = 0x2545_f491;
    let px = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            x32 ^= x32 << 13;
            x32 ^= x32 >> 17;
            x32 ^= x32 << 5;
            let n = if (y / 50) % 2 == 0 {
                (x32 & 15) as u8
            } else {
                0
            };
            Rgb {
                r: (x / 4) as u8 ^ n,
                g: (y / 4) as u8,
                b: ((x + y) / 8) as u8,
            }
        })
        .collect();
    ImgVec::new(px, w, h)
}

fn bytes(img: &ImgVec<Rgb<u8>>) -> Vec<u8> {
    img.buf().iter().flat_map(|p| [p.r, p.g, p.b]).collect()
}

#[test]
fn strip_encode_is_thread_count_independent_and_lossless() {
    let img = image();
    let src = bytes(&img);
    for effort in [1u32, 2, 5, 7] {
        let mut outs = Vec::new();
        for threads in [2usize, 3, 8] {
            let mut cfg = EncodeConfig::default()
                .with_compression(Compression::Effort(effort))
                .with_parallel(true);
            cfg.max_threads = threads;
            let png = encode_rgb8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
            let back = decode(&png, &PngDecodeConfig::strict(), &Unstoppable).unwrap();
            assert_eq!(
                back.pixels.copy_to_contiguous_bytes(),
                src,
                "effort {effort}, {threads} threads"
            );
            outs.push(png);
        }
        assert!(
            outs.windows(2).all(|w| w[0] == w[1]),
            "effort {effort}: output depends on thread count"
        );
        let serial = encode_rgb8(
            img.as_ref(),
            None,
            &EncodeConfig::default().with_compression(Compression::Effort(effort)),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        // Strips reset the match history; the cost is small.
        let growth = outs[0].len() as f64 / serial.len() as f64;
        assert!(
            growth < 1.02,
            "effort {effort}: strips cost {:.2}%",
            (growth - 1.0) * 100.0
        );
    }
}
