/// Minimal decode loop for profiling (callgrind, perf, etc.)
///
/// Usage:
///   cargo build --release --example decode_only
///   valgrind --tool=callgrind target/release/examples/decode_only [image.png]
use enough::Unstoppable;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/home/lilith/work/codec-corpus/qoi-benchmark/screenshot_web/reddit.com.png".to_string()
    });
    let source = std::fs::read(&path).expect("read");
    let config = zenpng::PngDecodeConfig::none();
    // Warmup
    let _ = zenpng::decode(&source, &config, &Unstoppable).unwrap();
    // Profile iterations
    for _ in 0..3 {
        let d = zenpng::decode(&source, &config, &Unstoppable).unwrap();
        std::hint::black_box(&d);
    }
}
