/// Measure filter and compress costs separately to determine optimal
/// strategy tier boundaries at each effort level.
///
/// For each filter strategy, measures:
/// - Filter time (applying the filter to the image)
/// - Compress time at each zenflate effort (0-15)
///
/// This tells us whether Turbo/FastHt efforts are fast enough
/// that we could screen more strategies at low effort levels.
///
/// Usage:
///   cargo run --release --no-default-features --features _dev --example effort_timing [-- image.png]
use std::time::Instant;

use enough::Unstoppable;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/lilith/work/codec-corpus/gb82-sc/0001.png".to_string());

    let source = std::fs::read(&path).expect("read");
    let decoded = zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable).unwrap();

    let (w, h) = (decoded.info.width as usize, decoded.info.height as usize);
    let (pixel_bytes, bpp): (Vec<u8>, usize) = match &decoded.pixels {
        zencodec_types::PixelData::Rgb8(img) => {
            (bytemuck::cast_slice::<_, u8>(img.buf()).to_vec(), 3)
        }
        zencodec_types::PixelData::Rgba8(img) => {
            (bytemuck::cast_slice::<_, u8>(img.buf()).to_vec(), 4)
        }
        _ => panic!("unsupported"),
    };
    let row_bytes = w * bpp;
    let raw_bytes = pixel_bytes.len();
    let megapixels = (w * h) as f64 / 1_000_000.0;

    let fname = std::path::Path::new(&path)
        .file_name()
        .unwrap()
        .to_string_lossy();
    eprintln!(
        "Image: {} ({}x{}, bpp={}, {:.2} MiB raw, {:.2} MP)\n",
        fname,
        w,
        h,
        bpp,
        raw_bytes as f64 / 1_048_576.0,
        megapixels,
    );

    // === Section 1: Filter costs ===
    eprintln!("=== Filter Costs ===\n");

    let filter_names = ["None", "Sub", "Up", "Average", "Paeth"];
    let heuristic_names = ["MinSum", "Entropy", "Bigrams", "BigEnt"];

    // Measure single-filter application
    let mut single_filtered: Vec<Vec<u8>> = Vec::new();
    eprintln!("{:<18} {:>8} {:>10}", "Filter", "Time ms", "MP/s");
    eprintln!("{}", "-".repeat(38));

    for (f, name) in filter_names.iter().enumerate() {
        let iters = 5;
        let mut filtered = Vec::new();
        let t = Instant::now();
        for _ in 0..iters {
            filtered = filter_single(&pixel_bytes, row_bytes, h, bpp, f as u8);
            std::hint::black_box(&filtered);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        let mp_s = megapixels / (ms / 1000.0);
        eprintln!(
            "{:<18} {:>8.2} {:>10.0}",
            format!("Single({name})"),
            ms,
            mp_s
        );
        single_filtered.push(filtered);
    }

    // Measure adaptive filter heuristics (apply all 5 + pick best per row)
    let mut adaptive_filtered: Vec<Vec<u8>> = Vec::new();
    eprintln!();
    for (i, name) in heuristic_names.iter().enumerate() {
        let heuristic = match i {
            0 => Heuristic::MinSum,
            1 => Heuristic::Entropy,
            2 => Heuristic::Bigrams,
            _ => Heuristic::BigEnt,
        };
        let iters = 3;
        let mut filtered = Vec::new();
        let t = Instant::now();
        for _ in 0..iters {
            filtered = filter_adaptive(&pixel_bytes, row_bytes, h, bpp, heuristic);
            std::hint::black_box(&filtered);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        let mp_s = megapixels / (ms / 1000.0);
        eprintln!(
            "{:<18} {:>8.2} {:>10.0}",
            format!("Adaptive({name})"),
            ms,
            mp_s
        );
        adaptive_filtered.push(filtered);
    }

    // === Section 2: Compress costs at each effort ===
    eprintln!("\n=== Compress Costs (zenflate effort 0-15, Single(Paeth) data) ===\n");

    let test_data = &single_filtered[4]; // Paeth
    let efforts: Vec<u32> = (0..=15).collect();

    eprintln!(
        "{:>6} {:>8} {:>10} {:>10} {:>8}",
        "Effort", "Time ms", "MiB/s", "Size", "Ratio"
    );
    eprintln!("{}", "-".repeat(46));

    let mut compress_ms_by_effort: Vec<f64> = Vec::new();
    for &effort in &efforts {
        let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(effort));
        let bound = zenflate::Compressor::zlib_compress_bound(test_data.len());
        let mut buf = vec![0u8; bound];

        let iters = if effort <= 4 {
            10
        } else if effort <= 10 {
            5
        } else {
            3
        };
        let mut size = 0;
        let t = Instant::now();
        for _ in 0..iters {
            size = compressor
                .zlib_compress(test_data, &mut buf, zenflate::Unstoppable)
                .unwrap();
            std::hint::black_box(size);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        let mib_s = (test_data.len() as f64 / 1_048_576.0) / (ms / 1000.0);
        let ratio = size as f64 / raw_bytes as f64 * 100.0;
        eprintln!(
            "{:>6} {:>8.2} {:>10.1} {:>10} {:>7.1}%",
            effort, ms, mib_s, size, ratio,
        );
        compress_ms_by_effort.push(ms);
    }

    // === Section 3: Total screening cost analysis ===
    eprintln!("\n=== Screening Cost: N strategies x screen_effort ===");
    eprintln!("(filter_time + N * compress_time at screen_effort)\n");

    // Approximate filter cost: ~same for each strategy, dominated by the adaptive overhead
    // Single filter = cheap, adaptive = 5x single (applies all 5 + scoring)
    // For screening, each strategy either does 1 filter + compress or adaptive + compress

    // Use Paeth filter time as single-strategy cost
    let single_filter_ms = {
        let iters = 5;
        let t = Instant::now();
        for _ in 0..iters {
            let f = filter_single(&pixel_bytes, row_bytes, h, bpp, 4);
            std::hint::black_box(&f);
        }
        t.elapsed().as_secs_f64() * 1000.0 / iters as f64
    };

    // Use MinSum adaptive time as adaptive-strategy cost (applies all 5 filters + scores)
    let adaptive_filter_ms = {
        let iters = 3;
        let t = Instant::now();
        for _ in 0..iters {
            let f = filter_adaptive(&pixel_bytes, row_bytes, h, bpp, Heuristic::MinSum);
            std::hint::black_box(&f);
        }
        t.elapsed().as_secs_f64() * 1000.0 / iters as f64
    };

    eprintln!("Single filter cost:   {:.2} ms", single_filter_ms);
    eprintln!(
        "Adaptive filter cost: {:.2} ms (5 filters + scoring)",
        adaptive_filter_ms
    );
    eprintln!();

    // Strategy counts to evaluate: 1, 3, 5, 9
    let strategy_configs = [
        (1, "1 (single)", 1, 0),  // 1 single filter
        (3, "3 (minimal)", 1, 2), // 1 single + 2 adaptive (rough approximation)
        (5, "5 (fast)", 1, 4),    // 1 single + 4 adaptive
        (9, "9 (full)", 5, 4),    // 5 single + 4 adaptive
    ];

    eprintln!(
        "{:>7} {:>13} {:>10} {:>10} {:>10} {:>10}",
        "Screen@", "Strategies", "Filter ms", "Cmpr ms", "Total ms", "MP/s"
    );
    eprintln!("{}", "-".repeat(64));

    for screen_effort in [1u32, 2, 3, 4, 5, 6, 7] {
        let compress_per_strategy = if (screen_effort as usize) < compress_ms_by_effort.len() {
            compress_ms_by_effort[screen_effort as usize]
        } else {
            continue;
        };

        for &(n, label, n_single, n_adaptive) in &strategy_configs {
            // Filter cost: each strategy filters once
            // Single strategies: just 1 filter pass each
            // Adaptive strategies: 5 filter passes + scoring each
            let filter_total =
                n_single as f64 * single_filter_ms + n_adaptive as f64 * adaptive_filter_ms;
            let compress_total = n as f64 * compress_per_strategy;
            let total = filter_total + compress_total;
            let mp_s = megapixels / (total / 1000.0);
            eprintln!(
                "{:>7} {:>13} {:>10.1} {:>10.1} {:>10.1} {:>10.0}",
                screen_effort, label, filter_total, compress_total, total, mp_s,
            );
        }
        eprintln!();
    }

    // === Section 4: End-to-end encode at each effort ===
    eprintln!("=== End-to-end encode (effort 0-15) ===\n");

    eprintln!(
        "{:>6} {:>8} {:>10} {:>10} {:>8}",
        "Effort", "Time ms", "MiB/s", "Size", "Ratio"
    );
    eprintln!("{}", "-".repeat(46));

    for effort in 0..=15u32 {
        let config = zenpng::EncodeConfig {
            compression: zenpng::Compression::Effort(effort),
            source_gamma: decoded.info.source_gamma,
            srgb_intent: decoded.info.srgb_intent,
            chromaticities: decoded.info.chromaticities,
            ..Default::default()
        };

        let iters = if effort <= 4 {
            5
        } else if effort <= 10 {
            3
        } else {
            2
        };
        let mut size = 0;
        let t = Instant::now();
        for _ in 0..iters {
            let result = match &decoded.pixels {
                zencodec_types::PixelData::Rgb8(img) => {
                    zenpng::encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable)
                }
                zencodec_types::PixelData::Rgba8(img) => {
                    zenpng::encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable)
                }
                _ => panic!("unsupported"),
            };
            size = result.unwrap().len();
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        let mib_s = (raw_bytes as f64 / 1_048_576.0) / (ms / 1000.0);
        let ratio = size as f64 / raw_bytes as f64 * 100.0;
        eprintln!(
            "{:>6} {:>8.1} {:>10.1} {:>10} {:>7.1}%",
            effort, ms, mib_s, size, ratio,
        );
    }
}

// ---- Filter implementations (matching strategy_explorer.rs) ----

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
