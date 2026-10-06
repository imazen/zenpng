//! `EncodeConfig::with_decode_segments`: `iDOT`-segmented output must be a
//! standard PNG for every decoder, decode identically serially and in
//! parallel, and leave unsegmented output untouched.

#![cfg(not(target_arch = "wasm32"))]

use imgref::ImgVec;
use rgb::Rgba;
use zenflate::Unstoppable;
use zenpng::{Compression, EncodeConfig, PngDecodeConfig, decode, encode_rgba8};

/// Varied RGBA content: blocky gradients plus a noise band (not trivially
/// compressible, exercises every filter).
fn image(w: usize, h: usize) -> ImgVec<Rgba<u8>> {
    let mut x32: u32 = 0x9e37_79b9;
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            x32 ^= x32 << 13;
            x32 ^= x32 >> 17;
            x32 ^= x32 << 5;
            let noise = if (y / 37) % 3 == 0 {
                (x32 & 31) as u8
            } else {
                0
            };
            px.push(Rgba {
                r: ((x >> 3) as u8).wrapping_add(y as u8).wrapping_add(noise),
                g: (y >> 2) as u8,
                b: ((x ^ y) >> 4) as u8,
                a: if (x / 61) % 5 == 0 { 128 } else { 255 },
            });
        }
    }
    ImgVec::new(px, w, h)
}

/// Parse the iDOT table: (N, [(first_row, rows, offset)]).
fn idot_table(png: &[u8]) -> Option<Vec<(u32, u32, u32)>> {
    let mut p = 8;
    while p + 12 <= png.len() {
        let len = u32::from_be_bytes(png[p..p + 4].try_into().unwrap()) as usize;
        if &png[p + 4..p + 8] == b"iDOT" {
            let d = &png[p + 8..p + 8 + len];
            let w = |i: usize| u32::from_be_bytes(d[i * 4..i * 4 + 4].try_into().unwrap());
            let n = w(0) as usize;
            assert_eq!(d.len(), 4 + 12 * n);
            // The next chunk must be the first IDAT (Apple's placement).
            assert_eq!(&png[p + 12 + len + 4..p + 12 + len + 8], b"IDAT");
            return Some(
                (0..n)
                    .map(|k| (w(1 + 3 * k), w(2 + 3 * k), w(3 + 3 * k)))
                    .collect(),
            );
        }
        p += 12 + len;
    }
    None
}

fn decode_png_crate(png: &[u8]) -> Vec<u8> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

fn rgba_bytes(img: &ImgVec<Rgba<u8>>) -> Vec<u8> {
    img.buf()
        .iter()
        .flat_map(|p| [p.r, p.g, p.b, p.a])
        .collect()
}

