//! Speed/size Pareto harness: zenpng against other PNG codecs, encode and
//! decode, single- and multi-threaded.
//!
//! Inputs: `ZENPNG_PARETO_DIR`, PNGs named `<id>_<format>_<longedge>.png`
//! (`scripts/vs_png_inputs.sh`). Every arm's output size is printed once,
//! untimed, as `SIZE\t<group>\t<arm>\t<bytes>` on stderr; times come from
//! the zenbench result file. `scripts/pareto_report.py` joins both into a TSV,
//! fits `ms = a + b·px` and `bytes = a + b·px` per arm, and lists every
//! zenpng arm that another codec's arm beats on both axes.
//!
//! Encode (RGB8 and RGBA8 inputs), group `enc/<name>`:
//! - `zenpng_e<E>_st`: `Compression::Effort(E)`, one thread
//!   (`ZENPNG_PARETO_EFFORTS`, default `1,2,3,5,7,9,13,19`; efforts above 13
//!   are skipped for long edges over 2560 unless `ZENPNG_PARETO_BIG=1`).
//! - `zenpng_e<E>_mt`: the same with `with_parallel(true)`
//!   (`ZENPNG_PARETO_MT_EFFORTS`, default `7,13,19`).
//! - `zenpng_e7_idot8_mt`: effort 7, parallel, `with_decode_segments(8)`.
//! - `png_fast` / `png_balanced` / `png_high`: image-rs/image-png main.
//! - `zune`: zune-png's encoder (default options). `lodepng`: lodepng
//!   defaults.
//!
//! Decode, group `dec/<name>` (the libpng-written input) and
//! `dec_idot/<name>` (zenpng effort 7 with 8 iDOT strips, RGB8/RGBA8 only):
//! `zenpng_st` (`max_threads(1)`), `zenpng_mt` (`max_threads(0)`, only
//! differs on iDOT files), `png`, `zune`, `lodepng`. No arm verifies CRC or
//! Adler-32; all expand palette and sub-byte gray to 8 bits. An untimed
//! check makes every decoder agree on 8-bit output first.
//!
//! Run: `ZENPNG_PARETO_DIR=... cargo bench --bench pareto -- [--group=enc|dec]`

use std::path::PathBuf;

use zenbench::prelude::*;
use zenflate::Unstoppable;

fn env_list(name: &str, default: &[u32]) -> Vec<u32> {
    std::env::var(name).map_or_else(
        |_| default.to_vec(),
        |v| {
            v.split(',')
                .map(|e| e.trim().parse().expect("effort"))
                .collect()
        },
    )
}

fn inputs() -> Vec<(String, u32, &'static [u8])> {
    let dir = PathBuf::from(
        std::env::var("ZENPNG_PARETO_DIR")
            .expect("set ZENPNG_PARETO_DIR to the input PNG directory"),
    );
    let mut v: Vec<(String, u32, &'static [u8])> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "png").then(|| {
                let name = p.file_stem().unwrap().to_string_lossy().to_string();
                let edge = name
                    .rsplit('_')
                    .next()
                    .and_then(|e| e.parse().ok())
                    .unwrap_or(0);
                let data: &'static [u8] = Box::leak(std::fs::read(&p).unwrap().into_boxed_slice());
                (name, edge, data)
            })
        })
        .collect();
    v.sort();
    v
}

fn leak<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

fn size_line(group: &str, arm: &str, bytes: usize) {
    eprintln!("SIZE\t{group}\t{arm}\t{bytes}");
}

// ── decoders ─────────────────────────────────────────────────────────────

fn zenpng_decode(data: &[u8], threads: usize) -> zenpng::PngDecodeOutput {
    let cfg = zenpng::PngDecodeConfig::default().with_max_threads(threads);
    zenpng::decode(data, &cfg, &Unstoppable).unwrap()
}

