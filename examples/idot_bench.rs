//! Time `zenpng::decode` on `iDOT` PNGs: serial (`max_threads = 1`) against
//! automatic and fixed thread budgets, checking byte-identity each time.
//!
//! Core-tier pinning is chosen per process with `ZENPNG_PIN=off|workers|all`
//! (the `_dev` build honours it; the default is the library default).
//!
//! Usage:
//!   cargo run --release --example idot_bench --features _dev -- [--iters N] a.png [b.png ...]

use std::time::{Duration, Instant};

use zenflate::Unstoppable;
use zenpng::{PngDecodeConfig, decode};

fn median(mut v: Vec<Duration>) -> f64 {
    v.sort();
    v[v.len() / 2].as_secs_f64() * 1e3
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut iters = 15;
    if args.first().map(String::as_str) == Some("--iters") {
        iters = args[1].parse().unwrap();
        args.drain(..2);
    }
    let pin = std::env::var("ZENPNG_PIN").unwrap_or_else(|_| "default".into());
    println!("# pin={pin} iters={iters} (median ms)");
    println!("file\tmp\tserial\tauto\tx_auto\tt2\tt4\tt8\tidentical\tengaged");
    for path in &args {
        let data = std::fs::read(path).unwrap();
        let serial_cfg = PngDecodeConfig::default().with_max_threads(1);
        let reference = decode(&data, &serial_cfg, &Unstoppable).unwrap();
        let ref_bytes = reference.pixels.copy_to_contiguous_bytes();
        let mp = reference.info.width as f64 * reference.info.height as f64 / 1e6;
        let mut identical = true;
        let before = zenpng::__idot_stats();
        let mut run = |threads: usize| {
            let cfg = PngDecodeConfig::default().with_max_threads(threads);
            // Identity once, outside the timed loop (the 20 MB copy + compare
            // would otherwise change which pages the allocator hands back).
            let out = decode(&data, &cfg, &Unstoppable).unwrap();
            if out.pixels.copy_to_contiguous_bytes() != ref_bytes {
                identical = false;
            }
            drop(out);
            let mut times = Vec::with_capacity(iters);
            for _ in 0..iters {
                let t = Instant::now();
                let out = decode(&data, &cfg, &Unstoppable).unwrap();
                times.push(t.elapsed());
                drop(out);
            }
            median(times)
        };
        let (s, a, t2, t4, t8) = (run(1), run(0), run(2), run(4), run(8));
        let after = zenpng::__idot_stats();
        let name = std::path::Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy();
        println!(
            "{name}\t{mp:.2}\t{s:.2}\t{a:.2}\t{:.2}x\t{t2:.2}\t{t4:.2}\t{t8:.2}\t{identical}\t{}ok/{}fb",
            s / a,
            after.0 - before.0,
            after.1 - before.1
        );
    }
}
