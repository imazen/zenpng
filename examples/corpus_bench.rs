/// Benchmark DEFLATE backends on a PNG corpus using zenpng's filter pipeline.
///
/// Usage: cargo run --release --example corpus_bench [-- /path/to/png/dir [level]]
///
/// Without arguments, auto-downloads the CID22-512 corpus via codec-corpus.
/// Decodes each PNG, applies MinSum filter selection, then compresses
/// with zenflate, libdeflate, flate2, and miniz_oxide. Reports per-level aggregates.
use std::path::Path;
use std::time::Instant;

// ---- PNG filter logic (replicated from zenpng::png_writer) ----

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i16;
    let b = b as i16;
    let c = c as i16;
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

fn apply_filter(filter: u8, row: &[u8], prev_row: &[u8], bpp: usize, out: &mut [u8]) {
    let len = row.len();
    match filter {
        0 => out[..len].copy_from_slice(row),
        1 => {
            let b = bpp.min(len);
            out[..b].copy_from_slice(&row[..b]);
            for i in bpp..len {
                out[i] = row[i].wrapping_sub(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..len {
                out[i] = row[i].wrapping_sub(prev_row[i]);
            }
        }
        3 => {
            for i in 0..bpp.min(len) {
                out[i] = row[i].wrapping_sub(prev_row[i] >> 1);
            }
            for i in bpp..len {
                let avg = ((row[i - bpp] as u16 + prev_row[i] as u16) >> 1) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
            for i in 0..bpp.min(len) {
                out[i] = row[i].wrapping_sub(paeth_predictor(0, prev_row[i], 0));
            }
            for i in bpp..len {
                let pred = paeth_predictor(row[i - bpp], prev_row[i], prev_row[i - bpp]);
                out[i] = row[i].wrapping_sub(pred);
            }
        }
        _ => out[..len].copy_from_slice(row),
    }
}

fn sav_score(data: &[u8]) -> u64 {
    data.iter()
        .map(|&b| if b > 128 { 256 - b as u64 } else { b as u64 })
        .sum()
}

/// Apply best single-filter strategy per MinSum heuristic and return filtered data.
fn filter_image_best(pixels: &[u8], row_bytes: usize, height: usize, bpp: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity((row_bytes + 1) * height);
    let mut prev_row = vec![0u8; row_bytes];
    let mut candidates: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();

    for y in 0..height {
        let row = &pixels[y * row_bytes..(y + 1) * row_bytes];
        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidates[f as usize]);
        }
        let mut best_f = 0u8;
        let mut best_score = u64::MAX;
        for f in 0..5u8 {
            let score = sav_score(&candidates[f as usize]);
            if score < best_score {
                best_score = score;
                best_f = f;
            }
        }
        out.push(best_f);
        out.extend_from_slice(&candidates[best_f as usize]);
        prev_row.copy_from_slice(row);
    }
    out
}

// ---- DEFLATE backends ----

fn compress_zenflate(data: &[u8], level: u32) -> (usize, f64) {
    let mut c = zenflate::Compressor::new(zenflate::CompressionLevel::new(level));
    let bound = zenflate::Compressor::zlib_compress_bound(data.len());
    let mut out = vec![0u8; bound];
    // warmup
    let _ = c.zlib_compress(data, &mut out).unwrap();
    let start = Instant::now();
    let len = c.zlib_compress(data, &mut out).unwrap();
    (len, start.elapsed().as_secs_f64())
}

fn compress_libdeflate(data: &[u8], level: i32) -> (usize, f64) {
    let mut c = libdeflater::Compressor::new(libdeflater::CompressionLvl::new(level).unwrap());
    let bound = c.zlib_compress_bound(data.len());
    let mut out = vec![0u8; bound];
    let _ = c.zlib_compress(data, &mut out).unwrap();
    let start = Instant::now();
    let len = c.zlib_compress(data, &mut out).unwrap();
    (len, start.elapsed().as_secs_f64())
}

fn compress_flate2(data: &[u8], level: u32) -> (usize, f64) {
    let fl = flate2::Compression::new(level.min(9));
    let mut comp = flate2::Compress::new(fl, true); // true = zlib
    let mut out = vec![0u8; data.len() + 4096];
    comp.compress(data, &mut out, flate2::FlushCompress::Finish).unwrap();
    let _ = comp.total_out();
    comp.reset();
    let start = Instant::now();
    comp.compress(data, &mut out, flate2::FlushCompress::Finish).unwrap();
    let len = comp.total_out() as usize;
    (len, start.elapsed().as_secs_f64())
}

fn compress_miniz_oxide(data: &[u8], level: u8) -> (usize, f64) {
    let _ = miniz_oxide::deflate::compress_to_vec_zlib(data, level);
    let start = Instant::now();
    let out = miniz_oxide::deflate::compress_to_vec_zlib(data, level);
    (out.len(), start.elapsed().as_secs_f64())
}

fn resolve_corpus_dir() -> String {
    // CLI argument takes priority
    if let Some(dir) = std::env::args().nth(1) {
        return dir;
    }
    // Auto-download via codec-corpus
    eprintln!("Downloading CID22-512 corpus via codec-corpus (cached after first run)...");
    let corpus = codec_corpus::Corpus::new().expect("can't initialize codec-corpus cache");
    let path = corpus
        .get("CID22/CID22-512")
        .expect("can't download CID22-512 corpus");
    path.to_string_lossy().into_owned()
}

fn main() {
    let dir = resolve_corpus_dir();

    let levels: Vec<u32> = if let Some(l) = std::env::args().nth(2) {
        vec![l.parse().expect("invalid level")]
    } else {
        vec![1, 6, 9, 12]
    };

    // Collect PNG files (recurse into subdirectories)
    let mut paths: Vec<_> = Vec::new();
    collect_pngs(Path::new(&dir), &mut paths);
    paths.sort();
    let n = paths.len();
    println!("Corpus: {dir} ({n} PNGs)\n");

    // Decode all PNGs and pre-filter
    let mut images: Vec<(String, Vec<u8>, usize, usize, usize)> = Vec::new(); // (name, filtered, raw_bytes, w, h)
    let mut total_raw = 0usize;

    for path in &paths {
        let (filtered, w, h, bpp) = decode_and_filter(path);
        let raw_bytes = w * h * bpp;
        total_raw += raw_bytes;
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        images.push((name, filtered, raw_bytes, w, h));
    }
    let total_raw_mib = total_raw as f64 / 1_048_576.0;
    println!("Total raw pixels: {total_raw_mib:.1} MiB\n");

    // Benchmark each level
    for &level in &levels {
        let mut zen_total = 0usize;
        let mut zen_time = 0.0f64;
        let mut lib_total = 0usize;
        let mut lib_time = 0.0f64;
        let mut fl2_total = 0usize;
        let mut fl2_time = 0.0f64;
        let mut mox_total = 0usize;
        let mut mox_time = 0.0f64;

        let fl2_level = level.min(9);
        let mox_level = level.min(9) as u8;

        for (_name, filtered, _raw, _w, _h) in &images {
            let (s, t) = compress_zenflate(filtered, level);
            zen_total += s;
            zen_time += t;

            let (s, t) = compress_libdeflate(filtered, level as i32);
            lib_total += s;
            lib_time += t;

            let (s, t) = compress_flate2(filtered, fl2_level);
            fl2_total += s;
            fl2_time += t;

            let (s, t) = compress_miniz_oxide(filtered, mox_level);
            mox_total += s;
            mox_time += t;
        }

        let fl2_label = if level > 9 {
            format!("L{fl2_level}(cap)")
        } else {
            format!("L{level}")
        };

        println!("=== Level {level} ({n} images, {total_raw_mib:.1} MiB raw) ===");
        println!(
            "{:<14} {:>12} {:>8} {:>10}",
            "Library", "IDAT total", "Ratio", "Speed"
        );
        println!("{}", "-".repeat(48));
        print_row("zenflate", zen_total, total_raw, zen_time);
        print_row("libdeflate", lib_total, total_raw, lib_time);
        print_row(&format!("flate2 {fl2_label}"), fl2_total, total_raw, fl2_time);
        print_row(
            &format!("miniz {fl2_label}"),
            mox_total,
            total_raw,
            mox_time,
        );
        println!();
    }
}

fn print_row(name: &str, compressed: usize, raw: usize, secs: f64) {
    let ratio = compressed as f64 / raw as f64 * 100.0;
    let mib = raw as f64 / 1_048_576.0;
    let speed = mib / secs;
    println!(
        "{:<14} {:>10} B {:>7.2}% {:>8.0} MiB/s",
        name, compressed, ratio, speed
    );
}

fn collect_pngs(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir).expect("can't read directory");
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_pngs(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}

fn decode_and_filter(path: &Path) -> (Vec<u8>, usize, usize, usize) {
    let file = std::fs::File::open(path).expect("can't open PNG");
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().expect("can't read PNG info");
    let info = reader.info();
    let w = info.width as usize;
    let h = info.height as usize;
    let color = info.color_type;
    let bpp = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::GrayscaleAlpha => 2,
        _ => 3, // indexed → treat as RGB
    };
    let row_bytes = w * bpp;
    let mut pixels = vec![0u8; row_bytes * h];
    for y in 0..h {
        let row = reader.next_row().expect("can't read row").unwrap();
        pixels[y * row_bytes..(y + 1) * row_bytes].copy_from_slice(row.data());
    }
    let filtered = filter_image_best(&pixels, row_bytes, h, bpp);
    (filtered, w, h, bpp)
}
