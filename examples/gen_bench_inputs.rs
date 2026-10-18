//! Build `<id>_<format>_<longedge>.png` bench inputs for benches/{vs_png,pareto}.rs
//! and the coz harness, from imazen-26 png-v3 renders.
//!
//! Replacement for scripts/vs_png_inputs.sh on hosts without ImageMagick:
//! zenpng decodes, zenresize (Mitchell) downscales, image-png main encodes —
//! so decode benchmarks don't run on our own encoder's output.
//!
//!   gen_bench_inputs <out_dir> <src.png>...
//!
//! Env: SIZES="64 256 1024 2560" (never upscale), FORMAT_SOURCES="1207 8107
//! 9007 5207 6807" (RGBA8/gray8/pal8/rgb16 at FORMAT_SIZES="1024"),
//! GRAY1_SOURCES="6007".
//!
//! `rgb16` mirrors the IM script's `-depth 16`: every sample is `v * 257`, so
//! it reads the same in either byte order and the encoder's default lossless
//! downcast writes it as 8-bit. EXACT_FORMATS="rgb16full rgba16full gray16full
//! ga16full ga8" (set it empty to skip them) adds, for the same sources and
//! sizes, 16-bit images resampled at 16-bit precision (zenresize's u16 path on
//! the exactly widened source) whose low bytes differ from the high bytes, and
//! gray+alpha at 8 bits. Every 16-bit one is checked to contain such a sample.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use enough::Unstoppable;
use png_main::{BitDepth, ColorType, Compression as PngCompression, Encoder};
use zenpng::{PngDecodeConfig, PngDecodeOutput, decode};
use zenresize::{Filter, PixelDescriptor, ResizeConfig, Resizer};

fn decode_one(path: &Path) -> PngDecodeOutput {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    decode(&data, &PngDecodeConfig::default(), &Unstoppable)
        .unwrap_or_else(|e| panic!("decode {}: {e}", path.display()))
}

fn write_png(
    path: &Path,
    w: u32,
    h: u32,
    color: ColorType,
    depth: BitDepth,
    palette: Option<&[u8]>,
    data: &[u8],
) {
    let mut out = Vec::new();
    let mut e = Encoder::new(&mut out, w, h);
    e.set_color(color);
    e.set_depth(depth);
    e.set_compression(PngCompression::Balanced);
    if let Some(p) = palette {
        e.set_palette(p.to_vec());
    }
    e.write_header().unwrap().write_image_data(data).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap_or_else(|e| panic!("create new {}: {e}", path.display()));
    file.write_all(&out).expect("write fixture");
    file.sync_all().expect("sync fixture");
}

/// Decoded pixels → contiguous RGB8 (gray replicated, alpha dropped, 16-bit
/// truncated to the high byte — input generation only).
fn to_rgb8(out: &PngDecodeOutput) -> (Vec<u8>, u32, u32) {
    let (w, h) = (out.info.width, out.info.height);
    if let Some(img) = out.pixels.try_as_imgref::<rgb::Rgb<u8>>() {
        let (buf, _, _) = img.to_contiguous_buf();
        return (bytemuck::cast_slice(buf.as_ref()).to_vec(), w, h);
    }
    if let Some(img) = out.pixels.try_as_imgref::<rgb::Rgba<u8>>() {
        let (buf, _, _) = img.to_contiguous_buf();
        let px: &[rgb::Rgba<u8>] = buf.as_ref();
        return (px.iter().flat_map(|p| [p.r, p.g, p.b]).collect(), w, h);
    }
    if let Some(img) = out.pixels.try_as_imgref::<rgb::Gray<u8>>() {
        let (buf, _, _) = img.to_contiguous_buf();
        let px: &[rgb::Gray<u8>] = buf.as_ref();
        return (
            px.iter()
                .flat_map(|p| [p.value(), p.value(), p.value()])
                .collect(),
            w,
            h,
        );
    }
    panic!(
        "unsupported layout: ct={} bd={}",
        out.info.color_type, out.info.bit_depth
    );
}

/// Output dimensions for a long edge of `edge`; `None` would upscale.
fn target_dims(w: u32, h: u32, edge: u32) -> Option<(u32, u32)> {
    let long = w.max(h);
    if edge > long {
        return None;
    }
    let scale = edge as f64 / long as f64;
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    Some((nw, nh))
}

fn resize_rgb8(px: &[u8], w: u32, h: u32, edge: u32) -> Option<(Vec<u8>, u32, u32)> {
    let (nw, nh) = target_dims(w, h, edge)?;
    let cfg = ResizeConfig::builder(w, h, nw, nh)
        .filter(Filter::Mitchell)
        .format(PixelDescriptor::RGB8_SRGB)
        .build();
    Some((Resizer::new(&cfg).resize(px), nw, nh))
}

