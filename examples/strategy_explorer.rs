/// Exhaustive strategy x compression level matrix with timing.
///
/// Tests all filter strategy and zenflate level combinations, outputting
/// CSV to stdout for analysis. Also prints a human-readable efficiency
/// frontier (top 20 by bytes-saved/ms) to stderr.
///
/// Usage:
///   cargo run --release --example strategy_explorer -- <image.png> > results.csv
///   cargo run --release --example strategy_explorer -- <image.png> > $ZENPNG_OUTPUT_DIR/explore.csv
use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use enough::Unstoppable;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/CID22/CID22-512/validation/1025469.png",
            std::env::var("CODEC_CORPUS_DIR")
                .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string())
        )
    });

    let source = std::fs::read(&path).unwrap();
    let decoded = zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable)
        .expect("failed to decode");
    let (w, h) = (decoded.info.width as usize, decoded.info.height as usize);

    let bpp = decoded.pixels.descriptor().bytes_per_pixel();
    let pixel_bytes = decoded.pixels.copy_to_contiguous_bytes();
    let row_bytes = w * bpp;

    let fname = Path::new(&path).file_name().unwrap().to_string_lossy();
    eprintln!(
        "Image: {} ({}x{}, bpp={}, {:.1} KiB raw)",
        fname,
        w,
        h,
        bpp,
        pixel_bytes.len() as f64 / 1024.0
    );

    // Baseline: Single(None) + L1
    let baseline_filtered = filter_single(&pixel_bytes, row_bytes, h, bpp, 0);
    let baseline_size = compress_zenflate(&baseline_filtered, 1);
    eprintln!("Baseline: Single(None)+L1 = {} bytes\n", baseline_size);

    // CSV header
    println!(
        "strategy,context,eval,block,zf_level,filter_ms,compress_ms,total_ms,size,vs_baseline_pct"
    );

    let mut results: Vec<Result> = Vec::new();

    // 1. Single filters x zenflate levels
    let zf_levels = [1, 6, 9, 12];
    let filter_names = ["None", "Sub", "Up", "Average", "Paeth"];

    for (f, name) in filter_names.iter().enumerate() {
        let t0 = Instant::now();
        let filtered = filter_single(&pixel_bytes, row_bytes, h, bpp, f as u8);
        let filter_ms = t0.elapsed().as_secs_f64() * 1000.0;

        for &level in &zf_levels {
            let t1 = Instant::now();
            let size = compress_zenflate(&filtered, level);
            let compress_ms = t1.elapsed().as_secs_f64() * 1000.0;

            let r = Result {
                strategy: format!("Single({name})"),
                context: 0,
                eval: 0,
                block: 0,
                zf_level: level,
                filter_ms,
                compress_ms,
                size,
                baseline_size,
            };
            r.print_csv();
            results.push(r);
        }
    }

    // 2. Adaptive heuristics x zenflate levels
    let heuristics = [
        ("MinSum", Heuristic::MinSum),
        ("Entropy", Heuristic::Entropy),
        ("Bigrams", Heuristic::Bigrams),
        ("BigEnt", Heuristic::BigEnt),
    ];

    for (name, heuristic) in &heuristics {
        let t0 = Instant::now();
        let filtered = filter_adaptive(&pixel_bytes, row_bytes, h, bpp, *heuristic);
        let filter_ms = t0.elapsed().as_secs_f64() * 1000.0;

        for &level in &zf_levels {
            let t1 = Instant::now();
            let size = compress_zenflate(&filtered, level);
            let compress_ms = t1.elapsed().as_secs_f64() * 1000.0;

            let r = Result {
                strategy: format!("Adaptive({name})"),
                context: 0,
                eval: 0,
                block: 0,
                zf_level: level,
                filter_ms,
                compress_ms,
                size,
                baseline_size,
            };
            r.print_csv();
            results.push(r);
        }
    }

    // 3. Per-row brute-force
    let brute_contexts = [1, 3, 5, 8, 10];
    let brute_evals = [1, 4];
    let brute_zf_levels = [6, 9, 12];

    for &ctx in &brute_contexts {
        for &eval in &brute_evals {
            eprintln!("  BruteForce ctx={ctx} eval=L{eval}...");
            let t0 = Instant::now();
            let filtered = filter_brute_force(&pixel_bytes, row_bytes, h, bpp, ctx, eval);
            let filter_ms = t0.elapsed().as_secs_f64() * 1000.0;

            for &level in &brute_zf_levels {
                let t1 = Instant::now();
                let size = compress_zenflate(&filtered, level);
                let compress_ms = t1.elapsed().as_secs_f64() * 1000.0;

                let r = Result {
                    strategy: "BruteForce".to_string(),
                    context: ctx,
                    eval,
                    block: 0,
                    zf_level: level,
                    filter_ms,
                    compress_ms,
                    size,
                    baseline_size,
                };
                r.print_csv();
                results.push(r);
            }
        }
    }

    // 4. Block-wise brute-force
    let block_sizes = [2, 3];
    let block_contexts = [3, 8, 10];
    let block_eval = 1u32;
    let block_zf_levels = [6, 9, 12];

    for &bs in &block_sizes {
        for &ctx in &block_contexts {
            eprintln!("  BlockBrute block={bs} ctx={ctx}...");
            let t0 = Instant::now();
            let filtered =
                filter_brute_force_block(&pixel_bytes, row_bytes, h, bpp, ctx, block_eval, bs);
            let filter_ms = t0.elapsed().as_secs_f64() * 1000.0;

            for &level in &block_zf_levels {
                let t1 = Instant::now();
                let size = compress_zenflate(&filtered, level);
                let compress_ms = t1.elapsed().as_secs_f64() * 1000.0;

                let r = Result {
                    strategy: "BlockBrute".to_string(),
                    context: ctx,
                    eval: block_eval,
                    block: bs,
                    zf_level: level,
                    filter_ms,
                    compress_ms,
                    size,
                    baseline_size,
                };
                r.print_csv();
                results.push(r);
            }
        }
    }

    // 5. Zopfli: test best strategies at zopfli compression (5, 15, 50 iterations)
    // Re-filter the best heuristic and brute-force strategies, then compress with zopfli.
    let zopfli_iters = [5, 15, 50];
    let zopfli_strategies: Vec<(&str, Vec<u8>)> = vec![
        (
            "Single(Paeth)",
            filter_single(&pixel_bytes, row_bytes, h, bpp, 4),
        ),
        (
            "Adaptive(MinSum)",
            filter_adaptive(&pixel_bytes, row_bytes, h, bpp, Heuristic::MinSum),
        ),
        (
            "Adaptive(Entropy)",
            filter_adaptive(&pixel_bytes, row_bytes, h, bpp, Heuristic::Entropy),
        ),
        (
            "Adaptive(BigEnt)",
            filter_adaptive(&pixel_bytes, row_bytes, h, bpp, Heuristic::BigEnt),
        ),
        (
            "BruteForce(c5e1)",
            filter_brute_force(&pixel_bytes, row_bytes, h, bpp, 5, 1),
        ),
    ];

    for (name, filtered) in &zopfli_strategies {
        for &iters in &zopfli_iters {
            eprintln!("  Zopfli {name} i={iters}...");
            let t1 = Instant::now();
            let size = compress_zopfli(filtered, iters);
            let compress_ms = t1.elapsed().as_secs_f64() * 1000.0;

            // Use zf_level = 100 + iters to signal zopfli in CSV
            let r = Result {
                strategy: format!("{name}+zopfli"),
                context: 0,
                eval: 0,
                block: 0,
                zf_level: 100 + iters,
                filter_ms: 0.0, // already measured above, not re-timing filter
                compress_ms,
                size,
                baseline_size,
            };
            r.print_csv();
            results.push(r);
        }
    }

    // Efficiency frontier: top 20 by bytes-saved per ms
    results.sort_by(|a, b| {
        let eff_a = a.efficiency();
        let eff_b = b.efficiency();
        eff_b.partial_cmp(&eff_a).unwrap()
    });

    eprintln!("\n=== Efficiency frontier (top 20 by bytes-saved/ms) ===");
    eprintln!(
        "{:<25} {:>4} {:>4} {:>3} {:>3} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "Strategy", "Ctx", "Eval", "Blk", "ZfL", "Filt ms", "Cmp ms", "Tot ms", "Size", "B/ms"
    );
    eprintln!("{}", "-".repeat(97));

    for r in results.iter().take(20) {
        let total_ms = r.filter_ms + r.compress_ms;
        let saved = baseline_size as i64 - r.size as i64;
        let eff = if total_ms > 0.1 {
            saved as f64 / total_ms
        } else {
            0.0
        };
        eprintln!(
            "{:<25} {:>4} {:>4} {:>3} {:>3} {:>8.1} {:>8.1} {:>8.1} {:>8} {:>8.0}",
            r.strategy,
            r.context,
            r.eval,
            r.block,
            r.zf_level,
            r.filter_ms,
            r.compress_ms,
            total_ms,
            r.size,
            eff,
        );
    }
    eprintln!();
}

