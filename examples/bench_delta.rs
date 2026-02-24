/// Benchmark encode_apng_auto on real APNG corpus with dither mode comparison.
/// Also checks how many source APNGs are already indexed (≤256 unique colors).
///
/// Usage: cargo run --release --example bench_delta --features _dev
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use rgb::Rgba;

use zenpng::{
    ApngEncodeConfig, ApngFrameInput, QualityGate, default_quantize_config, encode_apng_auto,
};

fn sample_files(dir: &Path, n: usize) -> Vec<std::path::PathBuf> {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "png" || ext == "apng")
            })
            .map(|e| e.path())
            .collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort();
    if entries.len() <= n {
        return entries;
    }
    let step = entries.len() as f64 / n as f64;
    (0..n)
        .map(|i| entries[(i as f64 * step) as usize].clone())
        .collect()
}

fn fmt_size(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}

struct DecodedApng {
    name: String,
    w: u32,
    h: u32,
    frame_data: Vec<Vec<u8>>,
    delays: Vec<(u16, u16)>,
    unique_colors: usize,
    orig_size: usize,
}

fn load_corpus() -> Vec<DecodedApng> {
    let corpus_dir = Path::new("/mnt/v/output/corpus-builder/apng");
    let files = sample_files(corpus_dir, 50);
    if files.is_empty() {
        eprintln!("No APNG corpus found");
        return Vec::new();
    }

    eprintln!("Decoding APNG corpus ({} files)...", files.len());
    let mut list = Vec::new();

    for path in &files {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let decoded = match zenpng::decode_apng(
            &data,
            &zenpng::PngDecodeConfig::none(),
            &enough::Unstoppable,
        ) {
            Ok(d) => d,
            Err(_) => continue,
        };

        if decoded.frames.is_empty() {
            continue;
        }

        let w = decoded.info.width;
        let h = decoded.info.height;
        let total_pixels = w as usize * h as usize * decoded.frames.len();
        if total_pixels > 10_000_000 {
            continue;
        }

        // Count unique colors across all frames
        let mut unique: HashSet<[u8; 4]> = HashSet::new();
        let frame_data: Vec<Vec<u8>> = decoded
            .frames
            .iter()
            .map(|f| {
                let rgba = f.pixels.to_rgba8();
                let buf: Vec<Rgba<u8>> = rgba.into_buf();
                for px in &buf {
                    if unique.len() <= 256 {
                        unique.insert([px.r, px.g, px.b, px.a]);
                    }
                }
                bytemuck::cast_slice::<Rgba<u8>, u8>(&buf).to_vec()
            })
            .collect();

        let delays: Vec<(u16, u16)> = decoded
            .frames
            .iter()
            .map(|f| (f.frame_info.delay_num, f.frame_info.delay_den))
            .collect();

        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        eprintln!(
            "  {} ({}x{}, {}fr, {} unique colors{})",
            fname,
            w,
            h,
            decoded.frames.len(),
            if unique.len() > 256 {
                ">256".to_string()
            } else {
                unique.len().to_string()
            },
            if unique.len() <= 256 {
                " ← exact palette"
            } else {
                ""
            },
        );

        list.push(DecodedApng {
            name: fname,
            w,
            h,
            frame_data,
            delays,
            unique_colors: unique.len(),
            orig_size: data.len(),
        });
    }

    list
}

struct PerFileResult {
    name: String,
    size: usize,
    time_us: u64,
    indexed: bool,
    delta_e: f64,
}

