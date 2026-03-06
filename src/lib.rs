//! PNG encoding and decoding with zencodec-types trait integration.
//!
//! Uses `zenflate` for both compression and decompression, typed pixel buffers
//! (`imgref` + `rgb`), metadata roundtrip (ICC/EXIF/XMP), and optional palette
//! quantization via `zenquant`.
//!
//! # Quick start
//!
//! ```no_run
//! use zenpng::{decode, probe, encode_rgb8, EncodeConfig, PngDecodeConfig};
//! use enough::Unstoppable;
//! use imgref::ImgVec;
//! use rgb::Rgb;
//!
//! // Decode
//! let data: &[u8] = &[]; // your PNG bytes
//! let output = decode(data, &PngDecodeConfig::default(), &Unstoppable)?;
//! println!("{}x{}", output.info.width, output.info.height);
//!
//! // Encode
//! let pixels = ImgVec::new(vec![Rgb { r: 0u8, g: 0, b: 0 }; 64], 8, 8);
//! let encoded = encode_rgb8(pixels.as_ref(), None, &EncodeConfig::default(), &Unstoppable, &Unstoppable)?;
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

mod chunk;
mod decode;
mod decoder;
mod encode;
mod encoder;
mod error;
#[cfg(feature = "quantize")]
mod indexed;
mod optimize;
mod quantize;
mod simd;
mod types;
mod zencodec;

#[allow(deprecated)]
pub use decode::PngLimits;
pub use decode::{
    ApngDecodeOutput, ApngFrame, ApngFrameInfo, PngChromaticities, PngDecodeConfig,
    PngDecodeOutput, PngInfo, PngWarning, decode, decode_apng, probe,
};
pub use encode::{
    ApngEncodeConfig, ApngFrameInput, EncodeConfig, encode_apng, encode_gray8, encode_gray16,
    encode_rgb8, encode_rgb16, encode_rgba8, encode_rgba16,
};
pub use error::PngError;
#[cfg(feature = "quantize")]
pub use indexed::{
    ApngEncodeParams, ApngQuantizeParams, AutoEncodeResult, QualityGate, default_quantize_config,
    encode_apng_auto, encode_apng_auto_q, encode_apng_indexed, encode_apng_indexed_q, encode_auto,
    encode_indexed, encode_indexed_rgba8, encode_rgba8_auto,
};
#[cfg(feature = "imagequant")]
pub use quantize::ImagequantQuantizer;
#[cfg(feature = "quantette")]
pub use quantize::QuantetteQuantizer;
#[cfg(feature = "quantize")]
pub use quantize::ZenquantQuantizer;
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
pub use quantize::default_quantizer;
pub use quantize::{
    MultiFrameOutput, QuantizeOutput, Quantizer, available_backends, quantizer_by_name,
};
pub use zencodec::{
    PngDecodeJob, PngDecoder, PngDecoderConfig, PngEncodeJob, PngEncoder, PngEncoderConfig,
    PngFrameDecoder, PngFrameEncoder,
};

pub use types::{Compression, Filter};

#[cfg(feature = "_dev")]
#[doc(hidden)]
pub use crate::encode::{encode_rgb8_with_stats, encode_rgba8_with_stats};
#[cfg(feature = "_dev")]
#[doc(hidden)]
pub use crate::encoder::{PhaseStat, PhaseStats};
#[cfg(feature = "_dev")]
#[doc(hidden)]
pub fn __bench_unfilter_row(filter_type: u8, row: &mut [u8], prev: &[u8], bpp: usize) {
    simd::bench_unfilter_row(filter_type, row, prev, bpp);
}
