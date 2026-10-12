//! Speed/size Pareto harness: zenpng against other PNG codecs, encode and
//! decode, single- and multi-threaded.
//!
//! Inputs: `ZENPNG_PARETO_DIR`, PNGs named `<id>_<format>_<longedge>.png`
//! (`scripts/vs_png_inputs.sh`). Every arm's output size is printed once,
//! untimed, as `SIZE\t<group>\t<arm>\t<bytes>` on stderr; times come from
//! the zenbench result file. `scripts/pareto_report.py` joins both into a TSV,
//! fits `ms = a + b·px` and `bytes = a + b·px` per arm, and lists every
//! zenpng arm that another codec's arm beats on both axes.
//!
//! Encode (RGB8 and RGBA8 inputs), group `enc/<name>`:
//! - `zenpng_e<E>_st`: `Compression::Effort(E)`, one thread
//!   (`ZENPNG_PARETO_EFFORTS`, default `1,2,3,5,7,9,13,19`; efforts above 13
//!   are skipped for long edges over 2560 unless `ZENPNG_PARETO_BIG=1`).
//! - `zenpng_e<E>_mt`: the same with `with_parallel(true)`
//!   (`ZENPNG_PARETO_MT_EFFORTS`, default `7,13,19`).
//! - `zenpng_e7_idot8_mt`: effort 7, parallel, `with_decode_segments(8)`
//!   (off with `ZENPNG_PARETO_NO_IDOT=1`).
//! - With the `_dev` feature, `ZENPNG_LADDER_E<n>` redefines effort n (see
//!   `EffortParams::dev_override`), so candidate ladders run without rebuilds.
//! - `png_fast` / `png_balanced` / `png_high`: image-rs/image-png main.
//! - `zune`: zune-png's encoder (default options). `lodepng`: lodepng
//!   defaults.
//!
//! Decode, group `dec/<name>` (the libpng-written input) and
//! `dec_idot/<name>` (zenpng effort 7 with 8 iDOT strips, RGB8/RGBA8 only):
//! `zenpng_st` (`max_threads(1)`), `zenpng_mt` (`max_threads(0)`, only
//! differs on iDOT files), `png`, `zune`, `lodepng`. No arm verifies CRC or
//! Adler-32; all expand palette and sub-byte gray to 8 bits. An untimed
//! check makes every decoder agree on 8-bit output first.
//!
//! Streaming vs whole-image (opt-in: only with `--group=sdec` / `--group=senc`):
//! - `sdec/<name>`: `whole_st` / `whole_mt` (`zenpng::decode`, 1 / all
//!   threads), `push` (`DecodeJob::push_decoder` into a `Vec` sink),
//!   `stream` (`streaming_decoder`, each row copied into one `Vec`), and
//!   `png_rows` (image-png main's `next_row` loop, same copy) as the
//!   reference row-streaming decoder.
//! - `senc/<name>` (RGB8/RGBA8): per effort in `ZENPNG_PARETO_STREAM_EFFORTS`
//!   (default `0,1,2,7,13`), `e<E>_whole` (zencodec `Encoder::encode`) and
//!   `e<E>_push16` (`push_rows` in 16-row strips after `with_canvas_size`).
//!
//! - `pdec/<name>`: decode threading per API and `iDOT` (see
//!   `bench_pipeline_decode`).
//!
//! Run: `ZENPNG_PARETO_DIR=... cargo bench --bench pareto -- [--group=enc|dec|sdec|senc|pdec]`

use std::path::PathBuf;

use zenbench::prelude::*;
use zenflate::Unstoppable;

fn env_list(name: &str, default: &[u32]) -> Vec<u32> {
    // Unset: the default list. Set but empty: no arms.
    std::env::var(name).map_or_else(
        |_| default.to_vec(),
        |v| {
            v.split(',')
                .filter(|e| !e.trim().is_empty())
                .map(|e| e.trim().parse().expect("effort"))
                .collect()
        },
    )
}