/// RGB8 source widened exactly (`v * 257`), then resampled by zenresize's
/// u16 path, so the result carries the filter's 16-bit precision.
fn resize_rgb16(px: &[u8], w: u32, h: u32, edge: u32) -> Option<(Vec<u16>, u32, u32)> {
    let (nw, nh) = target_dims(w, h, edge)?;
    let wide: Vec<u16> = px.iter().map(|&v| u16::from(v) * 257).collect();
    let cfg = ResizeConfig::builder(w, h, nw, nh)
        .filter(Filter::Mitchell)
        .format(PixelDescriptor::RGB16_SRGB)
        .build();
    Some((Resizer::new(&cfg).resize_u16(&wide), nw, nh))
}

/// Sinusoid alpha pattern in 0..=1 for pixel `i` of a `w`-wide image
/// (mirrors the IM script's `0.5+0.5*sin(i/37)*cos(j/53)`).
fn alpha_unit(i: usize, w: u32) -> f64 {
    let x = (i % w as usize) as f64;
    let y = (i / w as usize) as f64;
    0.5 + 0.5 * (x / 37.0).sin() * (y / 53.0).cos()
}

/// `channels`-sample pixels with `alpha` appended to each.
fn with_alpha(px: &[u16], channels: usize, alpha: &[u16]) -> Vec<u16> {
    px.chunks_exact(channels)
        .zip(alpha)
        .flat_map(|(c, &a)| c.iter().copied().chain([a]))
        .collect()
}

/// Big-endian bytes of 16-bit samples. Panics unless some sample's two bytes
/// differ: an image of `v * 257` samples cannot expose a byte-order error and
/// is downcast to 8 bits by the encoder's defaults.
fn exact_be16(name: &str, samples: &[u16]) -> Vec<u8> {
    assert!(
        samples.iter().any(|s| s >> 8 != s & 0xff),
        "{name}: every sample is v*257"
    );
    samples.iter().flat_map(|s| s.to_be_bytes()).collect()
}

const EXACT_FORMATS: [&str; 5] = ["rgb16full", "rgba16full", "gray16full", "ga16full", "ga8"];

/// Uniform 6×6×6 cube palette (216 entries) + per-pixel index mapping.
/// Dev-quality quantization for benchmark *inputs* (not output data).
fn quantize_cube(rgb: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut palette = Vec::with_capacity(216 * 3);
    for r in 0..6u8 {
        for g in 0..6u8 {
            for b in 0..6u8 {
                palette.extend_from_slice(&[r * 51, g * 51, b * 51]);
            }
        }
    }
    let idx: Vec<u8> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| (c[0] / 51) * 36 + (c[1] / 51) * 6 + c[2] / 51)
        .collect();
    (palette, idx)
}

fn env_list(name: &str, default: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .split_whitespace()
        .map(String::from)
        .collect()
}

