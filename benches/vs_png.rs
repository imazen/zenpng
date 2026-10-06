//! zenpng vs the `png` crate (image-rs/image-png, git main pinned in
//! Cargo.toml as `png_main`), decode and encode.
//!
//! Inputs: a directory of PNGs (`ZENPNG_BENCH_DIR`), named
//! `<id>_<format>_<longedge>.png`. The 2026-10-05 set is 20 imazen-26 png-v3
//! renders (one per content class) re-encoded by ImageMagick/libpng at long
//! edge 64/256/1024/2560, plus RGBA8/gray8/palette/RGB16/interlaced/1-bit
//! variants at 1024 — see `benchmarks/vs_png_2026-10-05.meta` for how to
//! regenerate it. libpng-encoded inputs keep either Rust decoder from
//! benchmarking its own encoder's output.
//!
//! Decode arms use equal checksum work: neither verifies CRC-32 or
//! Adler-32. Both expand to 8/16-bit samples (palette and sub-byte gray
//! expanded, 16-bit kept). zenpng is single-threaded here (`max_threads(1)`).
//!
//! Encode arms: zenpng efforts 1/7/13/19 vs png Fast/Balanced/High, on RGB8
//! input. With `ZENPNG_BENCH_SIZES=1`, output sizes are printed once
//! (untimed) as `SIZE` lines.
//!
//! Run: ZENPNG_BENCH_DIR=... cargo bench --bench vs_png -- [--group=decode|encode]

use std::path::PathBuf;

use zenbench::prelude::*;
use zenflate::Unstoppable;

fn inputs() -> Vec<(String, &'static [u8])> {
    let dir = PathBuf::from(
        std::env::var("ZENPNG_BENCH_DIR").expect("set ZENPNG_BENCH_DIR to the input PNG directory"),
    );
    let mut v: Vec<(String, &'static [u8])> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "png").then(|| {
                let name = p.file_stem().unwrap().to_string_lossy().to_string();
                let data: &'static [u8] = Box::leak(std::fs::read(&p).unwrap().into_boxed_slice());
                (name, data)
            })
        })
        .collect();
    v.sort();
    v
}

fn png_main_decode(data: &[u8]) -> Vec<u8> {
    let mut d = png_main::Decoder::new(std::io::Cursor::new(data));
    d.set_transformations(png_main::Transformations::EXPAND);
    d.ignore_checksums(true);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

fn zenpng_decode(data: &[u8]) -> zenpng::PngDecodeOutput {
    let cfg = zenpng::PngDecodeConfig::default().with_max_threads(1);
    zenpng::decode(data, &cfg, &Unstoppable).unwrap()
}

fn bench_decode(suite: &mut Suite) {
    for (name, data) in inputs() {
        let out = zenpng_decode(data);
        let px = out.info.width as u64 * out.info.height as u64;
        // Untimed sanity: both decoders must agree (8-bit outputs; the png
        // crate returns 16-bit samples big-endian, zenpng native-endian).
        let ours = out.pixels.copy_to_contiguous_bytes();
        let theirs = png_main_decode(data);
        if !name.contains("16") {
            assert!(ours == theirs, "{name}: decoders disagree");
        }
        suite.compare(format!("decode/{name}"), move |g| {
            g.throughput(Throughput::Elements(px));
            g.throughput_unit("px");
            g.bench("zenpng", move |b| b.iter(|| zenpng_decode(data)));
            g.bench("png_main", move |b| b.iter(|| png_main_decode(data)));
        });
    }
}

fn zenpng_encode(rgb: &[rgb::Rgb<u8>], w: usize, h: usize, effort: u32) -> Vec<u8> {
    let cfg = zenpng::EncodeConfig::default().with_compression(zenpng::Compression::Effort(effort));
    zenpng::encode_rgb8(
        imgref::ImgRef::new(rgb, w, h),
        None,
        &cfg,
        &Unstoppable,
        &Unstoppable,
    )
    .unwrap()
}

fn png_main_encode(rgb: &[u8], w: u32, h: u32, c: png_main::Compression) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png_main::Encoder::new(&mut out, w, h);
    e.set_color(png_main::ColorType::Rgb);
    e.set_depth(png_main::BitDepth::Eight);
    e.set_compression(c);
    e.write_header().unwrap().write_image_data(rgb).unwrap();
    out
}

fn bench_encode(suite: &mut Suite) {
    if std::env::args().any(|a| a == "--group=decode") {
        return; // skip the (expensive) encode setup when only decode runs
    }
    // RGB8 inputs only, at 256/1024/2560 (64 px is pure fixed overhead and
    // is covered by the decode side).
    for (name, data) in inputs()
        .into_iter()
        .filter(|(n, _)| n.contains("_rgb8_") && !n.ends_with("_64"))
    {
        let out = zenpng_decode(data);
        let (w, h) = (out.info.width as usize, out.info.height as usize);
        let bytes = out.pixels.copy_to_contiguous_bytes();
        if bytes.len() != w * h * 3 {
            continue; // not RGB8 after decode
        }
        let rgb: &'static [rgb::Rgb<u8>] = Box::leak(
            bytemuck::cast_slice::<u8, rgb::Rgb<u8>>(&bytes)
                .to_vec()
                .into_boxed_slice(),
        );
        let raw: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        // ZENPNG_BENCH_EFFORTS=1,7,13,19 (default) picks the zenpng efforts.
        let efforts: &'static [u32] = Box::leak(
            std::env::var("ZENPNG_BENCH_EFFORTS")
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|e| e.trim().parse().expect("effort"))
                        .collect()
                })
                .unwrap_or_else(|| vec![1u32, 7, 13, 19])
                .into_boxed_slice(),
        );
        let levels = [
            ("fast", png_main::Compression::Fast),
            ("balanced", png_main::Compression::Balanced),
            ("high", png_main::Compression::High),
        ];
        if std::env::var_os("ZENPNG_BENCH_SIZES").is_some() {
            for &e in efforts {
                eprintln!(
                    "SIZE\t{name}\tzenpng_e{e}\t{}",
                    zenpng_encode(rgb, w, h, e).len()
                );
            }
            for (l, c) in levels {
                let n = png_main_encode(raw, w as u32, h as u32, c).len();
                eprintln!("SIZE\t{name}\tpng_main_{l}\t{n}");
            }
        }
        let px = (w * h) as u64;
        suite.compare(format!("encode/{name}"), move |g| {
            g.throughput(Throughput::Elements(px));
            g.throughput_unit("px");
            for &e in efforts {
                g.bench(format!("zenpng_e{e}"), move |b| {
                    b.iter(|| zenpng_encode(rgb, w, h, e))
                });
            }
            for (l, c) in levels {
                g.bench(format!("png_main_{l}"), move |b| {
                    b.iter(|| png_main_encode(raw, w as u32, h as u32, c))
                });
            }
        });
    }
}

fn main() {
    // zenbench's resource gate waits up to 30 s per round while it sees other
    // heavy processes. On a shared box that stretched a ~10 s group to two
    // minutes without measuring anything differently: the arms are already
    // interleaved within each round, and runs are pinned to one core. So this
    // bench runs ungated; `--group=` filtering and the report are unchanged.
    let group_filter = std::env::args().find_map(|a| a.strip_prefix("--group=").map(String::from));
    let result = zenbench::run_gated(zenbench::GateConfig::disabled(), |suite| {
        if let Some(f) = &group_filter {
            suite.set_group_filter(f.clone());
        }
        bench_decode(suite);
        bench_encode(suite);
    });
    zenbench::postprocess_result(&result);
}
