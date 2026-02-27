/// Monotonicity benchmark: verify that higher effort never produces larger output.
///
/// Tests all 31 effort levels (0-30) on real corpus images from codec-corpus.
/// Reports per-image tables with size, ratio, delta from previous effort, and
/// flags any monotonicity violations.
///
/// Usage:
///   cargo run --release --features unchecked --example monotonicity_bench
///   cargo run --release --features unchecked --example monotonicity_bench -- --quick
///
/// The `--quick` flag tests only efforts 0,1,3,5,7,8,9,10,12,15,20,24,30 (preset boundaries).
fn main() {
    // NearOptimal strategy in zenflate uses deep recursion; main thread stack
    // is too small for large images at effort 23+. Spawn a worker thread with
    // a generous stack.
    let builder = std::thread::Builder::new()
        .name("bench".into())
        .stack_size(64 * 1024 * 1024); // 64 MiB
    let handle = builder.spawn(run).expect("failed to spawn thread");
    let code = handle.join().expect("thread panicked");
    std::process::exit(code);
}

fn run() -> i32 {
    let quick = std::env::args().any(|a| a == "--quick");

    let efforts: Vec<u32> = if quick {
        vec![0, 1, 3, 5, 7, 8, 9, 10, 12, 15, 20, 24, 30]
    } else {
        (0..=30).collect()
    };

    eprintln!("Downloading corpus images (cached after first run)...");
    let corpus = codec_corpus::Corpus::new().expect("can't initialize codec-corpus cache");

    // Collect images from two corpora
    let mut images: Vec<(String, ImageData)> = Vec::new();

    // gb82-sc: 10 screenshot images
    let sc_path = corpus
        .get("gb82-sc")
        .expect("can't download gb82-sc corpus");
    let mut sc_pngs = collect_pngs(&sc_path);
    sc_pngs.sort();
    for path in &sc_pngs {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let data = decode_png(path);
        images.push((format!("sc/{name}"), data));
    }

    // CID22-512/validation: first 10 of ~41 diverse photos
    let cid_path = corpus
        .get("CID22/CID22-512/validation")
        .expect("can't download CID22-512/validation corpus");
    let mut cid_pngs = collect_pngs(&cid_path);
    cid_pngs.sort();
    for path in cid_pngs.iter().take(10) {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let data = decode_png(path);
        images.push((format!("cid/{name}"), data));
    }

    println!(
        "Monotonicity benchmark: {} images, {} effort levels\n",
        images.len(),
        efforts.len()
    );

    let mut total_violations = 0usize;

    for (name, img) in &images {
        let raw_size = img.width * img.height * img.bpp;
        println!(
            "=== {name} ({}x{}, {}, {:.1} KiB raw) ===",
            img.width,
            img.height,
            match img.bpp {
                1 => "Gray",
                2 => "GA",
                3 => "RGB",
                4 => "RGBA",
                _ => "?",
            },
            raw_size as f64 / 1024.0,
        );
        println!(
            "{:>6}  {:>10}  {:>7}  {:>10}  Status",
            "Effort", "Size", "Ratio", "Delta"
        );
        println!("{}", "-".repeat(52));

        let mut prev_size: Option<usize> = None;
        let mut image_violations = 0usize;

        for &effort in &efforts {
            let encoded = encode_image(img, effort);
            let size = encoded.len();
            let ratio = size as f64 / raw_size as f64 * 100.0;

            let (delta_str, violation) = if let Some(prev) = prev_size {
                let delta = size as i64 - prev as i64;
                if delta > 0 {
                    (format!("+{delta}"), true)
                } else {
                    (format!("{delta}"), false)
                }
            } else {
                ("--".to_string(), false)
            };

            let status = if violation {
                image_violations += 1;
                "  <<<"
            } else {
                ""
            };

            println!("  e{effort:<3}  {size:>10}  {ratio:>6.2}%  {delta_str:>10}  {status}",);

            prev_size = Some(size);
        }

        if image_violations > 0 {
            println!("  *** {image_violations} violation(s) ***");
        }
        println!();
        total_violations += image_violations;
    }

    // Summary
    println!("========================================");
    if total_violations == 0 {
        println!(
            "PASS: No monotonicity violations across {} images",
            images.len()
        );
    } else {
        println!(
            "FAIL: {total_violations} total violation(s) across {} images",
            images.len()
        );
    }

    if total_violations > 0 { 1 } else { 0 }
}

