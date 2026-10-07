//! Lossless roundtrip sweep: encode every PNG in a directory at each listed
//! effort and check that decoding gives the same pixels (compared as RGBA16,
//! so a smaller colour type or bit depth chosen by the encoder is fine).
//!
//!   cargo run --release --features _dev --example roundtrip_sweep -- DIR 0,1,7,13,19
//!
//! `ROUNDTRIP_SIZES=1` also prints each encode's size and time (one run, not
//! a benchmark; use benches/pareto.rs for timings). `ROUNDTRIP_SAVE=<dir>`
//! writes each encode to `<dir>/<input stem>_e<effort>.png`.
//!
//! Used 2026-10-06 to check the encoder's per-candidate decompress verify:
//! 2098 encodes over scripts/vs_png_inputs.sh output, efforts 0-24 and 30.
use zenflate::Unstoppable;
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let efforts: Vec<u32> = std::env::args()
        .nth(2)
        .unwrap()
        .split(',')
        .map(|e| e.parse().unwrap())
        .collect();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    files.sort();
    let mut n = 0;
    let verbose = std::env::var_os("ROUNDTRIP_SIZES").is_some();
    // ROUNDTRIP_THREADS: 1 (default) single-threaded, 0 all cores, N at most N.
    let threads: usize = std::env::var("ROUNDTRIP_THREADS").map_or(1, |t| t.parse().unwrap());
    for f in &files {
        let data = std::fs::read(f).unwrap();
        let d = zenpng::decode(&data, &zenpng::PngDecodeConfig::default(), &Unstoppable).unwrap();
        let src = rgba16(&d.pixels);
        for &e in &efforts {
            let mut cfg = zenpng::EncodeConfig::default()
                .with_compression(zenpng::Compression::Effort(e))
                .with_parallel(threads != 1);
            cfg.max_threads = threads;
            let px = &d.pixels;
            let t = std::time::Instant::now();
            let r = if let Some(i) = px.try_as_imgref::<rgb::Rgb<u8>>() {
                zenpng::encode_rgb8(i, None, &cfg, &Unstoppable, &Unstoppable)
            } else if let Some(i) = px.try_as_imgref::<rgb::Rgba<u8>>() {
                zenpng::encode_rgba8(i, None, &cfg, &Unstoppable, &Unstoppable)
            } else if let Some(i) = px.try_as_imgref::<rgb::Gray<u8>>() {
                zenpng::encode_gray8(i, None, &cfg, &Unstoppable, &Unstoppable)
            } else if let Some(i) = px.try_as_imgref::<rgb::Rgb<u16>>() {
                zenpng::encode_rgb16(i, None, &cfg, &Unstoppable, &Unstoppable)
            } else {
                println!("skip {} {:?}", f.display(), px.descriptor());
                break;
            };
            let png = r.unwrap_or_else(|err| panic!("{}: e{e}: {err}", f.display()));
            if let Some(dir) = std::env::var_os("ROUNDTRIP_SAVE") {
                let stem = f.file_stem().unwrap().to_string_lossy();
                std::fs::write(
                    std::path::Path::new(&dir).join(format!("{stem}_e{e}.png")),
                    &png,
                )
                .unwrap();
            }
            if verbose {
                println!(
                    "{}\te{e}\t{}\t{:.2} ms",
                    f.file_name().unwrap().to_string_lossy(),
                    png.len(),
                    t.elapsed().as_secs_f64() * 1e3
                );
            }
            let back =
                zenpng::decode(&png, &zenpng::PngDecodeConfig::strict(), &Unstoppable).unwrap();
            if rgba16(&back.pixels) != src {
                panic!(
                    "{} e{e}: pixel mismatch ({:?} -> {:?})",
                    f.display(),
                    d.pixels.descriptor(),
                    back.pixels.descriptor()
                );
            }
            n += 1;
        }
        println!("{} ok", f.file_name().unwrap().to_string_lossy());
    }
    println!("DONE {n} encodes");
}

/// Any gray/GA/RGB/RGBA 8- or 16-bit buffer as RGBA16 (8-bit scaled by 257).
fn rgba16(px: &zenpixels::PixelBuffer) -> Vec<u16> {
    use zenpixels::{ChannelLayout, ChannelType};
    let desc = px.descriptor();
    let ch = desc.layout().channels();
    let wide = match desc.channel_type() {
        ChannelType::U8 => false,
        ChannelType::U16 => true,
        t => panic!("channel type {t:?}"),
    };
    let b = px.copy_to_contiguous_bytes();
    let v: Vec<u16> = if wide {
        b.as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_ne_bytes(*c))
            .collect()
    } else {
        b.iter().map(|&x| x as u16 * 257).collect()
    };
    let mut out = Vec::with_capacity(v.len() / ch * 4);
    for p in v.chunks_exact(ch) {
        let px = match desc.layout() {
            ChannelLayout::Gray => [p[0], p[0], p[0], 65535],
            ChannelLayout::GrayAlpha => [p[0], p[0], p[0], p[1]],
            ChannelLayout::Rgb => [p[0], p[1], p[2], 65535],
            ChannelLayout::Rgba => [p[0], p[1], p[2], p[3]],
            l => panic!("layout {l:?}"),
        };
        // Fully transparent pixels: colour is not preserved (documented zeroing).
        out.extend(if px[3] == 0 { [0, 0, 0, 0] } else { px });
    }
    out
}
