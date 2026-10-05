/// Minimal decode loop for profiling (callgrind, perf, etc.)
///
/// Usage:
///   cargo build --release --example decode_only
///   valgrind --tool=callgrind target/release/examples/decode_only [image.png] [iterations]
///   perf record -g target/release/examples/decode_only image.png 200
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
    let config = zenpng::PngDecodeConfig::none();
    // Warmup
    let _ = zenpng::decode(&source, &config, &Unstoppable).unwrap();
    // Profile iterations (default 3; pass more for sampling profilers)
    let iters: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(3);
    for _ in 0..iters {
        let d = zenpng::decode(&source, &config, &Unstoppable).unwrap();
        std::hint::black_box(&d);
    }
}