struct Result {
    strategy: String,
    context: usize,
    eval: u32,
    block: usize,
    zf_level: u32,
    filter_ms: f64,
    compress_ms: f64,
    size: usize,
    baseline_size: usize,
}

impl Result {
    fn print_csv(&self) {
        let pct =
            (self.size as f64 - self.baseline_size as f64) / self.baseline_size as f64 * 100.0;
        let total = self.filter_ms + self.compress_ms;
        println!(
            "{},{},{},{},{},{:.2},{:.2},{:.2},{},{:.2}",
            self.strategy,
            self.context,
            self.eval,
            self.block,
            self.zf_level,
            self.filter_ms,
            self.compress_ms,
            total,
            self.size,
            pct,
        );
        // Flush after each line so piping works
        let _ = std::io::stdout().flush();
    }

    fn efficiency(&self) -> f64 {
        let total_ms = self.filter_ms + self.compress_ms;
        let saved = self.baseline_size as i64 - self.size as i64;
        if total_ms > 0.1 && saved > 0 {
            saved as f64 / total_ms
        } else {
            0.0
        }
    }
}

// ---- Filter implementations (standalone, matching png_writer.rs logic) ----

fn filter_single(
    pixel_bytes: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    filter: u8,
) -> Vec<u8> {
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
enum Heuristic {
    MinSum,
    Entropy,
    Bigrams,
    BigEnt,
}

fn filter_adaptive(
    pixel_bytes: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    heuristic: Heuristic,
) -> Vec<u8> {
    let mut out = Vec::with_capacity((row_bytes + 1) * height);
    let mut prev_row = vec![0u8; row_bytes];
    let mut candidates: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();

    for y in 0..height {
        let row = &pixel_bytes[y * row_bytes..(y + 1) * row_bytes];
        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidates[f as usize]);
        }

        let best_f = match heuristic {
            Heuristic::MinSum => (0..5u8)
                .min_by_key(|&f| {
                    candidates[f as usize]
                        .iter()
                        .map(|&b| (b as i8).unsigned_abs() as u64)
                        .sum::<u64>()
                })
                .unwrap(),
            Heuristic::Entropy => (0..5u8)
                .min_by(|&a, &b| {
                    entropy_score(&candidates[a as usize])
                        .partial_cmp(&entropy_score(&candidates[b as usize]))
                        .unwrap()
                })
                .unwrap(),
            Heuristic::Bigrams => (0..5u8)
                .min_by_key(|&f| bigrams_score(&candidates[f as usize]))
                .unwrap(),
            Heuristic::BigEnt => (0..5u8)
                .min_by(|&a, &b| {
                    bigram_entropy_score(&candidates[a as usize])
                        .partial_cmp(&bigram_entropy_score(&candidates[b as usize]))
                        .unwrap()
                })
                .unwrap(),
        };

        out.push(best_f);
        out.extend_from_slice(&candidates[best_f as usize]);
        prev_row.copy_from_slice(row);
    }
    out
}

