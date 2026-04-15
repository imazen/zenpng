//! Bench the zencodec `PngDecoderConfig::decode` path, which is where
//! the redundant `detect::probe` scan used to live. Measures end-to-end
//! decode throughput on the regression corpus.

#![forbid(unsafe_code)]

use std::path::Path;
use std::time::Instant;

use enough::Unstoppable;
use imgref::ImgVec;
use rgb::Rgba;
use zenpng::detect::PngProbe;
use zenpng::{Compression, EncodeConfig, PngDecoderConfig, encode_rgba8};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus = root.join("tests/regression");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in std::fs::read_dir(&corpus).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        files.push((name, std::fs::read(&path).unwrap()));
    }
    // Append a synthesized big-text-chunks case (worst case for old full scan).
    files.push((
        "synthetic_many_text_chunks.png".into(),
        synth_png_with_text_chunks(),
    ));

    eprintln!("loaded {} PNG files", files.len());

    let iters: u32 = 1000;
    let dec = PngDecoderConfig::new();

    // Warmup
    for (_, data) in &files {
        let _ = dec.decode(data).unwrap();
    }

    // NEW path: codec's zencodec::decode::Decode impl already embeds
    // PngProbe::from_info inside decode() — no extra scan.
    let start = Instant::now();
    for _ in 0..iters {
        for (_, data) in &files {
            let out = dec.decode(data).unwrap();
            std::hint::black_box(out);
        }
    }
    let new_elapsed = start.elapsed();
    let new_per = new_elapsed.as_nanos() as f64 / (iters as f64 * files.len() as f64);

    // OLD path simulation: full decode + separate detect::probe scan.
    let start = Instant::now();
    for _ in 0..iters {
        for (_, data) in &files {
            let out = dec.decode(data).unwrap();
            let p = zenpng::detect::probe(data).unwrap();
            std::hint::black_box(&out);
            std::hint::black_box(p);
        }
    }
    let old_elapsed = start.elapsed();
    let old_per = old_elapsed.as_nanos() as f64 / (iters as f64 * files.len() as f64);

    // Isolate detect::probe cost alone
    let start = Instant::now();
    for _ in 0..iters {
        for (_, data) in &files {
            let p = zenpng::detect::probe(data).unwrap();
            std::hint::black_box(p);
        }
    }
    let probe_elapsed = start.elapsed();
    let probe_per = probe_elapsed.as_nanos() as f64 / (iters as f64 * files.len() as f64);

    // Isolate PngProbe::from_info cost (decode once, call from_info N times)
    let mut infos = Vec::new();
    for (_, data) in &files {
        let info = zenpng::probe(data).unwrap();
        infos.push(info);
    }
    let start = Instant::now();
    for _ in 0..iters {
        for info in &infos {
            let p = PngProbe::from_info(info);
            std::hint::black_box(p);
        }
    }
    let from_info_elapsed = start.elapsed();
    let from_info_per = from_info_elapsed.as_nanos() as f64 / (iters as f64 * files.len() as f64);

    println!();
    println!(
        "Per-file timings (mean across {} files, {iters} iters):",
        files.len()
    );
    println!("  decode (NEW, probe from decoder state):   {new_per:>10.0} ns/file");
    println!("  decode + detect::probe (OLD scan path):   {old_per:>10.0} ns/file");
    println!("  detect::probe alone:                      {probe_per:>10.0} ns/file");
    println!("  PngProbe::from_info alone:                {from_info_per:>10.0} ns/file");
    println!();
    let saved = old_per - new_per;
    let pct = 100.0 * saved / old_per;
    println!("  saved vs OLD path:                        {saved:>10.0} ns/file  ({pct:.1}%)");

    // Focused timing on the synthetic "many text chunks" file where probe cost peaks.
    let (_, synth) = files.last().unwrap();
    let iters_s = 20000u32;
    let _ = Unstoppable; // silence unused if
    let start = Instant::now();
    for _ in 0..iters_s {
        let p = zenpng::detect::probe(synth).unwrap();
        std::hint::black_box(p);
    }
    let synth_probe = start.elapsed().as_nanos() as f64 / iters_s as f64;
    let synth_info = zenpng::probe(synth).unwrap();
    let start = Instant::now();
    for _ in 0..iters_s {
        let p = PngProbe::from_info(&synth_info);
        std::hint::black_box(p);
    }
    let synth_from_info = start.elapsed().as_nanos() as f64 / iters_s as f64;
    println!();
    println!("Synthetic many-text-chunks PNG ({} bytes):", synth.len());
    println!("  detect::probe:        {synth_probe:>10.0} ns");
    println!("  PngProbe::from_info:  {synth_from_info:>10.0} ns");
    println!(
        "  ratio:                {:>10.1}x faster",
        synth_probe / synth_from_info
    );
}

/// 32x32 RGBA8 PNG with 40 tEXt chunks — worst case for detect::probe's
/// full-file scan (has to traverse every chunk header).
fn synth_png_with_text_chunks() -> Vec<u8> {
    let pixels: Vec<Rgba<u8>> = (0..32 * 32)
        .map(|i| Rgba {
            r: (i & 0xff) as u8,
            g: ((i * 3) & 0xff) as u8,
            b: ((i * 7) & 0xff) as u8,
            a: 255,
        })
        .collect();
    let img = ImgVec::new(pixels, 32, 32);
    let config = EncodeConfig::default().with_compression(Compression::Fastest);
    let mut png = encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();

    // Splice tEXt chunks in front of IEND.
    let iend_start = png.len() - 12;
    let iend = png.split_off(iend_start);
    for i in 0..40 {
        let keyword = b"Comment";
        let text = format!("benchmark text chunk number {i} with some filler content").into_bytes();
        let mut chunk_data = Vec::new();
        chunk_data.extend_from_slice(keyword);
        chunk_data.push(0);
        chunk_data.extend_from_slice(&text);
        let len = (chunk_data.len() as u32).to_be_bytes();
        png.extend_from_slice(&len);
        let type_bytes = *b"tEXt";
        png.extend_from_slice(&type_bytes);
        png.extend_from_slice(&chunk_data);
        let crc = crc32_chunk(&type_bytes, &chunk_data);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    png.extend_from_slice(&iend);
    png
}

fn crc32_chunk(chunk_type: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xffffffffu32;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}
