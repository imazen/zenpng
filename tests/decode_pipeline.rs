//! Two-thread decode (inflate on a second thread, unfilter on the caller's)
//! must give exactly the serial decoder's output and errors. Every image here
//! has at least 1 MiB of filtered data, so `max_threads(0)` pipelines it.

#![cfg(not(target_arch = "wasm32"))]

use zenflate::Unstoppable;
use zenpng::{PngDecodeConfig, decode};

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed | 1;
    (0..n)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            // Smooth bands with noise so every filter type gets used.
            if (i / 4000) % 3 == 0 {
                (s >> 24) as u8
            } else {
                (i / 7) as u8
            }
        })
        .collect()
}

fn encode(w: u32, h: u32, color: png::ColorType, depth: png::BitDepth, trns: bool) -> Vec<u8> {
    let channels = match color {
        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
    };
    let bytes = if depth == png::BitDepth::Sixteen {
        2
    } else {
        1
    };
    let data = noise(w as usize * h as usize * channels * bytes, w ^ h);
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(depth);
        if color == png::ColorType::Indexed {
            e.set_palette((0..768).map(|i| (i * 7) as u8).collect::<Vec<_>>());
            if trns {
                e.set_trns((0..256).map(|i| i as u8).collect::<Vec<_>>());
            }
        }
        e.set_filter(png::Filter::Adaptive);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(&data).unwrap();
    }
    out
}

fn cases() -> Vec<(&'static str, Vec<u8>)> {
    use png::{BitDepth::*, ColorType::*};
    vec![
        ("rgb8", encode(700, 600, Rgb, Eight, false)),
        ("rgba8", encode(600, 500, Rgba, Eight, false)),
        ("gray8", encode(1400, 900, Grayscale, Eight, false)),
        ("gray16", encode(900, 700, Grayscale, Sixteen, false)),
        ("ga8", encode(900, 700, GrayscaleAlpha, Eight, false)),
        ("pal8_trns", encode(1400, 900, Indexed, Eight, true)),
    ]
}

#[test]
fn pipelined_decode_matches_serial() {
    for (name, png) in cases() {
        for cfg in [PngDecodeConfig::default(), PngDecodeConfig::strict()] {
            let serial = decode(&png, &cfg.clone().with_max_threads(1), &Unstoppable).unwrap();
            let piped = decode(&png, &cfg.with_max_threads(0), &Unstoppable).unwrap();
            assert!(
                serial.pixels.copy_to_contiguous_bytes() == piped.pixels.copy_to_contiguous_bytes(),
                "{name}: pipelined output differs"
            );
            assert_eq!(serial.warnings, piped.warnings, "{name}: warnings differ");
        }
    }
}

#[test]
fn pipelined_decode_reports_the_same_errors() {
    let png = encode(700, 600, png::ColorType::Rgb, png::BitDepth::Eight, false);
    // Truncated mid-IDAT: both paths fail.
    let cut = &png[..png.len() * 2 / 3];
    for t in [1, 0] {
        assert!(
            decode(
                cut,
                &PngDecodeConfig::default().with_max_threads(t),
                &Unstoppable
            )
            .is_err()
        );
    }
    // Corrupt Adler-32 (last 4 bytes of IDAT data, before IDAT CRC + IEND):
    // strict fails, default warns, on both paths.
    let mut bad = png.clone();
    let adler_end = bad.len() - 12 - 4;
    bad[adler_end - 1] ^= 1;
    for t in [1, 0] {
        assert!(
            decode(
                &bad,
                &PngDecodeConfig::strict().with_max_threads(t),
                &Unstoppable
            )
            .is_err(),
            "strict, {t} threads"
        );
        let ok = decode(
            &bad,
            &PngDecodeConfig::default().with_max_threads(t),
            &Unstoppable,
        )
        .unwrap();
        assert!(
            ok.warnings
                .contains(&zenpng::PngWarning::DecompressionChecksumSkipped),
            "default, {t} threads: {:?}",
            ok.warnings
        );
    }
}