fn main() {
    let mut a = std::env::args().skip(1);
    let out_dir = PathBuf::from(
        a.next()
            .expect("usage: gen_bench_inputs <out_dir> <src.png>..."),
    );
    let sources: Vec<String> = a.collect();
    assert!(!sources.is_empty(), "at least one source PNG is required");
    std::fs::create_dir(&out_dir).unwrap_or_else(|e| {
        panic!(
            "a new output directory is required ({}): {e}",
            out_dir.display()
        )
    });
    let sizes: Vec<u32> = env_list("SIZES", "64 256 1024 2560")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    let fmt_sizes: Vec<u32> = env_list("FORMAT_SIZES", "1024")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    let fmt_sources = env_list("FORMAT_SOURCES", "1207 8107 9007 5207 6807");
    let gray1_sources = env_list("GRAY1_SOURCES", "6007");
    let exact_formats = env_list("EXACT_FORMATS", &EXACT_FORMATS.join(" "));
    for f in &exact_formats {
        assert!(
            EXACT_FORMATS.contains(&f.as_str()),
            "EXACT_FORMATS entry {f:?} is not one of {EXACT_FORMATS:?}"
        );
    }

    for arg in sources {
        let src = PathBuf::from(&arg);
        let stem = src.file_stem().unwrap().to_string_lossy();
        // The first four characters (a byte slice would split a multi-byte
        // character in a non-ASCII file name).
        let id: String = stem.chars().take(4).collect();
        let out = decode_one(&src);
        let (rgb, w, h) = to_rgb8(&out);
        eprintln!(
            "{id} {w}x{h} ct={} bd={}",
            out.info.color_type, out.info.bit_depth
        );

        for &edge in &sizes {
            if let Some((px, nw, nh)) = resize_rgb8(&rgb, w, h, edge) {
                write_png(
                    &out_dir.join(format!("{id}_rgb8_{edge}.png")),
                    nw,
                    nh,
                    ColorType::Rgb,
                    BitDepth::Eight,
                    None,
                    &px,
                );
            }
        }

        if fmt_sources.contains(&id) {
            for &edge in &fmt_sizes {
                let Some((px, nw, nh)) = resize_rgb8(&rgb, w, h, edge) else {
                    continue;
                };
                // RGBA8 with a sinusoid alpha pattern (mirrors the IM script).
                let rgba: Vec<u8> = px
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .enumerate()
                    .flat_map(|(i, c)| [c[0], c[1], c[2], (alpha_unit(i, nw) * 255.0) as u8])
                    .collect();
                write_png(
                    &out_dir.join(format!("{id}_rgba8_{edge}.png")),
                    nw,
                    nh,
                    ColorType::Rgba,
                    BitDepth::Eight,
                    None,
                    &rgba,
                );
                // gray8 (luma)
                let gray: Vec<u8> = px
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| {
                        (0.2126 * c[0] as f64 + 0.7152 * c[1] as f64 + 0.0722 * c[2] as f64).round()
                            as u8
                    })
                    .collect();
                write_png(
                    &out_dir.join(format!("{id}_gray8_{edge}.png")),
                    nw,
                    nh,
                    ColorType::Grayscale,
                    BitDepth::Eight,
                    None,
                    &gray,
                );
                // pal8 (cube-quantized)
                let (pal, idx) = quantize_cube(&px);
                write_png(
                    &out_dir.join(format!("{id}_pal8_{edge}.png")),
                    nw,
                    nh,
                    ColorType::Indexed,
                    BitDepth::Eight,
                    Some(&pal),
                    &idx,
                );
                // rgb16 (v*257 replicated, big-endian on the wire)
                let mut be16 = Vec::with_capacity(px.len() * 2);
                for &v in &px {
                    be16.extend_from_slice(&[v, v]);
                }
                write_png(
                    &out_dir.join(format!("{id}_rgb16_{edge}.png")),
                    nw,
                    nh,
                    ColorType::Rgb,
                    BitDepth::Sixteen,
                    None,
                    &be16,
                );
                if exact_formats.is_empty() {
                    continue;
                }
                let (rgb16, w16, h16) = resize_rgb16(&rgb, w, h, edge).expect("same edge as RGB8");
                assert_eq!((w16, h16), (nw, nh));
                let gray16: Vec<u16> = rgb16
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| {
                        (0.2126 * f64::from(c[0])
                            + 0.7152 * f64::from(c[1])
                            + 0.0722 * f64::from(c[2]))
                        .round() as u16
                    })
                    .collect();
                let alpha16: Vec<u16> = (0..gray16.len())
                    .map(|i| (alpha_unit(i, nw) * 65535.0) as u16)
                    .collect();
                for f in &exact_formats {
                    let (color, depth, data) = match f.as_str() {
                        "rgb16full" => (ColorType::Rgb, BitDepth::Sixteen, exact_be16(f, &rgb16)),
                        "rgba16full" => (
                            ColorType::Rgba,
                            BitDepth::Sixteen,
                            exact_be16(f, &with_alpha(&rgb16, 3, &alpha16)),
                        ),
                        "gray16full" => (
                            ColorType::Grayscale,
                            BitDepth::Sixteen,
                            exact_be16(f, &gray16),
                        ),
                        "ga16full" => (
                            ColorType::GrayscaleAlpha,
                            BitDepth::Sixteen,
                            exact_be16(f, &with_alpha(&gray16, 1, &alpha16)),
                        ),
                        "ga8" => (
                            ColorType::GrayscaleAlpha,
                            BitDepth::Eight,
                            gray.iter()
                                .enumerate()
                                .flat_map(|(i, &g)| [g, (alpha_unit(i, nw) * 255.0) as u8])
                                .collect(),
                        ),
                        _ => unreachable!("validated against EXACT_FORMATS"),
                    };
                    write_png(
                        &out_dir.join(format!("{id}_{f}_{edge}.png")),
                        nw,
                        nh,
                        color,
                        depth,
                        None,
                        &data,
                    );
                }
            }
        }

        if gray1_sources.contains(&id) {
            for &edge in &fmt_sizes {
                let Some((px, nw, nh)) = resize_rgb8(&rgb, w, h, edge) else {
                    continue;
                };
                // Packed bilevel rows: MSB-first, threshold 128.
                let mut packed = Vec::with_capacity((nw.div_ceil(8) * nh) as usize);
                for y in 0..nh {
                    let row = &px[y as usize * nw as usize * 3..(y + 1) as usize * nw as usize * 3];
                    let mut byte = 0u8;
                    let mut bits = 0;
                    for c in row.as_chunks::<3>().0 {
                        let g = (0.2126 * c[0] as f64 + 0.7152 * c[1] as f64 + 0.0722 * c[2] as f64)
                            .round() as u8;
                        byte = (byte << 1) | u8::from(g >= 128);
                        bits += 1;
                        if bits == 8 {
                            packed.push(byte);
                            byte = 0;
                            bits = 0;
                        }
                    }
                    if bits > 0 {
                        packed.push(byte << (8 - bits));
                    }
                }
                write_png(
                    &out_dir.join(format!("{id}_gray1_{edge}.png")),
                    nw,
                    nh,
                    ColorType::Grayscale,
                    BitDepth::One,
                    None,
                    &packed,
                );
            }
        }
    }
    eprintln!("done -> {}", out_dir.display());
}
