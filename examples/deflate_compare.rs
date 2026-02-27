/// Compare DEFLATE backends on real filtered PNG data.
///
/// For each image: decode → filter with brute-force → compress with
/// zenflate L10/L11/L12, libdeflate-C L12, flate2 best, and zopfli → report sizes.
///
/// Usage: cargo run --release --example deflate_compare [-- /path/to/png/dir]
use enough::Unstoppable;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/home/lilith/work/codec-corpus/clic2025-1024".to_string());

    let mut paths: Vec<PathBuf> = Vec::new();
    collect_pngs(Path::new(&dir), &mut paths);
    paths.sort();
    if paths.len() > 20 {
        paths.truncate(20);
    }

    eprintln!("Comparing DEFLATE backends on {} images", paths.len());

    // Print header
    println!(
        "{:<18} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>7} {:>7}",
        "image",
        "zf-L10",
        "zf-L11",
        "zf-L12",
        "ldc-L12",
        "fl2-best",
        "zopfli",
        "zf/ldc%",
        "zf/zop%"
    );
    println!("{}", "-".repeat(105));

    let mut total_zf_best = 0usize;
    let mut total_ldc12 = 0usize;
    let mut total_fl2_best = 0usize;
    let mut total_zopfli = 0usize;

    for (i, path) in paths.iter().enumerate() {
        let name = path.file_stem().unwrap().to_string_lossy();
        let short = if name.len() > 16 { &name[..16] } else { &name };
        eprintln!("[{}/{}] {short}...", i + 1, paths.len());

        let source = std::fs::read(path).unwrap();
        let decoded = match zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable)
        {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  SKIP: {e}");
                continue;
            }
        };

        let (w, h) = (decoded.info.width as usize, decoded.info.height as usize);

        // Get raw pixel bytes
        let (pixel_bytes, bpp): (Vec<u8>, usize) = match &decoded.pixels {
            zencodec_types::PixelData::Rgb8(img) => {
                let buf = img.buf();
                (bytemuck::cast_slice::<_, u8>(buf).to_vec(), 3)
            }
            zencodec_types::PixelData::Rgba8(img) => {
                let buf = img.buf();
                (bytemuck::cast_slice::<_, u8>(buf).to_vec(), 4)
            }
            _ => {
                eprintln!("  SKIP: unsupported pixel format");
                continue;
            }
        };
        let row_bytes = w * bpp;

        // Filter with brute-force (L1 eval, 10 context rows) — same as Best level
        let filtered = filter_brute_force(&pixel_bytes, row_bytes, h, bpp);

        // Compress with zenflate at L10, L11, L12
        let zf10 = compress_zenflate(&filtered, 10);
        let zf11 = compress_zenflate(&filtered, 11);
        let zf12 = compress_zenflate(&filtered, 12);
        let zf_best = zf10.min(zf11).min(zf12);

        // Compress with libdeflate-C at L12
        let ldc12 = compress_libdeflate_c(&filtered, 12);

        // Compress with flate2 at levels 1-9 + miniz_oxide L10
        let mut fl2_best_size = usize::MAX;
        for level in 1..=9 {
            let size = compress_flate2(&filtered, level);
            fl2_best_size = fl2_best_size.min(size);
        }
        let mz_size = compress_miniz_oxide(&filtered);
        fl2_best_size = fl2_best_size.min(mz_size);

        // Compress with zopfli (15 iterations)
        let zopfli_size = compress_zopfli(&filtered, 15);

        let zf_vs_ldc = (zf_best as f64 - ldc12 as f64) / ldc12 as f64 * 100.0;
        let zf_vs_zop = (zf_best as f64 - zopfli_size as f64) / zopfli_size as f64 * 100.0;

        println!(
            "{:<18} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>+7.3} {:>+7.3}",
            short, zf10, zf11, zf12, ldc12, fl2_best_size, zopfli_size, zf_vs_ldc, zf_vs_zop,
        );

        total_zf_best += zf_best;
        total_ldc12 += ldc12;
        total_fl2_best += fl2_best_size;
        total_zopfli += zopfli_size;
    }

    println!("{}", "-".repeat(105));
    let zf_vs_ldc = (total_zf_best as f64 - total_ldc12 as f64) / total_ldc12 as f64 * 100.0;
    let zf_vs_zop = (total_zf_best as f64 - total_zopfli as f64) / total_zopfli as f64 * 100.0;
    println!(
        "{:<18} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>+7.3} {:>+7.3}",
        "TOTAL",
        "",
        "",
        total_zf_best,
        total_ldc12,
        total_fl2_best,
        total_zopfli,
        zf_vs_ldc,
        zf_vs_zop,
    );
    println!("\nzenflate-best vs libdeflate-C: {:+.3}%", zf_vs_ldc);
    println!("zenflate-best vs zopfli:       {:+.3}%", zf_vs_zop);
    println!(
        "zenflate-best vs flate2-best:  {:+.3}%",
        (total_zf_best as f64 - total_fl2_best as f64) / total_fl2_best as f64 * 100.0
    );
}

fn filter_brute_force(pixel_bytes: &[u8], row_bytes: usize, height: usize, bpp: usize) -> Vec<u8> {
    let filtered_row_size = row_bytes + 1;
    let max_context_bytes = 32 * 1024;
    let context_rows = (max_context_bytes / filtered_row_size).clamp(1, 10);
    let max_context = context_rows * filtered_row_size;

    let eval_level = zenflate::CompressionLevel::new(1);
    let mut eval_compressor = zenflate::Compressor::new(eval_level);

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
            {
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
        0 => out.copy_from_slice(row), // None
        1 => {
            // Sub
            out[..bpp].copy_from_slice(&row[..bpp]);
            for i in bpp..row.len() {
                out[i] = row[i].wrapping_sub(row[i - bpp]);
            }
        }
        2 => {
            // Up
            for i in 0..row.len() {
                out[i] = row[i].wrapping_sub(prev_row[i]);
            }
        }
        3 => {
            // Average
            for i in 0..bpp {
                out[i] = row[i].wrapping_sub(prev_row[i] / 2);
            }
            for i in bpp..row.len() {
                let avg = ((row[i - bpp] as u16 + prev_row[i] as u16) / 2) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
            // Paeth
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

fn compress_zenflate(data: &[u8], level: u32) -> usize {
    let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(level));
    let bound = zenflate::Compressor::zlib_compress_bound(data.len());
    let mut buf = vec![0u8; bound];
    compressor
        .zlib_compress(data, &mut buf, zenflate::Unstoppable)
        .unwrap()
}

fn compress_libdeflate_c(data: &[u8], level: i32) -> usize {
    let mut compressor =
        libdeflater::Compressor::new(libdeflater::CompressionLvl::new(level).unwrap());
    let bound = compressor.zlib_compress_bound(data.len());
    let mut buf = vec![0u8; bound];
    compressor.zlib_compress(data, &mut buf).unwrap()
}

fn compress_flate2(data: &[u8], level: u32) -> usize {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level));
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap().len()
}

fn compress_miniz_oxide(data: &[u8]) -> usize {
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(data, 10);
    compressed.len()
}

fn compress_zopfli(data: &[u8], iterations: i32) -> usize {
    use zopfli::Options;
    let options = Options {
        iteration_count: std::num::NonZeroU64::new(iterations as u64).unwrap(),
        ..Default::default()
    };
    let mut output = Vec::new();
    zopfli::compress(options, zopfli::Format::Zlib, data, &mut output).unwrap();
    output.len()
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