fn png_decode(data: &[u8]) -> Vec<u8> {
    let mut d = png_main::Decoder::new(std::io::Cursor::new(data));
    d.set_transformations(png_main::Transformations::EXPAND);
    d.ignore_checksums(true);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

fn zune_decode(data: &[u8]) -> Vec<u8> {
    let opts = zune_core::options::DecoderOptions::new_fast()
        .png_set_confirm_crc(false)
        .inflate_set_confirm_adler(false);
    let mut d =
        zune_png::PngDecoder::new_with_options(zune_core::bytestream::ZCursor::new(data), opts);
    d.decode_headers().unwrap();
    let mut out = vec![0u8; d.output_buffer_size().unwrap()];
    d.decode_into(&mut out).unwrap();
    out
}

fn lodepng_decode(data: &[u8]) -> lodepng::Image {
    // Native colour type, no conversion. lodepng's safe API has no switch for
    // its CRC/Adler-32 checks, so this arm does slightly more work.
    let mut d = lodepng::Decoder::new();
    d.color_convert(false);
    d.decode(data).unwrap()
}

// ── encoders ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Img {
    px: &'static [u8],
    w: usize,
    h: usize,
    alpha: bool,
}

fn zenpng_encode(img: Img, cfg: &zenpng::EncodeConfig) -> Vec<u8> {
    if img.alpha {
        let p: &[rgb::Rgba<u8>] = bytemuck::cast_slice(img.px);
        zenpng::encode_rgba8(
            imgref::ImgRef::new(p, img.w, img.h),
            None,
            cfg,
            &Unstoppable,
            &Unstoppable,
        )
    } else {
        let p: &[rgb::Rgb<u8>] = bytemuck::cast_slice(img.px);
        zenpng::encode_rgb8(
            imgref::ImgRef::new(p, img.w, img.h),
            None,
            cfg,
            &Unstoppable,
            &Unstoppable,
        )
    }
    .unwrap()
}

fn png_encode(img: Img, c: png_main::Compression) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png_main::Encoder::new(&mut out, img.w as u32, img.h as u32);
    e.set_color(if img.alpha {
        png_main::ColorType::Rgba
    } else {
        png_main::ColorType::Rgb
    });
    e.set_depth(png_main::BitDepth::Eight);
    e.set_compression(c);
    let mut wr = e.write_header().unwrap();
    wr.write_image_data(img.px).unwrap();
    wr.finish().unwrap();
    out
}

fn zune_encode(img: Img) -> Vec<u8> {
    let cs = if img.alpha {
        zune_core::colorspace::ColorSpace::RGBA
    } else {
        zune_core::colorspace::ColorSpace::RGB
    };
    let opts = zune_core::options::EncoderOptions::new(
        img.w,
        img.h,
        cs,
        zune_core::bit_depth::BitDepth::Eight,
    );
    let mut out = Vec::new();
    zune_png::PngEncoder::new(img.px, opts)
        .encode(&mut out)
        .unwrap();
    out
}

fn lodepng_encode(img: Img) -> Vec<u8> {
    if img.alpha {
        lodepng::encode32(img.px, img.w, img.h).unwrap()
    } else {
        lodepng::encode24(img.px, img.w, img.h).unwrap()
    }
}

/// One encode arm: image in, PNG bytes out.
type EncodeFn = Box<dyn Fn(Img) -> Vec<u8> + Send + Sync>;

fn effort_cfg(e: u32, parallel: bool) -> zenpng::EncodeConfig {
    zenpng::EncodeConfig::default()
        .with_compression(zenpng::Compression::Effort(e))
        .with_parallel(parallel)
}

/// Encode arms: (name, encoder). Sizes printed once per arm.
fn encode_arms(edge: u32) -> Vec<(String, EncodeFn)> {
    let big_ok = std::env::var_os("ZENPNG_PARETO_BIG").is_some() || edge <= 2560;
    let mut arms: Vec<(String, EncodeFn)> = Vec::new();
    for e in env_list("ZENPNG_PARETO_EFFORTS", &[1, 2, 3, 5, 7, 9, 13, 19]) {
        if e > 13 && !big_ok {
            continue;
        }
        let cfg = effort_cfg(e, false);
        arms.push((
            format!("zenpng_e{e}_st"),
            Box::new(move |i| zenpng_encode(i, &cfg)),
        ));
    }
    for e in env_list("ZENPNG_PARETO_MT_EFFORTS", &[7, 13, 19]) {
        if e > 13 && !big_ok {
            continue;
        }
        let cfg = effort_cfg(e, true);
        arms.push((
            format!("zenpng_e{e}_mt"),
            Box::new(move |i| zenpng_encode(i, &cfg)),
        ));
    }
    let idot = effort_cfg(7, true).with_decode_segments(8);
    arms.push((
        "zenpng_e7_idot8_mt".into(),
        Box::new(move |i| zenpng_encode(i, &idot)),
    ));
    for (n, c) in [
        ("png_fast", png_main::Compression::Fast),
        ("png_balanced", png_main::Compression::Balanced),
        ("png_high", png_main::Compression::High),
    ] {
        arms.push((n.into(), Box::new(move |i| png_encode(i, c))));
    }
    arms.push(("zune".into(), Box::new(zune_encode)));
    arms.push(("lodepng".into(), Box::new(lodepng_encode)));
    arms
}

