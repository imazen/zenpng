//! Streaming encode output through zencodec's `Encoder::begin` /
//! `pending_output`. Effort 0 writes each row as it arrives, byte-identical
//! to the one-shot encode. The strip efforts write each ~512 KiB strip as its
//! own IDAT chunk once its last row arrives: the zlib stream equals the
//! non-streamed strip encode's, only split into one chunk per strip.
//! Configurations that need the whole image report `AT_FINISH`.

#![cfg(not(target_arch = "wasm32"))]

use enough::Unstoppable;
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig, OutputTiming};
use zencodec::{ResourceLimits, ThreadingPolicy};
use zenpixels::{PixelDescriptor, PixelSlice};
use zenpng::{Compression, DowncastFlags, PngDecodeConfig, PngDecoderConfig, PngEncoderConfig};

/// RGB8 noise over gradients: compresses, but not to nothing.
fn image(w: u32, h: u32) -> Vec<u8> {
    let mut x32: u32 = 0x2545_f491;
    (0..w as usize * h as usize)
        .flat_map(|i| {
            let (x, y) = (i % w as usize, i / w as usize);
            x32 ^= x32 << 13;
            x32 ^= x32 >> 17;
            x32 ^= x32 << 5;
            let n = if (y / 50) % 2 == 0 {
                (x32 & 15) as u8
            } else {
                0
            };
            [(x / 4) as u8 ^ n, (y / 4) as u8, ((x + y) / 8) as u8]
        })
        .collect()
}

/// Sequential, so strips are written as they complete (worker threads add
/// the strips in flight to the lag).
fn cfg(effort: u32) -> PngEncoderConfig {
    PngEncoderConfig::new()
        .with_compression(Compression::Effort(effort))
        .with_downcast(DowncastFlags::none())
        .with_threading(ThreadingPolicy::Sequential)
}

const RGB8: PixelDescriptor = PixelDescriptor::RGB8_SRGB;

fn one_shot(cfg: &PngEncoderConfig, px: &[u8], w: u32, h: u32) -> Vec<u8> {
    let s = PixelSlice::new(px, w, h, w as usize * 3, RGB8).unwrap();
    cfg.clone()
        .job()
        .encoder()
        .unwrap()
        .encode(s)
        .unwrap()
        .into_vec()
}

/// Push `strip` rows at a time; with `declare`, call `begin` first and drain
/// after every push. Returns the timing (if declared), the file, and the
/// bytes available after `begin` and after each push.
fn rows(
    cfg: &PngEncoderConfig,
    px: &[u8],
    (w, h): (u32, u32),
    strip: u32,
    declare: bool,
) -> (Option<OutputTiming>, Vec<u8>, Vec<(u32, usize)>) {
    let stride = w as usize * 3;
    let mut job = cfg.clone().job();
    if !declare {
        job = job.with_canvas_size(w, h);
    }
    let mut enc = job.encoder().unwrap();
    let timing = declare.then(|| enc.begin(w, h, RGB8).unwrap());
    let mut out = Vec::new();
    let mut seen = vec![(0, enc.pending_output().len())];
    let mut y = 0;
    loop {
        let n = enc.pending_output().len();
        out.extend_from_slice(enc.pending_output());
        enc.consume_output(n);
        if y == h {
            break;
        }
        let k = strip.min(h - y);
        let s = PixelSlice::new(
            &px[y as usize * stride..(y + k) as usize * stride],
            w,
            k,
            stride,
            RGB8,
        )
        .unwrap();
        enc.push_rows(s).unwrap();
        y += k;
        seen.push((y, out.len() + enc.pending_output().len()));
    }
    out.extend_from_slice(enc.finish().unwrap().data());
    (timing, out, seen)
}

fn decode(png: &[u8]) -> Vec<u8> {
    zenpng::decode(png, &PngDecodeConfig::strict(), &Unstoppable)
        .unwrap()
        .pixels
        .copy_to_contiguous_bytes()
}

/// The chunks of a PNG: (type, data).
fn chunks(png: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut p = 8;
    while p + 12 <= png.len() {
        let len = u32::from_be_bytes(png[p..p + 4].try_into().unwrap()) as usize;
        let ty: [u8; 4] = png[p + 4..p + 8].try_into().unwrap();
        out.push((ty, &png[p + 8..p + 8 + len]));
        p += 12 + len;
    }
    out
}

/// The zlib stream: every IDAT's data, concatenated.
fn zlib(png: &[u8]) -> Vec<u8> {
    chunks(png)
        .into_iter()
        .filter(|(t, _)| t == b"IDAT")
        .flat_map(|(_, d)| d.to_vec())
        .collect()
}

#[test]
fn effort_0_streams_each_row_byte_identical_to_one_shot() {
    let (w, h) = (97u32, 61u32);
    let px = image(w, h);
    let c = cfg(0);
    let want = one_shot(&c, &px, w, h);
    for strip in [1u32, 7, h] {
        let (timing, got, seen) = rows(&c, &px, (w, h), strip, true);
        assert_eq!(timing, Some(OutputTiming::streaming(0)));
        assert_eq!(got, want, "strip {strip}");
        assert!(
            seen.windows(2).all(|p| p[1].1 > p[0].1),
            "every push adds bytes"
        );
    }
    assert_eq!(decode(&want), px);
}