#[test]
fn segment_count_follows_size_rule() {
    // 600x400 RGBA ~= 0.96 MB filtered: under the 2 MiB floor, no iDOT.
    let small = image(600, 400);
    let cfg = EncodeConfig::default().with_decode_segments(8);
    let png = encode_rgba8(small.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    assert!(idot_table(&png).is_none());

    // 1024x700 RGBA ~= 2.87 MB: two workers' worth, so 8 requested -> 2.
    let big = image(1024, 700);
    let png = encode_rgba8(big.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    let t = idot_table(&png).expect("iDOT expected");
    assert_eq!(t.len(), 2);
    assert_eq!((t[0].0, t[0].1, t[1].0, t[1].1), (0, 350, 350, 350));
    assert_eq!(
        t[0].2, 40,
        "first IDAT must directly follow the 40-byte iDOT chunk"
    );
}

#[test]
fn zero_or_one_segment_leaves_output_unchanged() {
    let img = image(1024, 700);
    for effort in [1u32, 7] {
        let base = EncodeConfig::default().with_compression(Compression::Effort(effort));
        let a = encode_rgba8(img.as_ref(), None, &base, &Unstoppable, &Unstoppable).unwrap();
        for n in [0u32, 1] {
            let b = encode_rgba8(
                img.as_ref(),
                None,
                &base.clone().with_decode_segments(n),
                &Unstoppable,
                &Unstoppable,
            )
            .unwrap();
            assert_eq!(a, b, "effort {effort}, segments {n}");
        }
    }
}

#[test]
fn segmented_output_roundtrips_everywhere() {
    // 1600x1400 RGBA ~= 8.96 MB filtered: room for up to 3 workers.
    let img = image(1600, 1400);
    let src = rgba_bytes(&img);
    for effort in [1u32, 7, 13] {
        for n in [2u32, 3, 8] {
            let cfg = EncodeConfig::default()
                .with_compression(Compression::Effort(effort))
                .with_decode_segments(n);
            let png = encode_rgba8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
            let table = idot_table(&png).expect("iDOT expected");
            assert!(
                table.len() >= 2 && table.len() <= n as usize,
                "effort {effort} n {n}"
            );
            let rows: u32 = table.iter().map(|t| t.1).sum();
            assert_eq!(rows, 1400);

            // Independent decoder (ignores iDOT).
            assert_eq!(
                decode_png_crate(&png),
                src,
                "png crate, effort {effort} n {n}"
            );

            // zenpng serial and parallel.
            #[cfg(feature = "_dev")]
            let before = zenpng::__idot_stats();
            let serial = decode(
                &png,
                &PngDecodeConfig::strict().with_max_threads(1),
                &Unstoppable,
            )
            .unwrap();
            let par = decode(
                &png,
                &PngDecodeConfig::strict().with_max_threads(0),
                &Unstoppable,
            )
            .unwrap();
            assert_eq!(serial.pixels.copy_to_contiguous_bytes(), src);
            assert_eq!(par.pixels.copy_to_contiguous_bytes(), src);
            assert_eq!(serial.warnings, par.warnings);
            #[cfg(feature = "_dev")]
            if std::thread::available_parallelism().map_or(1, |n| n.get()) >= 2 {
                assert!(
                    zenpng::__idot_stats().0 > before.0,
                    "parallel path did not run: effort {effort} n {n}"
                );
            }
        }
    }
}

#[test]
fn effort_zero_stays_unsegmented() {
    let img = image(1024, 700);
    let cfg = EncodeConfig::default()
        .with_compression(Compression::None)
        .with_decode_segments(4);
    let png = encode_rgba8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
    assert!(idot_table(&png).is_none());
}

/// With parallel encoding on, `iDOT` segments come straight from the strip
/// encoder (no serial encode + re-split). The file must have the same shape
/// (table, rows, first IDAT right after `iDOT`), decode identically
/// everywhere, and not depend on the thread count.
#[test]
fn parallel_strip_segments_roundtrip_everywhere() {
    let img = image(1600, 1400);
    let src = rgba_bytes(&img);
    for effort in [1u32, 2, 7, 13] {
        let mut outs = Vec::new();
        for threads in [2usize, 8] {
            let mut cfg = EncodeConfig::default()
                .with_compression(Compression::Effort(effort))
                .with_decode_segments(3)
                .with_parallel(true);
            cfg.max_threads = threads;
            let png = encode_rgba8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
            let table = idot_table(&png).expect("iDOT expected");
            assert_eq!(table.len(), 3, "effort {effort}");
            assert_eq!(table.iter().map(|t| t.1).sum::<u32>(), 1400);
            assert_eq!(decode_png_crate(&png), src, "png crate, effort {effort}");
            #[cfg(feature = "_dev")]
            let before = zenpng::__idot_stats();
            for t in [1, 0] {
                let d = decode(
                    &png,
                    &PngDecodeConfig::strict().with_max_threads(t),
                    &Unstoppable,
                )
                .unwrap();
                assert_eq!(
                    d.pixels.copy_to_contiguous_bytes(),
                    src,
                    "zenpng {t}, e{effort}"
                );
            }
            // The segments must decode in parallel, not fall back to serial
            // (which a bad boundary row would trigger silently).
            #[cfg(feature = "_dev")]
            if std::thread::available_parallelism().map_or(1, |n| n.get()) >= 2 {
                assert!(
                    zenpng::__idot_stats().0 > before.0,
                    "parallel decode did not run: effort {effort}"
                );
            }
            outs.push(png);
        }
        assert_eq!(outs[0], outs[1], "effort {effort}: depends on thread count");
        let serial = encode_rgba8(
            img.as_ref(),
            None,
            &EncodeConfig::default()
                .with_compression(Compression::Effort(effort))
                .with_decode_segments(3),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let growth = outs[0].len() as f64 / serial.len() as f64;
        assert!(
            growth < 1.02,
            "effort {effort}: {:.2}% vs serial + re-split",
            (growth - 1.0) * 100.0
        );
    }
}