fn filter_brute_force(
    pixel_bytes: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    context_rows: usize,
    eval_level: u32,
) -> Vec<u8> {
    let filtered_row_size = row_bytes + 1;
    let max_context_bytes = 32 * 1024;
    let context_rows = context_rows
        .min(max_context_bytes / filtered_row_size)
        .max(1);
    let max_context = context_rows * filtered_row_size;

    let mut eval_compressor =
        zenflate::Compressor::new(zenflate::CompressionLevel::new(eval_level));

    let eval_max_input = max_context + filtered_row_size;
    let compress_bound = zenflate::Compressor::zlib_compress_bound(eval_max_input);
    let mut compress_buf = vec![0u8; compress_bound];
    let mut candidate_data: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();
    let mut eval_buf = Vec::with_capacity(eval_max_input);
    let mut prev_row = vec![0u8; row_bytes];
    let mut out = Vec::with_capacity(filtered_row_size * height);

    for y in 0..height {
        let row = &pixel_bytes[y * row_bytes..(y + 1) * row_bytes];
        let context_start = if out.len() > max_context {
            out.len() - max_context
        } else {
            0
        };
        let context = &out[context_start..];

        let mut best_f = 0u8;
        let mut best_size = usize::MAX;

        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidate_data[f as usize]);
            eval_buf.clear();
            eval_buf.extend_from_slice(context);
            eval_buf.push(f);
            eval_buf.extend_from_slice(&candidate_data[f as usize]);

            if let Ok(len) =
                eval_compressor.zlib_compress(&eval_buf, &mut compress_buf, zenflate::Unstoppable)
                && len < best_size
            {
                best_size = len;
                best_f = f;
            }
        }

        out.push(best_f);
        out.extend_from_slice(&candidate_data[best_f as usize]);
        prev_row.copy_from_slice(row);
    }
    out
}