#[test]
fn strip_efforts_stream_one_idat_per_strip() {
    // 3,073,000 filtered bytes / 512 KiB: five strips of 200 rows.
    let (w, h) = (1024u32, 1000u32);
    let px = image(w, h);
    for effort in [1u32, 2, 5, 9, 13] {
        let c = cfg(effort);
        let (_, plain, _) = rows(&c, &px, (w, h), 16, false);
        let (timing, got, seen) = rows(&c, &px, (w, h), 16, true);
        let lag = timing.and_then(|t| t.lag_rows()).expect("streams");
        assert_eq!(lag, 199, "effort {effort}: the largest strip less one row");
        assert_eq!(decode(&got), px, "effort {effort}");
        // The same zlib stream, framed one chunk per strip.
        assert_eq!(zlib(&got), zlib(&plain), "effort {effort}");
        let idats = chunks(&got).iter().filter(|(t, _)| t == b"IDAT").count();
        assert_eq!(idats, 5, "effort {effort}");
        // Bytes appear once a strip completes, never before the first does.
        let first = seen
            .iter()
            .skip(1)
            .find(|&&(_, n)| n > seen[0].1)
            .unwrap()
            .0;
        assert!(
            first > 16 && first <= lag + 16 + 1,
            "effort {effort}: first after {first}"
        );
        // Drained in strips of 1 row, the file is the same.
        let (_, rowwise, _) = rows(&c, &px, (w, h), 1, true);
        assert_eq!(rowwise, got, "effort {effort}: depends on push size");
    }
}

#[test]
fn worker_threads_give_the_same_file_and_a_larger_lag() {
    let (w, h) = (1024u32, 1000u32);
    let px = image(w, h);
    let serial = cfg(5);
    let parallel = cfg(5)
        .with_threading(ThreadingPolicy::Parallel)
        .with_max_threads(4);
    let (_, a, _) = rows(&serial, &px, (w, h), 16, true);
    let (timing, b, _) = rows(&parallel, &px, (w, h), 16, true);
    assert_eq!(a, b, "worker threads change the bytes");
    // Strips in flight count too, capped at the image less one row.
    assert_eq!(timing.unwrap().lag_rows(), Some(h - 1));
}

#[test]
fn whole_image_configurations_report_at_finish() {
    let (w, h) = (1024u32, 1000u32);
    let px = image(w, h);
    let small = image(64, 48);
    for (c, (w, h), px, why) in [
        (
            PngEncoderConfig::new(),
            (w, h),
            &px,
            "default downcasts need the whole image",
        ),
        (cfg(20), (w, h), &px, "effort 20 searches the whole image"),
        (
            cfg(5),
            (64, 48),
            &small,
            "one strip is a whole-image search",
        ),
        (
            cfg(5).with_near_lossless_bits(2),
            (w, h),
            &px,
            "near-lossless",
        ),
    ] {
        let (timing, got, seen) = rows(&c, px, (w, h), 16, true);
        assert_eq!(timing, Some(OutputTiming::AT_FINISH), "{why}");
        assert!(seen.iter().all(|&(_, n)| n == 0), "{why}: nothing pending");
        assert_eq!(got, one_shot(&c, px, w, h), "{why}");
    }
}

#[test]
fn begin_checks_the_declaration() {
    let px = image(32, 32);
    let mut enc = cfg(0).job().with_canvas_size(16, 16).encoder().unwrap();
    assert!(enc.begin(32, 32, RGB8).is_err(), "canvas mismatch");
    let mut enc = cfg(0).job().encoder().unwrap();
    enc.begin(32, 32, RGB8).unwrap();
    let rgba = PixelSlice::new(&px[..32 * 4], 8, 4, 32, PixelDescriptor::RGBA8_SRGB).unwrap();
    assert!(enc.push_rows(rgba).is_err(), "other pixel format");
    let mut enc = cfg(0).job().encoder().unwrap();
    enc.push_rows(PixelSlice::new(&px[..96], 32, 1, 96, RGB8).unwrap())
        .unwrap();
    assert!(enc.begin(32, 32, RGB8).is_err(), "begin after rows");
}

#[test]
fn output_limit_counts_bytes_already_sent() {
    let (w, h) = (97u32, 61u32);
    let px = image(w, h);
    let total = one_shot(&cfg(0), &px, w, h).len() as u64;
    let run = |max: u64| {
        let mut enc = cfg(0)
            .job()
            .with_limits(ResourceLimits::none().with_max_output(max))
            .encoder()
            .unwrap();
        enc.begin(w, h, RGB8)?;
        for y in 0..h as usize {
            let n = enc.pending_output().len();
            enc.consume_output(n);
            enc.push_rows(PixelSlice::new(&px[y * 291..(y + 1) * 291], w, 1, 291, RGB8).unwrap())?;
        }
        enc.finish().map(|_| ())
    };
    assert!(run(total).is_ok());
    assert!(run(total - 1).is_err());
}

#[test]
fn testkit_streaming_output_honesty() {
    let img = zencodec_testkit::TestImage::rgb8_gradient(1024, 600);
    zencodec_testkit::check_streaming_output_honesty(cfg(0), PngDecoderConfig::new(), &img)
        .unwrap();
    zencodec_testkit::check_streaming_output_honesty(cfg(5), PngDecoderConfig::new(), &img)
        .unwrap();
    zencodec_testkit::check_streaming_output_honesty(
        PngEncoderConfig::new(),
        PngDecoderConfig::new(),
        &img,
    )
    .unwrap();
}
