/// Per-phase cost/benefit decomposition for each compression level.
///
/// For each level (Balanced through Crush), shows exactly where time goes
/// and what compression improvement each phase delivers.
///
/// Usage:
///   cargo run --release --features _dev --example phase_timing -- <image.png>
///   cargo run --release --features _dev --example phase_timing -- <corpus_dir/>
///   cargo run --release --features "_dev,zopfli" --example phase_timing -- <image.png>
use std::path::{Path, PathBuf};
use std::time::Instant;

use enough::Unstoppable;
use zenpixels::descriptor::{ChannelLayout, ChannelType};
use zenpixels_convert::PixelBufferConvertExt;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/CID22/CID22-512/validation/1025469.png",
            std::env::var("CODEC_CORPUS_DIR")
                .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string())
        )
    });

    let p = Path::new(&path);
    if p.is_dir() {
        run_corpus(p);
    } else {
        run_single(p);
    }
}

fn run_single(path: &Path) {
    let source = std::fs::read(path).expect("failed to read image");
    let decoded = zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable)
        .expect("failed to decode");

    let (w, h) = (decoded.info.width, decoded.info.height);
    let desc = decoded.pixels.descriptor();
    let (bpp_label, raw_bytes) = match (desc.layout(), desc.channel_type()) {
        (ChannelLayout::Rgb, ChannelType::U8) => ("RGB8", w as usize * h as usize * 3),
        (ChannelLayout::Rgba, ChannelType::U8) => ("RGBA8", w as usize * h as usize * 4),
        _ => panic!("unsupported pixel format: {:?}", desc),
    };

    let fname = path.file_name().unwrap().to_string_lossy();
    println!(
        "Image: {} ({}x{}, {}, {:.2} MiB raw)\n",
        fname,
        w,
        h,
        bpp_label,
        raw_bytes as f64 / 1_048_576.0
    );

    let levels = [
        ("Balanced", zenpng::Compression::Balanced),
        ("Thorough", zenpng::Compression::Thorough),
        ("High", zenpng::Compression::High),
        ("Aggressive", zenpng::Compression::Aggressive),
        ("Intense", zenpng::Compression::Intense),
        ("Crush", zenpng::Compression::Crush),
        ("Maniac", zenpng::Compression::Maniac),
    ];

    println!(
        "{:<12} {:<40} {:>8} {:>10} {:>10} {:>8}",
        "Level", "Phase", "Time", "Size", "\u{0394} bytes", "\u{0394}/ms"
    );
    println!("{}", "-".repeat(92));

    let parallel = std::env::var("PARALLEL").is_ok();
    if parallel {
        println!("*** PARALLEL MODE ***\n");
    }

    for (name, comp) in &levels {
        let config = zenpng::EncodeConfig::default()
            .with_compression(*comp)
            .with_parallel(parallel)
            .with_source_gamma(decoded.info.source_gamma)
            .with_srgb_intent(decoded.info.srgb_intent)
            .with_chromaticities(decoded.info.chromaticities);

        let total_start = Instant::now();
        let result = match (desc.layout(), desc.channel_type()) {
            (ChannelLayout::Rgb, ChannelType::U8) => {
                let buf = decoded.pixels.to_rgb8();
                zenpng::encode_rgb8_with_stats(
                    buf.as_imgref(),
                    None,
                    &config,
                    &Unstoppable,
                    &Unstoppable,
                )
            }
            (ChannelLayout::Rgba, ChannelType::U8) => {
                let buf = decoded.pixels.to_rgba8();
                zenpng::encode_rgba8_with_stats(
                    buf.as_imgref(),
                    None,
                    &config,
                    &Unstoppable,
                    &Unstoppable,
                )
            }
            _ => panic!("unsupported format: {:?}", desc),
        };
        let total_elapsed = total_start.elapsed();

        match result {
            Ok((data, stats)) => {
                let mut prev_size: Option<usize> = None;
                for phase in &stats.phases {
                    let ms = phase.duration_ns as f64 / 1_000_000.0;
                    let time_str = format_duration_ms(ms);
                    let delta = prev_size.map(|prev| phase.best_size as i64 - prev as i64);
                    let delta_str = match delta {
                        Some(d) if d < 0 => format!("{d}"),
                        Some(d) => format!("+{d}"),
                        None => "-".to_string(),
                    };
                    let delta_per_ms = match delta {
                        Some(d) if ms > 0.1 => format!("{:.0}", d as f64 / ms),
                        _ => "-".to_string(),
                    };
                    println!(
                        "{:<12} {:<40} {:>8} {:>10} {:>10} {:>8}",
                        if prev_size.is_none() { *name } else { "" },
                        phase.name,
                        time_str,
                        format_size(phase.best_size),
                        delta_str,
                        delta_per_ms,
                    );
                    prev_size = Some(phase.best_size);
                }
                // Total line
                let total_ms = total_elapsed.as_secs_f64() * 1000.0;
                println!(
                    "{:<12} {:<40} {:>8} {:>10}",
                    "",
                    "TOTAL",
                    format_duration_ms(total_ms),
                    format_size(data.len()),
                );
                println!();
            }
            Err(e) => {
                println!("{:<12} ERR: {e}\n", name);
            }
        }
    }
}