fn inputs() -> Vec<(String, u32, &'static [u8])> {
    let dir = PathBuf::from(
        std::env::var("ZENPNG_PARETO_DIR")
            .expect("set ZENPNG_PARETO_DIR to the input PNG directory"),
    );
    let mut v: Vec<(String, u32, &'static [u8])> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "png").then(|| {
                let name = p.file_stem().unwrap().to_string_lossy().to_string();
                let edge = name
                    .rsplit('_')
                    .next()
                    .and_then(|e| e.parse().ok())
                    .unwrap_or(0);
                let data: &'static [u8] = Box::leak(std::fs::read(&p).unwrap().into_boxed_slice());
                (name, edge, data)
            })
        })
        .collect();
    v.sort();
    v
}

fn leak<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

fn size_line(group: &str, arm: &str, bytes: usize) {
    eprintln!("SIZE\t{group}\t{arm}\t{bytes}");
}

// ── decoders ─────────────────────────────────────────────────────────────

fn zenpng_decode(data: &[u8], threads: usize) -> zenpng::PngDecodeOutput {
    let cfg = zenpng::PngDecodeConfig::default().with_max_threads(threads);
    zenpng::decode(data, &cfg, &Unstoppable).unwrap()
}

fn png_decode(data: &[u8]) -> Vec<u8> {
    let mut d = png_main::Decoder::new(std::io::Cursor::new(data));
    d.set_transformations(png_main::Transformations::EXPAND);
    d.ignore_checksums(true);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

fn zune_decode(data: &[u8]) -> Vec<u8> {
    let opts = zune_core::options::DecoderOptions::new_fast()
        .png_set_confirm_crc(false)
        .inflate_set_confirm_adler(false);
    let mut d =
        zune_png::PngDecoder::new_with_options(zune_core::bytestream::ZCursor::new(data), opts);
    d.decode_headers().unwrap();
    let mut out = vec![0u8; d.output_buffer_size().unwrap()];
    d.decode_into(&mut out).unwrap();
    out
}

fn lodepng_decode(data: &[u8]) -> lodepng::Image {
    // Native colour type, no conversion. lodepng's safe API has no switch for
    // its CRC/Adler-32 checks, so this arm does slightly more work.
    let mut d = lodepng::Decoder::new();
    d.color_convert(false);
    d.decode(data).unwrap()
}

// ── encoders ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Img {
    px: &'static [u8],
    w: usize,
    h: usize,
    alpha: bool,
}

fn zenpng_encode(img: Img, cfg: &zenpng::EncodeConfig) -> Vec<u8> {
    if img.alpha {
        let p: &[rgb::Rgba<u8>] = bytemuck::cast_slice(img.px);
        zenpng::encode_rgba8(
            imgref::ImgRef::new(p, img.w, img.h),
            None,
            cfg,
            &Unstoppable,
            &Unstoppable,
        )
    } else {
        let p: &[rgb::Rgb<u8>] = bytemuck::cast_slice(img.px);
        zenpng::encode_rgb8(
            imgref::ImgRef::new(p, img.w, img.h),
            None,
            cfg,
            &Unstoppable,
            &Unstoppable,
        )
    }
    .unwrap()
}

fn png_encode(img: Img, c: png_main::Compression) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png_main::Encoder::new(&mut out, img.w as u32, img.h as u32);
    e.set_color(if img.alpha {
        png_main::ColorType::Rgba
    } else {
        png_main::ColorType::Rgb
    });
    e.set_depth(png_main::BitDepth::Eight);
    e.set_compression(c);
    let mut wr = e.write_header().unwrap();
    wr.write_image_data(img.px).unwrap();
    wr.finish().unwrap();
    out
}

fn zune_encode(img: Img) -> Vec<u8> {
    let cs = if img.alpha {
        zune_core::colorspace::ColorSpace::RGBA
    } else {
        zune_core::colorspace::ColorSpace::RGB
    };
    let opts = zune_core::options::EncoderOptions::new(
        img.w,
        img.h,
        cs,
        zune_core::bit_depth::BitDepth::Eight,
    );
    let mut out = Vec::new();
    zune_png::PngEncoder::new(img.px, opts)
        .encode(&mut out)
        .unwrap();
    out
}

