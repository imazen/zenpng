/// Benchmark zenpng compression against external PNG crushers.
///
/// For each test image:
///  1. Decode the source PNG
///  2. Re-encode with zenpng at each compression level → measure size & time
///  3. Write an unfiltered "raw" PNG (fastest) for external tools to recompress
///  4. Run oxipng, optipng, zopflipng, pngcrush, ECT on the raw PNG
///  5. Report comparative sizes
///
/// Usage: cargo run --release --example crusher_bench [-- /path/to/png/dir]
use enough::Unstoppable;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/lilith/work/codec-corpus/clic2025-1024".to_string());

    let out_dir = PathBuf::from("/mnt/v/output/zenpng/crusher_bench");
    std::fs::create_dir_all(&out_dir).unwrap();

    // Collect PNG files
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_pngs(Path::new(&dir), &mut paths);
    paths.sort();
    // Limit to first N images for reasonable bench time
    let limit: usize = std::env::var("LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    if paths.len() > limit {
        paths.truncate(limit);
    }
    let n = paths.len();
    eprintln!("Benchmarking {n} images from {dir}");

    // Tool paths
    let oxipng = which("oxipng");
    let optipng = which_local("optipng");
    let zopflipng = which_local("zopflipng");
    let pngcrush = which_local("pngcrush");
    let ect = which_local("ect");

    // Results: (image_name, source_size, zenpng_sizes, external_sizes)
    struct ImageResult {
        name: String,
        source_size: usize,
        raw_pixels: usize,
        // zenpng levels: Fastest(L1), Fast(L4), Balanced(L6), High(L9), Intense(L12), Crush(zopfli)
        zenpng_sizes: [(usize, f64); 8], // (size, seconds)
        // External tools (on source PNG): oxipng -o2, oxipng -o4, oxipng -omax,
        //   optipng -o2, optipng -o5, zopflipng, pngcrush, ect -3, ect -9
        external_sizes: Vec<(String, usize, f64)>, // (name, size, seconds)
    }

    let mut results: Vec<ImageResult> = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let short_name = if name.len() > 16 { &name[..16] } else { &name };
        eprintln!("[{}/{}] {short_name}...", i + 1, n);

        let source_data = std::fs::read(path).unwrap();
        let source_size = source_data.len();

        // Decode
        let decoded =
            match zenpng::decode(&source_data, &zenpng::PngDecodeConfig::none(), &Unstoppable) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("  SKIP: decode error: {e}");
                    continue;
                }
            };

        let (w, h) = (decoded.info.width, decoded.info.height);
        let raw_pixels = match &decoded.pixels {
            zencodec_types::PixelData::Rgb8(img) => img.buf().len() * 3,
            zencodec_types::PixelData::Rgba8(img) => img.buf().len() * 4,
            zencodec_types::PixelData::Gray8(img) => img.buf().len(),
            _ => (w * h * 3) as usize,
        };

        // Re-encode with zenpng at each level
        let levels = [
            ("Fastest", zenpng::Compression::Fastest),
            ("Fast", zenpng::Compression::Fast),
            ("Balanced", zenpng::Compression::Balanced),
            ("High", zenpng::Compression::High),
            ("Intense", zenpng::Compression::Intense),
            ("Crush", zenpng::Compression::Crush),
            ("E31", zenpng::Compression::Effort(31)),
            ("Maniac", zenpng::Compression::Maniac),
        ];

        let mut zenpng_sizes = [(0usize, 0.0f64); 8];
        for (idx, (level_name, comp)) in levels.iter().enumerate() {
            let config = zenpng::EncodeConfig::default()
                .with_compression(*comp)
                .with_source_gamma(decoded.info.source_gamma)
                .with_srgb_intent(decoded.info.srgb_intent)
                .with_chromaticities(decoded.info.chromaticities);

            let start = Instant::now();
            let encoded = match &decoded.pixels {
                zencodec_types::PixelData::Rgb8(img) => zenpng::encode_rgb8(
                    img.as_ref(),
                    None,
                    &config,
                    &enough::Unstoppable,
                    &enough::Unstoppable,
                ),
                zencodec_types::PixelData::Rgba8(img) => zenpng::encode_rgba8(
                    img.as_ref(),
                    None,
                    &config,
                    &enough::Unstoppable,
                    &enough::Unstoppable,
                ),
                zencodec_types::PixelData::Gray8(img) => zenpng::encode_gray8(
                    img.as_ref(),
                    None,
                    &config,
                    &enough::Unstoppable,
                    &enough::Unstoppable,
                ),
                _ => {
                    eprintln!("  SKIP: unsupported pixel format for {level_name}");
                    continue;
                }
            };
            let elapsed = start.elapsed().as_secs_f64();

            match encoded {
                Ok(data) => {
                    zenpng_sizes[idx] = (data.len(), elapsed);
                }
                Err(e) => {
                    eprintln!("  WARN: zenpng {level_name} failed: {e}");
                    zenpng_sizes[idx] = (0, elapsed);
                }
            }
        }

        // Write source PNG to temp for external tools (they work on files)
        // Use the original source file directly - external tools re-compress in place
        let src_path = path.to_string_lossy().to_string();
        let mut external_sizes: Vec<(String, usize, f64)> = Vec::new();

        // --- oxipng ---
        if let Some(ref tool) = oxipng {
            for (label, args) in [
                ("oxipng -o2", vec!["-o", "2"]),
                ("oxipng -o4", vec!["-o", "4"]),
                ("oxipng -omax", vec!["-o", "max"]),
            ] {
                if let Some((size, secs)) = run_tool(tool, &args, &src_path, &out_dir) {
                    external_sizes.push((label.to_string(), size, secs));
                }
            }
        }

        // --- optipng ---
        if let Some(ref tool) = optipng {
            for (label, args) in [("optipng -o2", vec!["-o2"]), ("optipng -o5", vec!["-o5"])] {
                if let Some((size, secs)) = run_tool_optipng(tool, &args, &src_path, &out_dir) {
                    external_sizes.push((label.to_string(), size, secs));
                }
            }
        }

        // --- zopflipng ---
        if let Some(ref tool) = zopflipng {
            if let Some((size, secs)) = run_tool_zopfli(tool, &src_path, &out_dir) {
                external_sizes.push(("zopflipng".to_string(), size, secs));
            }
        }

        // --- pngcrush ---
        if let Some(ref tool) = pngcrush {
            if let Some((size, secs)) = run_tool_pngcrush(tool, &src_path, &out_dir) {
                external_sizes.push(("pngcrush".to_string(), size, secs));
            }
        }

        // --- ECT ---
        if let Some(ref tool) = ect {
            for (label, level) in [("ect -3", "3"), ("ect -9", "9")] {
                if let Some((size, secs)) = run_tool_ect(tool, level, &src_path, &out_dir) {
                    external_sizes.push((label.to_string(), size, secs));
                }
            }
        }

        results.push(ImageResult {
            name: short_name.to_string(),
            source_size,
            raw_pixels,
            zenpng_sizes,
            external_sizes,
        });
    }

    // === Print results ===
    println!();
    println!("=== PNG Compression Benchmark ({n} images) ===");
    println!();

    // Aggregate results
    let mut totals: std::collections::BTreeMap<String, (usize, f64)> =
        std::collections::BTreeMap::new();
    let mut total_source = 0usize;
    let mut total_raw = 0usize;

    let level_names = [
        "zenpng-Fastest",
        "zenpng-Fast",
        "zenpng-Balanced",
        "zenpng-High",
        "zenpng-Intense",
        "zenpng-Crush",
        "zenpng-E31",
        "zenpng-Maniac",
    ];

    for r in &results {
        total_source += r.source_size;
        total_raw += r.raw_pixels;

        for (idx, name) in level_names.iter().enumerate() {
            let entry = totals.entry(name.to_string()).or_default();
            entry.0 += r.zenpng_sizes[idx].0;
            entry.1 += r.zenpng_sizes[idx].1;
        }
        for (name, size, secs) in &r.external_sizes {
            let entry = totals.entry(name.clone()).or_default();
            entry.0 += size;
            entry.1 += secs;
        }
    }

    let total_raw_mib = total_raw as f64 / 1_048_576.0;
    println!(
        "{} images, {:.1} MiB raw pixels, {:.1} MiB source PNGs",
        results.len(),
        total_raw_mib,
        total_source as f64 / 1_048_576.0
    );
    println!();

    // Sort by size
    let mut sorted: Vec<(String, usize, f64)> =
        totals.into_iter().map(|(k, (s, t))| (k, s, t)).collect();
    sorted.sort_by_key(|x| x.1);

    println!(
        "{:<20} {:>12} {:>8} {:>10} {:>8}",
        "Encoder", "Total bytes", "Ratio", "Speed", "vs best"
    );
    println!("{}", "-".repeat(62));

    let best_size = sorted.first().map(|x| x.1).unwrap_or(1);
    for (name, size, secs) in &sorted {
        let ratio = *size as f64 / total_raw as f64 * 100.0;
        let speed = total_raw_mib / secs;
        let vs_best = *size as f64 / best_size as f64 * 100.0;
        println!(
            "{:<20} {:>12} {:>7.2}% {:>8.0} MiB/s {:>6.1}%",
            name, size, ratio, speed, vs_best
        );
    }

    // Per-image detail table
    println!();
    println!("=== Per-image sizes (bytes) ===");
    println!();

    // Header
    print!("{:<18}", "Image");
    print!("{:>10}", "source");
    for name in &level_names {
        let short = name.replace("zenpng-", "zp-");
        print!("{:>10}", short);
    }
    // Find unique external tool names
    let mut ext_names: Vec<String> = Vec::new();
    for r in &results {
        for (name, _, _) in &r.external_sizes {
            if !ext_names.contains(name) {
                ext_names.push(name.clone());
            }
        }
    }
    for name in &ext_names {
        let short = name
            .replace("oxipng ", "oxi")
            .replace("optipng ", "opt")
            .replace("zopflipng", "zopfli")
            .replace("pngcrush", "crush");
        print!("{:>10}", short);
    }
    println!();
    println!("{}", "-".repeat(18 + 10 + 5 * 10 + ext_names.len() * 10));

    for r in &results {
        print!("{:<18}", r.name);
        print!("{:>10}", r.source_size);
        for (size, _) in &r.zenpng_sizes {
            if *size == 0 {
                print!("{:>10}", "ERR");
            } else {
                print!("{:>10}", size);
            }
        }
        for ext_name in &ext_names {
            if let Some((_, size, _)) = r.external_sizes.iter().find(|(n, _, _)| n == ext_name) {
                print!("{:>10}", size);
            } else {
                print!("{:>10}", "-");
            }
        }
        println!();
    }

    // Write CSV for easy analysis
    let csv_path = out_dir.join("results.csv");
    let mut csv = std::fs::File::create(&csv_path).unwrap();
    write!(csv, "image,source").unwrap();
    for name in &level_names {
        write!(csv, ",{name}").unwrap();
    }
    for name in &ext_names {
        write!(csv, ",{name}").unwrap();
    }
    writeln!(csv).unwrap();
    for r in &results {
        write!(csv, "{},{}", r.name, r.source_size).unwrap();
        for (size, _) in &r.zenpng_sizes {
            write!(csv, ",{size}").unwrap();
        }
        for ext_name in &ext_names {
            if let Some((_, size, _)) = r.external_sizes.iter().find(|(n, _, _)| n == ext_name) {
                write!(csv, ",{size}").unwrap();
            } else {
                write!(csv, ",").unwrap();
            }
        }
        writeln!(csv).unwrap();
    }
    eprintln!("CSV written to {}", csv_path.display());
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
        } else if path.extension().is_some_and(|e| e == "png")
            && !path.to_string_lossy().contains("pareto")
        {
            out.push(path);
        }
    }
}

