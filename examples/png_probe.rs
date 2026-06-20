//! Resource probe for PNG encode/decode calibration
//! (`scripts/png_resource_calibrate.py`).
//!
//! Measures the marginal working set (`VmHWM` delta), wall time, and
//! user/sys CPU of a single encode OR decode call — isolated to the codec
//! call (the PNG load / process startup are excluded). Encode and decode
//! are separate invocations so `VmHWM` gives a clean per-op peak.
//! Compression is single-thread by default (`with_parallel(false)`) so wall ≈
//! user (clean per-pixel CPU model); pass a thread count as the optional 7th
//! arg for the vCPU resource sweep — zenpng parallelizes over filter
//! STRATEGIES (`std::thread::scope`, capped by `max_threads`), so peak grows
//! with concurrent strategies and wall drops until the strategy count caps it.
//!
//! Loads its input with the `png` dev-dependency (the source variants are
//! 8-bit RGB); `rgba` synthesizes a high-entropy alpha (= green channel),
//! `16` widens 8→16-bit, so the encoder exercises each path.
//!
//! Usage:
//!   png_probe <png> encode <effort> <8|16> <rgb|rgba> <out.png> [threads]
//!   png_probe <png> decode <effort> <8|16> <rgb|rgba> <in.png>
//! Prints (encode): `delta_kb=<n> peak_kb=<n> wall_ms=<f> user_ms=<f> sys_ms=<f> \
//!   bytes=<n> threads=<n> est_min_kb=<n> est_typ_kb=<n> est_max_kb=<n> est_time_ms=<f>`

use std::fs;
use std::time::Instant;

use enough::Unstoppable;
use imgref::Img;
use rgb::{Rgb, Rgba};
use zenpng::{Compression, EncodeConfig};

fn vmhwm_kb() -> u64 {
    let s = fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse()
                .unwrap_or(0);
        }
    }
    0
}

fn cpu_ticks() -> (u64, u64) {
    let s = fs::read_to_string("/proc/self/stat").unwrap_or_default();
    if let Some(p) = s.rfind(')') {
        let f: Vec<&str> = s[p + 1..].split_whitespace().collect();
        if f.len() > 12 {
            return (f[11].parse().unwrap_or(0), f[12].parse().unwrap_or(0));
        }
    }
    (0, 0)
}
const TICK_MS: f64 = 10.0;