fn lodepng_encode(img: Img) -> Vec<u8> {
    if img.alpha {
        lodepng::encode32(img.px, img.w, img.h).unwrap()
    } else {
        lodepng::encode24(img.px, img.w, img.h).unwrap()
    }
}

/// One encode arm: image in, PNG bytes out.
type EncodeFn = Box<dyn Fn(Img) -> Vec<u8> + Send + Sync>;

fn effort_cfg(e: u32, parallel: bool) -> zenpng::EncodeConfig {
    let mut c = zenpng::EncodeConfig::default()
        .with_compression(zenpng::Compression::Effort(e))
        .with_parallel(parallel);
    if !parallel {
        // Strictly one thread (Phase 4 recompression otherwise follows
        // max_threads alone).
        c.max_threads = 1;
    }
    c
}

/// Encode arms: (name, encoder). Sizes printed once per arm.
fn encode_arms(edge: u32) -> Vec<(String, EncodeFn)> {
    let big_ok = std::env::var_os("ZENPNG_PARETO_BIG").is_some() || edge <= 2560;
    let mut arms: Vec<(String, EncodeFn)> = Vec::new();
    for e in env_list("ZENPNG_PARETO_EFFORTS", &[1, 2, 3, 5, 7, 9, 13, 19]) {
        if e > 13 && !big_ok {
            continue;
        }
        let cfg = effort_cfg(e, false);
        arms.push((
            format!("zenpng_e{e}_st"),
            Box::new(move |i| zenpng_encode(i, &cfg)),
        ));
    }
    // ZENPNG_PARETO_MT_THREADS (e.g. "2,4,8"): one `zenpng_e<E>_t<N>` arm per
    // thread count instead of the all-cores `_mt` arm.
    let thread_counts = env_list("ZENPNG_PARETO_MT_THREADS", &[]);
    for e in env_list("ZENPNG_PARETO_MT_EFFORTS", &[7, 13, 19]) {
        if e > 13 && !big_ok {
            continue;
        }
        if thread_counts.is_empty() {
            let cfg = effort_cfg(e, true);
            arms.push((
                format!("zenpng_e{e}_mt"),
                Box::new(move |i| zenpng_encode(i, &cfg)),
            ));
        }
        for &t in &thread_counts {
            let mut cfg = effort_cfg(e, true);
            cfg.max_threads = t as usize;
            arms.push((
                format!("zenpng_e{e}_t{t}"),
                Box::new(move |i| zenpng_encode(i, &cfg)),
            ));
        }
    }
    if std::env::var_os("ZENPNG_PARETO_NO_OTHERS").is_some() {
        return arms;
    }
    if std::env::var_os("ZENPNG_PARETO_NO_IDOT").is_none() {
        let idot = effort_cfg(7, true).with_decode_segments(8);
        arms.push((
            "zenpng_e7_idot8_mt".into(),
            Box::new(move |i| zenpng_encode(i, &idot)),
        ));
    }
    for (n, c) in [
        ("png_fast", png_main::Compression::Fast),
        ("png_balanced", png_main::Compression::Balanced),
        ("png_high", png_main::Compression::High),
    ] {
        arms.push((n.into(), Box::new(move |i| png_encode(i, c))));
    }
    arms.push(("zune".into(), Box::new(zune_encode)));
    arms.push(("lodepng".into(), Box::new(lodepng_encode)));
    arms
}

