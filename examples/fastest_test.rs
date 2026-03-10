/// Quick benchmark for encode (None/Fastest/Fast) and decode (zenpng/png/lodepng).
/// Includes farbfeld-equivalent baseline (header + raw memcpy).
///
/// Usage:
///   cargo run --release --no-default-features --features _dev --example fastest_test [-- image.png]
use std::time::Instant;

use enough::Unstoppable;
use zenpixels::descriptor::{ChannelLayout, ChannelType};
use zenpixels_convert::PixelBufferConvertTypedExt;

fn bench_ms<F: FnMut()>(warmup: u32, iters: u32, mut f: F) -> f64 {
    for _ in 0..warmup {
        f();
    }
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    t.elapsed().as_secs_f64() * 1000.0 / iters as f64
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/qoi-benchmark/screenshot_web/reddit.com.png",
            std::env::var("CODEC_CORPUS_DIR")
                .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string())
        )
    });
    let source = std::fs::read(&path).expect("read");
    let decoded = zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable).unwrap();

    let (w, h) = (decoded.info.width, decoded.info.height);
    let desc = decoded.pixels.descriptor();
    let bpp_label = match (desc.layout(), desc.channel_type()) {
        (ChannelLayout::Rgb, ChannelType::U8) => "RGB8",
        (ChannelLayout::Rgba, ChannelType::U8) => "RGBA8",
        _ => panic!("unsupported pixel format: {:?}", desc),
    };
    let pixel_bytes_owned = decoded.pixels.copy_to_contiguous_bytes();
    let pixel_bytes: &[u8] = &pixel_bytes_owned;
    let bpp = desc.bytes_per_pixel();
    let raw = w as usize * h as usize * bpp;
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

    // ── Encode benchmarks ─────────────────────────────────────────────

    println!("=== Encode ===\n");

    // --- Farbfeld baseline: header + raw memcpy ---
    {
        let mut size = 0;
        let ms = bench_ms(1, 10, || {
            let out = farbfeld_write(w, h, pixel_bytes);
            size = out.len();
            std::hint::black_box(&out);
        });
        println!(
            "{:<14} {:>8.1}ms  {:.2}M  ({:.0} MB/s)",
            "farbfeld",
            ms,
            size as f64 / 1e6,
            raw as f64 / ms / 1000.0
        );
    }

    // --- memcpy-only baseline ---
    {
        let ms = bench_ms(1, 10, || {
            let mut out = Vec::with_capacity(pixel_bytes.len());
            out.extend_from_slice(pixel_bytes);
            std::hint::black_box(&out);
        });
        println!(
            "{:<14} {:>8.1}ms  {:.2}M  ({:.0} MB/s)",
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
        let config = zenpng::EncodeConfig::default().with_compression(*comp);
        let mut size = 0;
        let ms = bench_ms(1, 5, || {
            let result = match (desc.layout(), desc.channel_type()) {
                (ChannelLayout::Rgba, ChannelType::U8) => {
                    let buf = decoded.pixels.to_rgba8();
                    zenpng::encode_rgba8(buf.as_imgref(), None, &config, &Unstoppable, &Unstoppable)
                }
                (ChannelLayout::Rgb, ChannelType::U8) => {
                    let buf = decoded.pixels.to_rgb8();
                    zenpng::encode_rgb8(buf.as_imgref(), None, &config, &Unstoppable, &Unstoppable)
                }
                _ => panic!("unsupported format: {:?}", desc),
            };
            size = result.unwrap().len();
        });
        println!(
            "{name:<14} {ms:>8.1}ms  {:.2}M  ({:.0} MB/s)",
            size as f64 / 1e6,
            raw as f64 / ms / 1000.0
        );
    }

    // ── Decode benchmarks ─────────────────────────────────────────────

    // Re-encode at None for stored-block decode test
    let none_config = zenpng::EncodeConfig::default().with_compression(zenpng::Compression::None);
    let none_png = match (desc.layout(), desc.channel_type()) {
        (ChannelLayout::Rgba, ChannelType::U8) => {
            let buf = decoded.pixels.to_rgba8();
            zenpng::encode_rgba8(
                buf.as_imgref(),
                None,
                &none_config,
                &Unstoppable,
                &Unstoppable,
            )
            .unwrap()
        }
        (ChannelLayout::Rgb, ChannelType::U8) => {
            let buf = decoded.pixels.to_rgb8();
            zenpng::encode_rgb8(
                buf.as_imgref(),
                None,
                &none_config,
                &Unstoppable,
                &Unstoppable,
            )
            .unwrap()
        }
        _ => panic!("unsupported format: {:?}", desc),
    };

    println!(
        "\n=== Decode None ({:.2}M stored) ===\n",
        none_png.len() as f64 / 1e6
    );

    {
        let config = zenpng::PngDecodeConfig::none();
        let ms = bench_ms(1, 10, || {
            let d = zenpng::decode(&none_png, &config, &Unstoppable).unwrap();
            std::hint::black_box(&d);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "zenpng",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    {
        let ms = bench_ms(1, 10, || {
            let decoder = png::Decoder::new(std::io::Cursor::new(&none_png));
            let mut reader = decoder.read_info().unwrap();
            let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut buf).unwrap();
            std::hint::black_box(&buf);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "png",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    // Re-encode at Fast for a reasonably compressed test file
    let fast_config = zenpng::EncodeConfig::default().with_compression(zenpng::Compression::Fast);
    let test_png = match (desc.layout(), desc.channel_type()) {
        (ChannelLayout::Rgba, ChannelType::U8) => {
            let buf = decoded.pixels.to_rgba8();
            zenpng::encode_rgba8(
                buf.as_imgref(),
                None,
                &fast_config,
                &Unstoppable,
                &Unstoppable,
            )
            .unwrap()
        }
        (ChannelLayout::Rgb, ChannelType::U8) => {
            let buf = decoded.pixels.to_rgb8();
            zenpng::encode_rgb8(
                buf.as_imgref(),
                None,
                &fast_config,
                &Unstoppable,
                &Unstoppable,
            )
            .unwrap()
        }
        _ => panic!("unsupported format: {:?}", desc),
    };

    println!(
        "\n=== Decode ({:.2}M compressed) ===\n",
        test_png.len() as f64 / 1e6
    );

    // --- zenpng default ---
    {
        let config = zenpng::PngDecodeConfig::none();
        let ms = bench_ms(1, 10, || {
            let d = zenpng::decode(&test_png, &config, &Unstoppable).unwrap();
            std::hint::black_box(&d);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "zenpng",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    // --- zenpng lenient (skip checksums) ---
    {
        let config = zenpng::PngDecodeConfig::lenient();
        let ms = bench_ms(1, 10, || {
            let d = zenpng::decode(&test_png, &config, &Unstoppable).unwrap();
            std::hint::black_box(&d);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "zenpng-lenient",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    // --- png crate ---
    {
        let ms = bench_ms(1, 10, || {
            let decoder = png::Decoder::new(std::io::Cursor::new(&test_png));
            let mut reader = decoder.read_info().unwrap();
            let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut buf).unwrap();
            std::hint::black_box(&buf);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "png",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    // --- lodepng ---
    {
        let ms = bench_ms(1, 10, || {
            let result = lodepng::decode32(&test_png).unwrap();
            std::hint::black_box(&result);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "lodepng",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    // --- Also decode the original source file ---
    println!(
        "\n=== Decode original ({:.2}M) ===\n",
        source.len() as f64 / 1e6
    );

    {
        let config = zenpng::PngDecodeConfig::none();
        let ms = bench_ms(1, 10, || {
            let d = zenpng::decode(&source, &config, &Unstoppable).unwrap();
            std::hint::black_box(&d);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "zenpng",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    {
        let config = zenpng::PngDecodeConfig::lenient();
        let ms = bench_ms(1, 10, || {
            let d = zenpng::decode(&source, &config, &Unstoppable).unwrap();
            std::hint::black_box(&d);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "zenpng-lenient",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    {
        let ms = bench_ms(1, 10, || {
            let decoder = png::Decoder::new(std::io::Cursor::new(&source));
            let mut reader = decoder.read_info().unwrap();
            let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut buf).unwrap();
            std::hint::black_box(&buf);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "png",
            ms,
            raw as f64 / ms / 1000.0
        );
    }

    {
        let ms = bench_ms(1, 10, || {
            let result = lodepng::decode32(&source).unwrap();
            std::hint::black_box(&result);
        });
        println!(
            "{:<14} {:>8.1}ms  ({:.0} MB/s)",
            "lodepng",
            ms,
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
