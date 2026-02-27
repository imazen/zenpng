/// Benchmark zenpng APNG compression across effort levels.
///
/// Usage: cargo run --release --example apng_bench [-- /path/to/apng/dir]
use enough::Unstoppable;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/mnt/v/output/gauntlet/apng/_all".to_string());

    let out_dir = PathBuf::from("/mnt/v/output/zenpng/apng_bench");
    std::fs::create_dir_all(&out_dir).unwrap();

    let mut paths: Vec<PathBuf> = Vec::new();
    collect_pngs(Path::new(&dir), &mut paths);
    paths.sort();
    let n = paths.len();
    eprintln!("Benchmarking {n} APNG files from {dir}");

    // Best/Crush run brute-force + zopfli per frame — too slow for APNG bench.
    // High is the practical ceiling for APNG optimization.
    let levels: &[(&str, zenpng::Compression)] = &[
        ("Fastest", zenpng::Compression::Fastest),
        ("Fast", zenpng::Compression::Fast),
        ("Balanced", zenpng::Compression::Balanced),
        ("High", zenpng::Compression::High),
    ];

    struct ImageResult {
        name: String,
        source_size: usize,
        frame_count: usize,
        canvas_w: u32,
        canvas_h: u32,
        zenpng_sizes: Vec<(usize, f64)>, // (size, seconds)
    }

    let mut results: Vec<ImageResult> = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let short_name = if name.len() > 40 { &name[..40] } else { &name };
        let source_data = std::fs::read(path).unwrap();
        let source_size = source_data.len();

        eprint!("[{}/{}] {short_name} ({} bytes)... ", i + 1, n, source_size);

        // Decode APNG
        let decoded =
            match zenpng::decode_apng(&source_data, &zenpng::PngDecodeConfig::none(), &Unstoppable)
            {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("SKIP: {e}");
                    continue;
                }
            };

        let (w, h) = (decoded.info.width, decoded.info.height);
        let frame_count = decoded.frames.len();

        // Build frame inputs (RGBA8)
        let frame_data: Vec<Vec<u8>> = decoded
            .frames
            .iter()
            .map(|f| {
                let rgba = f.pixels.to_rgba8();
                let buf: Vec<rgb::Rgba<u8>> = rgba.into_buf();
                bytemuck::cast_slice::<rgb::Rgba<u8>, u8>(&buf).to_vec()
            })
            .collect();

        let inputs: Vec<zenpng::ApngFrameInput<'_>> = decoded
            .frames
            .iter()
            .zip(frame_data.iter())
            .map(|(f, data)| zenpng::ApngFrameInput {
                pixels: data,
                delay_num: f.frame_info.delay_num,
                delay_den: f.frame_info.delay_den,
            })
            .collect();

        let mut zenpng_sizes = Vec::with_capacity(levels.len());

        for (level_name, comp) in levels.iter() {
            let config = zenpng::ApngEncodeConfig {
                encode: zenpng::EncodeConfig {
                    compression: *comp,
                    ..Default::default()
                },
                num_plays: decoded.num_plays,
            };

            let start = Instant::now();
            let encoded =
                zenpng::encode_apng(&inputs, w, h, &config, None, &Unstoppable, &Unstoppable);
            let elapsed = start.elapsed().as_secs_f64();

            match encoded {
                Ok(data) => {
                    eprint!("{level_name}={} ", data.len());
                    zenpng_sizes.push((data.len(), elapsed));
                }
                Err(e) => {
                    eprint!("{level_name}=ERR({e}) ");
                    zenpng_sizes.push((0, elapsed));
                }
            }
        }
        eprintln!();

        results.push(ImageResult {
            name: short_name.to_string(),
            source_size,
            frame_count,
            canvas_w: w,
            canvas_h: h,
            zenpng_sizes,
        });
    }

    // === Print results ===
    println!();
    println!(
        "=== APNG Compression Benchmark ({} images) ===",
        results.len()
    );
    println!();

    let mut total_source = 0usize;
    let mut total_frames = 0usize;
    let mut totals = vec![(0usize, 0.0f64); levels.len()];

    for r in &results {
        total_source += r.source_size;
        total_frames += r.frame_count;
        for (idx, (size, secs)) in r.zenpng_sizes.iter().enumerate() {
            totals[idx].0 += size;
            totals[idx].1 += secs;
        }
    }

    println!(
        "{} images, {} total frames, {:.2} MiB source",
        results.len(),
        total_frames,
        total_source as f64 / 1_048_576.0
    );
    println!();

    println!(
        "{:<20} {:>12} {:>10} {:>10} {:>8}",
        "Level", "Total bytes", "% source", "Saved", "Time"
    );
    println!("{}", "-".repeat(64));
    for (idx, (level_name, _)) in levels.iter().enumerate() {
        let (size, secs) = totals[idx];
        let pct = if total_source > 0 {
            size as f64 / total_source as f64 * 100.0
        } else {
            0.0
        };
        let saved = if total_source > 0 {
            (1.0 - size as f64 / total_source as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "{:<20} {:>12} {:>9.1}% {:>9.1}% {:>7.1}s",
            format!("zenpng-{level_name}"),
            size,
            pct,
            saved,
            secs
        );
    }

    // Per-image detail
    println!();
    println!("=== Per-image sizes (bytes) ===");
    println!();
    print!("{:<42} {:>4} {:>10}", "Image", "frm", "source");
    for (name, _) in levels {
        print!("{:>10}", name);
    }
    println!();
    println!("{}", "-".repeat(42 + 4 + 10 + levels.len() * 10 + 4));

    for r in &results {
        print!("{:<42} {:>4} {:>10}", r.name, r.frame_count, r.source_size);
        for (size, _) in &r.zenpng_sizes {
            if *size == 0 {
                print!("{:>10}", "ERR");
            } else {
                print!("{:>10}", size);
            }
        }
        println!();
    }

    // Write CSV
    let csv_path = out_dir.join("results.csv");
    let mut csv = std::fs::File::create(&csv_path).unwrap();
    write!(csv, "image,frames,canvas,source").unwrap();
    for (name, _) in levels {
        write!(csv, ",zenpng-{name}").unwrap();
    }
    writeln!(csv).unwrap();
    for r in &results {
        write!(
            csv,
            "{},{},{}x{},{}",
            r.name, r.frame_count, r.canvas_w, r.canvas_h, r.source_size
        )
        .unwrap();
        for (size, _) in &r.zenpng_sizes {
            write!(csv, ",{size}").unwrap();
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
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}
