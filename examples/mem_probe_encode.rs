#![forbid(unsafe_code)]
//! Encode peak-memory probe — one PNG encode, report measured peak RSS (VmHWM).
//!
//! The ENCODE counterpart to the decode-side `mem_probe` examples. Used by the
//! heaptrack / VmHWM sweep to calibrate the encode peak-memory model
//! (`heuristics::estimate_encode`, surfaced as `estimate_encode_resources`)
//! against measured reality, *per effort level*. PNG's dominant cost knob is the
//! DEFLATE compression effort, which governs BOTH time and the encoder's working
//! set over an enormous range (more filter strategies + brute-force / Zopfli /
//! FullOptimal search buffers at high effort), so the sweep must hit several
//! effort levels.
//!
//!   cargo build -p zenpng --release --example mem_probe_encode
//!   GLIBC_TUNABLES=glibc.malloc.mmap_threshold=131072 \
//!     ./target/release/examples/mem_probe_encode <rgb8.bin> <w> <h> png <effort> <quality>
//!   heaptrack ./target/release/examples/mem_probe_encode ...   # allocator peak heap
//!
//! One encode per process — peak RSS is a per-process high-water mark, so the
//! input must come from a cheap file read (raw RGB8 bin), never an in-process
//! decode (whose own peak would pollute VmHWM above the encode peak).
//!
//! `<effort>` is the PNG compression effort (`Compression::effort()`, range
//! 0..=200 — VERIFY: `Compression::Effort(e)` clamps to 200, src/types.rs:100).
//! The memory model was calibrated over efforts {1,6,13,19,24,(27),30}; sweep
//! those. Effort 31+ (`Brag`/`Minutes`) engages FullOptimal recompression and is
//! slower/heavier still — and only uses zenzop with `--features zopfli` (off by
//! default → silent FullOptimal fallback), so 31+ numbers depend on that feature.
//!
//! `<quality>` is accepted for TSV-shape parity with the other codecs' probes
//! but PNG is lossless: it is recorded in the row and otherwise ignored.
//!
//! TSV row:
//!   w  h  pixels  mode  effort  quality
//!   out_bytes  pre_rss_kb  vmhwm_kb  marginal_kb

use enough::Unstoppable;
use imgref::Img;
use rgb::{Rgb, Rgba};
use std::hint::black_box;
use zenpng::{Compression, EncodeConfig};

