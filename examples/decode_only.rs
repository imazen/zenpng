/// Minimal decode loop for profiling (callgrind, perf, etc.)
///
/// Usage:
///   cargo build --release --example decode_only
///   valgrind --tool=callgrind target/release/examples/decode_only [image.png] [iterations]
///   perf record -g target/release/examples/decode_only image.png 200
///
/// `DECODE_WITH=png` runs image-rs/image-png main instead (the `png_main`
/// dev-dependency, configured as in `benches/pareto.rs`), for side-by-side
/// instruction counts.
use enough::Unstoppable;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/qoi-benchmark/screenshot_web/reddit.com.png",
            std::env::var("CODEC_CORPUS_DIR")
                .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string())
        )
    });
    let source = std::fs::read(&path).expect("read");
    // DECODE_THREADS: decoder max_threads (default 1, single-threaded).
    let threads = std::env::var("DECODE_THREADS").map_or(1, |t| t.parse().unwrap());
    let config = zenpng::PngDecodeConfig::none().with_max_threads(threads);
    let with_png = std::env::var("DECODE_WITH").is_ok_and(|w| w == "png");
    let decode = |data: &[u8]| -> usize {
        if with_png {
            let mut d = png_main::Decoder::new(std::io::Cursor::new(data));
            d.set_transformations(png_main::Transformations::EXPAND);
            d.ignore_checksums(true);
            let mut r = d.read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size().unwrap()];
            let info = r.next_frame(&mut buf).unwrap();
            std::hint::black_box(&buf);
            info.buffer_size()
        } else {
            let d = zenpng::decode(data, &config, &Unstoppable).unwrap();
            std::hint::black_box(&d);
            d.pixels.descriptor().bytes_per_pixel() * d.info.width as usize * d.info.height as usize
        }
    };
    // Warmup
    decode(&source);
    // Profile iterations (default 3; pass more for sampling profilers)
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let t = std::time::Instant::now();
    for _ in 0..iters {
        decode(&source);
    }
    eprintln!(
        "{:.2} us/decode ({iters} decodes, {threads} threads{})",
        t.elapsed().as_secs_f64() * 1e6 / iters as f64,
        if with_png { ", image-png" } else { "" },
    );
}
