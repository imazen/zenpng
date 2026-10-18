//! coz causal-profiler workload for zenpng (dev-only harness).
//!
//! Subcommands:
//!   coz_harness decode [--st] [--mt N] <iters> <png-or-dir>...
//!   coz_harness encode [--st] [--mt N] <effort> <iters> <png-or-dir>...
//!   coz_harness hash <effort> <png-or-dir>...
//!
//! `--mt N` runs N worker threads each completing whole units so coz's
//! pause-other-threads virtual-speedup mechanism can engage (a lone thread
//! produces flat experiment curves). Each unit still runs single-threaded:
//! decode uses max_threads=1 and encode uses parallel=false.
//!
//! `decode`/`encode` loop whole-image operations with one `coz::progress!()`
//! per completed unit, so coz's virtual-speedup experiments are credited only
//! when real units finish faster. The coz calls exist only on Linux (the only
//! OS coz profiles); every subcommand runs the same workload elsewhere.
//!
//! `hash` prints sha256 of decoded pixels+info and of encode output bytes per
//! input, and cross-checks every decode, and every re-encode, against
//! image-png (png_main) with a strict oracle: dimensions, channel count,
//! sample depth, then samples in PNG stream byte order (zenpng's native-endian
//! 16-bit samples are converted explicitly for the target's endianness). It is
//! the pre/post correctness probe for optimization work. Every decoded layout
//! is re-encoded; gray+alpha goes through the zencodec encoder, which has no
//! native `encode_*` entry but reaches the same writer.

use std::path::{Path, PathBuf};

use enough::Unstoppable;
use sha2::{Digest, Sha256};
use zencodec::encode::{EncodeJob as _, Encoder as _, EncoderConfig as _};
use zenpixels::{ChannelType, PixelSlice};
use zenpng::{Compression, EncodeConfig, PngDecodeConfig, PngDecodeOutput, PngEncoderConfig};

fn collect_pngs(args: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for a in args {
        let p = PathBuf::from(a);
        if p.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&p)
                .unwrap_or_else(|e| panic!("read_dir {}: {e}", p.display()))
                .filter_map(|e| {
                    let q = e.expect("read PNG directory entry").path();
                    (q.extension().is_some_and(|x| x == "png")).then_some(q)
                })
                .collect();
            entries.sort();
            out.extend(entries);
        } else {
            out.push(p);
        }
    }
    assert!(!out.is_empty(), "no PNG inputs");
    out
}

