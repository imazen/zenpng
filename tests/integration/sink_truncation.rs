//! `push_decoder` (the zencodec row-sink path) and the streaming decoder must reject a short IDAT
//! stream exactly like `decode()` does, for both its passthrough (RGB8/RGBA8)
//! and expander (every other format) branches. A partial image delivered to
//! the sink without an error would hand callers uninitialised rows.

use enough::Unstoppable;
use zencodec::decode::{DecodeJob, DecodeRowSink, DecoderConfig, SinkError, StreamingDecode};
use zenpixels::{PixelDescriptor, PixelSliceMut};
use zenpng::{PngDecodeConfig, PngDecoderConfig, decode};

struct CollectSink(Vec<u8>);

impl DecodeRowSink for CollectSink {
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

/// A valid `w`×`rows` PNG whose IHDR then claims `2 * rows` rows: every chunk
/// and the zlib stream are intact, the image data just ends early.
fn short_png(color: png::ColorType, channels: usize, w: u32, rows: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, rows);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        let data: Vec<u8> = (0..w as usize * rows as usize * channels)
            .map(|i| (i * 31 % 251) as u8)
            .collect();
        wr.write_image_data(&data).unwrap();
    }
    // IHDR data starts at byte 16: width(4) height(4). Height at 20..24.
    out[20..24].copy_from_slice(&(2 * rows).to_be_bytes());
    let crc = crc32(&out[12..29]);
    out[29..33].copy_from_slice(&crc.to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[test]
fn push_decoder_rejects_short_image_data_like_decode() {
    for (name, color, channels) in [
        ("rgb8 (passthrough)", png::ColorType::Rgb, 3),
        ("gray8 (expander)", png::ColorType::Grayscale, 1),
        ("gray+alpha8 (expander)", png::ColorType::GrayscaleAlpha, 2),
    ] {
        let data = short_png(color, channels, 40, 16);
        assert!(
            decode(&data, &PngDecodeConfig::default(), &Unstoppable).is_err(),
            "{name}: decode() accepts the short stream"
        );
        let mut sink = CollectSink(Vec::new());
        let res =
            PngDecoderConfig::new()
                .job()
                .push_decoder(data.as_slice().into(), &mut sink, &[]);
        assert!(
            res.is_err(),
            "{name}: push_decoder accepted the short stream"
        );

        let mut dec = PngDecoderConfig::new()
            .job()
            .streaming_decoder(data.as_slice().into(), &[])
            .unwrap();
        let mut rows = 0;
        let end = loop {
            match dec.next_batch() {
                Ok(Some(_)) => rows += 1,
                Ok(None) => break Ok(rows),
                Err(e) => break Err(e),
            }
        };
        assert!(
            end.is_err(),
            "{name}: streaming decoder ended cleanly after {rows} of 32 rows"
        );
    }
}

/// `push_decoder` and `decode()` must produce identical pixels for every
/// color type and bit depth (the 44 tiny fixtures cover all of them,
/// including tRNS, sub-byte gray and palette).
#[test]
fn push_decoder_matches_decode_on_every_format() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/idot");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "png") {
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        let Ok(expected) = decode(&data, &PngDecodeConfig::default(), &Unstoppable) else {
            continue; // deliberately broken fixtures
        };
        let mut sink = CollectSink(Vec::new());
        PngDecoderConfig::new()
            .job()
            .push_decoder(data.as_slice().into(), &mut sink, &[])
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            sink.0,
            expected.pixels.copy_to_contiguous_bytes(),
            "{}",
            path.display()
        );
        checked += 1;
    }
    assert!(checked >= 30, "only {checked} fixtures decoded");
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

/// Gray8 64×64 whose zlib stream carries 400 KB of trailing data (so the
/// last row is produced long before the footer) and a corrupt Adler-32.
fn bad_adler_png() -> Vec<u8> {
    let (w, h) = (64usize, 64usize);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0u8);
        raw.extend((0..w).map(|x| (x * 3 + y) as u8));
    }
    raw.extend(std::iter::repeat_n(7u8, 400 * 1024));
    let mut c = zenflate::Compressor::new(zenflate::CompressionLevel::new(6));
    let mut z = vec![0u8; zenflate::Compressor::zlib_compress_bound(raw.len())];
    let n = c.zlib_compress(&raw, &mut z, Unstoppable).unwrap();
    z.truncate(n);
    z[n - 1] ^= 1;
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&(w as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(h as u32).to_be_bytes());
    ihdr[8] = 8;
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

#[test]
fn strict_policy_verifies_adler_in_push_and_streaming() {
    use zencodec::decode::DecodePolicy;
    let data = bad_adler_png();
    for strict in [false, true] {
        let policy = DecodePolicy::none().with_strict(strict);
        let mut sink = CollectSink(Vec::new());
        let push = PngDecoderConfig::new()
            .job()
            .with_policy(policy)
            .push_decoder(data.as_slice().into(), &mut sink, &[]);
        assert_eq!(push.is_err(), strict, "push_decoder, strict={strict}");

        let mut dec = PngDecoderConfig::new()
            .job()
            .with_policy(policy)
            .streaming_decoder(data.as_slice().into(), &[])
            .unwrap();
        let end = loop {
            match dec.next_batch() {
                Ok(Some(_)) => {}
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        assert_eq!(end.is_err(), strict, "streaming, strict={strict}");
    }
}
