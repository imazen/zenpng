/// Lab for iterating on compression of a single image.
///
/// Tries many filter/compression combos and reports sizes.
/// Usage: cargo run --release --features zopfli --example single_image_lab [-- /path/to/image.png]
use std::io::Write as _;
use std::path::Path;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        // The lion face outlier
        "/home/lilith/work/codec-corpus/clic2025-1024/0d154749c7771f58e89ad343653ec4e20d6f037da829f47f5598e5d0a4ab61f0.png".to_string()
    });

    eprintln!("Loading {}", Path::new(&path).file_stem().unwrap().to_string_lossy());

    let source = std::fs::read(&path).unwrap();
    let decoded = zenpng::decode(&source, None).unwrap();
    let (w, h) = (decoded.info.width as usize, decoded.info.height as usize);

    let (pixel_bytes, bpp): (Vec<u8>, usize) = match &decoded.pixels {
        zencodec_types::PixelData::Rgb8(img) => {
            (bytemuck::cast_slice::<_, u8>(img.buf()).to_vec(), 3)
        }
        zencodec_types::PixelData::Rgba8(img) => {
            (bytemuck::cast_slice::<_, u8>(img.buf()).to_vec(), 4)
        }
        _ => panic!("unsupported format"),
    };
    let row_bytes = w * bpp;

    println!("Image: {}x{}, bpp={bpp}, {:.1} KiB raw", w, h, pixel_bytes.len() as f64 / 1024.0);
    println!("Source PNG: {} bytes", source.len());
    println!();

    // === Part 1: Filter strategy comparison ===
    println!("=== Filter strategies (final compress: zenflate L12) ===");
    println!("{:<45} {:>10} {:>10} {:>8}", "strategy", "zf-L12", "zopfli", "zf/zop%");
    println!("{}", "-".repeat(78));

    // Single filters
    for f in 0..5u8 {
        let name = match f {
            0 => "Single(None)",
            1 => "Single(Sub)",
            2 => "Single(Up)",
            3 => "Single(Average)",
            4 => "Single(Paeth)",
            _ => unreachable!(),
        };
        let filtered = filter_single(&pixel_bytes, row_bytes, h, bpp, f);
        print_sizes(name, &filtered);
    }

    // Adaptive heuristics
    for (name, heuristic) in [
        ("Adaptive(MinSum)", Heuristic::MinSum),
        ("Adaptive(Entropy)", Heuristic::Entropy),
        ("Adaptive(Bigrams)", Heuristic::Bigrams),
        ("Adaptive(BigEnt)", Heuristic::BigEnt),
    ] {
        let filtered = filter_adaptive(&pixel_bytes, row_bytes, h, bpp, heuristic);
        print_sizes(name, &filtered);
    }

    // Brute-force with various eval levels and context rows
    for ctx in [3, 5, 10, 20] {
        for eval in [1, 4, 6, 9] {
            let name = format!("BruteForce(ctx={ctx}, eval=L{eval})");
            let filtered = filter_brute_force(&pixel_bytes, row_bytes, h, bpp, ctx, eval);
            print_sizes(&name, &filtered);
        }
    }

    // === Part 2: What if we evaluate with zopfli? (very slow, just 1 config) ===
    println!();
    println!("=== Brute-force with L12 eval (slow) ===");
    let filtered_l12 = filter_brute_force(&pixel_bytes, row_bytes, h, bpp, 10, 12);
    print_sizes("BruteForce(ctx=10, eval=L12)", &filtered_l12);

    // === Part 2b: Zopfli iteration count sweep ===
    println!();
    println!("=== Zopfli iteration sweep (BruteForce ctx=10, eval=L1) ===");
    let filtered_best = filter_brute_force(&pixel_bytes, row_bytes, h, bpp, 10, 1);
    #[cfg(feature = "zopfli")]
    for iters in [5, 15, 30, 50, 100] {
        let size = compress_zopfli(&filtered_best, iters);
        println!("  zopfli iter={:<4} {:>10}", iters, size);
    }

    // === Part 3: Filter histogram — what does each brute-force pick? ===
    println!();
    println!("=== Filter choice histogram (BruteForce ctx=10) ===");
    println!("{:<30} {:>6} {:>6} {:>6} {:>6} {:>6}", "eval", "None", "Sub", "Up", "Avg", "Paeth");
    for eval in [1, 4, 6, 9, 12] {
        let filtered = filter_brute_force(&pixel_bytes, row_bytes, h, bpp, 10, eval);
        let mut counts = [0u32; 5];
        for y in 0..h {
            let filter_byte = filtered[y * (row_bytes + 1)];
            if (filter_byte as usize) < 5 {
                counts[filter_byte as usize] += 1;
            }
        }
        println!("{:<30} {:>6} {:>6} {:>6} {:>6} {:>6}",
            format!("eval=L{eval}"),
            counts[0], counts[1], counts[2], counts[3], counts[4]);
    }
}

