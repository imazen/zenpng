//! Writes gray PNGs with non-default color for independent readers to inspect.
//!
//! Usage: `just gray-icc-oracle <dir>`, which runs exiftool, pngcheck and libpng's
//! pngfix over them. `metadata-*` files take their color from `Metadata`. `context-*`
//! files take it from the pixels' `ColorContext`.

use std::path::PathBuf;
use std::sync::Arc;

use zencodec::encode::{EncodeJob as _, Encoder as _, EncoderConfig as _};
use zencodec::{Cicp, Metadata, MetadataPolicy};
use zenpixels::{ColorContext, PixelDescriptor, PixelSlice};
use zenpng::PngEncoderConfig;

const W: u32 = 16;
const H: u32 = 8;

fn samples(bytes_per_pixel: usize) -> Vec<u8> {
    (0..W as usize * H as usize * bytes_per_pixel)
        .map(|i| (i * 7 % 251) as u8)
        .collect()
}

#[allow(deprecated)] // Exercises the published best-effort job setter.
fn from_metadata(descriptor: PixelDescriptor, bytes_per_pixel: usize, cicp: Cicp) -> Vec<u8> {
    let data = samples(bytes_per_pixel);
    let stride = W as usize * bytes_per_pixel;
    PngEncoderConfig::new()
        .job()
        .with_metadata_policy(
            Metadata::none().with_cicp(cicp),
            MetadataPolicy::PreserveExact,
        )
        .encoder()
        .unwrap()
        .encode(PixelSlice::new(&data, W, H, stride, descriptor).unwrap())
        .unwrap()
        .into_vec()
}

fn from_context(descriptor: PixelDescriptor, bytes_per_pixel: usize, cicp: Cicp) -> Vec<u8> {
    let data = samples(bytes_per_pixel);
    let stride = W as usize * bytes_per_pixel;
    let pixels = PixelSlice::new(&data, W, H, stride, descriptor)
        .unwrap()
        .with_color_context(Arc::new(ColorContext::from_cicp(cicp)));
    PngEncoderConfig::new()
        .job()
        .encoder()
        .unwrap()
        .encode(pixels)
        .unwrap()
        .into_vec()
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&dir).unwrap();
    let pq = Cicp::new(1, 16, 0, true);
    let gamma22 = Cicp::new(1, 4, 0, true);
    let srgb = Cicp::new(1, 13, 0, true);
    let gray8 = PixelDescriptor::GRAY8_SRGB;
    let gray16 = PixelDescriptor::GRAY16_SRGB;
    let graya8 = PixelDescriptor::GRAYA8_SRGB;
    for (name, file) in [
        ("metadata-gray8-pq", from_metadata(gray8, 1, pq)),
        ("metadata-gray8-gamma22", from_metadata(gray8, 1, gamma22)),
        ("metadata-gray16-pq", from_metadata(gray16, 2, pq)),
        ("metadata-graya8-pq", from_metadata(graya8, 2, pq)),
        ("metadata-gray8-srgb", from_metadata(gray8, 1, srgb)),
        ("context-gray8-pq", from_context(gray8, 1, pq)),
    ] {
        std::fs::write(dir.join(format!("{name}.png")), file).unwrap();
    }
}