fn load(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn decode_one(data: &[u8], single_thread: bool) -> PngDecodeOutput {
    let cfg = if single_thread {
        PngDecodeConfig::default().with_max_threads(1)
    } else {
        PngDecodeConfig::default()
    };
    zenpng::decode(data, &cfg, &Unstoppable).unwrap_or_else(|e| panic!("zenpng decode failed: {e}"))
}

fn encode_one(out: &PngDecodeOutput, effort: u32) -> Vec<u8> {
    encode_one_cfg(out, effort, false)
}

fn encode_one_cfg(out: &PngDecodeOutput, effort: u32, single_thread: bool) -> Vec<u8> {
    let cfg = EncodeConfig::default()
        .with_compression(Compression::Effort(effort))
        .with_parallel(!single_thread);
    let px = &out.pixels;
    let encoded = if let Some(img) = px.try_as_imgref::<rgb::Rgb<u8>>() {
        zenpng::encode_rgb8(img, None, &cfg, &Unstoppable, &Unstoppable)
    } else if let Some(img) = px.try_as_imgref::<rgb::Rgba<u8>>() {
        zenpng::encode_rgba8(img, None, &cfg, &Unstoppable, &Unstoppable)
    } else if let Some(img) = px.try_as_imgref::<rgb::Gray<u8>>() {
        zenpng::encode_gray8(img, None, &cfg, &Unstoppable, &Unstoppable)
    } else if let Some(img) = px.try_as_imgref::<rgb::Rgb<u16>>() {
        zenpng::encode_rgb16(img, None, &cfg, &Unstoppable, &Unstoppable)
    } else if let Some(img) = px.try_as_imgref::<rgb::Rgba<u16>>() {
        zenpng::encode_rgba16(img, None, &cfg, &Unstoppable, &Unstoppable)
    } else if let Some(img) = px.try_as_imgref::<rgb::Gray<u16>>() {
        zenpng::encode_gray16(img, None, &cfg, &Unstoppable, &Unstoppable)
    } else {
        return encode_gray_alpha(out, effort, single_thread);
    };
    encoded.unwrap_or_else(|e| panic!("zenpng encode failed: {e}"))
}

/// Gray+alpha (8 or 16-bit) through the zencodec encoder, with the decoded
/// descriptor and the same compression, threading and default downcasts.
fn encode_gray_alpha(out: &PngDecodeOutput, effort: u32, single_thread: bool) -> Vec<u8> {
    let (w, h) = (out.info.width, out.info.height);
    let desc = out.pixels.descriptor();
    assert!(
        desc.channels() == 2 && matches!(desc.channel_type(), ChannelType::U8 | ChannelType::U16),
        "unsupported decoded layout for re-encode: {w}x{h} {desc:?}"
    );
    let bytes = out.pixels.copy_to_contiguous_bytes();
    let stride = w as usize * desc.bytes_per_pixel();
    let pixels = PixelSlice::new(&bytes, w, h, stride, desc)
        .unwrap_or_else(|e| panic!("gray+alpha pixel slice: {e}"));
    let mut cfg = PngEncoderConfig::new().with_compression(Compression::Effort(effort));
    if single_thread {
        cfg = cfg.with_max_threads(1);
    }
    cfg.job()
        .encoder()
        .and_then(|e| e.encode(pixels))
        .unwrap_or_else(|e| panic!("zenpng gray+alpha encode failed: {e}"))
        .data()
        .to_vec()
}

/// Decoded samples plus the layout they claim. 16-bit samples are held in
/// PNG stream order (big-endian) so two decoders compare byte for byte.
struct Samples {
    width: u32,
    height: u32,
    channels: usize,
    bits: u8,
    stream_order: Vec<u8>,
}

/// zenpng's decode as [`Samples`]. zenpng hands out 16-bit samples in native
/// byte order; the conversion to stream order depends only on the target's
/// endianness, so a decoder that skipped or doubled the swap cannot match.
fn zenpng_samples(out: &PngDecodeOutput) -> Samples {
    let desc = out.pixels.descriptor();
    let bits = match desc.channel_type() {
        ChannelType::U8 => 8,
        ChannelType::U16 => 16,
        other => panic!("PNG decode produced {other:?} samples"),
    };
    let mut stream_order = out.pixels.copy_to_contiguous_bytes();
    if bits == 16 && cfg!(target_endian = "little") {
        for s in stream_order.as_chunks_mut::<2>().0 {
            s.swap(0, 1);
        }
    }
    Samples {
        width: out.info.width,
        height: out.info.height,
        channels: desc.channels(),
        bits,
        stream_order,
    }
}

/// Third-party oracle: decode with image-png main (EXPAND, no checksums).
/// Also returns the file's own IHDR color type and bit depth.
fn png_main_decode(data: &[u8]) -> (Samples, u8, u8) {
    let mut d = png_main::Decoder::new(std::io::Cursor::new(data));
    d.set_transformations(png_main::Transformations::EXPAND);
    d.ignore_checksums(true);
    let mut r = d.read_info().unwrap();
    let (file_ct, file_bd) = (r.info().color_type as u8, r.info().bit_depth as u8);
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let channels = match info.color_type {
        png_main::ColorType::Grayscale => 1,
        png_main::ColorType::GrayscaleAlpha => 2,
        png_main::ColorType::Rgb => 3,
        png_main::ColorType::Rgba => 4,
        png_main::ColorType::Indexed => panic!("EXPAND left indexed output"),
    };
    let samples = Samples {
        width: info.width,
        height: info.height,
        channels,
        bits: info.bit_depth as u8,
        stream_order: buf,
    };
    (samples, file_ct, file_bd)
}

/// Strict oracle: dimensions, channel count, sample depth, then every sample
/// byte in stream order, against `theirs` mapped to the layout zenpng's
/// decoder documents (src/decoder/postprocess.rs): every layout as image-png
/// expands it, except 8-bit gray+alpha (gray+alpha, or gray with tRNS), which
/// zenpng returns as RGBA8. Returns the mapping applied or the first mismatch.
fn oracle(theirs: &Samples, ours: &Samples) -> Result<Option<&'static str>, &'static str> {
    let mapped;
    let (theirs, rule) = if theirs.channels == 2 && theirs.bits == 8 {
        mapped = Samples {
            width: theirs.width,
            height: theirs.height,
            channels: 4,
            bits: 8,
            stream_order: theirs
                .stream_order
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|&[g, a]| [g, g, g, a])
                .collect(),
        };
        (&mapped, Some("ga8->rgba8"))
    } else {
        (theirs, None)
    };
    if (theirs.width, theirs.height) != (ours.width, ours.height) {
        return Err("shape");
    }
    if theirs.channels != ours.channels {
        return Err("channels");
    }
    if theirs.bits != ours.bits {
        return Err("depth");
    }
    if theirs.stream_order != ours.stream_order {
        return Err("samples");
    }
    Ok(rule)
}