fn print_sizes(name: &str, filtered: &[u8]) {
    let zf12 = compress_zenflate(filtered, 12);
    let zf_best = compress_zenflate(filtered, 10)
        .min(compress_zenflate(filtered, 11))
        .min(zf12);

    #[cfg(feature = "zopfli")]
    let zop = compress_zopfli(filtered, 15);
    #[cfg(not(feature = "zopfli"))]
    let zop = 0usize;

    let gap = if zop > 0 {
        (zf_best as f64 - zop as f64) / zop as f64 * 100.0
    } else {
        0.0
    };

    println!("{:<45} {:>10} {:>10} {:>+8.3}", name, zf_best, zop, gap);
}

fn filter_single(pixel_bytes: &[u8], row_bytes: usize, height: usize, bpp: usize, filter: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity((row_bytes + 1) * height);
    let mut prev_row = vec![0u8; row_bytes];
    let mut filtered_row = vec![0u8; row_bytes];

    for y in 0..height {
        let row = &pixel_bytes[y * row_bytes..(y + 1) * row_bytes];
        apply_filter(filter, row, &prev_row, bpp, &mut filtered_row);
        out.push(filter);
        out.extend_from_slice(&filtered_row);
        prev_row.copy_from_slice(row);
    }
    out
}

#[derive(Clone, Copy)]
enum Heuristic { MinSum, Entropy, Bigrams, BigEnt }

fn filter_adaptive(pixel_bytes: &[u8], row_bytes: usize, height: usize, bpp: usize, heuristic: Heuristic) -> Vec<u8> {
    let mut out = Vec::with_capacity((row_bytes + 1) * height);
    let mut prev_row = vec![0u8; row_bytes];
    let mut candidates: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();

    for y in 0..height {
        let row = &pixel_bytes[y * row_bytes..(y + 1) * row_bytes];
        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidates[f as usize]);
        }

        let best_f = match heuristic {
            Heuristic::MinSum => {
                (0..5u8).min_by_key(|&f| {
                    candidates[f as usize].iter().map(|&b| (b as i8).unsigned_abs() as u64).sum::<u64>()
                }).unwrap()
            }
            Heuristic::Entropy => {
                (0..5u8).min_by(|&a, &b| {
                    entropy_score(&candidates[a as usize])
                        .partial_cmp(&entropy_score(&candidates[b as usize]))
                        .unwrap()
                }).unwrap()
            }
            Heuristic::Bigrams => {
                (0..5u8).min_by_key(|&f| bigrams_score(&candidates[f as usize])).unwrap()
            }
            Heuristic::BigEnt => {
                (0..5u8).min_by(|&a, &b| {
                    bigram_entropy_score(&candidates[a as usize])
                        .partial_cmp(&bigram_entropy_score(&candidates[b as usize]))
                        .unwrap()
                }).unwrap()
            }
        };

        out.push(best_f);
        out.extend_from_slice(&candidates[best_f as usize]);
        prev_row.copy_from_slice(row);
    }
    out
}