fn filter_brute_force_block(
    pixel_bytes: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    context_rows: usize,
    eval_level: u32,
    block_size: usize,
) -> Vec<u8> {
    if height == 0 {
        return Vec::new();
    }

    let filtered_row_size = row_bytes + 1;
    let max_context_bytes = 32 * 1024;
    let context_rows = context_rows
        .min(max_context_bytes / filtered_row_size)
        .max(1);
    let max_context = context_rows * filtered_row_size;

    let mut eval_compressor =
        zenflate::Compressor::new(zenflate::CompressionLevel::new(eval_level));

    let eval_max_input = max_context + block_size * filtered_row_size;
    let compress_bound = zenflate::Compressor::zlib_compress_bound(eval_max_input);
    let mut compress_buf = vec![0u8; compress_bound];

    // Precompute all filter variants
    let all_filters = precompute_all_filters(pixel_bytes, row_bytes, height, bpp);

    let mut eval_buf = Vec::with_capacity(eval_max_input);
    let mut out = Vec::with_capacity(filtered_row_size * height);

    let mut block_start = 0;
    while block_start < height {
        let actual_block = block_size.min(height - block_start);
        let combos = 5usize.pow(actual_block as u32);

        let context_start = if out.len() > max_context {
            out.len() - max_context
        } else {
            0
        };
        let context = &out[context_start..];

        let mut best_combo = 0usize;
        let mut best_size = usize::MAX;

        for combo in 0..combos {
            eval_buf.clear();
            eval_buf.extend_from_slice(context);

            let mut c = combo;
            for i in 0..actual_block {
                let f = (c % 5) as u8;
                c /= 5;
                eval_buf.push(f);
                let offset = ((block_start + i) * 5 + f as usize) * row_bytes;
                eval_buf.extend_from_slice(&all_filters[offset..offset + row_bytes]);
            }

            if let Ok(len) =
                eval_compressor.zlib_compress(&eval_buf, &mut compress_buf, zenflate::Unstoppable)
                && len < best_size
            {
                best_size = len;
                best_combo = combo;
            }
        }

        // Commit winning combination
        let mut c = best_combo;
        for i in 0..actual_block {
            let f = (c % 5) as u8;
            c /= 5;
            out.push(f);
            let offset = ((block_start + i) * 5 + f as usize) * row_bytes;
            out.extend_from_slice(&all_filters[offset..offset + row_bytes]);
        }

        block_start += actual_block;
    }
    out
}

