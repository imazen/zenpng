use enough::Unstoppable;

/// Time each compression level on a single image.
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/home/lilith/work/codec-corpus/clic2025-1024/0d154749c7771f58e89ad343653ec4e20d6f037da829f47f5598e5d0a4ab61f0.png".to_string()
    });

    let source = std::fs::read(&path).unwrap();
    let decoded = zenpng::decode(&source, None, &Unstoppable).unwrap();

    let levels = [
        ("Fastest", zenpng::Compression::Fastest),
        ("Fast", zenpng::Compression::Fast),
        ("Balanced", zenpng::Compression::Balanced),
        ("Thorough", zenpng::Compression::Thorough),
        ("High", zenpng::Compression::High),
        ("Aggressive", zenpng::Compression::Aggressive),
        ("Best", zenpng::Compression::Best),
        ("Crush", zenpng::Compression::Crush),
    ];

    println!(
        "{:<10} {:>10} {:>8} {:>10}",
        "Level", "Size", "Time", "MiB/s"
    );
    println!("{}", "-".repeat(42));

    let raw_mib = match &decoded.pixels {
        zencodec_types::PixelData::Rgb8(img) => img.buf().len() as f64 * 3.0 / 1_048_576.0,
        zencodec_types::PixelData::Rgba8(img) => img.buf().len() as f64 * 4.0 / 1_048_576.0,
        _ => 0.0,
    };

    for (name, comp) in &levels {
        let config = zenpng::EncodeConfig {
            compression: *comp,
            source_gamma: decoded.info.source_gamma,
            srgb_intent: decoded.info.srgb_intent,
            chromaticities: decoded.info.chromaticities,
            ..Default::default()
        };

        let start = std::time::Instant::now();
        let encoded = match &decoded.pixels {
            zencodec_types::PixelData::Rgb8(img) => {
                zenpng::encode_rgb8(img.as_ref(), None, &config)
            }
            zencodec_types::PixelData::Rgba8(img) => {
                zenpng::encode_rgba8(img.as_ref(), None, &config)
            }
            _ => panic!("unsupported"),
        };
        let elapsed = start.elapsed().as_secs_f64();

        match encoded {
            Ok(data) => {
                let speed = raw_mib / elapsed;
                println!(
                    "{:<10} {:>10} {:>7.2}s {:>8.1}",
                    name,
                    data.len(),
                    elapsed,
                    speed
                );
            }
            Err(e) => println!("{:<10} ERR: {e}", name),
        }
    }
}