fn bench_encode(suite: &mut Suite) {
    if std::env::args().any(|a| {
        a.starts_with("--group=dec") || a.starts_with("--group=s") || a.starts_with("--group=p")
    }) {
        return;
    }
    for (name, edge, data) in inputs() {
        if !(name.contains("_rgb8_") || name.contains("_rgba8_")) {
            continue;
        }
        let d = zenpng_decode(data, 1);
        let alpha = name.contains("_rgba8_");
        let bytes = d.pixels.copy_to_contiguous_bytes();
        let (w, h) = (d.info.width as usize, d.info.height as usize);
        if bytes.len() != w * h * if alpha { 4 } else { 3 } {
            continue; // the file decoded to another layout (e.g. gray)
        }
        let img = Img {
            px: leak(bytes),
            w,
            h,
            alpha,
        };
        let group = format!("enc/{name}");
        let arms = encode_arms(edge);
        for (arm, f) in &arms {
            size_line(&group, arm, f(img).len());
        }
        let px = (w * h) as u64;
        // ZENPNG_PARETO_MAX_WALL (seconds): zenbench's per-group wall clock
        // (default 120 s) runs out before one round of slow arms (e15+ at
        // 4096 px, e20+ at 1024 px) and reports 0 rounds.
        let max_wall = std::env::var("ZENPNG_PARETO_MAX_WALL")
            .ok()
            .and_then(|v| v.parse::<u64>().ok());
        suite.compare(group, move |g| {
            if let Some(s) = max_wall {
                g.config().max_wall_time(std::time::Duration::from_secs(s));
            }
            g.throughput(Throughput::Elements(px));
            g.throughput_unit("px");
            for (arm, f) in arms {
                let f: &'static (dyn Fn(Img) -> Vec<u8> + Send + Sync) = Box::leak(f);
                g.bench(arm, move |b| b.iter(|| f(img)));
            }
        });
    }
}

fn eight_bit(name: &str) -> bool {
    !name.contains("16")
}

fn add_decode_group(suite: &mut Suite, group: String, data: &'static [u8], check: bool) {
    let out = zenpng_decode(data, 1);
    let px = out.info.width as u64 * out.info.height as u64;
    size_line(&group, "input", data.len());
    if check {
        let ours = out.pixels.copy_to_contiguous_bytes();
        assert!(ours == png_decode(data), "{group}: png disagrees");
        let mt = zenpng_decode(data, 0).pixels.copy_to_contiguous_bytes();
        assert!(ours == mt, "{group}: zenpng MT disagrees with ST");
    }
    suite.compare(group, move |g| {
        g.throughput(Throughput::Elements(px));
        g.throughput_unit("px");
        g.bench("zenpng_st", move |b| b.iter(|| zenpng_decode(data, 1)));
        g.bench("zenpng_mt", move |b| b.iter(|| zenpng_decode(data, 0)));
        g.bench("png", move |b| b.iter(|| png_decode(data)));
        g.bench("zune", move |b| b.iter(|| zune_decode(data)));
        g.bench("lodepng", move |b| b.iter(|| lodepng_decode(data)));
    });
}

fn bench_decode(suite: &mut Suite) {
    if std::env::args().any(|a| {
        a.starts_with("--group=enc") || a.starts_with("--group=s") || a.starts_with("--group=p")
    }) {
        return;
    }
    for (name, _edge, data) in inputs() {
        add_decode_group(suite, format!("dec/{name}"), data, eight_bit(&name));
        if name.contains("_rgb8_") || name.contains("_rgba8_") {
            let d = zenpng_decode(data, 1);
            let (w, h) = (d.info.width as usize, d.info.height as usize);
            let alpha = name.contains("_rgba8_");
            let bytes = d.pixels.copy_to_contiguous_bytes();
            if bytes.len() != w * h * if alpha { 4 } else { 3 } {
                continue;
            }
            let img = Img {
                px: leak(bytes),
                w,
                h,
                alpha,
            };
            let idot = leak(zenpng_encode(
                img,
                &effort_cfg(7, true).with_decode_segments(8),
            ));
            add_decode_group(suite, format!("dec_idot/{name}"), idot, true);
        }
    }
}

struct VecSink(Vec<u8>);

impl zencodec::decode::DecodeRowSink for VecSink {
    fn provide_next_buffer(
        &mut self,
        _y: u32,
        height: u32,
        width: u32,
        descriptor: zenpixels::PixelDescriptor,
    ) -> Result<zenpixels::PixelSliceMut<'_>, zencodec::decode::SinkError> {
        let stride = width as usize * descriptor.bytes_per_pixel();
        self.0.resize(height as usize * stride, 0);
        Ok(
            zenpixels::PixelSliceMut::new(&mut self.0, width, height, stride, descriptor)
                .expect("sized"),
        )
    }
}