fn which(name: &str) -> Option<String> {
    Command::new("which").arg(name).output().ok().and_then(|o| {
        if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            None
        }
    })
}

fn which_local(name: &str) -> Option<String> {
    let local = format!(
        "{}/.local/bin/{name}",
        std::env::var("HOME").unwrap_or_default()
    );
    if Path::new(&local).exists() {
        Some(local)
    } else {
        which(name)
    }
}

/// Run oxipng: copies source to tmp, runs tool, measures output size.
fn run_tool(tool: &str, args: &[&str], src: &str, out_dir: &Path) -> Option<(usize, f64)> {
    let tmp = out_dir.join("_tmp_tool.png");
    std::fs::copy(src, &tmp).ok()?;
    let start = Instant::now();
    let status = Command::new(tool)
        .args(args)
        .arg("--strip")
        .arg("safe")
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let elapsed = start.elapsed().as_secs_f64();
    if !status.success() {
        return None;
    }
    let size = std::fs::metadata(&tmp).ok()?.len() as usize;
    std::fs::remove_file(&tmp).ok();
    Some((size, elapsed))
}

/// Run optipng: copies source to tmp, runs tool in-place.
fn run_tool_optipng(tool: &str, args: &[&str], src: &str, out_dir: &Path) -> Option<(usize, f64)> {
    let tmp = out_dir.join("_tmp_optipng.png");
    std::fs::copy(src, &tmp).ok()?;
    let start = Instant::now();
    let status = Command::new(tool)
        .args(args)
        .arg("-quiet")
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let elapsed = start.elapsed().as_secs_f64();
    if !status.success() {
        return None;
    }
    let size = std::fs::metadata(&tmp).ok()?.len() as usize;
    std::fs::remove_file(&tmp).ok();
    Some((size, elapsed))
}

