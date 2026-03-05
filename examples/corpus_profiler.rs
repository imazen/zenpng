/// Profile corpus images at multiple effort levels for clustering analysis.
///
/// Outputs CSV with compression ratios at each effort tier, enabling
/// clustering to find representative images for benchmarking.
///
/// Usage: cargo run --release --example corpus_profiler -- /path/to/png/dir [SAMPLE_SIZE]
///
/// Outputs to $ZENPNG_OUTPUT_DIR/corpus_profile/{dirname}.csv
use enough::Unstoppable;
use std::io::Write;
use std::path::{Path, PathBuf};
use zencodec_types::PixelBufferConvertExt;
use zenpixels::descriptor::{ChannelLayout, ChannelType};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("Usage: corpus_profiler /path/to/png/dir [SAMPLE_SIZE]");

    let sample_size: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    let dirname = Path::new(&dir)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let out_dir = PathBuf::from(
        std::env::var("ZENPNG_OUTPUT_DIR").unwrap_or_else(|_| "/mnt/v/output/zenpng".to_string()),
    )
    .join("corpus_profile");
    std::fs::create_dir_all(&out_dir).unwrap();
    let csv_path = out_dir.join(format!("{dirname}.csv"));

    // Collect PNG files
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_pngs(Path::new(&dir), &mut paths);
    paths.sort();

    eprintln!(
        "Found {} PNGs in {dir}, sampling {sample_size}",
        paths.len()
    );

    // Deterministic sampling: take every Nth file
    let step = if paths.len() <= sample_size {
        1
    } else {
        paths.len() / sample_size
    };
    let sampled: Vec<&PathBuf> = paths.iter().step_by(step).take(sample_size).collect();
    let n = sampled.len();
    eprintln!("Processing {n} images...");

    // Effort levels to profile — chosen to span strategy boundaries
    // e2=Turbo, e7=FastHt, e13=Lazy, e22=Lazy2
    // Skipping e30 (NearOptimal) — too slow for corpus profiling.
    // The 4 levels capture enough strategy diversity for clustering.
    let efforts: &[u32] = &[2, 7, 13, 22];

    let mut csv = std::fs::File::create(&csv_path).unwrap();

    // Header
    write!(
        csv,
        "filename,filesize,width,height,color_type,bpp,raw_bytes"
    )
    .unwrap();
    for e in efforts {
        write!(csv, ",e{e}_size,e{e}_ms").unwrap();
    }
    writeln!(csv).unwrap();

    let mut ok = 0usize;
    let mut skip = 0usize;

    for (i, path) in sampled.iter().enumerate() {
        if (i + 1) % 50 == 0 || i + 1 == n {
            eprintln!("[{}/{}] ok={ok} skip={skip}", i + 1, n);
        }

        let fname = path.file_name().unwrap().to_string_lossy();

        // Read file
        let source_data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => {
                skip += 1;
                continue;
            }
        };
        let filesize = source_data.len();

        // Size limit: skip very large files (>2MB) to keep profiling fast
        if filesize > 2_000_000 {
            skip += 1;
            continue;
        }

        // Decode
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

        // Skip tiny images (< 16x16) and huge images (> 2K×2K)
        if w < 16 || h < 16 || (w as u64 * h as u64) > 4_000_000 {
            skip += 1;
            continue;
        }

        let desc = decoded.pixels.descriptor();
        let (color_type, bpp) = match (desc.layout(), desc.channel_type()) {
            (ChannelLayout::Gray, ChannelType::U8) => ("gray8", 1),
            (ChannelLayout::Rgb, ChannelType::U8) => ("rgb8", 3),
            (ChannelLayout::Rgba, ChannelType::U8) => ("rgba8", 4),
            (ChannelLayout::Gray, ChannelType::U16) => ("gray16", 2),
            (ChannelLayout::Rgb, ChannelType::U16) => ("rgb16", 6),
            (ChannelLayout::Rgba, ChannelType::U16) => ("rgba16", 8),
            _ => {
                skip += 1;
                continue;
            }
        };
        let raw_bytes = w as usize * h as usize * bpp;

        write!(
            csv,
            "{fname},{filesize},{w},{h},{color_type},{bpp},{raw_bytes}"
        )
        .unwrap();

        // Compress at each effort level
        for &effort in efforts {
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
                Ok(data) => write!(csv, ",{},{elapsed_ms}", data.len()).unwrap(),
                Err(_) => write!(csv, ",0,0").unwrap(),
            }
        }
        writeln!(csv).unwrap();
        ok += 1;
    }

    eprintln!(
        "Done: {ok} profiled, {skip} skipped. CSV: {}",
        csv_path.display()
    );
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
