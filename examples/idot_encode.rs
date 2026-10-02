//! Re-encode PNGs with `iDOT` segment counts 1/2/4/8/16 and report size and
//! encode time; writes the outputs for `idot_bench`.
//!
//! Usage:
//!   cargo run --release --example idot_encode --features _dev -- OUT_DIR EFFORT a.png [b.png ...]

use std::time::Instant;

use imgref::ImgVec;
use zenflate::Unstoppable;
use zenpng::{Compression, EncodeConfig, PngDecodeConfig, decode, encode_rgba8};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out_dir = std::path::PathBuf::from(&args[0]);
    let effort: u32 = args[1].parse().unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();
    println!("file\teffort\tsegments\tbytes\tdelta_vs_1\tencode_ms\tidot_written\troundtrip_ok");
    for path in &args[2..] {
        let data = std::fs::read(path).unwrap();
        let dec = decode(
            &data,
            &PngDecodeConfig::default().with_max_threads(1),
            &Unstoppable,
        )
        .unwrap();
        let w = dec.info.width as usize;
        let h = dec.info.height as usize;
        let bytes = dec.pixels.copy_to_contiguous_bytes();
        if bytes.len() != w * h * 4 {
            eprintln!("{path}: not RGBA8, skipped");
            continue;
        }
        let rgba: Vec<rgb::Rgba<u8>> = bytemuck::cast_slice(&bytes).to_vec();
        // Optional crop (top-left) for size sweeps: IDOT_CROP=WxH.
        let (rgba, w, h, tag) = match std::env::var("IDOT_CROP").ok().and_then(|c| {
            let (a, b) = c.split_once('x')?;
            Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
        }) {
            Some((cw, ch)) if cw <= w && ch <= h => {
                let mut v = Vec::with_capacity(cw * ch);
                for y in 0..ch {
                    v.extend_from_slice(&rgba[y * w..y * w + cw]);
                }
                (v, cw, ch, format!("_{cw}x{ch}"))
            }
            _ => (rgba, w, h, String::new()),
        };
        let img = ImgVec::new(rgba, w, h);
        let stem = std::path::Path::new(path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string()
            + &tag;
        let mut base = 0usize;
        let mut base_pixels = Vec::new();
        for n in [1u32, 2, 4, 8, 16] {
            let cfg = EncodeConfig::default()
                .with_compression(Compression::Effort(effort))
                .with_decode_segments(n);
            let t = Instant::now();
            let png = encode_rgba8(img.as_ref(), None, &cfg, &Unstoppable, &Unstoppable).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            if n == 1 {
                base = png.len();
            }
            let has_idot = png.windows(4).any(|w| w == b"iDOT");
            let back = decode(&png, &PngDecodeConfig::default(), &Unstoppable).unwrap();
            // The encoder may downcast (e.g. opaque RGBA -> RGB), so compare
            // every segment count against the unsegmented encode's decode.
            let px = back.pixels.copy_to_contiguous_bytes();
            if n == 1 {
                base_pixels = px.clone();
            }
            let ok = px == base_pixels;
            println!(
                "{stem}\t{effort}\t{n}\t{}\t{:+.3}%\t{ms:.1}\t{has_idot}\t{ok}",
                png.len(),
                (png.len() as f64 / base as f64 - 1.0) * 100.0
            );
            std::fs::write(out_dir.join(format!("{stem}_e{effort}_n{n}.png")), &png).unwrap();
        }
    }
}
