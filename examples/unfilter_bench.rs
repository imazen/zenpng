//! Clean per-filter unfilter throughput bench (SIMD production path only).
//!
//! Measures the runtime-dispatched (SIMD) `__bench_unfilter_row` for each of
//! Sub/Up/Average/Paeth at bpp=3 and bpp=4, printing one flushed line per
//! result. Unlike `decode_bench`, this does not toggle archmage tokens (which
//! is x86-specific and truncates output on aarch64) — it measures the path
//! users actually get.
//!
//! Usage:
//!   cargo run --release --example unfilter_bench --features _dev [-- /path/to/rgb.png]

use std::io::Write;
use std::time::Instant;

/// Scalar Paeth predictor for forward filtering reference.
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

/// Apply forward filter to generate filtered row data (so unfilter has work to do).
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

fn bench_unfilter(
    filtered_rows: &[Vec<u8>],
    filter_type: u8,
    bpp: usize,
    stride: usize,
    iters: u32,
) -> f64 {
    let mut work_rows: Vec<Vec<u8>> = filtered_rows.to_vec();
    let mut prev = vec![0u8; stride];

    // Warmup
    for (work, src) in work_rows.iter_mut().zip(filtered_rows.iter()) {
        work.copy_from_slice(src);
        zenpng::__bench_unfilter_row(filter_type, work, &prev, bpp);
        prev.copy_from_slice(work);
    }

    let total_bytes = stride * filtered_rows.len();
    let start = Instant::now();
    for _ in 0..iters {
        prev.fill(0);
        for (work, src) in work_rows.iter_mut().zip(filtered_rows.iter()) {
            work.copy_from_slice(src);
            zenpng::__bench_unfilter_row(filter_type, work, &prev, bpp);
            prev.copy_from_slice(work);
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    (total_bytes as f64 * iters as f64) / elapsed / 1_000_000.0
}

/// Build `h` rows of `stride` bytes from a seed buffer (tiled).
fn build_rows(raw: &[u8], stride: usize, h: usize, bpp: usize) -> Vec<Vec<u8>> {
    // Reconstruct contiguous raw pixels of stride*h by tiling the seed.
    let mut raw_full = Vec::with_capacity(stride * h);
    while raw_full.len() < stride * h {
        let need = stride * h - raw_full.len();
        let take = need.min(raw.len());
        raw_full.extend_from_slice(&raw[..take]);
    }
    let _ = bpp;
    let mut rows = Vec::with_capacity(h);
    for y in 0..h {
        rows.push(raw_full[y * stride..(y + 1) * stride].to_vec());
    }
    rows
}

fn run_for_bpp(label: &str, raw_seed: &[u8], w: usize, bpp: usize) {
    let stride = w * bpp;
    let min_rows = 4096usize;
    let h = min_rows;
    let raw_rows = build_rows(raw_seed, stride, h, bpp);

    println!(
        "\n=== {label} (bpp={bpp}, {w}x{h}, {:.1} MB raw) ===",
        (stride * h) as f64 / 1e6
    );
    println!("{:<10} {:>12}", "Filter", "SIMD MB/s");
    println!("{}", "-".repeat(24));
    let _ = std::io::stdout().flush();

    let filters: &[(u8, &str)] = &[(1, "Sub"), (2, "Up"), (3, "Average"), (4, "Paeth")];
    let iters = 10u32;
    for &(ft, name) in filters {
        // Pre-filter all rows.
        let mut filtered_rows: Vec<Vec<u8>> = Vec::with_capacity(h);
        let mut prev_row = vec![0u8; stride];
        for r in &raw_rows {
            let mut filtered = vec![0u8; stride];
            apply_filter(ft, r, &prev_row, bpp, &mut filtered);
            filtered_rows.push(filtered);
            prev_row.copy_from_slice(r);
        }
        let tp = bench_unfilter(&filtered_rows, ft, bpp, stride, iters);
        println!("{name:<10} {tp:>12.0}");
        let _ = std::io::stdout().flush();
    }
}

fn main() {
    // bpp=3 seed from a real RGB PNG (frymire), or synthetic if not provided.
    let arg = std::env::args().nth(1);
    let (rgb_seed, w3) = if let Some(path) = arg {
        let bytes = std::fs::read(&path).expect("read png");
        let dec = png::Decoder::new(std::io::Cursor::new(&bytes));
        let mut reader = dec.read_info().unwrap();
        let mut raw = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut raw).unwrap();
        raw.truncate(info.buffer_size());
        let w = info.width as usize;
        let bpp = info.line_size / w;
        // Only use as bpp=3 seed if it actually is RGB.
        if bpp == 3 {
            (raw, w)
        } else {
            // Take the RGB channels of whatever it is by truncating to a width.
            (raw, w)
        }
    } else {
        // Synthetic RGB gradient.
        let w = 1024usize;
        let mut v = Vec::with_capacity(w * 3 * 256);
        for i in 0..(w * 256) {
            v.extend_from_slice(&[(i * 7 + 3) as u8, (i * 5 + 11) as u8, (i * 3 + 17) as u8]);
        }
        (v, w)
    };
    run_for_bpp("RGB seed", &rgb_seed, w3, 3);

    // bpp=4 synthetic RGBA — exercises the batched Sub/Avg/Paeth bpp=4 NEON paths.
    let w4 = 1024usize;
    let mut rgba = Vec::with_capacity(w4 * 4 * 256);
    for i in 0..(w4 * 256) {
        rgba.extend_from_slice(&[
            (i * 7 + 3) as u8,
            (i * 5 + 11) as u8,
            (i * 3 + 17) as u8,
            (i * 11 + 5) as u8,
        ]);
    }
    run_for_bpp("RGBA synthetic", &rgba, w4, 4);
}
