/// Benchmark decode unfilter throughput: SIMD vs scalar.
///
/// Usage: cargo run --release --example decode_bench --features _dev [-- /path/to/image.png]
///
/// Two benchmarks:
/// 1. Isolated unfilter micro-benchmark (calls unfilter directly)
/// 2. Full decode pipeline (SIMD everywhere vs scalar everywhere)
use std::time::Instant;

use enough::Unstoppable;
use zenpng::{PngDecodeConfig, decode};

fn bench_decode(png_data: &[u8], iterations: u32, raw_bytes: usize) -> f64 {
    let config = PngDecodeConfig::default();
    let _ = decode(png_data, &config, &Unstoppable).unwrap();
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = decode(png_data, &config, &Unstoppable).unwrap();
    }
    let elapsed = start.elapsed().as_secs_f64();
    (raw_bytes as f64 * iterations as f64) / elapsed / 1_000_000.0
}

/// Scalar Paeth predictor for reference unfiltering.
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i16, b as i16, c as i16);
    let p = a + b - c;
    let pa = (p - a).unsigned_abs();
    let pb = (p - b).unsigned_abs();
    let pc = (p - c).unsigned_abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// Apply forward filter to generate filtered row data.
fn apply_filter(filter: u8, row: &[u8], prev: &[u8], bpp: usize, out: &mut [u8]) {
    let len = row.len();
    match filter {
        0 => out[..len].copy_from_slice(row),
        1 => {
            out[..bpp].copy_from_slice(&row[..bpp]);
            for i in bpp..len {
                out[i] = row[i].wrapping_sub(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..len {
                out[i] = row[i].wrapping_sub(prev[i]);
            }
        }
        3 => {
            for i in 0..bpp.min(len) {
                out[i] = row[i].wrapping_sub(prev[i] >> 1);
            }
            for i in bpp..len {
                let avg = ((row[i - bpp] as u16 + prev[i] as u16) >> 1) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
            for i in 0..bpp.min(len) {
                out[i] = row[i].wrapping_sub(paeth_predictor(0, prev[i], 0));
            }
            for i in bpp..len {
                let pred = paeth_predictor(row[i - bpp], prev[i], prev[i - bpp]);
                out[i] = row[i].wrapping_sub(pred);
            }
        }
        _ => {}
    }
}

/// Disable all SIMD tokens including SSE2 baseline.
fn disable_all_simd() {
    // Sse2Token disable cascades upward to V2, V3, V4
    archmage::Sse2Token::dangerously_disable_token_process_wide(true).unwrap();
}

/// Re-enable all SIMD tokens.
fn enable_all_simd() {
    archmage::Sse2Token::dangerously_disable_token_process_wide(false).unwrap();
}

fn bench_unfilter(
    filtered_rows: &[Vec<u8>],
    filter_type: u8,
    bpp: usize,
    stride: usize,
    iters: u32,
) -> f64 {
    let h = filtered_rows.len();
    let total_bytes = stride * h;

    // Warmup
    let mut rows: Vec<Vec<u8>> = filtered_rows.to_vec();
    let mut prev = vec![0u8; stride];
    for row in &mut rows {
        zenpng::__bench_unfilter_row(filter_type, row, &prev, bpp);
        prev.copy_from_slice(row);
    }

    let start = Instant::now();
    for _ in 0..iters {
        let mut rows: Vec<Vec<u8>> = filtered_rows.to_vec();
        let mut prev = vec![0u8; stride];
        for row in &mut rows {
            zenpng::__bench_unfilter_row(filter_type, row, &prev, bpp);
            prev.copy_from_slice(row);
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    (total_bytes as f64 * iters as f64) / elapsed / 1_000_000.0
}

fn main() {
    let image_path = std::env::args().nth(1).unwrap_or_else(|| {
        "/home/lilith/work/codec-corpus/imageflow/test_inputs/frymire-srgb.png".to_string()
    });

    // Decode real image to get raw pixel data
    let png_bytes = std::fs::read(&image_path).unwrap_or_else(|e| {
        panic!("Failed to read {image_path}: {e}");
    });
    let png_decoder = png::Decoder::new(std::io::Cursor::new(&png_bytes));
    let mut reader = png_decoder.read_info().unwrap();
    let mut raw_pixels = vec![0u8; reader.output_buffer_size().unwrap()];
    let output_info = reader.next_frame(&mut raw_pixels).unwrap();
    raw_pixels.truncate(output_info.buffer_size());
    let w = output_info.width as usize;
    let h = output_info.height as usize;
    let bpp = output_info.line_size / w;

    let stride = w * bpp;

    // For small images, tile rows to get enough data for stable timings
    let min_rows = 4096usize;
    let (raw_pixels, h) = if h < min_rows {
        let mut tiled = Vec::with_capacity(stride * min_rows);
        while tiled.len() < stride * min_rows {
            let remaining = stride * min_rows - tiled.len();
            let copy_len = remaining.min(raw_pixels.len());
            tiled.extend_from_slice(&raw_pixels[..copy_len]);
        }
        let new_h = tiled.len() / stride;
        (tiled, new_h)
    } else {
        (raw_pixels, h)
    };

    println!("Image: {image_path}");
    println!("  {w}x{h} (original), bpp={bpp}, {:.1} MB raw\n", (stride * h) as f64 / 1_000_000.0);
    println!("=== Isolated unfilter micro-benchmark ===\n");

    let filters: &[(u8, &str)] = &[
        (1, "Sub"),
        (2, "Up"),
        (3, "Average"),
        (4, "Paeth"),
    ];

    println!(
        "{:<10} {:>12} {:>12} {:>8}",
        "Filter", "SIMD MB/s", "Scalar MB/s", "Speedup"
    );
    println!("{}", "-".repeat(46));

    for &(filter_type, name) in filters {
        // Pre-filter all rows
        let mut filtered_rows: Vec<Vec<u8>> = Vec::with_capacity(h);
        let mut prev_row = vec![0u8; stride];
        for y in 0..h {
            let row = &raw_pixels[y * stride..(y + 1) * stride];
            let mut filtered = vec![0u8; stride];
            apply_filter(filter_type, row, &prev_row, bpp, &mut filtered);
            filtered_rows.push(filtered);
            prev_row.copy_from_slice(row);
        }

        let iters = 10u32;

        // Benchmark SIMD unfilter (all tokens enabled)
        let simd_tp = bench_unfilter(&filtered_rows, filter_type, bpp, stride, iters);

        // Benchmark scalar unfilter (all tokens disabled, including SSE2)
        disable_all_simd();
        let scalar_tp = bench_unfilter(&filtered_rows, filter_type, bpp, stride, iters);
        enable_all_simd();

        let speedup = simd_tp / scalar_tp;
        println!(
            "{:<10} {:>10.0}  {:>10.0}  {:>7.2}x",
            name, simd_tp, scalar_tp, speedup,
        );
    }

    // Full pipeline comparison — decode the original PNG file
    println!("\n=== Full decode pipeline (SIMD vs scalar-only unfilter) ===\n");
    let raw_bytes = stride * h;

    let simd_tp = bench_decode(&png_bytes, 5, raw_bytes);

    // Disable SSE2 (cascades to all higher tiers) for unfilter scalar fallback
    // Note: this also disables zenflate's SIMD, so it understates unfilter-only impact
    disable_all_simd();
    let scalar_tp = bench_decode(&png_bytes, 5, raw_bytes);
    enable_all_simd();

    println!(
        "Full pipeline: SIMD {simd_tp:.0} MB/s, scalar {scalar_tp:.0} MB/s, speedup {:.2}x",
        simd_tp / scalar_tp,
    );
    println!("  (scalar also disables zenflate SIMD, so this understates unfilter-only gains)");
}
