/// Run a single effort level on the test corpus and output CSV columns.
///
/// Usage: cargo run --release --features _dev --example single_effort -- EFFORT [CORPUS]
use enough::Unstoppable;
use std::path::{Path, PathBuf};
use zenpixels::descriptor::{ChannelLayout, ChannelType};
use zenpixels_convert::PixelBufferConvertTypedExt;

fn main() {
    let effort: u32 = std::env::args()
        .nth(1)
        .expect("Usage: single_effort EFFORT [CORPUS]")
        .parse()
        .expect("effort must be a number");

    let corpus_dir = std::env::args().nth(2).unwrap_or_else(|| {
        format!(
            "{}/test_corpus",
            std::env::var("ZENPNG_OUTPUT_DIR")
                .unwrap_or_else(|_| "/mnt/v/output/zenpng".to_string())
        )
    });

    let mut paths: Vec<PathBuf> = Vec::new();
    collect_pngs(Path::new(&corpus_dir), &mut paths);
    paths.sort();
    let n = paths.len();
    eprintln!("Found {n} PNGs, testing effort {effort}");

    // Header
    println!("filename,e{effort}_size,e{effort}_ms");

    for (i, path) in paths.iter().enumerate() {
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        if (i + 1) % 10 == 0 || i + 1 == n {
            eprintln!("[{}/{}]", i + 1, n);
        }

        let source_data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if source_data.len() > 5_000_000 {
            continue;
        }

        let decoded =
            match zenpng::decode(&source_data, &zenpng::PngDecodeConfig::none(), &Unstoppable) {
                Ok(d) => d,
                Err(_) => continue,
            };

        let w = decoded.info.width;
        let h = decoded.info.height;
        if w < 4 || h < 4 || (w as u64 * h as u64) > 4_000_000 {
            continue;
        }

        let config = zenpng::EncodeConfig::default()
            .with_compression(zenpng::Compression::Effort(effort))
            .with_source_gamma(decoded.info.source_gamma)
            .with_srgb_intent(decoded.info.srgb_intent)
            .with_chromaticities(decoded.info.chromaticities);

        let desc = decoded.pixels.descriptor();
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
            _ => continue,
        };
        let elapsed_ms = start.elapsed().as_millis();

        match result {
            Ok(data) => println!("{fname},{},{elapsed_ms}", data.len()),
            Err(e) => eprintln!("  WARN: {fname} e{effort} failed: {e}"),
        }
    }
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