fn bench_encode(suite: &mut Suite) {
    if std::env::args().any(|a| a.starts_with("--group=dec")) {
        return;
    }
    for (name, edge, data) in inputs() {
        if !(name.contains("_rgb8_") || name.contains("_rgba8_")) {
            continue;
        }
        let d = zenpng_decode(data, 1);
        let alpha = name.contains("_rgba8_");
        let bytes = d.pixels.copy_to_contiguous_bytes();
        let (w, h) = (d.info.width as usize, d.info.height as usize);
        if bytes.len() != w * h * if alpha { 4 } else { 3 } {
            continue; // the file decoded to another layout (e.g. gray)
        }
        let img = Img {
            px: leak(bytes),
            w,
            h,
            alpha,
        };
        let group = format!("enc/{name}");
        let arms = encode_arms(edge);
        for (arm, f) in &arms {
            size_line(&group, arm, f(img).len());
        }
        let px = (w * h) as u64;
        suite.compare(group, move |g| {
            g.throughput(Throughput::Elements(px));
            g.throughput_unit("px");
            for (arm, f) in arms {
                let f: &'static (dyn Fn(Img) -> Vec<u8> + Send + Sync) = Box::leak(f);
                g.bench(arm, move |b| b.iter(|| f(img)));
            }
        });
    }
}

fn eight_bit(name: &str) -> bool {
    !name.contains("16")
}

fn add_decode_group(suite: &mut Suite, group: String, data: &'static [u8], check: bool) {
    let out = zenpng_decode(data, 1);
    let px = out.info.width as u64 * out.info.height as u64;
    size_line(&group, "input", data.len());
    if check {
        let ours = out.pixels.copy_to_contiguous_bytes();
        assert!(ours == png_decode(data), "{group}: png disagrees");
        let mt = zenpng_decode(data, 0).pixels.copy_to_contiguous_bytes();
        assert!(ours == mt, "{group}: zenpng MT disagrees with ST");
    }
    suite.compare(group, move |g| {
        g.throughput(Throughput::Elements(px));
        g.throughput_unit("px");
        g.bench("zenpng_st", move |b| b.iter(|| zenpng_decode(data, 1)));
        g.bench("zenpng_mt", move |b| b.iter(|| zenpng_decode(data, 0)));
        g.bench("png", move |b| b.iter(|| png_decode(data)));
        g.bench("zune", move |b| b.iter(|| zune_decode(data)));
        g.bench("lodepng", move |b| b.iter(|| lodepng_decode(data)));
    });
}

fn bench_decode(suite: &mut Suite) {
    if std::env::args().any(|a| a.starts_with("--group=enc")) {
        return;
    }
    for (name, _edge, data) in inputs() {
        add_decode_group(suite, format!("dec/{name}"), data, eight_bit(&name));
        if name.contains("_rgb8_") || name.contains("_rgba8_") {
            let d = zenpng_decode(data, 1);
            let (w, h) = (d.info.width as usize, d.info.height as usize);
            let alpha = name.contains("_rgba8_");
            let bytes = d.pixels.copy_to_contiguous_bytes();
            if bytes.len() != w * h * if alpha { 4 } else { 3 } {
                continue;
            }
            let img = Img {
                px: leak(bytes),
                w,
                h,
                alpha,
            };
            let idot = leak(zenpng_encode(
                img,
                &effort_cfg(7, true).with_decode_segments(8),
            ));
            add_decode_group(suite, format!("dec_idot/{name}"), idot, true);
        }
    }
}

fn main() {
    // Ungated like benches/vs_png.rs (the resource gate stalls on shared
    // boxes); arms are interleaved within each round.
    let group_filter = std::env::args().find_map(|a| a.strip_prefix("--group=").map(String::from));
    let result = zenbench::run_gated(zenbench::GateConfig::disabled(), |suite| {
        if let Some(f) = &group_filter {
            suite.set_group_filter(f.clone());
        }
        bench_encode(suite);
        bench_decode(suite);
    });
    zenbench::postprocess_result(&result);
}