fn run_corpus(dir: &Path) {
    let mut images: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("failed to read directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == "png" || ext == "PNG")
        })
        .collect();
    images.sort();

    if images.is_empty() {
        eprintln!("No PNG files found in {}", dir.display());
        return;
    }

    eprintln!("Found {} images in {}", images.len(), dir.display());

    let levels = [
        ("Balanced", zenpng::Compression::Balanced),
        ("Thorough", zenpng::Compression::Thorough),
        ("High", zenpng::Compression::High),
        ("Aggressive", zenpng::Compression::Aggressive),
        ("Intense", zenpng::Compression::Intense),
        ("Crush", zenpng::Compression::Crush),
        ("Maniac", zenpng::Compression::Maniac),
    ];

    struct LevelAggregate<'a> {
        name: &'a str,
        phases: Vec<(String, u64, i64, u32)>, // (phase_name, total_ns, total_delta, count)
        total_ns: u64,
        total_size: usize,
    }
    let mut aggregates: Vec<LevelAggregate<'_>> = Vec::new();

    for (level_name, comp) in &levels {
        let mut phase_map: std::collections::BTreeMap<String, (u64, i64, u32)> =
            std::collections::BTreeMap::new();
        let mut total_ns = 0u64;
        let mut total_size = 0usize;

        for img_path in &images {
            let source = match std::fs::read(img_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let decoded =
                match zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

            let config = zenpng::EncodeConfig::default()
                .with_compression(*comp)
                .with_source_gamma(decoded.info.source_gamma)
                .with_srgb_intent(decoded.info.srgb_intent)
                .with_chromaticities(decoded.info.chromaticities);

            let desc = decoded.pixels.descriptor();
            let start = Instant::now();
            let result = match (desc.layout(), desc.channel_type()) {
                (ChannelLayout::Rgb, ChannelType::U8) => {
                    let buf = decoded.pixels.to_rgb8();
                    zenpng::encode_rgb8_with_stats(
                        buf.as_imgref(),
                        None,
                        &config,
                        &Unstoppable,
                        &Unstoppable,
                    )
                }
                (ChannelLayout::Rgba, ChannelType::U8) => {
                    let buf = decoded.pixels.to_rgba8();
                    zenpng::encode_rgba8_with_stats(
                        buf.as_imgref(),
                        None,
                        &config,
                        &Unstoppable,
                        &Unstoppable,
                    )
                }
                _ => continue,
            };
            let elapsed_ns = start.elapsed().as_nanos() as u64;

            if let Ok((data, stats)) = result {
                total_ns += elapsed_ns;
                total_size += data.len();

                let mut prev_size: Option<usize> = None;
                for phase in &stats.phases {
                    let delta = prev_size
                        .map(|prev| phase.best_size as i64 - prev as i64)
                        .unwrap_or(0);
                    let entry = phase_map.entry(phase.name.clone()).or_insert((0, 0, 0));
                    entry.0 += phase.duration_ns;
                    entry.1 += delta;
                    entry.2 += 1;
                    prev_size = Some(phase.best_size);
                }
            }

            eprint!(".");
        }
        eprintln!(" {level_name} done");

        let phases: Vec<_> = phase_map
            .into_iter()
            .map(|(k, v)| (k, v.0, v.1, v.2))
            .collect();
        aggregates.push(LevelAggregate {
            name: level_name,
            phases,
            total_ns,
            total_size,
        });
    }

    println!("\n=== Corpus aggregate ({} images) ===\n", images.len());
    println!(
        "{:<12} {:<40} {:>10} {:>12} {:>10}",
        "Level", "Phase", "Total ms", "Total \u{0394}B", "Avg \u{0394}/ms"
    );
    println!("{}", "-".repeat(78));

    for agg in &aggregates {
        let mut first = true;
        for (phase_name, ns, delta_bytes, _count) in &agg.phases {
            let ms = *ns as f64 / 1_000_000.0;
            let avg_delta_per_ms = if ms > 0.1 {
                format!("{:.0}", *delta_bytes as f64 / ms)
            } else {
                "-".to_string()
            };
            println!(
                "{:<12} {:<40} {:>10.1} {:>12} {:>10}",
                if first { agg.name } else { "" },
                phase_name,
                ms,
                delta_bytes,
                avg_delta_per_ms,
            );
            first = false;
        }
        let total_ms = agg.total_ns as f64 / 1_000_000.0;
        println!(
            "{:<12} {:<40} {:>10.1} {:>12}",
            "",
            "TOTAL",
            total_ms,
            format_size(agg.total_size),
        );
        println!();
    }
}

fn format_duration_ms(ms: f64) -> String {
    if ms < 1.0 {
        format!("{ms:.1}ms")
    } else if ms < 1000.0 {
        format!("{ms:.0}ms")
    } else if ms < 10000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else {
        format!("{:.1}s", ms / 1000.0)
    }
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.2}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}")
    }
}
