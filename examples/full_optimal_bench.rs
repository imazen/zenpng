/// Benchmark zenflate FullOptimal vs NearOptimal vs zenzop on raw zlib streams.
///
/// Two sections:
/// 1. End-to-end PNG encoding at various effort levels
/// 2. Raw zlib compression of identical filtered data through each engine
///
/// Usage: cargo run --release --features zopfli --example full_optimal_bench [-- /path/to/image.png]
use enough::Unstoppable;
use std::time::Instant;
use zenpixels::descriptor::{ChannelLayout, ChannelType};
use zenpixels_convert::PixelBufferConvertExt;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/clic2025-1024/0d154749c7771f58e89ad343653ec4e20d6f037da829f47f5598e5d0a4ab61f0.png",
            std::env::var("CODEC_CORPUS_DIR")
                .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string())
        )
    });

    let source = std::fs::read(&path).unwrap();
    let decoded = zenpng::decode(&source, &zenpng::PngDecodeConfig::none(), &Unstoppable).unwrap();
    let short_name = std::path::Path::new(&path)
        .file_stem()
        .unwrap()
        .to_string_lossy();
    let short = if short_name.len() > 20 {
        &short_name[..20]
    } else {
        &short_name
    };

    println!("Image: {short}");
    println!("Original: {} bytes", source.len());

    // ── Section 1: End-to-end PNG ──
    println!("\n=== End-to-end PNG encoding ===");
    println!(
        "{:<30} {:>10} {:>8} {:>8}",
        "Effort", "Size", "Time", "vs E30"
    );
    println!("{}", "-".repeat(60));

    let efforts: &[(&str, u32)] = &[
        ("E30 (NearOptimal)", 30),
        ("E31 (FullOpt 15i)", 31),
        ("E36 (FullOpt 20i)", 36),
        ("E46 (FullOpt 30i)", 46),
        ("E76 (FullOpt 60i)", 76),
    ];

    let desc = decoded.pixels.descriptor();
    let mut base_size = 0usize;

    for &(label, effort) in efforts {
        let config = zenpng::EncodeConfig::default()
            .with_compression(zenpng::Compression::Effort(effort))
            .with_source_gamma(decoded.info.source_gamma)
            .with_srgb_intent(decoded.info.srgb_intent)
            .with_chromaticities(decoded.info.chromaticities);

        let start = Instant::now();
        let encoded = match (desc.layout(), desc.channel_type()) {
            (ChannelLayout::Rgb, ChannelType::U8) => {
                let buf = decoded.pixels.to_rgb8();
                zenpng::encode_rgb8(buf.as_imgref(), None, &config, &Unstoppable, &Unstoppable)
            }
            (ChannelLayout::Rgba, ChannelType::U8) => {
                let buf = decoded.pixels.to_rgba8();
                zenpng::encode_rgba8(buf.as_imgref(), None, &config, &Unstoppable, &Unstoppable)
            }
            _ => panic!("unsupported pixel format: {:?}", desc),
        };
        let elapsed = start.elapsed();

        match encoded {
            Ok(data) => {
                let size = data.len();
                if effort == 30 {
                    base_size = size;
                }
                let delta = if base_size > 0 && effort > 30 {
                    let diff = size as f64 - base_size as f64;
                    format!("{:+.3}%", diff / base_size as f64 * 100.0)
                } else {
                    "baseline".to_string()
                };
                println!(
                    "{:<30} {:>10} {:>7.2}s {:>8}",
                    label,
                    size,
                    elapsed.as_secs_f64(),
                    delta
                );
            }
            Err(e) => println!("{:<30} ERR: {e}", label),
        }
    }

    // ── Section 2: Raw zlib comparison ──
    // Get filtered data from a Maniac-pipeline encode (without zopfli feature, uses E30)
    let maniac_config = zenpng::EncodeConfig::default()
        .with_compression(zenpng::Compression::Maniac)
        .with_source_gamma(decoded.info.source_gamma)
        .with_srgb_intent(decoded.info.srgb_intent)
        .with_chromaticities(decoded.info.chromaticities);
    let maniac_png = match (desc.layout(), desc.channel_type()) {
        (ChannelLayout::Rgb, ChannelType::U8) => {
            let buf = decoded.pixels.to_rgb8();
            zenpng::encode_rgb8(
                buf.as_imgref(),
                None,
                &maniac_config,
                &Unstoppable,
                &Unstoppable,
            )
        }
        (ChannelLayout::Rgba, ChannelType::U8) => {
            let buf = decoded.pixels.to_rgba8();
            zenpng::encode_rgba8(
                buf.as_imgref(),
                None,
                &maniac_config,
                &Unstoppable,
                &Unstoppable,
            )
        }
        _ => panic!("unsupported pixel format: {:?}", desc),
    }
    .unwrap();

    let idat_data = extract_idat(&maniac_png);
    let filtered_bytes = decompress_zlib(&idat_data);

    println!(
        "\n=== Raw zlib compression ({} bytes filtered) ===",
        filtered_bytes.len()
    );
    println!(
        "{:<35} {:>10} {:>8} {:>10}",
        "Engine", "ZlibSize", "Time", "vs NearOpt"
    );
    println!("{}", "-".repeat(67));

    let nearopt_size;

    // zenflate NearOptimal (effort 30)
    {
        let start = Instant::now();
        let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(30));
        let bound = zenflate::Compressor::zlib_compress_bound(filtered_bytes.len());
        let mut output = vec![0u8; bound];
        let len = compressor
            .zlib_compress(&filtered_bytes, &mut output, Unstoppable)
            .unwrap();
        let elapsed = start.elapsed();
        nearopt_size = len;
        println!(
            "{:<35} {:>10} {:>7.2}s {:>10}",
            "zenflate NearOptimal (E30)",
            len,
            elapsed.as_secs_f64(),
            "baseline"
        );
    }

    // zenflate FullOptimal at various efforts
    for &(label, effort) in &[
        ("zenflate FullOpt 15i (E31)", 31u32),
        ("zenflate FullOpt 30i (E46)", 46),
        ("zenflate FullOpt 60i (E76)", 76),
    ] {
        let start = Instant::now();
        let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(effort));
        let bound = zenflate::Compressor::zlib_compress_bound(filtered_bytes.len());
        let mut output = vec![0u8; bound];
        let len = compressor
            .zlib_compress(&filtered_bytes, &mut output, Unstoppable)
            .unwrap();
        let elapsed = start.elapsed();
        let diff = len as f64 - nearopt_size as f64;
        let delta = format!("{:+.3}%", diff / nearopt_size as f64 * 100.0);
        println!(
            "{:<35} {:>10} {:>7.2}s {:>10}",
            label,
            len,
            elapsed.as_secs_f64(),
            delta
        );
    }

    // zenzop at various iteration counts
    #[cfg(feature = "zopfli")]
    {
        for &(label, iters) in &[
            ("zenzop enhanced 15i", 15u64),
            ("zenzop enhanced 30i", 30),
            ("zenzop enhanced 60i", 60),
        ] {
            let start = Instant::now();
            let mut options = zenzop::Options::default();
            options.iteration_count = core::num::NonZeroU64::new(iters).unwrap();
            options.enhanced = true;
            let mut encoder =
                zenzop::ZlibEncoder::with_stop(options, Vec::new(), &Unstoppable).unwrap();
            std::io::Write::write_all(&mut encoder, &filtered_bytes).unwrap();
            let result = encoder.finish().unwrap();
            let len = result.into_inner().len();
            let elapsed = start.elapsed();
            let diff = len as f64 - nearopt_size as f64;
            let delta = format!("{:+.3}%", diff / nearopt_size as f64 * 100.0);
            println!(
                "{:<35} {:>10} {:>7.2}s {:>10}",
                label,
                len,
                elapsed.as_secs_f64(),
                delta
            );
        }
    }
}

fn extract_idat(png: &[u8]) -> Vec<u8> {
    let mut idat = Vec::new();
    let mut pos = 8; // skip PNG signature
    while pos + 12 <= png.len() {
        let len = u32::from_be_bytes([png[pos], png[pos + 1], png[pos + 2], png[pos + 3]]) as usize;
        let chunk_type = &png[pos + 4..pos + 8];
        if chunk_type == b"IDAT" {
            idat.extend_from_slice(&png[pos + 8..pos + 8 + len]);
        }
        pos += 12 + len;
    }
    idat
}

fn decompress_zlib(data: &[u8]) -> Vec<u8> {
    let mut decompressor = zenflate::Decompressor::new();
    let mut output = vec![0u8; data.len() * 10];
    loop {
        match decompressor.zlib_decompress(data, &mut output, Unstoppable) {
            Ok(outcome) => {
                output.truncate(outcome.output_written);
                return output;
            }
            Err(zenflate::DecompressionError::InsufficientSpace) => {
                output.resize(output.len() * 2, 0);
            }
            Err(e) => panic!("decompression failed: {e}"),
        }
    }
}