/// A `/proc/self/status` field in KiB (e.g. `VmRSS:`, `VmHWM:`).
fn status_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 7 {
        eprintln!(
            "usage: mem_probe_encode <rgb8.bin> <w> <h> <png|rgba|gray|rgb16|rgba16> <effort 0..200> <quality> [est] [threads]"
        );
        std::process::exit(2);
    }
    let path = &a[1];
    let w: u32 = a[2].parse().expect("w");
    let h: u32 = a[3].parse().expect("h");
    // `png` (== rgb8) is the default truecolor mode. The other modes synthesize
    // a wider input from the same RGB8 bytes so the encoder exercises the alpha /
    // 16-bit working-set strata (matches `png_probe.rs`'s input_bpp choices).
    let mode = match a[4].as_str() {
        "png" | "rgb" | "rgb8" => "png",
        "rgba" | "rgba8" => "rgba",
        "rgb16" => "rgb16",
        "rgba16" => "rgba16",
        // VERIFY: there's an encode_gray8, but the .bin is RGB8 so "gray" would
        // need an R==G==B source; left out to avoid a misleading stratum.
        other => panic!("mode must be png|rgba|rgb16|rgba16, got {other}"),
    };
    let effort: u32 = a[5].parse().expect("effort");
    let quality: f32 = a[6].parse().expect("quality");

    // Optional 7th arg = "est" (estimate-only) OR a thread count. Optional 8th
    // arg = thread count when the 7th is "est". Default: single-thread (matches
    // the calibration; `with_parallel(false)` so wall ≈ user CPU).
    let arg7 = a.get(7).map(String::as_str);
    let est_mode = arg7 == Some("est");
    let threads: usize = if est_mode {
        a.get(8).and_then(|s| s.parse().ok()).unwrap_or(1)
    } else {
        arg7.and_then(|s| s.parse().ok()).unwrap_or(1)
    };

    let data = std::fs::read(path).expect("read rgb8.bin");
    assert_eq!(
        data.len(),
        (w as usize) * (h as usize) * 3,
        "bin size {} != w*h*3 {}",
        data.len(),
        (w as usize) * (h as usize) * 3
    );

    // input_bpp: the stratum the cost model keys on (3=RGB8, 4=RGBA8, 6=RGB16,
    // 8=RGBA16). Alpha (4/8) and 16-bit (6/8) each cost extra working set.
    let input_bpp: u8 = match mode {
        "rgba" => 4,
        "rgb16" => 6,
        "rgba16" => 8,
        _ => 3,
    };

    // Estimate-only mode (`est` as the 7th arg): print what the CURRENT model
    // predicts for this cell (typical / min / max peak + time), no encode — so
    // we can compare model vs measured without an encode polluting anything.
    if est_mode {
        let pixels = (w as u64) * (h as u64);
        match zenpng::heuristics::estimate_encode(w, h, input_bpp, effort) {
            Some(e) => println!(
                "{w}\t{h}\t{pixels}\t{mode}\t{effort}\t{quality}\tEST\ttyp_kb={}\tmin_kb={}\tmax_kb={}\ttime_ms={:.1}\ttyp_bpp={:.2}\tthreads={threads}",
                e.peak_memory_bytes / 1024,
                e.peak_memory_bytes_min / 1024,
                e.peak_memory_bytes_max / 1024,
                e.time_ms,
                e.peak_memory_bytes as f64 / pixels as f64,
            ),
            None => println!(
                "{w}\t{h}\t{pixels}\t{mode}\t{effort}\t{quality}\tEST\toverflow",
            ),
        }
        return;
    }

    let mut cfg = EncodeConfig::default()
        .with_compression(Compression::Effort(effort))
        .with_parallel(threads > 1);
    // max_threads: 0 = no cap, 1 = fully serial, N>1 = cap. Drive it explicitly
    // so the probe controls the strategy-parallelism that grows peak working set.
    cfg.max_threads = threads;

    // Baseline RSS: process + libs + the input `data` we hold + the typed pixel
    // buffer we build below. Marginal = VmHWM − pre isolates the encode's own
    // working set (what the model predicts). We build the typed buffer BEFORE
    // sampling `pre` so its allocation is part of the baseline, not the encode
    // delta — the cost model excludes the caller-owned input buffer.
    let encoded = match mode {
        "rgba" => {
            // Synthesize a high-entropy alpha (= green channel) so the alpha
            // path isn't trivially all-opaque (which the encoder downcasts away).
            let px: Vec<Rgba<u8>> = data
                .chunks_exact(3)
                .map(|p| Rgba {
                    r: p[0],
                    g: p[1],
                    b: p[2],
                    a: p[1],
                })
                .collect();
            let pre = status_kb("VmRSS:");
            let out = zenpng::encode_rgba8(
                Img::new(px, w as usize, h as usize).as_ref(),
                None,
                &cfg,
                &Unstoppable,
                &Unstoppable,
            );
            (pre, out)
        }
        "rgb16" => {
            // Widen 8→16-bit by bit-replication (v*0x0101). VERIFY: the default
            // `downcast_16_to_8_replicated` flag would collapse this straight
            // back to 8-bit, defeating the 16-bit stratum — disable it so the
            // probe actually exercises the 16-bit working set.
            let px: Vec<Rgb<u16>> = data
                .chunks_exact(3)
                .map(|p| {
                    let w16 = |v: u8| ((v as u16) << 8) | v as u16;
                    Rgb {
                        r: w16(p[0]),
                        g: w16(p[1]),
                        b: w16(p[2]),
                    }
                })
                .collect();
            cfg = cfg.with_downcast(zenpng::DowncastFlags::none());
            let pre = status_kb("VmRSS:");
            let out = zenpng::encode_rgb16(
                Img::new(px, w as usize, h as usize).as_ref(),
                None,
                &cfg,
                &Unstoppable,
                &Unstoppable,
            );
            (pre, out)
        }
        "rgba16" => {
            let px: Vec<Rgba<u16>> = data
                .chunks_exact(3)
                .map(|p| {
                    let w16 = |v: u8| ((v as u16) << 8) | v as u16;
                    Rgba {
                        r: w16(p[0]),
                        g: w16(p[1]),
                        b: w16(p[2]),
                        a: w16(p[1]),
                    }
                })
                .collect();
            cfg = cfg.with_downcast(zenpng::DowncastFlags::none());
            let pre = status_kb("VmRSS:");
            let out = zenpng::encode_rgba16(
                Img::new(px, w as usize, h as usize).as_ref(),
                None,
                &cfg,
                &Unstoppable,
                &Unstoppable,
            );
            (pre, out)
        }
        _ => {
            let px: Vec<Rgb<u8>> = data
                .chunks_exact(3)
                .map(|p| Rgb {
                    r: p[0],
                    g: p[1],
                    b: p[2],
                })
                .collect();
            let pre = status_kb("VmRSS:");
            let out = zenpng::encode_rgb8(
                Img::new(px, w as usize, h as usize).as_ref(),
                None,
                &cfg,
                &Unstoppable,
                &Unstoppable,
            );
            (pre, out)
        }
    };
    let (pre, out) = encoded;
    let out = out.expect("encode failed");

    // High-water mark immediately after finish — VmHWM is monotonic, so it
    // reflects the peak *during* the encode.
    let peak = status_kb("VmHWM:");

    let pixels = (w as u64) * (h as u64);
    println!(
        "{w}\t{h}\t{pixels}\t{mode}\t{effort}\t{quality}\t{}\t{pre}\t{peak}\t{}",
        out.len(),
        peak.saturating_sub(pre)
    );
    black_box(&out);
}
