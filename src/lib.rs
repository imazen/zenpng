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

mod alloc_util;
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
/// Calibrated encode/decode resource estimation (peak memory + time).
pub mod heuristics;
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
mod indexed;
/// Cross-codec uniformity bundle (`__expert`-gated). Mirrors
/// `zenjpeg`'s `InternalParams` so external pipelines (calibration
/// sweeps, picker training) can drive every codec the same way. See
/// [`internal_params::InternalParams`] and
/// [`EncodeConfig::with_internal_params`].
#[cfg(feature = "__expert")]
pub mod internal_params;
mod optimize;
mod quantize;
mod simd;
mod types;

/// Sweep-plan construction over the encoder knob space (variant-
/// generation playbook; see `zenjpeg/docs/VARIANT_GENERATION.md`).
/// The entire curated space is trial-class (lossless).
pub mod sweep;
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
#[cfg(feature = "__expert")]
pub use internal_params::InternalParams;
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

// ---------------------------------------------------------------------------
// One-shot convenience functions
//
// The shortest path for the two most common jobs — encode tightly-packed RGBA8
// bytes to a PNG, and decode any PNG back to tightly-packed RGBA8 + dimensions
// — with sane defaults, for someone who hasn't read the rest of the docs. Both
// are purely additive wrappers over the builder/config path.
//
// Reach for the typed [`encode_rgba8`] (`ImgRef<Rgba<u8>>` + metadata + a custom
// [`EncodeConfig`] + cancellation) or [`decode`] / [`PngDecodeConfig`] when you
// need a specific filter/effort, embedded ICC/EXIF/XMP, 16-bit / grayscale /
// indexed I/O, resource limits, or cooperative cancellation.
//
// Note: the typed [`encode_rgba8`] (taking `ImgRef<Rgba<u8>>`) already occupies
// that name, so the byte-slice one-shot is [`encode_rgba8_bytes`] — it sits
// right beside it and signals the flat-`&[u8]` input.
// ---------------------------------------------------------------------------

/// Encode tightly-packed 8-bit RGBA pixels to a PNG in one call.
///
/// `rgba` must be exactly `width * height * 4` bytes, row-major with no stride
/// padding (`R, G, B, A` per pixel). Uses the default [`EncodeConfig`]
/// (balanced lossless compression, no embedded metadata). For a specific
/// filter/effort, embedded ICC/EXIF/XMP, or 16-bit / grayscale / indexed
/// output, use the typed [`encode_rgba8`] / [`EncodeConfig`] path.
///
/// # Errors
/// Returns [`PngError::InvalidInput`] if `rgba.len()` is not exactly
/// `width * height * 4` bytes (this also rejects dimensions that overflow
/// `usize`), plus any encode error bubbled up from the underlying pipeline.
///
/// ```
/// use zenpng::{encode_rgba8_bytes, decode_rgba8};
///
/// // 2×2 RGBA, tightly packed (width * height * 4 bytes)
/// let (width, height) = (2u32, 2u32);
/// let rgba = vec![
///     255, 0, 0, 255,    0, 255, 0, 255,
///     0, 0, 255, 255,    255, 255, 255, 255,
/// ];
///
/// let png = encode_rgba8_bytes(&rgba, width, height)?;
/// let (pixels, w, h) = decode_rgba8(&png)?;
///
/// assert_eq!((w, h), (width, height));
/// assert_eq!(pixels, rgba); // PNG is lossless — exact round-trip
/// # Ok::<(), whereat::At<zenpng::PngError>>(())
/// ```
pub fn encode_rgba8_bytes(rgba: &[u8], width: u32, height: u32) -> crate::error::Result<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if expected != Some(rgba.len()) {
        return Err(whereat::at!(crate::error::PngError::InvalidInput(
            alloc::format!(
                "encode_rgba8_bytes: expected width*height*4 bytes for {width}x{height}, got {} bytes",
                rgba.len()
            )
        )));
    }
    // `cast_slice` cannot panic here: length is a verified multiple of 4 and
    // `Rgba<u8>` has alignment 1. The `ImgRef` is tight (stride == width); the
    // underlying typed `encode_rgba8` is itself stride-correct.
    let pixels: &[rgb::Rgba<u8>] = bytemuck::cast_slice(rgba);
    let img = imgref::ImgRef::new(pixels, width as usize, height as usize);
    crate::encode::encode_rgba8(
        img,
        None,
        &crate::EncodeConfig::default(),
        &enough::Unstoppable,
        &enough::Unstoppable,
    )
}

/// Decode a PNG (any color type / bit depth) to tightly-packed 8-bit RGBA in
/// one call.
///
/// Returns `(rgba, width, height)` where `rgba` is exactly `width * height * 4`
/// bytes (`R, G, B, A` per pixel, no stride padding). Grayscale, indexed, RGB
/// and 16-bit sources are all normalized to 8-bit RGBA; opaque sources get
/// `A = 255`. Uses the default [`PngDecodeConfig`] (checksums skipped for
/// speed, default resource limits). For 16-bit output, the native pixel buffer,
/// decode warnings, or strict checksum verification, use [`decode`] /
/// [`PngDecodeConfig`].
///
/// # Errors
/// Returns a [`PngError`] if `png` is not a valid PNG or a resource limit is
/// exceeded.
///
/// ```
/// use zenpng::{encode_rgba8_bytes, decode_rgba8};
///
/// // 2×2 RGBA, tightly packed (width * height * 4 bytes)
/// let (width, height) = (2u32, 2u32);
/// let rgba = vec![
///     255, 0, 0, 255,    0, 255, 0, 255,
///     0, 0, 255, 255,    255, 255, 255, 255,
/// ];
///
/// let png = encode_rgba8_bytes(&rgba, width, height)?;
/// let (pixels, w, h) = decode_rgba8(&png)?;
///
/// assert_eq!((w, h), (width, height));
/// assert_eq!(pixels, rgba); // PNG is lossless — exact round-trip
/// # Ok::<(), whereat::At<zenpng::PngError>>(())
/// ```
pub fn decode_rgba8(png: &[u8]) -> crate::error::Result<(Vec<u8>, u32, u32)> {
    use zenpixels_convert::PixelBufferConvertTypedExt as _;
    let output = crate::decode::decode(
        png,
        &crate::PngDecodeConfig::default(),
        &enough::Unstoppable,
    )?;
    let width = output.info.width;
    let height = output.info.height;
    // `to_rgba8()` normalizes any native color type to RGBA8;
    // `copy_to_contiguous_bytes()` strips any stride padding.
    let rgba = output.pixels.to_rgba8().copy_to_contiguous_bytes();
    Ok((rgba, width, height))
}

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
