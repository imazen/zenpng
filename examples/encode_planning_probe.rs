//! Allocation probe for effective-job planning; profile it with heaptrack
//! (`scripts/profile-encode-planning.py`). It has no timers, allocates no
//! source raster, reads no pixels and consumes no encoded output.
//!
//! `encode_planning_probe prepare|plan|begin 1|4 stored|prefiltered|strips|strips-parallel|buffered srgb|p3-icc|p3-descriptor`
//! retains every job (and, for `begin`, every started encoder) at once, so the
//! process peak includes all of them. After the stage it prints the planned or
//! started timings and the process thread count (heaptrack adds one thread).
use std::hint::black_box;

use zencodec::encode::{EncodeDeclaration, EncodeJob as _, Encoder as _, EncoderConfig as _};
use zencodec::output_patch::OutputAccess;
use zencodec::{ResourceLimits, ThreadingPolicy};
use zenpixels::{ColorContext, ColorPrimaries, PixelDescriptor};
use zenpng::{Compression, DowncastFlags, PngEncoderConfig};

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 2048;

/// Threads in this process, from `/proc/self/status`.
fn threads() -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .ok_or("no Threads line in /proc/self/status")?;
    Ok(line.trim().parse()?)
}

// zencodec's boxed planning error is `Send + Sync`; `?` cannot widen it into a
// plain `Box<dyn Error>`, so main returns the same envelope.
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, stage, count, config, colour] = args.as_slice() else {
        return Err("usage: encode_planning_probe prepare|plan|begin 1|4 \
                    stored|prefiltered|strips|strips-parallel|buffered \
                    srgb|p3-icc|p3-descriptor"
            .into());
    };
    let count: usize = match count.as_str() {
        "1" => 1,
        "4" => 4,
        _ => return Err("only counts 1 and 4 are measured".into()),
    };
    let effort = |e| PngEncoderConfig::new().with_compression(Compression::Effort(e));
    let (config, threading) = match config.as_str() {
        "stored" => (effort(0), ThreadingPolicy::Sequential),
        "prefiltered" => (effort(1), ThreadingPolicy::Sequential),
        "strips" => (
            effort(5).with_downcast(DowncastFlags::none()),
            ThreadingPolicy::Sequential,
        ),
        "strips-parallel" => (
            effort(5).with_downcast(DowncastFlags::none()),
            ThreadingPolicy::Parallel,
        ),
        "buffered" => (effort(19), ThreadingPolicy::Sequential),
        _ => {
            return Err("expected stored, prefiltered, strips, strips-parallel or buffered".into());
        }
    };
    let p3 = PixelDescriptor::RGB8_SRGB.with_primaries(ColorPrimaries::DisplayP3);
    let (descriptor, context) = match colour.as_str() {
        "srgb" => (PixelDescriptor::RGB8_SRGB, None),
        "p3-icc" => (
            p3,
            Some(ColorContext::from_icc(
                zenpixels_convert::icc_profiles::DISPLAY_P3_V4,
            )),
        ),
        "p3-descriptor" => (p3, None),
        _ => return Err("expected srgb, p3-icc or p3-descriptor".into()),
    };
    let declaration =
        EncodeDeclaration::new(WIDTH, HEIGHT, descriptor).with_access(OutputAccess::Forward);
    let declaration = match &context {
        Some(context) => declaration.with_color(context),
        None => declaration,
    };
    let limits = ResourceLimits::none().with_threading(threading);
    let mut jobs: Vec<_> = (0..count)
        .map(|_| config.clone().job().with_limits(limits))
        .collect();
    // Stage results live on the stack and are printed after the thread count
    // is read, so every stage ends with the same reporting allocations.
    let mut timings = [None; 4];
    let mut encoders = Vec::new();
    match stage.as_str() {
        "prepare" => {}
        "plan" => {
            for (job, timing) in jobs.iter().zip(&mut timings) {
                *timing = Some(job.plan_encode(&declaration)?.timing);
            }
        }
        "begin" => {
            encoders.reserve_exact(count);
            for (job, timing) in jobs.drain(..).zip(&mut timings) {
                let mut encoder = job.encoder()?;
                *timing = Some(encoder.begin_declared(&declaration)?.timing);
                encoders.push(encoder);
            }
        }
        _ => return Err("expected prepare, plan or begin".into()),
    }
    black_box((&jobs, &encoders, &timings));
    let threads = threads()?;
    for timing in timings.iter().flatten() {
        println!("timing {timing:?}");
    }
    println!("threads {threads}");
    Ok(())
}
