//! PNG encoding and decoding with zencodec trait integration.
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
//! # Ok::<(), whereat::At<zenpng::PngError>>(())
//! ```
//!
//! # zencodec traits
//!
//! [`PngEncoderConfig`] implements [`zencodec::EncoderConfig`] and [`PngDecoderConfig`]
//! implements [`zencodec::DecoderConfig`] for use with multi-codec dispatchers.
//! Note: [`PngDecodeConfig`] (used in the quick start) is the lower-level decode config;
//! [`PngDecoderConfig`] wraps it for the zencodec trait interface.

#![forbid(unsafe_code)]

extern crate alloc;
extern crate std;

whereat::define_at_crate_info!();

mod chunk;
mod codec;
mod decode;
mod decoder;
/// PNG source analysis, compression assessment, and re-encoding recommendations.
pub mod detect;
mod encode;
mod encoder;
mod error;
mod gamut;
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
mod indexed;
mod optimize;
mod quantize;
mod simd;
mod types;
// #[cfg(feature = "zennode")]
// pub mod zennode_defs;

pub use codec::{
    PngAnimationFrameDecoder, PngAnimationFrameEncoder, PngDecodeJob, PngDecoder, PngDecoderConfig,
    PngEncodeJob, PngEncoder, PngEncoderConfig,
};
#[allow(deprecated)]
pub use decode::PngLimits;
pub use decode::{
    ApngDecodeOutput, ApngFrame, ApngFrameInfo, PhysUnit, PngBackground, PngChromaticities,
    PngDecodeConfig, PngDecodeOutput, PngInfo, PngTime, PngWarning, SignificantBits, TextChunk,
    decode, decode_apng, probe,
};
pub use encode::{
    ApngEncodeConfig, ApngFrameInput, DowncastFlags, EncodeConfig, encode_apng, encode_gray8,
    encode_gray16, encode_rgb8, encode_rgb16, encode_rgba8, encode_rgba16,
};
pub use error::PngError;
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
pub use indexed::{
    ApngEncodeParams, AutoEncodeResult, QualityGate, encode_apng_auto, encode_apng_indexed,
    encode_auto, encode_indexed,
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

/// Benchmarking access to the SIMD downcast predicates. Public via `_dev`
/// only; the names are unstable. See `benches/scan_predicates.rs`.
#[cfg(feature = "_dev")]
#[doc(hidden)]
pub mod __bench_scan {
    pub use crate::simd::scan::{
        FusedRequest, FusedResult, alpha_is_binary_rgba8, bit_replication_lossless_be16,
        fused_predicates_rgba8, fused_predicates_rgba8_cg, is_grayscale_rgb8, is_grayscale_rgba8,
        is_opaque_rgba8,
    };

    // Hand-written scalar references for benchmarking. Match the
    // semantics of the SIMD predicates exactly so the bench is
    // apples-to-apples — only the dispatch differs.

    pub fn scalar_is_opaque_rgba8(rgba: &[u8]) -> bool {
        let mut i = 0;
        while i + 4 <= rgba.len() {
            if rgba[i + 3] != 255 {
                return false;
            }
            i += 4;
        }
        true
    }

    pub fn scalar_is_grayscale_rgba8(rgba: &[u8]) -> bool {
        let mut i = 0;
        while i + 4 <= rgba.len() {
            if rgba[i] != rgba[i + 1] || rgba[i + 1] != rgba[i + 2] {
                return false;
            }
            i += 4;
        }
        true
    }

    pub fn scalar_alpha_is_binary_rgba8(rgba: &[u8]) -> bool {
        let mut i = 0;
        while i + 4 <= rgba.len() {
            let a = rgba[i + 3];
            if a != 0 && a != 255 {
                return false;
            }
            i += 4;
        }
        true
    }

    pub fn scalar_is_grayscale_rgb8(rgb: &[u8]) -> bool {
        let mut i = 0;
        while i + 3 <= rgb.len() {
            if rgb[i] != rgb[i + 1] || rgb[i + 1] != rgb[i + 2] {
                return false;
            }
            i += 3;
        }
        true
    }

    pub fn scalar_bit_replication_lossless_be16(be: &[u8]) -> bool {
        be.chunks_exact(2).all(|p| p[0] == p[1])
    }

    pub fn scalar_fused_predicates_rgba8(rgba: &[u8], req: FusedRequest) -> FusedResult {
        let mut o = req.check_opaque;
        let mut g = req.check_grayscale;
        let mut b = req.check_binary_alpha;
        let mut i = 0;
        while i + 4 <= rgba.len() && (o | g | b) {
            let r = rgba[i];
            let gg = rgba[i + 1];
            let bb = rgba[i + 2];
            let a = rgba[i + 3];
            if o && a != 255 {
                o = false;
            }
            if g && (r != gg || gg != bb) {
                g = false;
            }
            if b && a != 0 && a != 255 {
                b = false;
            }
            i += 4;
        }
        FusedResult {
            is_opaque: o,
            is_grayscale: g,
            is_binary_alpha: b,
        }
    }
}