fn bench_corpus(
    label: &str,
    corpus: &[DecodedApng],
    qconfig: &zenquant::QuantizeConfig,
) -> Vec<PerFileResult> {
    let config = ApngEncodeConfig::default();
    let gate = QualityGate::MaxDeltaE(0.05);
    let mut results = Vec::new();

    for item in corpus {
        let frames: Vec<ApngFrameInput<'_>> = item
            .frame_data
            .iter()
            .zip(item.delays.iter())
            .map(|(data, (num, den))| ApngFrameInput {
                pixels: data,
                delay_num: *num,
                delay_den: *den,
            })
            .collect();

        let start = Instant::now();
        let result = match encode_apng_auto(
            &frames,
            item.w,
            item.h,
            &config,
            qconfig,
            gate,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  {}: error: {}", item.name, e);
                continue;
            }
        };
        let elapsed = start.elapsed().as_micros() as u64;

        results.push(PerFileResult {
            name: item.name.clone(),
            size: result.data.len(),
            time_us: elapsed,
            indexed: result.indexed,
            delta_e: result.quality_loss,
        });
    }

    results
}

fn main() {
    let corpus = load_corpus();
    if corpus.is_empty() {
        return;
    }

    let n_exact_palette = corpus.iter().filter(|c| c.unique_colors <= 256).count();
    println!("\n=== APNG corpus: {} files ===", corpus.len());
    println!("  ≤256 unique colors (exact palette): {}", n_exact_palette);
    println!(
        "  >256 unique colors (need quantize): {}",
        corpus.len() - n_exact_palette
    );

    let qc_default = default_quantize_config();
    let qc_no_dither = default_quantize_config()._no_dither();

    eprintln!("\nBenchmarking floyd-steinberg...");
    let fs_results = bench_corpus("floyd-steinberg", &corpus, &qc_default);
    eprintln!("Benchmarking no-dither...");
    let nd_results = bench_corpus("no dither", &corpus, &qc_no_dither);

    // Per-file comparison
    println!(
        "\n{:<50} {:>7} {:>8} {:>8} {:>8} {:>8} {:>6}",
        "File", "Orig", "FS size", "ND size", "FS time", "ND time", "UniqC"
    );
    println!("{}", "-".repeat(100));

    let mut total_orig = 0usize;
    let mut total_fs = 0usize;
    let mut total_nd = 0usize;
    let mut total_fs_time = 0u64;
    let mut total_nd_time = 0u64;

    for (i, item) in corpus.iter().enumerate() {
        let fs = &fs_results[i];
        let nd = &nd_results[i];

        let short = if item.name.len() > 48 {
            format!("{}...", &item.name[..45])
        } else {
            item.name.clone()
        };

        total_orig += item.orig_size;
        total_fs += fs.size;
        total_nd += nd.size;
        total_fs_time += fs.time_us;
        total_nd_time += nd.time_us;

        let uniq_str = if item.unique_colors > 256 {
            ">256".to_string()
        } else {
            item.unique_colors.to_string()
        };

        println!(
            "{:<50} {:>7} {:>8} {:>8} {:>7.1}s {:>7.1}s {:>6}",
            short,
            fmt_size(item.orig_size),
            fmt_size(fs.size),
            fmt_size(nd.size),
            fs.time_us as f64 / 1e6,
            nd.time_us as f64 / 1e6,
            uniq_str,
        );
    }

    println!("{}", "-".repeat(100));
    println!(
        "{:<50} {:>7} {:>8} {:>8} {:>7.1}s {:>7.1}s",
        "TOTAL",
        fmt_size(total_orig),
        fmt_size(total_fs),
        fmt_size(total_nd),
        total_fs_time as f64 / 1e6,
        total_nd_time as f64 / 1e6,
    );

    let fs_idx = fs_results.iter().filter(|r| r.indexed).count();
    let nd_idx = nd_results.iter().filter(|r| r.indexed).count();
    println!(
        "\n  Floyd-Steinberg: {}/{} indexed, {} total",
        fs_idx,
        fs_results.len(),
        fmt_size(total_fs)
    );
    println!(
        "  No dither:       {}/{} indexed, {} total",
        nd_idx,
        nd_results.len(),
        fmt_size(total_nd)
    );
    println!("  Originals:       {} total", fmt_size(total_orig));
}