/// A zencodec decode job; `sequential` sets the limits' threading policy to
/// `Sequential` (no inflate thread), otherwise the default (`Parallel`).
fn decode_job(sequential: bool) -> zenpng::PngDecodeJob {
    use zencodec::decode::{DecodeJob, DecoderConfig};
    let job = zenpng::PngDecoderConfig::new().job();
    if sequential {
        job.with_limits(
            zencodec::ResourceLimits::none().with_threading(zencodec::ThreadingPolicy::Sequential),
        )
    } else {
        job
    }
}

fn push_decode(data: &[u8]) -> Vec<u8> {
    push_decode_with(data, false)
}

fn push_decode_with(data: &[u8], sequential: bool) -> Vec<u8> {
    use zencodec::decode::DecodeJob;
    let mut sink = VecSink(Vec::new());
    decode_job(sequential)
        .push_decoder(data.into(), &mut sink, &[])
        .unwrap();
    sink.0
}

fn stream_decode(data: &[u8]) -> Vec<u8> {
    stream_decode_with(data, false)
}

fn stream_decode_with(data: &[u8], sequential: bool) -> Vec<u8> {
    use zencodec::decode::{DecodeJob, StreamingDecode};
    let mut dec = decode_job(sequential)
        .streaming_decoder(data.into(), &[])
        .unwrap();
    let h = dec.info().height as usize;
    let mut out = Vec::new();
    while let Some((_, rows)) = dec.next_batch().unwrap() {
        for y in 0..rows.rows() {
            // Sized once, as a consumer that keeps the image would.
            if out.capacity() == 0 {
                out.reserve_exact(rows.row(y).len() * h);
            }
            out.extend_from_slice(rows.row(y));
        }
    }
    out
}

fn png_rows_decode(data: &[u8]) -> Vec<u8> {
    let mut d = png_main::Decoder::new(std::io::Cursor::new(data));
    d.set_transformations(png_main::Transformations::EXPAND);
    d.ignore_checksums(true);
    let mut r = d.read_info().unwrap();
    let mut out = Vec::with_capacity(r.output_buffer_size().unwrap());
    while let Some(row) = r.next_row().unwrap() {
        out.extend_from_slice(row.data());
    }
    out
}

/// Re-encode `data` with zenpng at effort 7 and 8 `iDOT` segments, keeping
/// its decoded format; `None` when the encoder wrote no `iDOT` chunk (it
/// doesn't for images under ~2 MiB of row data or for sub-byte gray).
fn idot_reencode(data: &[u8]) -> Option<Vec<u8>> {
    let d = zenpng_decode(data, 1);
    let cfg = effort_cfg(7, false).with_decode_segments(8);
    let px = &d.pixels;
    let out = if let Some(i) = px.try_as_imgref::<rgb::Rgb<u8>>() {
        zenpng::encode_rgb8(i, None, &cfg, &Unstoppable, &Unstoppable)
    } else if let Some(i) = px.try_as_imgref::<rgb::Rgba<u8>>() {
        zenpng::encode_rgba8(i, None, &cfg, &Unstoppable, &Unstoppable)
    } else if let Some(i) = px.try_as_imgref::<rgb::Gray<u8>>() {
        zenpng::encode_gray8(i, None, &cfg, &Unstoppable, &Unstoppable)
    } else {
        let i = px.try_as_imgref::<rgb::Rgb<u16>>()?;
        zenpng::encode_rgb16(i, None, &cfg, &Unstoppable, &Unstoppable)
    }
    .unwrap();
    out.windows(4).any(|w| w == b"iDOT").then_some(out)
}

