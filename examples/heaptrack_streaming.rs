/// Peak-memory harness: streaming vs whole-image encode and decode.
///
/// Usage (one mode per process, so `/usr/bin/time -v` or heaptrack sees only
/// that mode's peak):
///   cargo build --release --example heaptrack_streaming
///   /usr/bin/time -v target/release/examples/heaptrack_streaming <mode> image.png
///   heaptrack target/release/examples/heaptrack_streaming <mode> image.png
///
/// Modes:
/// - `load`: decode the input only (the baseline every encode mode pays to
///   get its pixels).
/// - `oneshot<E>`: zencodec `Encoder::encode` at effort E.
/// - `stream<E>`: `push_rows` in strips of `STRIP` rows (default 16) at
///   effort E, after `with_canvas_size`.
/// - `sstream<E>` / `soneshot<E>`: the same with `DowncastFlags::none()`
///   (efforts 1-15 then stream strip by strip).
/// - `PARALLEL=1`: every encode mode with `with_parallel(true)`.
/// - `ALPHA=1`: an RGB8 input gets an alpha ramp (RGBA8 measurements);
///   `SAVE=<path>` writes the encoded PNG.
/// - `dec_whole` / `dec_whole_mt`: `zenpng::decode`, 1 / all threads.
/// - `dec_push`: `push_decoder` into a `Vec` sink.
/// - `dec_stream`: `streaming_decoder`, rows consumed one at a time and
///   dropped (a consumer that never holds the image).
///
/// Input: an RGB8 or RGBA8 PNG, or for encode modes a synthetic
/// `SYNTH_SIZE`² RGBA8 image (default 2048; no decode, so heaptrack's peak is
/// the encoder's).
use enough::Unstoppable;
use zencodec::decode::{DecodeJob, DecodeRowSink, DecoderConfig, SinkError, StreamingDecode};
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};
use zenpng::{Compression, PngDecoderConfig, PngEncoderConfig};

struct Pixels {
    data: Vec<u8>,
    w: usize,
    h: usize,
    desc: PixelDescriptor,
}

impl Pixels {
    fn bpp(&self) -> usize {
        self.desc.bytes_per_pixel()
    }
    fn rows(&self, y: usize, n: usize) -> PixelSlice<'_> {
        let stride = self.w * self.bpp();
        PixelSlice::new(
            &self.data[y * stride..(y + n) * stride],
            self.w as u32,
            n as u32,
            stride,
            self.desc,
        )
        .unwrap()
    }
}

fn synthetic(w: usize, h: usize) -> Pixels {
    let data = (0..w * h * 4)
        .map(|i| (i.wrapping_mul(7) ^ (i >> 9)) as u8)
        .collect();
    Pixels {
        data,
        w,
        h,
        desc: PixelDescriptor::RGBA8_SRGB,
    }
}

/// Decode the input row by row into one exactly-sized buffer, so the load
/// itself peaks at about file + pixels and doesn't hide the encoder's peak.
fn load(file: &[u8]) -> Pixels {
    // Sequential: a pipelined decode would copy the input for its thread.
    let mut dec = PngDecoderConfig::new()
        .job()
        .with_limits(
            zencodec::ResourceLimits::none().with_threading(zencodec::ThreadingPolicy::Sequential),
        )
        .streaming_decoder(file.into(), &[])
        .unwrap();
    let (w, h) = (dec.info().width as usize, dec.info().height as usize);
    let mut data = Vec::new();
    while let Some((_, rows)) = dec.next_batch().unwrap() {
        for y in 0..rows.rows() {
            let row = rows.row(y);
            if data.capacity() == 0 {
                data.reserve_exact(row.len() * h);
            }
            data.extend_from_slice(row);
        }
    }
    let desc = match data.len() / (w * h) {
        3 => PixelDescriptor::RGB8_SRGB,
        4 => PixelDescriptor::RGBA8_SRGB,
        n => panic!("expected an RGB8 or RGBA8 PNG, got {n} bytes/pixel"),
    };
    // ALPHA=1: give an RGB8 input a horizontal alpha ramp (the left 1/16
    // fully transparent), for RGBA8 measurements on real content.
    if desc == PixelDescriptor::RGB8_SRGB && std::env::var_os("ALPHA").is_some_and(|v| v == "1") {
        let mut rgba = Vec::with_capacity(w * h * 4);
        for (i, px) in data.as_chunks::<3>().0.iter().enumerate() {
            let x = i % w;
            let a = if x < w / 16 { 0 } else { (x * 255 / w) as u8 };
            rgba.extend_from_slice(&[px[0], px[1], px[2], a]);
        }
        return Pixels {
            data: rgba,
            w,
            h,
            desc: PixelDescriptor::RGBA8_SRGB,
        };
    }
    Pixels { data, w, h, desc }
}