fn verdict(r: Result<Option<&str>, &str>) -> String {
    match r {
        Ok(None) => "agree".into(),
        Ok(Some(rule)) => format!("agree({rule})"),
        Err(why) => format!("DISAGREE({why})"),
    }
}

/// Whether 16-bit samples can expose a byte-order error at all: when every
/// sample is `v * 257` both byte orders read the same.
fn order_discriminating(s: &Samples) -> bool {
    s.bits == 16
        && s.stream_order
            .as_chunks::<2>()
            .0
            .iter()
            .any(|p| p[0] != p[1])
}

/// Every pixel widened to RGBA16 by PNG's own value rules (gray replicated,
/// a missing alpha opaque, 8-bit samples times 257), so a lossless layout
/// change by the encoder (a 16-to-8-bit, gray or alpha downcast) compares
/// equal here, and only here.
fn canonical_rgba16(s: &Samples) -> Vec<[u16; 4]> {
    let sample = |i: usize| -> u16 {
        if s.bits == 16 {
            u16::from_be_bytes([s.stream_order[2 * i], s.stream_order[2 * i + 1]])
        } else {
            u16::from(s.stream_order[i]) * 257
        }
    };
    (0..s.width as usize * s.height as usize)
        .map(|p| {
            let b = p * s.channels;
            match s.channels {
                1 => [sample(b), sample(b), sample(b), u16::MAX],
                2 => [sample(b), sample(b), sample(b), sample(b + 1)],
                3 => [sample(b), sample(b + 1), sample(b + 2), u16::MAX],
                4 => [sample(b), sample(b + 1), sample(b + 2), sample(b + 3)],
                n => panic!("{n}-channel PNG samples"),
            }
        })
        .collect()
}

