use enough::Unstoppable;

/// Test Crush level with various deadlines on a single image.
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/home/lilith/work/codec-corpus/clic2025-1024/0d154749c7771f58e89ad343653ec4e20d6f037da829f47f5598e5d0a4ab61f0.png".to_string()
    });

    let source = std::fs::read(&path).unwrap();
    let decoded = zenpng::decode(&source, None, &Unstoppable).unwrap();

    println!("{:<15} {:>10} {:>8}", "Deadline", "Size", "Time");
    println!("{}", "-".repeat(36));

    // Test various deadlines
    for deadline_ms in [
        None,
        Some(5_000u64),
        Some(10_000),
        Some(15_000),
        Some(30_000),
        Some(60_000),
    ] {
        let config = zenpng::EncodeConfig {
            compression: zenpng::Compression::Crush,
            source_gamma: decoded.info.source_gamma,
            srgb_intent: decoded.info.srgb_intent,
            chromaticities: decoded.info.chromaticities,
            ..Default::default()
        };

        let deadline: Box<dyn enough::Stop> = match deadline_ms {
            Some(ms) => Box::new(almost_enough::time::WithTimeout::new(
                Unstoppable,
                std::time::Duration::from_millis(ms),
            )),
            None => Box::new(Unstoppable),
        };

        let start = std::time::Instant::now();
        let encoded = match &decoded.pixels {
            zencodec_types::PixelData::Rgb8(img) => {
                zenpng::encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &*deadline)
            }
            zencodec_types::PixelData::Rgba8(img) => {
                zenpng::encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &*deadline)
            }
            _ => panic!("unsupported"),
        };
        let elapsed = start.elapsed();

        let label = match deadline_ms {
            None => "unlimited".to_string(),
            Some(ms) => format!("{ms}ms"),
        };

        match encoded {
            Ok(data) => println!(
                "{:<15} {:>10} {:>7.2}s",
                label,
                data.len(),
                elapsed.as_secs_f64()
            ),
            Err(e) => println!("{:<15} ERR: {e}", label),
        }
    }

    println!("\nReference: ect -9 = 1,984,008 bytes");
}
