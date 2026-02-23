/// Benchmark zoint (joint deflate+quantization) across images and settings.
///
/// Usage: cargo run --release --features zoint --example zoint_bench [-- /path/to/png/dir]
///
/// Tests: standard Png vs PngZoint at varying tolerances, dither strengths,
/// and quality presets. Reports file size savings and MPE quality impact.
use std::path::{Path, PathBuf};
use std::time::Instant;

use imgref::ImgVec;
use rgb::Rgba;

use zenpng::{EncodeConfig, PngDecodeConfig, encode_indexed_rgba8, decode};

use zenquant::{OutputFormat, Quality, QuantizeConfig};

fn load_png_as_rgba(path: &Path) -> Option<ImgVec<Rgba<u8>>> {
    let data = std::fs::read(path).ok()?;
    let decoded = decode(
        &data,
        &PngDecodeConfig::none(),
        &enough::Unstoppable,
    )
    .ok()?;
    let w = decoded.info.width as usize;
    let h = decoded.info.height as usize;
    let rgba_img = decoded.pixels.into_rgba8();
    let pixels: Vec<Rgba<u8>> = rgba_img
        .buf()
        .iter()
        .map(|c| Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        })
        .collect();
    if pixels.len() == w * h {
        Some(ImgVec::new(pixels, w, h))
    } else {
        None
    }
}

struct EncResult {
    size: usize,
    mpe: Option<f32>,
    ssim2: Option<f32>,
    elapsed_ms: u64,
}

fn encode_with_config(
    img: &ImgVec<Rgba<u8>>,
    quant_config: &QuantizeConfig,
) -> Option<EncResult> {
    let enc_config = EncodeConfig::default();
    let rgba_slice: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(img.buf().as_slice());

    // Quantize (includes zoint if PngZoint format)
    let result = zenquant::quantize_rgba(
        rgba_slice,
        img.width(),
        img.height(),
        quant_config,
    )
    .ok()?;

    // Encode to PNG from the quantized result
    let start = Instant::now();
    let data = encode_indexed_rgba8(
        img.as_ref(),
        &enc_config,
        quant_config,
        None,
        &enough::Unstoppable,
        &enough::Unstoppable,
    )
    .ok()?;
    let elapsed_ms = start.elapsed().as_millis() as u64;

    // Compute fast-ssim2 on the ACTUAL indices (including zoint modifications)
    let mpe_result = zenquant::_internals::compute_mpe_rgba(
        rgba_slice,
        result.palette_rgba(),
        result.indices(),
        img.width(),
        img.height(),
        None,
    );

    Some(EncResult {
        size: data.len(),
        mpe: Some(mpe_result.score),
        ssim2: Some(mpe_result.ssimulacra2_estimate),
        elapsed_ms,
    })
}

