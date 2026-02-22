/// Quick benchmark for low-level compression modes (None, Fastest, Fast).
/// Includes farbfeld-equivalent baseline (header + raw memcpy).
///
/// Usage:
///   cargo run --release --no-default-features --features _dev --example fastest_test [-- image.png]
use std::time::Instant;

use enough::Unstoppable;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/home/lilith/work/codec-corpus/qoi-benchmark/screenshot_web/reddit.com.png".to_string()
    });
    let source = std::fs::read(&path).expect("read");
    let decoded =
        zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable).unwrap();

    let (w, h) = (decoded.info.width, decoded.info.height);
    let bpp_label = match &decoded.pixels {
        zencodec_types::PixelData::Rgb8(_) => "RGB8",
        zencodec_types::PixelData::Rgba8(_) => "RGBA8",
        _ => panic!("unsupported pixel format"),
    };
    let pixel_bytes: &[u8] = match &decoded.pixels {
        zencodec_types::PixelData::Rgba8(img) => bytemuck::cast_slice(img.buf()),
        zencodec_types::PixelData::Rgb8(img) => bytemuck::cast_slice(img.buf()),
        _ => panic!("unsupported"),
    };
    let raw = w as usize * h as usize * 4;
    println!(
        "Image: {} ({}x{}, {}, {:.2} MiB raw)\n",
        std::path::Path::new(&path)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        w,
        h,
        bpp_label,
        raw as f64 / 1_048_576.0
    );

    // --- Farbfeld baseline: header + raw memcpy ---
    {
        // Warmup
        let _ = farbfeld_write(w, h, pixel_bytes);

        let iters = 10;
        let t = Instant::now();
        let mut size = 0;
        for _ in 0..iters {
            let out = farbfeld_write(w, h, pixel_bytes);
            size = out.len();
            std::hint::black_box(&out);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{:<10} {:>8.1}ms  {:.2}M  ({:.0} MB/s raw throughput)",
            "farbfeld",
            ms,
            size as f64 / 1e6,
            raw as f64 / ms / 1000.0
        );
    }

    // --- memcpy-only baseline: just Vec::with_capacity + extend_from_slice ---
    {
        let iters = 10;
        let t = Instant::now();
        for _ in 0..iters {
            let mut out = Vec::with_capacity(pixel_bytes.len());
            out.extend_from_slice(pixel_bytes);
            std::hint::black_box(&out);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{:<10} {:>8.1}ms  {:.2}M  ({:.0} MB/s raw throughput)",
            "memcpy",
            ms,
            pixel_bytes.len() as f64 / 1e6,
            raw as f64 / ms / 1000.0
        );
    }

    println!();

    let levels = [
        ("None", zenpng::Compression::None),
        ("Fastest", zenpng::Compression::Fastest),
        ("Fast", zenpng::Compression::Fast),
    ];

    for (name, comp) in &levels {
        let config = zenpng::EncodeConfig {
            compression: *comp,
            ..Default::default()
        };
        // Warmup
        let _ = match &decoded.pixels {
            zencodec_types::PixelData::Rgba8(img) => {
                zenpng::encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable)
            }
            zencodec_types::PixelData::Rgb8(img) => {
                zenpng::encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable)
            }
            _ => panic!("unsupported"),
        };

        let t = Instant::now();
        let iters = 5;
        let mut size = 0;
        for _ in 0..iters {
            let result = match &decoded.pixels {
                zencodec_types::PixelData::Rgba8(img) => {
                    zenpng::encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable)
                }
                zencodec_types::PixelData::Rgb8(img) => {
                    zenpng::encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable)
                }
                _ => panic!("unsupported"),
            };
            size = result.unwrap().len();
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{name:<10} {ms:>8.1}ms  {:.2}M  ({:.0} MB/s raw throughput)",
            size as f64 / 1e6,
            raw as f64 / ms / 1000.0
        );
    }
}

/// Farbfeld-equivalent: 16-byte header + raw RGBA8 pixel data.
/// This is the theoretical minimum for an uncompressed image format write.
fn farbfeld_write(w: u32, h: u32, pixel_bytes: &[u8]) -> Vec<u8> {
    let size = 16 + pixel_bytes.len();
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"farbfeld");
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(pixel_bytes);
    out
}
