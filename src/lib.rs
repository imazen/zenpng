//! PNG encoding and decoding with zencodec-types trait integration.
//!
//! Wraps the `png` crate (0.18) with typed pixel buffers (`imgref` + `rgb`),
//! metadata roundtrip (ICC/EXIF/XMP), and optional palette quantization
//! via `zenquant`.
//!
//! # Quick start
//!
//! ```no_run
//! use zenpng::{decode, probe, encode_rgb8, EncodeConfig};
//! use imgref::ImgVec;
//! use rgb::Rgb;
//!
//! // Decode
//! let data: &[u8] = &[]; // your PNG bytes
//! let output = decode(data, None)?;
//! println!("{}x{}", output.info.width, output.info.height);
//!
//! // Encode
//! let pixels = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 64], 8, 8);
//! let encoded = encode_rgb8(pixels.as_ref(), None, &EncodeConfig::default())?;
//! # Ok::<(), zenpng::PngError>(())
//! ```
//!
//! # zencodec-types traits
//!
//! [`PngEncoderConfig`] implements [`zencodec_types::EncoderConfig`] and [`PngDecoderConfig`]
//! implements [`zencodec_types::DecoderConfig`] for use with multi-codec dispatchers.

#![forbid(unsafe_code)]

extern crate alloc;
extern crate std;

mod decode;
mod encode;
mod error;
#[cfg(feature = "quantize")]
mod indexed;
mod png_writer;
mod zencodec;

pub use decode::{PngChromaticities, PngDecodeOutput, PngInfo, PngLimits, decode, probe};
pub use encode::{
    EncodeConfig, encode_gray8, encode_gray16, encode_rgb8, encode_rgb16, encode_rgba8,
    encode_rgba16,
};
pub use error::PngError;
#[cfg(feature = "quantize")]
pub use indexed::{default_quantize_config, encode_indexed_rgba8};
pub use zencodec::{
    PngDecodeJob, PngDecoder, PngDecoderConfig, PngEncodeJob, PngEncoder, PngEncoderConfig,
    PngFrameDecoder, PngFrameEncoder,
};

// Re-export png types that appear in our public API.
pub use png::{Compression, Filter};
