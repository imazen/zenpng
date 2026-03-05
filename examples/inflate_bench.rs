/// Compare raw inflate speed: zenflate vs miniz_oxide vs flate2 vs libdeflater.
///
/// Extracts zlib data from IDAT chunks, then benchmarks inflate only.
///
/// Usage:
///   cargo run --release --no-default-features --example inflate_bench [-- image.png]
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/qoi-benchmark/screenshot_web/reddit.com.png",
            std::env::var("CODEC_CORPUS_DIR")
                .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string())
        )
    });
    let source = std::fs::read(&path).expect("read");

    // Extract concatenated IDAT payload (zlib stream)
    let zlib_data = extract_idat_payload(&source);
    println!(
        "Image: {}\nIDAT payload: {:.2}M zlib\n",
        std::path::Path::new(&path)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        zlib_data.len() as f64 / 1e6
    );

    // Figure out decompressed size via miniz_oxide first
    let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&zlib_data).expect("inflate");
    let dec_size = decompressed.len();
    drop(decompressed);
    println!(
        "Decompressed: {:.2}M ({:.1}x ratio)\n",
        dec_size as f64 / 1e6,
        dec_size as f64 / zlib_data.len() as f64
    );

    let iters = 10;

    // --- zenflate (one-shot) ---
    {
        use enough::Unstoppable;
        let mut d = zenflate::Decompressor::new();
        let mut out = vec![0u8; dec_size];
        d.zlib_decompress(&zlib_data, &mut out, Unstoppable)
            .unwrap();

        let t = Instant::now();
        for _ in 0..iters {
            let mut d = zenflate::Decompressor::new();
            d.zlib_decompress(&zlib_data, &mut out, Unstoppable)
                .unwrap();
            std::hint::black_box(&out);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{:<16} {:>8.1}ms  ({:.0} MB/s)",
            "zenflate",
            ms,
            dec_size as f64 / ms / 1000.0
        );
    }

    // --- zenflate streaming (what decode actually uses) ---
    {
        struct SliceSource<'a> {
            data: &'a [u8],
        }
        impl<'a> zenflate::InputSource for SliceSource<'a> {
            type Error = String;
            fn fill_buf(&mut self) -> Result<&[u8], String> {
                Ok(self.data)
            }
            fn consume(&mut self, n: usize) {
                self.data = &self.data[n..];
            }
        }

        // Warmup
        {
            let src = SliceSource { data: &zlib_data };
            let mut sd = zenflate::StreamDecompressor::zlib(src, 32768);
            while !sd.is_done() {
                sd.fill().unwrap();
                let n = sd.peek().len();
                sd.advance(n);
            }
        }

        let t = Instant::now();
        for _ in 0..iters {
            let src = SliceSource { data: &zlib_data };
            let mut sd = zenflate::StreamDecompressor::zlib(src, 32768);
            while !sd.is_done() {
                sd.fill().unwrap();
                let n = sd.peek().len();
                sd.advance(n);
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{:<16} {:>8.1}ms  ({:.0} MB/s)",
            "zenflate-stream",
            ms,
            dec_size as f64 / ms / 1000.0
        );
    }

    // --- miniz_oxide ---
    {
        let _ = miniz_oxide::inflate::decompress_to_vec_zlib(&zlib_data).unwrap();
        let t = Instant::now();
        for _ in 0..iters {
            let r = miniz_oxide::inflate::decompress_to_vec_zlib(&zlib_data).unwrap();
            std::hint::black_box(&r);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{:<16} {:>8.1}ms  ({:.0} MB/s)",
            "miniz_oxide",
            ms,
            dec_size as f64 / ms / 1000.0
        );
    }

    // --- flate2 (wraps miniz_oxide by default) ---
    {
        use std::io::Read;
        let mut out = Vec::with_capacity(dec_size);
        flate2::read::ZlibDecoder::new(&zlib_data[..])
            .read_to_end(&mut out)
            .unwrap();
        out.clear();

        let t = Instant::now();
        for _ in 0..iters {
            let mut out = Vec::with_capacity(dec_size);
            flate2::read::ZlibDecoder::new(&zlib_data[..])
                .read_to_end(&mut out)
                .unwrap();
            std::hint::black_box(&out);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{:<16} {:>8.1}ms  ({:.0} MB/s)",
            "flate2",
            ms,
            dec_size as f64 / ms / 1000.0
        );
    }

    // --- libdeflater ---
    {
        let mut out = vec![0u8; dec_size];
        let mut d = libdeflater::Decompressor::new();
        d.zlib_decompress(&zlib_data, &mut out).unwrap();

        let t = Instant::now();
        for _ in 0..iters {
            let mut d = libdeflater::Decompressor::new();
            d.zlib_decompress(&zlib_data, &mut out).unwrap();
            std::hint::black_box(&out);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!(
            "{:<16} {:>8.1}ms  ({:.0} MB/s)",
            "libdeflater",
            ms,
            dec_size as f64 / ms / 1000.0
        );
    }
}

/// Extract concatenated IDAT chunk data (zlib stream) from a PNG file.
fn extract_idat_payload(png: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut pos = 8; // skip signature
    while pos + 12 <= png.len() {
        let length = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        let chunk_type: [u8; 4] = png[pos + 4..pos + 8].try_into().unwrap();
        let data_start = pos + 8;
        let data_end = data_start + length;
        let crc_end = data_end + 4;
        if crc_end > png.len() {
            break;
        }
        if chunk_type == *b"IDAT" {
            result.extend_from_slice(&png[data_start..data_end]);
        }
        pos = crc_end;
    }
    result
}