fn full_sha(bytes: &[u8]) -> String {
    let h = Sha256::digest(bytes);
    h.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn cmd_decode(single_thread: bool, workers: usize, iters: u32, files: Vec<PathBuf>) {
    let inputs: Vec<(PathBuf, Vec<u8>)> =
        files.into_iter().map(|p| (p.clone(), load(&p))).collect();
    // Warmup + first-error surfacing outside the profile loop.
    for (p, d) in &inputs {
        let _ = decode_one(d, single_thread);
        eprintln!("warm {}", p.display());
    }
    let inputs = std::sync::Arc::new(inputs);
    let single_thread = single_thread || workers > 1;
    std::thread::scope(|s| {
        for _ in 0..workers {
            let inputs = &inputs;
            s.spawn(move || {
                #[cfg(target_os = "linux")]
                coz::thread_init();
                for _ in 0..iters {
                    for (_, d) in inputs.iter() {
                        let out = decode_one(d, single_thread);
                        std::hint::black_box(&out);
                        #[cfg(target_os = "linux")]
                        coz::progress!("decode_unit");
                    }
                }
            });
        }
    });
}

fn cmd_encode(single_thread: bool, workers: usize, effort: u32, iters: u32, files: Vec<PathBuf>) {
    let inputs: Vec<(PathBuf, PngDecodeOutput)> = files
        .into_iter()
        .map(|p| (p.clone(), decode_one(&load(&p), true)))
        .collect();
    for (p, _) in &inputs {
        eprintln!("warm {}", p.display());
    }
    let inputs = std::sync::Arc::new(inputs);
    let single_thread = single_thread || workers > 1;
    std::thread::scope(|s| {
        for _ in 0..workers {
            let inputs = &inputs;
            s.spawn(move || {
                #[cfg(target_os = "linux")]
                coz::thread_init();
                for _ in 0..iters {
                    for (_, out) in inputs.iter() {
                        let enc = encode_one_cfg(out, effort, single_thread);
                        std::hint::black_box(&enc);
                        #[cfg(target_os = "linux")]
                        coz::progress!("encode_unit");
                    }
                }
            });
        }
    });
}

fn cmd_hash(effort: u32, files: Vec<PathBuf>) {
    for p in files {
        let data = load(&p);
        let out = decode_one(&data, true);
        let px = out.pixels.copy_to_contiguous_bytes();
        let mut h = Sha256::new();
        h.update(&px);
        h.update(out.info.width.to_le_bytes());
        h.update(out.info.height.to_le_bytes());
        h.update([out.info.bit_depth, out.info.color_type]);
        let dhash = full_sha(&h.finalize());
        let ours = zenpng_samples(&out);
        let (theirs, _, _) = png_main_decode(&data);
        let mut line = format!(
            "HASH {} decode={} {}x{} ct={} bd={} out={}x{} oracle={}",
            p.display(),
            dhash,
            out.info.width,
            out.info.height,
            out.info.color_type,
            out.info.bit_depth,
            ours.channels,
            ours.bits,
            verdict(oracle(&theirs, &ours))
        );
        if ours.bits == 16 {
            line.push_str(if order_discriminating(&ours) {
                " order_check=yes"
            } else {
                " order_check=no(every_sample_v*257)"
            });
        }
        // Re-encode the decoded pixels, byte-hash the output, and decode it
        // with image-png. Three diagnostics, all reported, none rewriting
        // another: `rt_raw` is the strict oracle against our decode of the
        // input; `rt_value` compares canonical RGBA16 values (equal across a
        // lossless downcast to fewer channels or 8 bits); `rt_visible` also
        // ignores RGB under alpha == 0 (zenpng zeroes it in RGBA8 by default).
        let enc = encode_one(&out, effort);
        let (rt, file_ct, file_bd) = png_main_decode(&enc);
        let (want, got) = (canonical_rgba16(&ours), canonical_rgba16(&rt));
        let rt_value = if want == got { "agree" } else { "DISAGREE" };
        let rt_visible = if want.len() == got.len()
            && want
                .iter()
                .zip(&got)
                .all(|(o, t)| o[3] == t[3] && (o[3] == 0 || o[..3] == t[..3]))
        {
            "agree"
        } else {
            "DISAGREE"
        };
        line.push_str(&format!(
            " encode_e{effort}={} {}B file_ct={file_ct} file_bd={file_bd} rt_raw={} rt_value={rt_value} rt_visible={rt_visible}",
            full_sha(&enc),
            enc.len(),
            verdict(oracle(&rt, &ours))
        ));
        println!("{line}");
    }
}

fn write_new_result(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .expect("create new result.bin");
    file.write_all(bytes).expect("write result.bin");
    file.sync_all().expect("sync result.bin");
}

/// zenbench-cli `pair` driver: fixed single-threaded codec workload, one
/// `elapsed_ms=` metric line, deterministic `result.bin` for the pair
/// files-check. Full outputs and work counts are hashed outside the timer;
/// the timer covers codec calls, their allocations and output destruction.
/// The result path must be new. This worker alone does not establish parity;
/// the external comparison must check both result files.
///   coz_harness pair enc <effort> <iters> <out.bin> <png...>
///   coz_harness pair dec <iters> <out.bin> <png...>
fn cmd_pair(args: &[String]) {
    let enc_mode = args[1] == "enc";
    let (effort, iters, out_bin, files): (u32, u32, PathBuf, Vec<PathBuf>) = if enc_mode {
        (
            args[2].parse().expect("effort"),
            args[3].parse().expect("iters"),
            PathBuf::from(&args[4]),
            collect_pngs(&args[5..]),
        )
    } else if args[1] == "dec" {
        (
            0u32,
            args[2].parse().expect("iters"),
            PathBuf::from(&args[3]),
            collect_pngs(&args[4..]),
        )
    } else {
        panic!("pair mode must be enc|dec");
    };
    assert!(iters > 0, "pair iterations must be positive");
    let inputs: Vec<Vec<u8>> = files.iter().map(|p| load(p)).collect();
    if enc_mode {
        let decoded: Vec<PngDecodeOutput> = inputs.iter().map(|d| decode_one(d, true)).collect();
        // Two untimed warmup passes before the measured loop.
        for _ in 0..2 {
            for out in &decoded {
                std::hint::black_box(encode_one_cfg(out, effort, true));
            }
        }
        let mut h = Sha256::new();
        h.update(iters.to_le_bytes());
        for out in &decoded {
            let encoded = encode_one_cfg(out, effort, true);
            h.update((encoded.len() as u64).to_le_bytes());
            h.update(&encoded);
        }
        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            for out in &decoded {
                std::hint::black_box(encode_one_cfg(out, effort, true));
            }
        }
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        write_new_result(&out_bin, &h.finalize());
        println!("elapsed_ms={elapsed_ms:.3}");
    } else {
        for _ in 0..2 {
            for d in &inputs {
                std::hint::black_box(decode_one(d, true));
            }
        }
        let mut h = Sha256::new();
        h.update(iters.to_le_bytes());
        for data in &inputs {
            let out = decode_one(data, true);
            h.update(out.info.width.to_le_bytes());
            h.update(out.info.height.to_le_bytes());
            h.update([out.info.color_type, out.info.bit_depth]);
            let pixels = out.pixels.copy_to_contiguous_bytes();
            h.update((pixels.len() as u64).to_le_bytes());
            h.update(&pixels);
        }
        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            for data in &inputs {
                std::hint::black_box(decode_one(data, true));
            }
        }
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        write_new_result(&out_bin, &h.finalize());
        println!("elapsed_ms={elapsed_ms:.3}");
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let single_thread = if let Some(pos) = args.iter().position(|a| a == "--st") {
        args.remove(pos);
        true
    } else {
        false
    };
    let workers = if let Some(pos) = args.iter().position(|a| a == "--mt") {
        args.remove(pos);
        args.remove(pos).parse().expect("--mt N")
    } else {
        1usize
    };
    let usage = "usage: coz_harness decode [--st] [--mt N] <iters> <png...> | encode [--st] [--mt N] <effort> <iters> <png...> | hash <effort> <png...> | pair enc <effort> <iters> <out.bin> <png...> | pair dec <iters> <out.bin> <png...>";
    let cmd = args.first().map(String::as_str).expect(usage);
    match cmd {
        "decode" => {
            let iters: u32 = args[1].parse().expect("iters");
            cmd_decode(single_thread, workers, iters, collect_pngs(&args[2..]));
        }
        "encode" => {
            let effort: u32 = args[1].parse().expect("effort");
            let iters: u32 = args[2].parse().expect("iters");
            cmd_encode(
                single_thread,
                workers,
                effort,
                iters,
                collect_pngs(&args[3..]),
            );
        }
        "hash" => {
            let effort: u32 = args[1].parse().expect("effort");
            cmd_hash(effort, collect_pngs(&args[2..]));
        }
        "pair" => cmd_pair(&args),
        _ => panic!("{usage}"),
    }
}