/// Decode threading per API (opt-in: `--group=pdec`), group `pdec/<name>`:
/// `decode_st` / `decode_mt` (`zenpng::decode`, `max_threads` 1 / 0),
/// `push_st` / `push_mt` and `stream_st` / `stream_mt` (zencodec
/// `push_decoder` / `streaming_decoder` with a sequential / the default
/// parallel threading policy), and, when zenpng's effort-7 re-encode with
/// 8 `iDOT` segments has an `iDOT` chunk, `idot_st` / `idot_mt` decoding it.
fn bench_pipeline_decode(suite: &mut Suite) {
    if !stream_groups_requested("pdec") {
        return;
    }
    for (name, _edge, data) in inputs() {
        if name.contains("interlaced") {
            continue;
        }
        let whole = zenpng_decode(data, 1).pixels.copy_to_contiguous_bytes();
        for seq in [true, false] {
            assert!(push_decode_with(data, seq) == whole, "{name}: push differs");
            assert!(
                stream_decode_with(data, seq) == whole,
                "{name}: stream differs"
            );
        }
        // The re-encode keeps default downcasts, so e.g. a 16-bit file of
        // 8-bit values comes back 8-bit; the iDOT decode is checked against
        // its own serial decode.
        // ZENPNG_PARETO_PDEC_DECODE_ONLY: only decode_st / decode_mt, no iDOT
        // (threshold sweeps; the three APIs pipeline alike).
        let decode_only = std::env::var_os("ZENPNG_PARETO_PDEC_DECODE_ONLY").is_some();
        let idot = (!decode_only)
            .then(|| idot_reencode(data))
            .flatten()
            .map(leak);
        if let Some(i) = idot {
            assert!(
                zenpng_decode(i, 0).pixels.copy_to_contiguous_bytes()
                    == zenpng_decode(i, 1).pixels.copy_to_contiguous_bytes(),
                "{name}: parallel iDOT decode differs from serial"
            );
            let ihdr = &i[16..29];
            eprintln!(
                "IDOT\tpdec/{name}\tcolor_type {} bit_depth {}",
                ihdr[9], ihdr[8]
            );
        }
        size_line(&format!("pdec/{name}"), "idot", idot.map_or(0, <[u8]>::len));
        let out = zenpng_decode(data, 1);
        let px = out.info.width as u64 * out.info.height as u64;
        suite.compare(format!("pdec/{name}"), move |g| {
            g.throughput(Throughput::Elements(px));
            g.throughput_unit("px");
            g.bench("decode_st", move |b| b.iter(|| zenpng_decode(data, 1)));
            g.bench("decode_mt", move |b| b.iter(|| zenpng_decode(data, 0)));
            if decode_only {
                return;
            }
            g.bench("push_st", move |b| b.iter(|| push_decode_with(data, true)));
            g.bench("push_mt", move |b| b.iter(|| push_decode_with(data, false)));
            g.bench("stream_st", move |b| {
                b.iter(|| stream_decode_with(data, true))
            });
            g.bench("stream_mt", move |b| {
                b.iter(|| stream_decode_with(data, false))
            });
            if let Some(i) = idot {
                g.bench("idot_st", move |b| b.iter(|| zenpng_decode(i, 1)));
                g.bench("idot_mt", move |b| b.iter(|| zenpng_decode(i, 0)));
            }
        });
    }
}

fn stream_groups_requested(prefix: &str) -> bool {
    std::env::args().any(|a| {
        a.strip_prefix("--group=")
            .is_some_and(|g| g.starts_with(prefix))
    })
}

fn bench_stream_decode(suite: &mut Suite) {
    if !stream_groups_requested("sdec") {
        return;
    }
    for (name, _edge, data) in inputs() {
        if name.contains("interlaced") {
            continue; // streaming_decoder rejects interlaced input
        }
        let whole = zenpng_decode(data, 1).pixels.copy_to_contiguous_bytes();
        assert!(push_decode(data) == whole, "{name}: push differs");
        assert!(stream_decode(data) == whole, "{name}: stream differs");
        let out = zenpng_decode(data, 1);
        let px = out.info.width as u64 * out.info.height as u64;
        let png_ok = eight_bit(&name) && png_rows_decode(data) == whole;
        suite.compare(format!("sdec/{name}"), move |g| {
            g.throughput(Throughput::Elements(px));
            g.throughput_unit("px");
            g.bench("whole_st", move |b| b.iter(|| zenpng_decode(data, 1)));
            g.bench("whole_mt", move |b| b.iter(|| zenpng_decode(data, 0)));
            g.bench("push", move |b| b.iter(|| push_decode(data)));
            g.bench("stream", move |b| b.iter(|| stream_decode(data)));
            if png_ok {
                g.bench("png_rows", move |b| b.iter(|| png_rows_decode(data)));
            }
        });
    }
}

