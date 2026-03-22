/// Heaptrack harness for streaming encode memory profiling.
///
/// Usage:
///   cargo build --release --example heaptrack_streaming
///   heaptrack target/release/examples/heaptrack_streaming <mode> [/path/to/image.png]
///
/// Modes: stream0, stream1, stream7, oneshot0, oneshot1, oneshot7, all
use enough::Unstoppable;
use imgref::Img;
use rgb::Rgba;
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenpng::{Compression, PngEncoderConfig};

fn make_test_image(w: u32, h: u32) -> imgref::ImgVec<Rgba<u8>> {
    let pixels: Vec<Rgba<u8>> = (0..w * h)
        .map(|i| Rgba {
            r: (i.wrapping_mul(7) & 0xFF) as u8,
            g: (i.wrapping_mul(13) & 0xFF) as u8,
            b: (i.wrapping_mul(17) & 0xFF) as u8,
            a: 200,
        })
        .collect();
    Img::new(pixels, w as usize, h as usize)
}

fn streaming_encode(img: &imgref::ImgVec<Rgba<u8>>, effort: u32) -> usize {
    let w = img.width() as u32;
    let h = img.height() as u32;
    let config = PngEncoderConfig::new().with_compression(Compression::Effort(effort));
    let mut encoder = config.job().with_canvas_size(w, h).encoder().unwrap();
    for y in 0..h {
        let strip = img.sub_image(0, y as usize, w as usize, 1);
        encoder
            .push_rows(zenpixels::PixelSlice::from(strip).erase())
            .unwrap();
    }
    let output = encoder.finish().unwrap();
    output.data().len()
}

fn oneshot_encode(img: &imgref::ImgRef<'_, Rgba<u8>>, effort: u32) -> usize {
    let config = PngEncoderConfig::new().with_compression(Compression::Effort(effort));
    let encoder = config.job().encoder().unwrap();
    let output = encoder
        .encode(zenpixels::PixelSlice::from(*img).erase())
        .unwrap();
    output.data().len()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("all");

    let img_path = args.get(2);
    let img = if let Some(path) = img_path {
        let data = std::fs::read(path).expect("failed to read file");
        let config = zenpng::PngDecodeConfig::default();
        let info = zenpng::decode(&data, &config, &Unstoppable).expect("decode failed");
        let w = info.info.width as usize;
        let h = info.info.height as usize;
        eprintln!("Loaded {}x{} from {}", w, h, path);
        let bytes = info.pixels.copy_to_contiguous_bytes();
        let pixels: Vec<Rgba<u8>> = bytemuck::cast_slice(&bytes).to_vec();
        Img::new(pixels, w, h)
    } else {
        let w = 2048u32;
        let h = 2048u32;
        eprintln!("Using synthetic {}x{} RGBA8 image", w, h);
        make_test_image(w, h)
    };

    let raw_size = img.width() * img.height() * 4;
    eprintln!(
        "Raw: {} bytes ({:.1} MiB)",
        raw_size,
        raw_size as f64 / 1048576.0
    );

    match mode {
        "stream0" => {
            let size = streaming_encode(&img, 0);
            eprintln!("stream e0 (stored):      {} bytes", size);
        }
        "stream1" => {
            let size = streaming_encode(&img, 1);
            eprintln!("stream e1 (paeth+turbo): {} bytes", size);
        }
        "stream7" => {
            let size = streaming_encode(&img, 7);
            eprintln!("stream e7 (buffered):    {} bytes", size);
        }
        "oneshot0" => {
            let size = oneshot_encode(&img.as_ref(), 0);
            eprintln!("oneshot e0 (stored):     {} bytes", size);
        }
        "oneshot1" => {
            let size = oneshot_encode(&img.as_ref(), 1);
            eprintln!("oneshot e1 (paeth+turbo):{} bytes", size);
        }
        "oneshot7" => {
            let size = oneshot_encode(&img.as_ref(), 7);
            eprintln!("oneshot e7 (fast):       {} bytes", size);
        }
        _ => {
            eprintln!("\n=== Streaming ===");
            eprintln!("  e0: {} bytes", streaming_encode(&img, 0));
            eprintln!("  e1: {} bytes", streaming_encode(&img, 1));
            eprintln!("  e7: {} bytes", streaming_encode(&img, 7));
            eprintln!("\n=== One-shot ===");
            let r = img.as_ref();
            eprintln!("  e0: {} bytes", oneshot_encode(&r, 0));
            eprintln!("  e1: {} bytes", oneshot_encode(&r, 1));
            eprintln!("  e7: {} bytes", oneshot_encode(&r, 7));
        }
    }
}
