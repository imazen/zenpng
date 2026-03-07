/// Evaluate the effort curve: size and time at every effort level.
///
/// For each image in the test corpus, compresses at every effort level 0-31
/// and records (effort, size, time_ms). Outputs CSV for analysis.
///
/// Usage: cargo run --release --example effort_curve [-- /path/to/corpus [MAX_EFFORT]]
///
/// Default corpus: $ZENPNG_OUTPUT_DIR/test_corpus/
/// Default max effort: 31
use enough::Unstoppable;
use std::io::Write;
use std::path::{Path, PathBuf};
use zenpixels::descriptor::{ChannelLayout, ChannelType};
use zenpixels_convert::PixelBufferConvertExt;

fn main() {
    let corpus_dir = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/test_corpus",
            std::env::var("ZENPNG_OUTPUT_DIR")
                .unwrap_or_else(|_| "/mnt/v/output/zenpng".to_string())
        )
    });

    let max_effort: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(31);

    let out_dir = PathBuf::from(
        std::env::var("ZENPNG_OUTPUT_DIR").unwrap_or_else(|_| "/mnt/v/output/zenpng".to_string()),
    )
    .join("effort_curve");
    std::fs::create_dir_all(&out_dir).unwrap();

    // Collect PNG files
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_pngs(Path::new(&corpus_dir), &mut paths);
    paths.sort();
    let n = paths.len();
    eprintln!("Found {n} PNGs in {corpus_dir}, testing efforts 0-{max_effort}");

    // Effort levels to test
    let efforts: Vec<u32> = (0..=max_effort).collect();

    // Per-image CSV
    let csv_path = out_dir.join("effort_curve.csv");
    let mut csv = std::fs::File::create(&csv_path).unwrap();

    // Header
    write!(csv, "filename,width,height,color_type,bpp,raw_bytes").unwrap();
    for e in &efforts {
        write!(csv, ",e{e}_size,e{e}_ms").unwrap();
    }
    writeln!(csv).unwrap();

    // Aggregate tracking for monotonicity violations
    let mut total_violations = 0usize;
    let mut violation_details: Vec<String> = Vec::new();

    // Track per-strategy wins
    // We'll also output a strategy analysis CSV
    let strategy_csv_path = out_dir.join("strategy_wins.csv");

    let mut ok = 0usize;
    let mut skip = 0usize;

    for (i, path) in paths.iter().enumerate() {
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        if (i + 1) % 10 == 0 || i + 1 == n {
            eprintln!("[{}/{}] ok={ok} skip={skip}", i + 1, n);
        }

        let source_data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => {
                skip += 1;
                continue;
            }
        };

        // Skip very large files
        if source_data.len() > 5_000_000 {
            skip += 1;
            continue;
        }

        let decoded =
            match zenpng::decode(&source_data, &zenpng::PngDecodeConfig::none(), &Unstoppable) {
                Ok(d) => d,
                Err(_) => {
                    skip += 1;
                    continue;
                }
            };

        let w = decoded.info.width;
        let h = decoded.info.height;

        if w < 4 || h < 4 || (w as u64 * h as u64) > 4_000_000 {
            skip += 1;
            continue;
        }

        let desc = decoded.pixels.descriptor();
        let (color_type, bpp) = match (desc.layout(), desc.channel_type()) {
            (ChannelLayout::Gray, ChannelType::U8) => ("gray8", 1),
            (ChannelLayout::Rgb, ChannelType::U8) => ("rgb8", 3),
            (ChannelLayout::Rgba, ChannelType::U8) => ("rgba8", 4),
            _ => {
                skip += 1;
                continue;
            }
        };
        let raw_bytes = w as usize * h as usize * bpp;

        write!(csv, "{fname},{w},{h},{color_type},{bpp},{raw_bytes}").unwrap();

        let mut prev_size = usize::MAX;
        let mut sizes: Vec<(u32, usize, u128)> = Vec::new();

        for &effort in &efforts {
            let config = zenpng::EncodeConfig::default()
                .with_compression(zenpng::Compression::Effort(effort))
                .with_source_gamma(decoded.info.source_gamma)
                .with_srgb_intent(decoded.info.srgb_intent)
                .with_chromaticities(decoded.info.chromaticities);

            let start = std::time::Instant::now();
            let result = match (desc.layout(), desc.channel_type()) {
                (ChannelLayout::Rgb, ChannelType::U8) => {
                    let buf = decoded.pixels.to_rgb8();
                    zenpng::encode_rgb8(buf.as_imgref(), None, &config, &Unstoppable, &Unstoppable)
                }
                (ChannelLayout::Rgba, ChannelType::U8) => {
                    let buf = decoded.pixels.to_rgba8();
                    zenpng::encode_rgba8(buf.as_imgref(), None, &config, &Unstoppable, &Unstoppable)
                }
                (ChannelLayout::Gray, ChannelType::U8) => {
                    let buf = decoded.pixels.to_gray8();
                    zenpng::encode_gray8(buf.as_imgref(), None, &config, &Unstoppable, &Unstoppable)
                }
                _ => {
                    write!(csv, ",0,0").unwrap();
                    continue;
                }
            };
            let elapsed_ms = start.elapsed().as_millis();

            match result {
                Ok(data) => {
                    let size = data.len();
                    write!(csv, ",{size},{elapsed_ms}").unwrap();

                    // Check monotonicity: size should not increase with effort
                    if size > prev_size && effort > 0 {
                        let delta = size - prev_size;
                        let pct = delta as f64 / prev_size as f64 * 100.0;
                        total_violations += 1;
                        if pct > 0.05 {
                            // Only log significant violations
                            violation_details.push(format!(
                                "{fname}: e{} ({prev_size}) -> e{effort} ({size}), +{delta} bytes (+{pct:.3}%)",
                                effort - 1
                            ));
                        }
                    }
                    prev_size = prev_size.min(size);

                    sizes.push((effort, size, elapsed_ms));
                }
                Err(e) => {
                    eprintln!("  WARN: {fname} e{effort} failed: {e}");
                    write!(csv, ",0,0").unwrap();
                }
            }
        }

        writeln!(csv).unwrap();
        ok += 1;
    }

    eprintln!("\nDone: {ok} profiled, {skip} skipped");
    eprintln!("CSV: {}", csv_path.display());

    // Report monotonicity violations
    eprintln!("\n=== Monotonicity Violations ===");
    eprintln!("Total violations (any size): {total_violations}");
    eprintln!("Significant violations (>0.05%):");
    for v in &violation_details {
        eprintln!("  {v}");
    }
    if violation_details.is_empty() {
        eprintln!("  (none)");
    }

    // Write violations to file
    let viol_path = out_dir.join("monotonicity_violations.txt");
    let mut viol_file = std::fs::File::create(&viol_path).unwrap();
    writeln!(viol_file, "Total violations: {total_violations}").unwrap();
    writeln!(viol_file, "\nSignificant (>0.05%):").unwrap();
    for v in &violation_details {
        writeln!(viol_file, "  {v}").unwrap();
    }
    eprintln!("Violations: {}", viol_path.display());

    // Write strategy wins CSV
    let _ = strategy_csv_path;
}

fn collect_pngs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_pngs(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}