/// Load an 8-bit RGB PNG to (rgb bytes, w, h) via the `png` crate.
fn load_rgb8(path: &str) -> (Vec<u8>, usize, usize) {
    let dec = png::Decoder::new(std::io::BufReader::new(
        fs::File::open(path).expect("open png"),
    ));
    let mut rdr = dec.read_info().expect("read_info");
    let mut buf = vec![0u8; rdr.output_buffer_size().expect("output_buffer_size")];
    let info = rdr.next_frame(&mut buf).expect("next_frame");
    let (w, h) = (info.width as usize, info.height as usize);
    buf.truncate(info.buffer_size());
    // Normalize to RGB8 (the variants are saved RGB8, but be defensive).
    let rgb = match info.color_type {
        png::ColorType::Rgb => buf,
        png::ColorType::Rgba => buf
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&v| [v, v, v]).collect(),
        _ => panic!("unexpected source color type {:?}", info.color_type),
    };
    (rgb, w, h)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 7 {
        eprintln!("usage: png_probe <png> <encode|decode> <effort> <8|16> <rgb|rgba> <out.png>");
        std::process::exit(2);
    }
    let (path, mode, effort, depth, alpha, outp) = (
        &a[1],
        &a[2],
        a[3].parse::<u32>().unwrap(),
        a[4].parse::<u8>().unwrap(),
        &a[5],
        &a[6],
    );

    let threads: usize = a.get(7).and_then(|s| s.parse().ok()).unwrap_or(1);

    if mode == "encode" {
        let (rgb, w, h) = load_rgb8(path);
        let mut cfg = EncodeConfig::default()
            .with_compression(Compression::Effort(effort))
            .with_parallel(threads > 1);
        cfg.max_threads = threads;

        // Model prediction (thread-independent, calibrated single-thread).
        let input_bpp: u8 = match (depth, alpha.as_str()) {
            (16, "rgba") => 8,
            (16, _) => 6,
            (_, "rgba") => 4,
            _ => 3,
        };
        let est = zenpng::heuristics::estimate_encode(w as u32, h as u32, input_bpp, effort);
        let (est_min, est_typ, est_max, est_t) = est
            .map(|e| {
                (
                    e.peak_memory_bytes_min / 1024,
                    e.peak_memory_bytes / 1024,
                    e.peak_memory_bytes_max / 1024,
                    e.time_ms,
                )
            })
            .unwrap_or((0, 0, 0, 0.0));

        let (b0, t0) = (vmhwm_kb(), Instant::now());
        let (cu0, cs0) = cpu_ticks();
        let encoded = match (depth, alpha.as_str()) {
            (16, "rgba") => {
                let px: Vec<Rgba<u16>> = rgb
                    .chunks_exact(3)
                    .map(|p| {
                        let w16 = |v: u8| ((v as u16) << 8) | v as u16;
                        Rgba {
                            r: w16(p[0]),
                            g: w16(p[1]),
                            b: w16(p[2]),
                            a: w16(p[1]),
                        }
                    })
                    .collect();
                zenpng::encode_rgba16(
                    Img::new(px, w, h).as_ref(),
                    None,
                    &cfg,
                    &Unstoppable,
                    &Unstoppable,
                )
            }
            (16, _) => {
                let px: Vec<Rgb<u16>> = rgb
                    .chunks_exact(3)
                    .map(|p| {
                        let w16 = |v: u8| ((v as u16) << 8) | v as u16;
                        Rgb {
                            r: w16(p[0]),
                            g: w16(p[1]),
                            b: w16(p[2]),
                        }
                    })
                    .collect();
                zenpng::encode_rgb16(
                    Img::new(px, w, h).as_ref(),
                    None,
                    &cfg,
                    &Unstoppable,
                    &Unstoppable,
                )
            }
            (_, "rgba") => {
                let px: Vec<Rgba<u8>> = rgb
                    .chunks_exact(3)
                    .map(|p| Rgba {
                        r: p[0],
                        g: p[1],
                        b: p[2],
                        a: p[1],
                    })
                    .collect();
                zenpng::encode_rgba8(
                    Img::new(px, w, h).as_ref(),
                    None,
                    &cfg,
                    &Unstoppable,
                    &Unstoppable,
                )
            }
            _ => {
                let px: Vec<Rgb<u8>> = rgb
                    .chunks_exact(3)
                    .map(|p| Rgb {
                        r: p[0],
                        g: p[1],
                        b: p[2],
                    })
                    .collect();
                zenpng::encode_rgb8(
                    Img::new(px, w, h).as_ref(),
                    None,
                    &cfg,
                    &Unstoppable,
                    &Unstoppable,
                )
            }
        };
        let wall = t0.elapsed();
        let (cu1, cs1) = cpu_ticks();
        let peak = vmhwm_kb();
        let enc = encoded.expect("encode failed");
        fs::write(outp, &enc).expect("write png");
        println!(
            "delta_kb={} peak_kb={} wall_ms={:.1} user_ms={:.1} sys_ms={:.1} bytes={} \
             threads={} est_min_kb={} est_typ_kb={} est_max_kb={} est_time_ms={:.1}",
            peak.saturating_sub(b0),
            peak,
            wall.as_secs_f64() * 1000.0,
            (cu1 - cu0) as f64 * TICK_MS,
            (cs1 - cs0) as f64 * TICK_MS,
            enc.len(),
            threads,
            est_min,
            est_typ,
            est_max,
            est_t,
        );
    } else {
        let data = fs::read(outp).expect("read png");
        let (b0, t0) = (vmhwm_kb(), Instant::now());
        let (cu0, cs0) = cpu_ticks();
        let out = zenpng::decode(&data, &zenpng::PngDecodeConfig::default(), &Unstoppable)
            .expect("decode failed");
        let wall = t0.elapsed();
        let (cu1, cs1) = cpu_ticks();
        let peak = vmhwm_kb();
        let px = (out.info.width as u64) * (out.info.height as u64);
        println!(
            "delta_kb={} peak_kb={} wall_ms={:.1} user_ms={:.1} sys_ms={:.1} bytes={}",
            peak.saturating_sub(b0),
            peak,
            wall.as_secs_f64() * 1000.0,
            (cu1 - cu0) as f64 * TICK_MS,
            (cs1 - cs0) as f64 * TICK_MS,
            px
        );
    }
}