fn filter_brute_force(
    pixel_bytes: &[u8], row_bytes: usize, height: usize, bpp: usize,
    context_rows: usize, eval_level: u32,
) -> Vec<u8> {
    let filtered_row_size = row_bytes + 1;
    let max_context_bytes = 32 * 1024;
    let context_rows = context_rows
        .min(max_context_bytes / filtered_row_size)
        .max(1);
    let max_context = context_rows * filtered_row_size;

    let mut eval_compressor = zenflate::Compressor::new(
        zenflate::CompressionLevel::new(eval_level),
    );

    let eval_max_input = max_context + filtered_row_size;
    let compress_bound = zenflate::Compressor::zlib_compress_bound(eval_max_input);
    let mut compress_buf = vec![0u8; compress_bound];
    let mut candidate_data: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();
    let mut eval_buf = Vec::with_capacity(eval_max_input);
    let mut prev_row = vec![0u8; row_bytes];
    let mut out = Vec::with_capacity(filtered_row_size * height);

    for y in 0..height {
        let row = &pixel_bytes[y * row_bytes..(y + 1) * row_bytes];
        let context_start = if out.len() > max_context { out.len() - max_context } else { 0 };
        let context = &out[context_start..];

        let mut best_f = 0u8;
        let mut best_size = usize::MAX;

        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidate_data[f as usize]);
            eval_buf.clear();
            eval_buf.extend_from_slice(context);
            eval_buf.push(f);
            eval_buf.extend_from_slice(&candidate_data[f as usize]);

            if let Ok(len) = eval_compressor.zlib_compress(&eval_buf, &mut compress_buf) {
                if len < best_size {
                    best_size = len;
                    best_f = f;
                }
            }
        }

        out.push(best_f);
        out.extend_from_slice(&candidate_data[best_f as usize]);
        prev_row.copy_from_slice(row);
    }
    out
}

fn apply_filter(filter: u8, row: &[u8], prev_row: &[u8], bpp: usize, out: &mut [u8]) {
    match filter {
        0 => out.copy_from_slice(row),
        1 => {
            out[..bpp].copy_from_slice(&row[..bpp]);
            for i in bpp..row.len() { out[i] = row[i].wrapping_sub(row[i - bpp]); }
        }
        2 => {
            for i in 0..row.len() { out[i] = row[i].wrapping_sub(prev_row[i]); }
        }
        3 => {
            for i in 0..bpp { out[i] = row[i].wrapping_sub(prev_row[i] / 2); }
            for i in bpp..row.len() {
                let avg = ((row[i - bpp] as u16 + prev_row[i] as u16) / 2) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
            for i in 0..bpp { out[i] = row[i].wrapping_sub(paeth(0, prev_row[i], 0)); }
            for i in bpp..row.len() {
                out[i] = row[i].wrapping_sub(paeth(row[i - bpp], prev_row[i], prev_row[i - bpp]));
            }
        }
        _ => out.copy_from_slice(row),
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).unsigned_abs();
    let pb = (p - b as i16).unsigned_abs();
    let pc = (p - c as i16).unsigned_abs();
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

fn entropy_score(data: &[u8]) -> f64 {
    let mut counts = [0u32; 256];
    for &b in data { counts[b as usize] += 1; }
    let len = data.len() as f64;
    let mut entropy = 0.0f64;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

fn bigrams_score(data: &[u8]) -> usize {
    if data.len() < 2 { return 0; }
    let mut seen = vec![false; 65536];
    let mut count = 0;
    for pair in data.windows(2) {
        let key = (pair[0] as usize) << 8 | pair[1] as usize;
        if !seen[key] { seen[key] = true; count += 1; }
    }
    count
}

fn bigram_entropy_score(data: &[u8]) -> f64 {
    if data.len() < 2 { return 0.0; }
    let mut counts = vec![0u32; 65536];
    let n = data.len() - 1;
    for pair in data.windows(2) {
        counts[(pair[0] as usize) << 8 | pair[1] as usize] += 1;
    }
    let nf = n as f64;
    let mut entropy = 0.0f64;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / nf;
            entropy -= p * p.log2();
        }
    }
    entropy
}

fn compress_zenflate(data: &[u8], level: u32) -> usize {
    let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(level));
    let bound = zenflate::Compressor::zlib_compress_bound(data.len());
    let mut buf = vec![0u8; bound];
    compressor.zlib_compress(data, &mut buf).unwrap()
}

#[cfg(feature = "zopfli")]
fn compress_zopfli(data: &[u8], iterations: i32) -> usize {
    let options = zopfli::Options {
        iteration_count: std::num::NonZeroU64::new(iterations as u64).unwrap(),
        ..Default::default()
    };
    let mut output = Vec::new();
    zopfli::compress(options, zopfli::Format::Zlib, data, &mut output).unwrap();
    output.len()
}
