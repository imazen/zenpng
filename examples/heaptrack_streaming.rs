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
/// - `dec_whole` / `dec_whole_mt`: `zenpng::decode`, 1 / all threads.
/// - `dec_push`: `push_decoder` into a `Vec` sink.
/// - `dec_stream`: `streaming_decoder`, rows consumed one at a time and
///   dropped (a consumer that never holds the image).
///
/// Input: an RGB8 or RGBA8 PNG (default: a synthetic 2048x2048 RGBA8 image
/// for encode modes).
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

fn load(file: &[u8]) -> Pixels {
    let d = zenpng::decode(file, &zenpng::PngDecodeConfig::default(), &Unstoppable).unwrap();
    let (w, h) = (d.info.width as usize, d.info.height as usize);
    let data = d.pixels.copy_to_contiguous_bytes();
    let desc = match data.len() / (w * h) {
        3 => PixelDescriptor::RGB8_SRGB,
        4 => PixelDescriptor::RGBA8_SRGB,
        n => panic!("expected an RGB8 or RGBA8 PNG, got {n} bytes/pixel"),
    };
    Pixels { data, w, h, desc }
}

fn oneshot(px: &Pixels, e: u32) -> usize {
    let enc = PngEncoderConfig::new()
        .with_compression(Compression::Effort(e))
        .job()
        .encoder()
        .unwrap();
    enc.encode(px.rows(0, px.h)).unwrap().data().len()
}

fn stream(px: &Pixels, e: u32, strip: usize) -> usize {
    let mut enc = PngEncoderConfig::new()
        .with_compression(Compression::Effort(e))
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
    enc.finish().unwrap().data().len()
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

    let px = match &file {
        Some(f) => load(f),
        None => synthetic(2048, 2048),
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
        eprintln!("oneshot e{e}: {} bytes", oneshot(&px, e));
    } else if let Some(e) = effort("stream") {
        eprintln!(
            "stream e{e} ({strip}-row strips): {} bytes",
            stream(&px, e, strip)
        );
    } else {
        panic!("unknown mode {mode}");
    }
}
