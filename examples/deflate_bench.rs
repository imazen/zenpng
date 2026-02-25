/// Benchmark zenflate effort levels vs zenzop (Zopfli) on real PNG IDAT data.
///
/// Decodes PNGs, applies Sub filter, compresses the filtered bytes with multiple
/// compressor configurations, and reports size and speed.
///
/// Usage: cargo run --release --features zopfli --example deflate_bench [-- /path/to/png/dir]
use std::path::{Path, PathBuf};
use std::time::Instant;

use enough::Unstoppable;
use zenflate::{CompressionLevel, Compressor};

const EFFORTS: &[(&str, u32)] = &[("zf22", 22), ("zf26", 26), ("zf30", 30)];

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/lilith/work/codec-corpus/clic2025-1024".to_string());

    let mut paths: Vec<PathBuf> = Vec::new();
    collect_pngs(Path::new(&dir), &mut paths);
    paths.sort();
    if paths.len() > 10 {
        paths.truncate(10);
    }
    let n = paths.len();
    eprintln!("Benchmarking {n} images from {dir}\n");

    let num_configs = EFFORTS.len();
    let mut totals_size = vec![0usize; num_configs];
    let mut totals_ms = vec![0f64; num_configs];
    let mut total_zop_size = 0usize;
    let mut total_zop_ms = 0f64;
    let mut total_raw = 0usize;

    print!("{:<12} {:>8}", "image", "raw_KB");
    for (name, _) in EFFORTS {
        print!(" {:>9} {:>6}", name, "ms");
    }
    print!(" {:>9} {:>6}", "zenzop", "ms");
    println!();
    println!("{}", "-".repeat(12 + 9 + (num_configs + 1) * 16));

    for path in &paths {
        let name: String = path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .chars()
            .take(12)
            .collect();

        let filtered = match prepare_filtered_idat(path) {
            Some(d) => d,
            None => continue,
        };
        let raw_len = filtered.len();
        total_raw += raw_len;

        print!("{:<12} {:>8}", name, raw_len / 1024);

        for (i, &(_name, effort)) in EFFORTS.iter().enumerate() {
            let (size, ms) = bench_zenflate(&filtered, effort);
            totals_size[i] += size;
            totals_ms[i] += ms;
            print!(" {:>9} {:>6.0}", size, ms);
        }

        let (zop_size, zop_ms) = bench_zenzop(&filtered);
        total_zop_size += zop_size;
        total_zop_ms += zop_ms;
        print!(" {:>9} {:>6.0}", zop_size, zop_ms);

        println!();
    }

    println!("{}", "-".repeat(12 + 9 + (num_configs + 1) * 16));
    print!("{:<12} {:>8}", "TOTAL", total_raw / 1024);
    for i in 0..num_configs {
        print!(" {:>9} {:>6.0}", totals_size[i], totals_ms[i]);
    }
    print!(" {:>9} {:>6.0}", total_zop_size, total_zop_ms);
    println!();

    eprintln!();
    if total_zop_size > 0 {
        for (i, &(name, _)) in EFFORTS.iter().enumerate() {
            let ratio = totals_size[i] as f64 / total_zop_size as f64;
            let speedup = total_zop_ms / totals_ms[i];
            eprintln!(
                "{:<10}: {:.4}x size vs zenzop, {:.1}x faster ({:.0}ms vs {:.0}ms)",
                name, ratio, speedup, totals_ms[i], total_zop_ms
            );
        }
    }
}

fn prepare_filtered_idat(path: &Path) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    let decoded = zenpng::decode(&data, &zenpng::PngDecodeConfig::none(), &Unstoppable).ok()?;
    let info = &decoded.info;
    let w = info.width as usize;
    let h = info.height as usize;

    let raw_pixels: Vec<u8> = match &decoded.pixels {
        zencodec_types::PixelData::Rgb8(img) => bytemuck::cast_slice(img.buf()).to_vec(),
        zencodec_types::PixelData::Rgba8(img) => bytemuck::cast_slice(img.buf()).to_vec(),
        zencodec_types::PixelData::Gray8(img) => img.buf().iter().map(|g| g.value()).collect(),
        _ => return None,
    };

    let bpp = raw_pixels.len() / (w * h);
    let row_bytes = w * bpp;

    if raw_pixels.len() != row_bytes * h {
        return None;
    }

    // Apply Sub filter to all rows
    let mut filtered = Vec::with_capacity(h * (1 + row_bytes));
    for y in 0..h {
        filtered.push(1); // Sub filter byte
        let row = &raw_pixels[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..row_bytes {
            let left = if x >= bpp { row[x - bpp] } else { 0 };
            filtered.push(row[x].wrapping_sub(left));
        }
    }

    Some(filtered)
}

fn bench_zenflate(data: &[u8], effort: u32) -> (usize, f64) {
    let mut compressor = Compressor::new(CompressionLevel::new(effort));
    let bound = Compressor::zlib_compress_bound(data.len());
    let mut out = vec![0u8; bound];

    let start = Instant::now();
    let size = compressor
        .zlib_compress(data, &mut out, Unstoppable)
        .expect("zenflate compress failed");
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    (size, ms)
}

#[cfg(feature = "zopfli")]
fn bench_zenzop(data: &[u8]) -> (usize, f64) {
    let mut out = Vec::new();

    let start = Instant::now();
    zenzop::compress(
        zenzop::Options::default(),
        zenzop::Format::Zlib,
        std::io::Cursor::new(data),
        &mut out,
    )
    .expect("zenzop compress failed");
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    (out.len(), ms)
}

#[cfg(not(feature = "zopfli"))]
fn bench_zenzop(_data: &[u8]) -> (usize, f64) {
    eprintln!("zenzop not available (build with --features zopfli)");
    (0, 0.0)
}

fn collect_pngs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}