fn precompute_all_filters(
    pixel_bytes: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
) -> Vec<u8> {
    let mut buf = vec![0u8; 5 * height * row_bytes];
    let mut prev_row = vec![0u8; row_bytes];

    for y in 0..height {
        let row = &pixel_bytes[y * row_bytes..(y + 1) * row_bytes];
        for f in 0..5u8 {
            let offset = (y * 5 + f as usize) * row_bytes;
            apply_filter(f, row, &prev_row, bpp, &mut buf[offset..offset + row_bytes]);
        }
        prev_row.copy_from_slice(row);
    }
    buf
}

fn apply_filter(filter: u8, row: &[u8], prev_row: &[u8], bpp: usize, out: &mut [u8]) {
    match filter {
        0 => out.copy_from_slice(row),
        1 => {
            out[..bpp].copy_from_slice(&row[..bpp]);
            for i in bpp..row.len() {
                out[i] = row[i].wrapping_sub(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..row.len() {
                out[i] = row[i].wrapping_sub(prev_row[i]);
            }
        }
        3 => {
            for i in 0..bpp {
                out[i] = row[i].wrapping_sub(prev_row[i] / 2);
            }
            for i in bpp..row.len() {
                let avg = ((row[i - bpp] as u16 + prev_row[i] as u16) / 2) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
            for i in 0..bpp {
                out[i] = row[i].wrapping_sub(paeth(0, prev_row[i], 0));
            }
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
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn entropy_score(data: &[u8]) -> f64 {
    let mut counts = [0u32; 256];
    for &b in data {
        counts[b as usize] += 1;
    }
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
    if data.len() < 2 {
        return 0;
    }
    let mut seen = vec![false; 65536];
    let mut count = 0;
    for pair in data.windows(2) {
        let key = (pair[0] as usize) << 8 | pair[1] as usize;
        if !seen[key] {
            seen[key] = true;
            count += 1;
        }
    }
    count
}

fn bigram_entropy_score(data: &[u8]) -> f64 {
    if data.len() < 2 {
        return 0.0;
    }
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
    compressor
        .zlib_compress(data, &mut buf, zenflate::Unstoppable)
        .unwrap()
}

fn compress_zopfli(data: &[u8], iterations: u32) -> usize {
    use std::io::Write;
    let options = zopfli::Options {
        iteration_count: core::num::NonZeroU64::new(iterations as u64).unwrap(),
        ..Default::default()
    };
    let mut output = Vec::new();
    let mut encoder = zopfli::DeflateEncoder::new(options, zopfli::BlockType::Dynamic, &mut output);
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap();
    // Wrap in zlib envelope: 2-byte header + deflate data + 4-byte adler32
    let adler = adler32(data);
    let mut zlib = Vec::with_capacity(2 + output.len() + 4);
    zlib.push(0x78);
    zlib.push(0xDA); // CMF=78 FLG=DA (level 3, no dict)
    zlib.extend_from_slice(&output);
    zlib.extend_from_slice(&adler.to_be_bytes());
    zlib.len()
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