fn enc_config(e: u32, verbatim: bool) -> PngEncoderConfig {
    // PARALLEL=1: multi-threaded encode (strip-parallel at efforts 1-15).
    let parallel = std::env::var_os("PARALLEL").is_some_and(|v| v == "1");
    let c = PngEncoderConfig::new()
        .with_compression(Compression::Effort(e))
        .with_parallel(parallel);
    if verbatim {
        c.with_downcast(zenpng::DowncastFlags::none())
    } else {
        c
    }
}

fn oneshot(px: &Pixels, e: u32, verbatim: bool) -> usize {
    let enc = enc_config(e, verbatim).job().encoder().unwrap();
    save(enc.encode(px.rows(0, px.h)).unwrap().data())
}

fn stream(px: &Pixels, e: u32, strip: usize, verbatim: bool) -> usize {
    let mut enc = enc_config(e, verbatim)
        .job()
        .with_canvas_size(px.w as u32, px.h as u32)
        .encoder()
        .unwrap();
    let mut y = 0;
    while y < px.h {
        let n = strip.min(px.h - y);
        enc.push_rows(px.rows(y, n)).unwrap();
        y += n;
    }
    save(enc.finish().unwrap().data())
}

/// SAVE=<path>: write the encoded PNG there. Returns its length.
fn save(png: &[u8]) -> usize {
    if let Some(p) = std::env::var_os("SAVE") {
        std::fs::write(p, png).unwrap();
    }
    png.len()
}

struct VecSink(Vec<u8>);

impl DecodeRowSink for VecSink {
    fn provide_next_buffer(
        &mut self,
        _y: u32,
        height: u32,
        width: u32,
        descriptor: PixelDescriptor,
    ) -> Result<PixelSliceMut<'_>, SinkError> {
        let stride = width as usize * descriptor.bytes_per_pixel();
        self.0.resize(height as usize * stride, 0);
        Ok(PixelSliceMut::new(&mut self.0, width, height, stride, descriptor).expect("sized"))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("stream7");
    let file = args.get(2).map(|p| std::fs::read(p).expect("read input"));
    let strip: usize = std::env::var("STRIP").map_or(16, |s| s.parse().unwrap());
    let effort = |prefix: &str| -> Option<u32> { mode.strip_prefix(prefix)?.parse().ok() };

    if let Some(file) = &file
        && mode.starts_with("dec_")
    {
        let n = match mode {
            "dec_whole" | "dec_whole_mt" => {
                let threads = usize::from(mode == "dec_whole");
                let cfg = zenpng::PngDecodeConfig::default().with_max_threads(threads);
                let d = zenpng::decode(file, &cfg, &Unstoppable).unwrap();
                d.pixels.descriptor().bytes_per_pixel()
                    * d.info.width as usize
                    * d.info.height as usize
            }
            "dec_push" => {
                let mut sink = VecSink(Vec::new());
                PngDecoderConfig::new()
                    .job()
                    .push_decoder(file.as_slice().into(), &mut sink, &[])
                    .unwrap();
                sink.0.len()
            }
            "dec_stream" => {
                let mut dec = PngDecoderConfig::new()
                    .job()
                    .streaming_decoder(file.as_slice().into(), &[])
                    .unwrap();
                let mut sum = 0usize;
                while let Some((_, rows)) = dec.next_batch().unwrap() {
                    for y in 0..rows.rows() {
                        sum += rows.row(y).len();
                    }
                }
                sum
            }
            _ => panic!("unknown mode {mode}"),
        };
        eprintln!("{mode}: {n} pixel bytes");
        return;
    }

    let px = match file {
        // The file bytes are dropped once decoded.
        Some(f) => load(&f),
        None => {
            let n = std::env::var("SYNTH_SIZE").map_or(2048, |s| s.parse().unwrap());
            synthetic(n, n)
        }
    };
    eprintln!(
        "{}x{} {:?}, raw {:.1} MiB",
        px.w,
        px.h,
        px.desc,
        px.data.len() as f64 / 1048576.0
    );
    if mode == "load" {
        return;
    }
    if let Some(e) = effort("oneshot") {
        eprintln!("oneshot e{e}: {} bytes", oneshot(&px, e, false));
    } else if let Some(e) = effort("stream") {
        let n = stream(&px, e, strip, false);
        eprintln!("stream e{e} ({strip}-row strips): {n} bytes");
    } else if let Some(e) = effort("soneshot") {
        eprintln!("soneshot e{e}: {} bytes", oneshot(&px, e, true));
    } else if let Some(e) = effort("sstream") {
        let n = stream(&px, e, strip, true);
        eprintln!("sstream e{e} ({strip}-row strips): {n} bytes");
    } else {
        panic!("unknown mode {mode}");
    }
}