/// Run zopflipng: outputs to new file.
fn run_tool_zopfli(tool: &str, src: &str, out_dir: &Path) -> Option<(usize, f64)> {
    let tmp = out_dir.join("_tmp_zopfli.png");
    let start = Instant::now();
    let status = Command::new(tool)
        .arg("-y")
        .arg(src)
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let elapsed = start.elapsed().as_secs_f64();
    if !status.success() {
        return None;
    }
    let size = std::fs::metadata(&tmp).ok()?.len() as usize;
    std::fs::remove_file(&tmp).ok();
    Some((size, elapsed))
}

/// Run pngcrush: outputs to new file.
fn run_tool_pngcrush(tool: &str, src: &str, out_dir: &Path) -> Option<(usize, f64)> {
    let tmp = out_dir.join("_tmp_crush.png");
    let _ = std::fs::remove_file(&tmp); // pngcrush won't overwrite
    let start = Instant::now();
    let status = Command::new(tool)
        .arg("-reduce")
        .arg("-brute")
        .arg(src)
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let elapsed = start.elapsed().as_secs_f64();
    if !status.success() {
        return None;
    }
    let size = std::fs::metadata(&tmp).ok()?.len() as usize;
    std::fs::remove_file(&tmp).ok();
    Some((size, elapsed))
}

/// Run ECT: copies source to tmp, runs in-place.
fn run_tool_ect(tool: &str, level: &str, src: &str, out_dir: &Path) -> Option<(usize, f64)> {
    let tmp = out_dir.join("_tmp_ect.png");
    std::fs::copy(src, &tmp).ok()?;
    let start = Instant::now();
    let status = Command::new(tool)
        .arg(format!("-{level}"))
        .arg("--strict")
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let elapsed = start.elapsed().as_secs_f64();
    if !status.success() {
        return None;
    }
    let size = std::fs::metadata(&tmp).ok()?.len() as usize;
    std::fs::remove_file(&tmp).ok();
    Some((size, elapsed))
}