fn collect_images(dir: &Path, max: usize) -> Vec<PathBuf> {
    let mut images = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "png") {
                images.push(path);
                if images.len() >= max {
                    break;
                }
            }
        }
    }
    images.sort();
    images
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Collect images from multiple sources
    let mut images: Vec<(String, ImgVec<Rgba<u8>>)> = Vec::new();

    // Synthetic gradient
    {
        let mut pixels = Vec::with_capacity(256 * 256);
        for y in 0..256u32 {
            for x in 0..256u32 {
                pixels.push(Rgba {
                    r: x.min(255) as u8,
                    g: y.min(255) as u8,
                    b: ((x + y) / 2).min(255) as u8,
                    a: 255,
                });
            }
        }
        images.push(("gradient-256".into(), ImgVec::new(pixels, 256, 256)));
    }

    // Load from directory or default corpus paths
    let dirs: Vec<PathBuf> = if args.len() > 1 {
        vec![PathBuf::from(&args[1])]
    } else {
        vec![
            PathBuf::from("/home/lilith/work/codec-corpus/imageflow/test_inputs"),
            PathBuf::from("/home/lilith/work/codec-corpus/kadid10k"),
            PathBuf::from("/home/lilith/work/codec-corpus/CID22/CID22-512/validation"),
        ]
    };

    for dir in &dirs {
        let limit = if dir.to_string_lossy().contains("kadid") {
            10
        } else if dir.to_string_lossy().contains("CID22") {
            10
        } else {
            20
        };
        for path in collect_images(dir, limit) {
            if let Some(img) = load_png_as_rgba(&path) {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                // Skip very large images (>2MP) to keep bench manageable
                if img.width() * img.height() > 2_000_000 {
                    continue;
                }
                images.push((name, img));
            }
        }
    }

    eprintln!("Loaded {} images\n", images.len());

    // === Test 1: Tolerance sweep at default settings ===
    println!("=== Tolerance sweep (Png dither=0.5 vs PngZoint dither=0.3) ===");
    println!(
        "{:30} {:>5} {:>8} {:>8} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "Image", "tol", "std", "zoint", "save%", "ss2_s", "ss2_z", "Δss2", "mpe_z"
    );
    println!("{}", "-".repeat(110));

    for tol in [0.005, 0.01, 0.015, 0.02, 0.03] {
        let mut total_std = 0usize;
        let mut total_zoint = 0usize;

        for (name, img) in &images {
            let std_config = QuantizeConfig::new(OutputFormat::Png)
                .compute_quality_metric(true);
            let zoint_config = QuantizeConfig::new(OutputFormat::PngZoint)
                ._zoint_tolerance(tol)
                .compute_quality_metric(true);

            let std_res = match encode_with_config(img, &std_config) {
                Some(r) => r,
                None => continue,
            };
            let zoint_res = match encode_with_config(img, &zoint_config) {
                Some(r) => r,
                None => continue,
            };

            total_std += std_res.size;
            total_zoint += zoint_res.size;

            let save = (1.0 - zoint_res.size as f64 / std_res.size as f64) * 100.0;
            let delta_ss2 = match (std_res.ssim2, zoint_res.ssim2) {
                (Some(s), Some(z)) => Some(z - s),
                _ => None,
            };
            println!(
                "{:30} {:5.3} {:>8} {:>8} {:>+6.1}% {:>7} {:>7} {:>7} {:>7}",
                truncate_name(name, 30),
                tol,
                std_res.size,
                zoint_res.size,
                save,
                fmt_ss2(std_res.ssim2),
                fmt_ss2(zoint_res.ssim2),
                delta_ss2.map_or("  n/a".into(), |d| format!("{:+.1}", d)),
                fmt_mpe(zoint_res.mpe),
            );
        }

        let save = (1.0 - total_zoint as f64 / total_std as f64) * 100.0;
        println!(
            "{:30} {:5.3} {:>8} {:>8} {:>+6.1}%",
            "** TOTAL **", tol, total_std, total_zoint, save
        );
        println!();
    }

    // === Test 2: Dither strength sweep at tol=0.01 ===
    println!("\n=== Dither strength sweep (tol=0.01) ===");
    println!(
        "{:30} {:>7} {:>8} {:>8} {:>7} {:>7} {:>7}",
        "Image", "dither", "std", "zoint", "save%", "mpe_z", "ss2_z"
    );
    println!("{}", "-".repeat(95));

    for dither_str in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
        let mut total_std = 0usize;
        let mut total_zoint = 0usize;

        for (name, img) in &images {
            let base_std = if dither_str == 0.0 {
                QuantizeConfig::new(OutputFormat::Png)._no_dither()
            } else {
                QuantizeConfig::new(OutputFormat::Png)._dither_strength(dither_str)
            };
            let base_zoint = if dither_str == 0.0 {
                QuantizeConfig::new(OutputFormat::PngZoint)
                    ._no_dither()
                    ._zoint_tolerance(0.01)
            } else {
                QuantizeConfig::new(OutputFormat::PngZoint)
                    ._dither_strength(dither_str)
                    ._zoint_tolerance(0.01)
            };

            let std_res = match encode_with_config(img, &base_std) {
                Some(r) => r,
                None => continue,
            };
            let zoint_res = match encode_with_config(img, &base_zoint) {
                Some(r) => r,
                None => continue,
            };

            total_std += std_res.size;
            total_zoint += zoint_res.size;

            let save = (1.0 - zoint_res.size as f64 / std_res.size as f64) * 100.0;
            println!(
                "{:30} {:>7.2} {:>8} {:>8} {:>+6.1}% {:>7} {:>7}",
                truncate_name(name, 30),
                dither_str,
                std_res.size,
                zoint_res.size,
                save,
                fmt_mpe(zoint_res.mpe),
                fmt_ss2(zoint_res.ssim2),
            );
        }

        let save = (1.0 - total_zoint as f64 / total_std as f64) * 100.0;
        println!(
            "{:30} {:>7.2} {:>8} {:>8} {:>+6.1}%",
            "** TOTAL **", dither_str, total_std, total_zoint, save
        );
        println!();
    }

    // === Test 3: Quality preset comparison at tol=0.01 ===
    println!("\n=== Quality preset comparison (tol=0.01) ===");
    println!(
        "{:30} {:>8} {:>8} {:>7}   {:>8} {:>8} {:>7}",
        "Image", "fast_s", "fast_z", "fsave%", "best_s", "best_z", "bsave%"
    );
    println!("{}", "-".repeat(105));

    let mut ft_std = 0usize;
    let mut ft_zoint = 0usize;
    let mut bt_std = 0usize;
    let mut bt_zoint = 0usize;

    for (name, img) in &images {
        let fast_std = QuantizeConfig::new(OutputFormat::Png)
            .quality(Quality::Fast);
        let fast_zoint = QuantizeConfig::new(OutputFormat::PngZoint)
            .quality(Quality::Fast)
            ._zoint_tolerance(0.01);
        let best_std = QuantizeConfig::new(OutputFormat::Png)
            .quality(Quality::Best);
        let best_zoint = QuantizeConfig::new(OutputFormat::PngZoint)
            .quality(Quality::Best)
            ._zoint_tolerance(0.01);

        let fs = match encode_with_config(img, &fast_std) {
            Some(r) => r,
            None => continue,
        };
        let fz = match encode_with_config(img, &fast_zoint) {
            Some(r) => r,
            None => continue,
        };
        let bs = match encode_with_config(img, &best_std) {
            Some(r) => r,
            None => continue,
        };
        let bz = match encode_with_config(img, &best_zoint) {
            Some(r) => r,
            None => continue,
        };

        ft_std += fs.size;
        ft_zoint += fz.size;
        bt_std += bs.size;
        bt_zoint += bz.size;

        let fs_save = (1.0 - fz.size as f64 / fs.size as f64) * 100.0;
        let bs_save = (1.0 - bz.size as f64 / bs.size as f64) * 100.0;

        println!(
            "{:30} {:>8} {:>8} {:>+6.1}% {:>7} {:>8} {:>8} {:>+6.1}%",
            truncate_name(name, 30),
            fs.size,
            fz.size,
            fs_save,
            "",
            bs.size,
            bz.size,
            bs_save,
        );
    }

    println!(
        "{:30} {:>8} {:>8} {:>+6.1}% {:>7} {:>8} {:>8} {:>+6.1}%",
        "** TOTAL **",
        ft_std,
        ft_zoint,
        (1.0 - ft_zoint as f64 / ft_std as f64) * 100.0,
        "",
        bt_std,
        bt_zoint,
        (1.0 - bt_zoint as f64 / bt_std as f64) * 100.0,
    );

    // === Test 4: target_ssim2 interaction ===
    println!("\n\n=== target_ssim2 interaction (tol=0.01) ===");
    println!(
        "{:30} {:>6} {:>8} {:>8} {:>7} {:>7} {:>7}",
        "Image", "tgt_s2", "std", "zoint", "save%", "mpe_z", "ss2_z"
    );
    println!("{}", "-".repeat(90));

    for target in [90.0_f32, 80.0, 70.0] {
        let mut total_std = 0usize;
        let mut total_zoint = 0usize;

        for (name, img) in &images {
            let std_config = QuantizeConfig::new(OutputFormat::Png)
                .target_ssim2(target);
            let zoint_config = QuantizeConfig::new(OutputFormat::PngZoint)
                .target_ssim2(target)
                ._zoint_tolerance(0.01);

            let std_res = match encode_with_config(img, &std_config) {
                Some(r) => r,
                None => continue,
            };
            let zoint_res = match encode_with_config(img, &zoint_config) {
                Some(r) => r,
                None => continue,
            };

            total_std += std_res.size;
            total_zoint += zoint_res.size;

            let save = (1.0 - zoint_res.size as f64 / std_res.size as f64) * 100.0;
            println!(
                "{:30} {:>6.1} {:>8} {:>8} {:>+6.1}% {:>7} {:>7}",
                truncate_name(name, 30),
                target,
                std_res.size,
                zoint_res.size,
                save,
                fmt_mpe(zoint_res.mpe),
                fmt_ss2(zoint_res.ssim2),
            );
        }

        let save = (1.0 - total_zoint as f64 / total_std as f64) * 100.0;
        println!(
            "{:30} {:>6.1} {:>8} {:>8} {:>+6.1}%",
            "** TOTAL **", target, total_std, total_zoint, save
        );
        println!();
    }
}

fn truncate_name(name: &str, max: usize) -> String {
    if name.len() <= max {
        name.to_string()
    } else {
        format!("{}…", &name[..max - 1])
    }
}

fn fmt_mpe(mpe: Option<f32>) -> String {
    mpe.map_or("  n/a".into(), |v| format!("{:.4}", v))
}

fn fmt_ss2(ss2: Option<f32>) -> String {
    ss2.map_or("  n/a".into(), |v| format!("{:.1}", v))
}