fn zencodec_whole(img: Img, e: u32) -> Vec<u8> {
    use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
    let enc = zenpng::PngEncoderConfig::new()
        .with_compression(zenpng::Compression::Effort(e))
        .job()
        .encoder()
        .unwrap();
    enc.encode(img_slice(img, 0, img.h))
        .unwrap()
        .data()
        .to_vec()
}

fn zencodec_push(img: Img, e: u32, strip: usize) -> Vec<u8> {
    use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
    let mut enc = zenpng::PngEncoderConfig::new()
        .with_compression(zenpng::Compression::Effort(e))
        .job()
        .with_canvas_size(img.w as u32, img.h as u32)
        .encoder()
        .unwrap();
    let mut y = 0;
    while y < img.h {
        let n = strip.min(img.h - y);
        enc.push_rows(img_slice(img, y, n)).unwrap();
        y += n;
    }
    enc.finish().unwrap().data().to_vec()
}

fn img_slice(img: Img, y: usize, rows: usize) -> zenpixels::PixelSlice<'static> {
    let bpp = if img.alpha { 4 } else { 3 };
    let stride = img.w * bpp;
    let desc = if img.alpha {
        zenpixels::PixelDescriptor::RGBA8_SRGB
    } else {
        zenpixels::PixelDescriptor::RGB8_SRGB
    };
    zenpixels::PixelSlice::new(
        &img.px[y * stride..(y + rows) * stride],
        img.w as u32,
        rows as u32,
        stride,
        desc,
    )
    .unwrap()
}

fn bench_stream_encode(suite: &mut Suite) {
    if !stream_groups_requested("senc") {
        return;
    }
    let efforts = env_list("ZENPNG_PARETO_STREAM_EFFORTS", &[0, 1, 2, 7, 13]);
    for (name, _edge, data) in inputs() {
        if !(name.contains("_rgb8_") || name.contains("_rgba8_")) {
            continue;
        }
        let d = zenpng_decode(data, 1);
        let alpha = name.contains("_rgba8_");
        let bytes = d.pixels.copy_to_contiguous_bytes();
        let (w, h) = (d.info.width as usize, d.info.height as usize);
        if bytes.len() != w * h * if alpha { 4 } else { 3 } {
            continue;
        }
        let img = Img {
            px: leak(bytes),
            w,
            h,
            alpha,
        };
        let group = format!("senc/{name}");
        for &e in &efforts {
            size_line(&group, &format!("e{e}_whole"), zencodec_whole(img, e).len());
            size_line(
                &group,
                &format!("e{e}_push16"),
                zencodec_push(img, e, 16).len(),
            );
        }
        let px = (w * h) as u64;
        let efforts = efforts.clone();
        suite.compare(group, move |g| {
            g.throughput(Throughput::Elements(px));
            g.throughput_unit("px");
            for &e in &efforts {
                g.bench(format!("e{e}_whole"), move |b| {
                    b.iter(|| zencodec_whole(img, e))
                });
                g.bench(format!("e{e}_push16"), move |b| {
                    b.iter(|| zencodec_push(img, e, 16))
                });
            }
        });
    }
}

fn main() {
    // Ungated like benches/vs_png.rs (the resource gate stalls on shared
    // boxes); arms are interleaved within each round.
    let group_filter = std::env::args().find_map(|a| a.strip_prefix("--group=").map(String::from));
    let result = zenbench::run_gated(zenbench::GateConfig::disabled(), |suite| {
        if let Some(f) = &group_filter {
            suite.set_group_filter(f.clone());
        }
        bench_encode(suite);
        bench_decode(suite);
        bench_stream_decode(suite);
        bench_pipeline_decode(suite);
        bench_stream_encode(suite);
    });
    zenbench::postprocess_result(&result);
}