struct ImageData {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    bpp: usize,
}

fn encode_image(img: &ImageData, effort: u32) -> Vec<u8> {
    use enough::Unstoppable;
    use zenpng::{Compression, EncodeConfig};

    let config = EncodeConfig::default().with_compression(Compression::Effort(effort));

    match img.bpp {
        3 => {
            let pixels: &[rgb::Rgb<u8>] = bytemuck::cast_slice(&img.pixels);
            let imgref = imgref::Img::new(pixels, img.width, img.height);
            zenpng::encode_rgb8(imgref, None, &config, &Unstoppable, &Unstoppable)
                .expect("encode failed")
        }
        4 => {
            let pixels: &[rgb::Rgba<u8>] = bytemuck::cast_slice(&img.pixels);
            let imgref = imgref::Img::new(pixels, img.width, img.height);
            zenpng::encode_rgba8(imgref, None, &config, &Unstoppable, &Unstoppable)
                .expect("encode failed")
        }
        1 => {
            // Grayscale: expand to RGB for now
            let rgb_pixels: Vec<u8> = img.pixels.iter().flat_map(|&g| [g, g, g]).collect();
            let pixels: &[rgb::Rgb<u8>] = bytemuck::cast_slice(&rgb_pixels);
            let imgref = imgref::Img::new(pixels, img.width, img.height);
            zenpng::encode_rgb8(imgref, None, &config, &Unstoppable, &Unstoppable)
                .expect("encode failed")
        }
        _ => {
            // Fallback: treat as RGB
            let pixels: &[rgb::Rgb<u8>] = bytemuck::cast_slice(&img.pixels);
            let imgref = imgref::Img::new(pixels, img.width, img.height);
            zenpng::encode_rgb8(imgref, None, &config, &Unstoppable, &Unstoppable)
                .expect("encode failed")
        }
    }
}

fn decode_png(path: &std::path::Path) -> ImageData {
    let file =
        std::fs::File::open(path).unwrap_or_else(|e| panic!("can't open {}: {e}", path.display()));
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().expect("can't read PNG info");
    let info = reader.info();
    let width = info.width as usize;
    let height = info.height as usize;
    let (bpp, output_bpp) = match info.color_type {
        png::ColorType::Grayscale => (1, 1),
        png::ColorType::Rgb => (3, 3),
        png::ColorType::Rgba => (4, 4),
        png::ColorType::GrayscaleAlpha => (2, 4), // will expand
        png::ColorType::Indexed => (3, 3),        // png crate expands to RGB
    };
    let _ = bpp;
    let row_bytes = width * output_bpp;
    let mut pixels = vec![0u8; row_bytes * height];

    // Use output_buffer_size from reader to handle expanded output
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(row_bytes * height)];
    let output_info = reader.next_frame(&mut buf).expect("can't decode frame");
    let decoded_bpp = output_info.line_size / width;

    // Copy decoded data
    for y in 0..height {
        let src = &buf[y * output_info.line_size..y * output_info.line_size + width * decoded_bpp];
        let dst = &mut pixels[y * width * decoded_bpp..(y + 1) * width * decoded_bpp];
        dst.copy_from_slice(src);
    }

    ImageData {
        pixels,
        width,
        height,
        bpp: decoded_bpp,
    }
}

fn collect_pngs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    collect_pngs_recursive(dir, &mut out);
    out
}

fn collect_pngs_recursive(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("can't read {}: {e}", dir.display()));
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_pngs_recursive(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}
