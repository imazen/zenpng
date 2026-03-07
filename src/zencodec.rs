//! zencodec-types trait implementations for PNG.
//!
//! Provides [`PngEncoderConfig`] and [`PngDecoderConfig`] types that implement the
//! [`EncoderConfig`] / [`DecoderConfig`] traits from zencodec-types.
#![allow(dead_code)]

extern crate std;

use alloc::vec::Vec;

use whereat::{At, ErrorAtExt};
use zc::decode::{DecodeCapabilities, DecodeFrame, DecodeOutput, OutputInfo};
use zc::encode::{EncodeCapabilities, EncodeOutput};
use zc::{ImageFormat, ImageInfo, MetadataView, ResourceLimits};
use zenpixels::{PixelDescriptor, PixelSlice, PixelSliceMut};

use crate::decode::PngDecodeConfig;
use crate::encode::EncodeConfig;
use crate::error::PngError;

/// Default encode timeout: 120 seconds.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

// ── Supported descriptors ────────────────────────────────────────────

static ENCODE_DESCRIPTORS: &[PixelDescriptor] = &[
    PixelDescriptor::RGB8_SRGB,
    PixelDescriptor::RGBA8_SRGB,
    PixelDescriptor::GRAY8_SRGB,
    PixelDescriptor::BGRA8_SRGB,
    PixelDescriptor::RGB16_SRGB,
    PixelDescriptor::RGBA16_SRGB,
    PixelDescriptor::GRAY16_SRGB,
    PixelDescriptor::RGBF32_LINEAR,
    PixelDescriptor::RGBAF32_LINEAR,
    PixelDescriptor::GRAYF32_LINEAR,
];

static DECODE_DESCRIPTORS: &[PixelDescriptor] = &[
    PixelDescriptor::RGB8_SRGB,
    PixelDescriptor::RGBA8_SRGB,
    PixelDescriptor::GRAY8_SRGB,
    PixelDescriptor::BGRA8_SRGB,
    PixelDescriptor::RGB16_SRGB,
    PixelDescriptor::RGBA16_SRGB,
    PixelDescriptor::GRAY16_SRGB,
    PixelDescriptor::RGBF32_LINEAR,
    PixelDescriptor::RGBAF32_LINEAR,
    PixelDescriptor::GRAYF32_LINEAR,
];

// ── PngEncoderConfig ─────────────────────────────────────────────────

/// PNG encoder configuration implementing [`EncoderConfig`](zc::EncoderConfig).
///
/// Use [`with_compression`](PngEncoderConfig::with_compression) to control compression level.
/// When the `quantize` feature is enabled, setting quality < 100 enables
/// auto-indexed encoding via [`encode_auto`](crate::encode_auto),
/// which quantizes RGBA8 images to ≤256 colors when quality is acceptable.
#[derive(Clone, Debug)]
pub struct PngEncoderConfig {
    config: EncodeConfig,
    effort: Option<i32>,
    quality: Option<f32>,
    lossless: bool,
}

impl PngEncoderConfig {
    /// Create a default PNG encoder config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: EncodeConfig::default(),
            effort: None,
            quality: None,
            lossless: true,
        }
    }

    /// Set PNG compression level directly.
    #[must_use]
    pub fn with_compression(mut self, compression: crate::Compression) -> Self {
        self.config.compression = compression;
        self
    }

    /// Set PNG row filter strategy directly.
    #[must_use]
    pub fn with_filter(mut self, filter: crate::Filter) -> Self {
        self.config.filter = filter;
        self
    }

    /// Convenience: encode RGB8 pixels in one call.
    pub fn encode_rgb8(
        &self,
        img: imgref::ImgRef<'_, Rgb<u8>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGBA8 pixels in one call.
    pub fn encode_rgba8(
        &self,
        img: imgref::ImgRef<'_, Rgba<u8>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode Gray8 pixels in one call.
    pub fn encode_gray8(
        &self,
        img: imgref::ImgRef<'_, Gray<u8>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGB16 pixels in one call.
    pub fn encode_rgb16(
        &self,
        img: imgref::ImgRef<'_, Rgb<u16>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGBA16 pixels in one call.
    pub fn encode_rgba16(
        &self,
        img: imgref::ImgRef<'_, Rgba<u16>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode Gray16 pixels in one call.
    pub fn encode_gray16(
        &self,
        img: imgref::ImgRef<'_, Gray<u16>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGB F32 pixels in one call.
    pub fn encode_rgb_f32(
        &self,
        img: imgref::ImgRef<'_, Rgb<f32>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGBA F32 pixels in one call.
    pub fn encode_rgba_f32(
        &self,
        img: imgref::ImgRef<'_, Rgba<f32>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode Gray F32 pixels in one call.
    pub fn encode_gray_f32(
        &self,
        img: imgref::ImgRef<'_, Gray<f32>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode BGRA8 pixels (swizzles to RGBA) in one call.
    pub fn encode_bgra8(
        &self,
        img: imgref::ImgRef<'_, rgb::alt::BGRA<u8>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zc::encode::{EncodeJob, Encoder, EncoderConfig};
        self.job().encoder()?.encode(PixelSlice::from(img).erase())
    }
}

impl Default for PngEncoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

fn effort_to_compression(effort: i32) -> crate::Compression {
    use crate::Compression;
    match effort {
        ..=0 => Compression::None,
        1 => Compression::Fastest,
        2 => Compression::Turbo,
        3 => Compression::Fast,
        4 => Compression::Balanced,
        5 => Compression::Thorough,
        6 => Compression::High,
        7 => Compression::Aggressive,
        8 => Compression::Intense,
        9 => Compression::Crush,
        10 => Compression::Maniac,
        11 => Compression::Brag,
        _ => Compression::Minutes,
    }
}

/// Convert generic quality (0–100) to MPE threshold.
///
/// Piecewise-linear interpolation calibrated against JPEG quality equivalences
/// from zenquant's MPE↔SSIM2↔butteraugli corpus data (1992 images).
///
/// | quality | ≈ JPEG q | MPE    |
/// |---------|----------|--------|
/// | 100     | lossless | 0.0    |
/// | 99      | —        | 0.001  |
/// | 95      | 95       | 0.008  |
/// | 90      | 90       | 0.012  |
/// | 85      | 85       | 0.015  |
/// | 75      | 75       | 0.020  |
/// | 50      | 50       | 0.028  |
/// | 30      | 30       | 0.034  |
/// | 0       | —        | 0.100  |
///
/// Values above q99 are near-lossless; values below q30 are increasingly lossy.
fn quality_to_mpe(quality: f32) -> f32 {
    // (quality, mpe) — sorted descending by quality
    // Near-lossless range (99–100) plus JPEG-equivalent calibration points.
    const TABLE: [(f32, f32); 11] = [
        (100.0, 0.0),
        (99.0, 0.001),
        (95.0, 0.008),
        (90.0, 0.012),
        (85.0, 0.015),
        (80.0, 0.017),
        (75.0, 0.020),
        (60.0, 0.025),
        (50.0, 0.028),
        (30.0, 0.034),
        (0.0, 0.100),
    ];

    let quality = quality.clamp(0.0, 100.0);

    // Exact endpoint matches
    if quality >= TABLE[0].0 {
        return TABLE[0].1;
    }
    let last = TABLE.len() - 1;
    if quality <= TABLE[last].0 {
        return TABLE[last].1;
    }

    // Find the bracketing interval (table is sorted descending by quality)
    for i in 0..last {
        let (q_hi, mpe_hi) = TABLE[i];
        let (q_lo, mpe_lo) = TABLE[i + 1];
        if quality >= q_lo {
            let t = (q_hi - quality) / (q_hi - q_lo);
            return mpe_hi + t * (mpe_lo - mpe_hi);
        }
    }
    TABLE[last].1
}

static PNG_ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_icc(true)
    .with_exif(true)
    .with_xmp(true)
    .with_cicp(true)
    .with_cancel(true)
    .with_animation(true)
    .with_lossless(true)
    .with_lossy(true)
    .with_native_gray(true)
    .with_native_16bit(true)
    .with_native_alpha(true)
    .with_enforces_max_pixels(true)
    .with_enforces_max_memory(true)
    .with_effort_range(0, 12)
    .with_quality_range(0.0, 100.0);

impl zc::encode::EncoderConfig for PngEncoderConfig {
    type Error = At<PngError>;
    type Job<'a> = PngEncodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Png
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ENCODE_DESCRIPTORS
    }

    fn capabilities() -> &'static EncodeCapabilities {
        &PNG_ENCODE_CAPS
    }

    fn with_generic_effort(mut self, effort: i32) -> Self {
        self.effort = Some(effort);
        self.config.compression = effort_to_compression(effort);
        self
    }

    fn generic_effort(&self) -> Option<i32> {
        self.effort
    }

    fn with_generic_quality(mut self, quality: f32) -> Self {
        self.quality = Some(quality);
        // Setting quality < 100 implicitly enables lossy (auto-indexed) mode
        if quality < 100.0 {
            self.lossless = false;
        }
        self
    }

    fn generic_quality(&self) -> Option<f32> {
        self.quality
    }

    fn with_lossless(mut self, lossless: bool) -> Self {
        self.lossless = lossless;
        self
    }

    fn is_lossless(&self) -> Option<bool> {
        Some(self.lossless)
    }

    fn job(&self) -> PngEncodeJob<'_> {
        PngEncodeJob {
            config: self,
            stop: None,
            metadata: None,
            limits: None,
            canvas_width: 0,
            canvas_height: 0,
            loop_count: None,
        }
    }
}

// ── PngEncodeJob ─────────────────────────────────────────────────────

/// Per-operation PNG encode job.
pub struct PngEncodeJob<'a> {
    config: &'a PngEncoderConfig,
    stop: Option<&'a dyn enough::Stop>,
    metadata: Option<&'a MetadataView<'a>>,
    limits: Option<ResourceLimits>,
    canvas_width: u32,
    canvas_height: u32,
    loop_count: Option<u32>,
}

impl<'a> zc::encode::EncodeJob<'a> for PngEncodeJob<'a> {
    type Error = At<PngError>;
    type Enc = PngEncoder<'a>;
    type FrameEnc = PngFrameEncoder;

    fn with_stop(mut self, stop: &'a dyn enough::Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_metadata(mut self, meta: &'a MetadataView<'a>) -> Self {
        self.metadata = Some(meta);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn with_canvas_size(mut self, width: u32, height: u32) -> Self {
        self.canvas_width = width;
        self.canvas_height = height;
        self
    }

    fn with_loop_count(mut self, count: Option<u32>) -> Self {
        self.loop_count = count;
        self
    }

    fn encoder(self) -> Result<PngEncoder<'a>, At<PngError>> {
        Ok(PngEncoder {
            config: self.config,
            stop: self.stop,
            metadata: self.metadata,
            limits: self.limits,
        })
    }

    fn frame_encoder(self) -> Result<PngFrameEncoder, At<PngError>> {
        let owned_meta = self.metadata.map(OwnedMetadata::from_metadata);
        let mut enc = PngFrameEncoder::new(
            self.config.config.clone(),
            self.canvas_width,
            self.canvas_height,
            owned_meta,
        );
        enc.loop_count = self.loop_count.unwrap_or(0);
        Ok(enc)
    }
}

// ── PngEncoder ───────────────────────────────────────────────────────

/// Single-image PNG encoder.
pub struct PngEncoder<'a> {
    config: &'a PngEncoderConfig,
    stop: Option<&'a dyn enough::Stop>,
    metadata: Option<&'a MetadataView<'a>>,
    limits: Option<ResourceLimits>,
}

impl<'a> PngEncoder<'a> {
    fn do_encode(
        &self,
        bytes: &[u8],
        w: u32,
        h: u32,
        color_type: crate::encode::ColorType,
    ) -> Result<EncodeOutput, At<PngError>> {
        self.do_encode_with_depth(bytes, w, h, color_type, crate::encode::BitDepth::Eight)
    }

    fn do_encode_with_depth(
        &self,
        bytes: &[u8],
        w: u32,
        h: u32,
        color_type: crate::encode::ColorType,
        bit_depth: crate::encode::BitDepth,
    ) -> Result<EncodeOutput, At<PngError>> {
        let cancel: &dyn enough::Stop = self.stop.unwrap_or(&enough::Unstoppable);
        // Pre-flight stop check
        cancel.check().map_err(PngError::from)?;
        // Pre-flight limit checks
        if let Some(ref limits) = self.limits {
            limits
                .check_dimensions(w, h)
                .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
            let channels: u64 = match color_type {
                crate::encode::ColorType::Grayscale => 1,
                crate::encode::ColorType::Rgb => 3,
                crate::encode::ColorType::GrayscaleAlpha => 2,
                crate::encode::ColorType::Rgba => 4,
            };
            let depth_bytes: u64 = match bit_depth {
                crate::encode::BitDepth::Eight => 1,
                crate::encode::BitDepth::Sixteen => 2,
            };
            let bpp = channels * depth_bytes;
            let estimated_mem = w as u64 * h as u64 * bpp;
            limits
                .check_memory(estimated_mem)
                .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
        }
        let config = &self.config.config;
        let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
        let deadline = almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);
        let data = crate::encode::encode_raw(
            bytes,
            w,
            h,
            color_type,
            bit_depth,
            self.metadata,
            config,
            cancel,
            &deadline,
        )?;
        Ok(EncodeOutput::new(data, ImageFormat::Png))
    }
}

impl zc::encode::Encoder for PngEncoder<'_> {
    type Error = At<PngError>;

    fn reject(op: zc::UnsupportedOperation) -> At<PngError> {
        PngError::from(op).start_at()
    }

    fn preferred_strip_height(&self) -> u32 {
        1 // PNG uses row-at-a-time filtering
    }

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, At<PngError>> {
        use linear_srgb::default::linear_to_srgb_u8;
        use zenpixels::PixelFormat;

        let w = pixels.width();
        let h = pixels.rows();

        match pixels.descriptor().pixel_format() {
            PixelFormat::Rgb8 => {
                let bytes = contiguous_bytes(&pixels);
                self.do_encode(&bytes, w, h, crate::encode::ColorType::Rgb)
            }
            PixelFormat::Rgba8 => {
                // Auto-indexed path when not lossless and quality < 100
                #[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
                if !self.config.lossless
                    && let Some(q) = self.config.quality
                    && q < 100.0
                {
                    let bytes = contiguous_bytes(&pixels);
                    let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&bytes);
                    let img = imgref::Img::new(rgba, w as usize, h as usize);
                    let mpe = quality_to_mpe(q);
                    let cancel: &dyn enough::Stop = self.stop.unwrap_or(&enough::Unstoppable);
                    let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
                    let deadline =
                        almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);
                    let quantizer = crate::default_quantizer();
                    let result = crate::encode_auto(
                        img,
                        &self.config.config,
                        &*quantizer,
                        crate::QualityGate::MaxMpe(mpe),
                        self.metadata,
                        cancel,
                        &deadline,
                    )?;
                    return Ok(EncodeOutput::new(result.data, ImageFormat::Png));
                }

                let bytes = contiguous_bytes(&pixels);
                self.do_encode(&bytes, w, h, crate::encode::ColorType::Rgba)
            }
            PixelFormat::Gray8 => {
                let bytes = contiguous_bytes(&pixels);
                self.do_encode(&bytes, w, h, crate::encode::ColorType::Grayscale)
            }
            PixelFormat::Rgb16 => {
                let bytes = contiguous_bytes(&pixels);
                let be = native_to_be_16(&bytes);
                self.do_encode_with_depth(
                    &be,
                    w,
                    h,
                    crate::encode::ColorType::Rgb,
                    crate::encode::BitDepth::Sixteen,
                )
            }
            PixelFormat::Rgba16 => {
                let bytes = contiguous_bytes(&pixels);
                let be = native_to_be_16(&bytes);
                self.do_encode_with_depth(
                    &be,
                    w,
                    h,
                    crate::encode::ColorType::Rgba,
                    crate::encode::BitDepth::Sixteen,
                )
            }
            PixelFormat::Gray16 => {
                let bytes = contiguous_bytes(&pixels);
                let be = native_to_be_16(&bytes);
                self.do_encode_with_depth(
                    &be,
                    w,
                    h,
                    crate::encode::ColorType::Grayscale,
                    crate::encode::BitDepth::Sixteen,
                )
            }
            PixelFormat::RgbF32 => {
                let src = contiguous_bytes(&pixels);
                let srgb: Vec<u8> = src
                    .chunks_exact(12)
                    .flat_map(|c| {
                        let r = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                        let g = f32::from_ne_bytes([c[4], c[5], c[6], c[7]]);
                        let b = f32::from_ne_bytes([c[8], c[9], c[10], c[11]]);
                        [
                            linear_to_srgb_u8(r.clamp(0.0, 1.0)),
                            linear_to_srgb_u8(g.clamp(0.0, 1.0)),
                            linear_to_srgb_u8(b.clamp(0.0, 1.0)),
                        ]
                    })
                    .collect();
                self.do_encode(&srgb, w, h, crate::encode::ColorType::Rgb)
            }
            PixelFormat::RgbaF32 => {
                let src = contiguous_bytes(&pixels);
                let srgb: Vec<u8> = src
                    .chunks_exact(16)
                    .flat_map(|c| {
                        let r = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                        let g = f32::from_ne_bytes([c[4], c[5], c[6], c[7]]);
                        let b = f32::from_ne_bytes([c[8], c[9], c[10], c[11]]);
                        let a = f32::from_ne_bytes([c[12], c[13], c[14], c[15]]);
                        [
                            linear_to_srgb_u8(r.clamp(0.0, 1.0)),
                            linear_to_srgb_u8(g.clamp(0.0, 1.0)),
                            linear_to_srgb_u8(b.clamp(0.0, 1.0)),
                            (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                        ]
                    })
                    .collect();

                // Auto-indexed path when not lossless and quality < 100
                #[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
                if !self.config.lossless
                    && let Some(q) = self.config.quality
                    && q < 100.0
                {
                    let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&srgb);
                    let img = imgref::Img::new(rgba, w as usize, h as usize);
                    let mpe = quality_to_mpe(q);
                    let cancel: &dyn enough::Stop = self.stop.unwrap_or(&enough::Unstoppable);
                    let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
                    let deadline =
                        almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);
                    let quantizer = crate::default_quantizer();
                    let result = crate::encode_auto(
                        img,
                        &self.config.config,
                        &*quantizer,
                        crate::QualityGate::MaxMpe(mpe),
                        self.metadata,
                        cancel,
                        &deadline,
                    )?;
                    return Ok(EncodeOutput::new(result.data, ImageFormat::Png));
                }

                self.do_encode(&srgb, w, h, crate::encode::ColorType::Rgba)
            }
            PixelFormat::GrayF32 => {
                let src = contiguous_bytes(&pixels);
                let srgb: Vec<u8> = src
                    .chunks_exact(4)
                    .map(|c| {
                        let v = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                        linear_to_srgb_u8(v.clamp(0.0, 1.0))
                    })
                    .collect();
                self.do_encode(&srgb, w, h, crate::encode::ColorType::Grayscale)
            }
            PixelFormat::Bgra8 => {
                let raw = contiguous_bytes(&pixels);
                let rgba: Vec<u8> = raw
                    .chunks_exact(4)
                    .flat_map(|c| [c[2], c[1], c[0], c[3]])
                    .collect();
                self.do_encode(&rgba, w, h, crate::encode::ColorType::Rgba)
            }
            _ => Err(PngError::from(zc::UnsupportedOperation::PixelFormat).start_at()),
        }
    }
}

// ── PngFrameEncoder ──────────────────────────────────────────────────

/// Accumulated frame data for APNG encoding.
struct AccumulatedFrame {
    pixels: Vec<u8>, // RGBA8 canvas-sized
    duration_ms: u32,
}

/// Owned copy of image metadata for frame encoder (avoids lifetime issues).
struct OwnedMetadata {
    icc_profile: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
    cicp: Option<zc::Cicp>,
    content_light_level: Option<zc::ContentLightLevel>,
    mastering_display: Option<zc::MasteringDisplay>,
}

impl OwnedMetadata {
    fn from_metadata(meta: &MetadataView<'_>) -> Self {
        Self {
            icc_profile: meta.icc_profile.map(|s| s.to_vec()),
            exif: meta.exif.map(|s| s.to_vec()),
            xmp: meta.xmp.map(|s| s.to_vec()),
            cicp: meta.cicp,
            content_light_level: meta.content_light_level,
            mastering_display: meta.mastering_display,
        }
    }

    fn as_metadata(&self) -> MetadataView<'_> {
        let mut meta = MetadataView::none();
        meta.icc_profile = self.icc_profile.as_deref();
        meta.exif = self.exif.as_deref();
        meta.xmp = self.xmp.as_deref();
        meta.cicp = self.cicp;
        meta.content_light_level = self.content_light_level;
        meta.mastering_display = self.mastering_display;
        meta
    }
}

/// APNG frame-by-frame encoder implementing [`FrameEncoder`](zc::encode::FrameEncoder).
///
/// Accumulates canvas-sized RGBA8 frames, then encodes them all on [`finish()`](PngFrameEncoder::do_finish).
pub struct PngFrameEncoder {
    frames: Vec<AccumulatedFrame>,
    canvas_width: u32,
    canvas_height: u32,
    config: crate::encode::EncodeConfig,
    metadata: Option<OwnedMetadata>,
    loop_count: u32,
    /// In-progress frame being built row-by-row.
    building_frame: Option<BuildingFrame>,
}

/// State for row-by-row frame construction.
struct BuildingFrame {
    pixels: Vec<u8>,
    duration_ms: u32,
    rows_pushed: u32,
}

impl PngFrameEncoder {
    fn new(
        config: crate::encode::EncodeConfig,
        canvas_width: u32,
        canvas_height: u32,
        metadata: Option<OwnedMetadata>,
    ) -> Self {
        Self {
            frames: Vec::new(),
            canvas_width,
            canvas_height,
            config,
            metadata,
            loop_count: 0,
            building_frame: None,
        }
    }

    /// Extract RGBA8 bytes from a PixelSlice, converting as needed.
    fn pixels_to_rgba8(pixels: &PixelSlice<'_>) -> Result<Vec<u8>, PngError> {
        let desc = pixels.descriptor();
        match (desc.channel_type(), desc.layout()) {
            (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Rgba) => {
                Ok(contiguous_bytes(pixels).into_owned())
            }
            (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Bgra) => {
                let src = contiguous_bytes(pixels);
                Ok(src
                    .chunks_exact(4)
                    .flat_map(|c| [c[2], c[1], c[0], c[3]])
                    .collect())
            }
            (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Rgb) => {
                let src = contiguous_bytes(pixels);
                Ok(src
                    .chunks_exact(3)
                    .flat_map(|c| [c[0], c[1], c[2], 255])
                    .collect())
            }
            _ => Err(PngError::InvalidInput(alloc::format!(
                "APNG frame encoder: unsupported pixel format {:?}, need RGBA8",
                desc
            ))),
        }
    }
}

impl zc::encode::FrameEncoder for PngFrameEncoder {
    type Error = At<PngError>;

    fn reject(op: zc::UnsupportedOperation) -> At<PngError> {
        PngError::from(op).start_at()
    }

    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), At<PngError>> {
        let rgba = Self::pixels_to_rgba8(&pixels)?;
        self.frames.push(AccumulatedFrame {
            pixels: rgba,
            duration_ms,
        });
        Ok(())
    }

    fn finish(self) -> Result<EncodeOutput, At<PngError>> {
        self.do_finish().map_err(ErrorAtExt::start_at)
    }
}

impl PngFrameEncoder {
    fn do_finish(self) -> Result<EncodeOutput, PngError> {
        if self.frames.is_empty() {
            return Err(PngError::InvalidInput(
                "APNG frame encoder: no frames pushed".into(),
            ));
        }

        // Convert accumulated frames to ApngFrameInput
        let inputs: Vec<crate::encode::ApngFrameInput<'_>> = self
            .frames
            .iter()
            .map(|f| {
                // Convert ms to delay_num/delay_den
                // Use den=1000 for ms precision
                crate::encode::ApngFrameInput {
                    pixels: &f.pixels,
                    delay_num: f.duration_ms.min(65535) as u16,
                    delay_den: 1000,
                }
            })
            .collect();

        let apng_config = crate::encode::ApngEncodeConfig {
            encode: self.config.clone(),
            num_plays: self.loop_count,
        };

        let meta_tmp = self.metadata.as_ref().map(|m| m.as_metadata());
        let metadata_ref = meta_tmp.as_ref();
        let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
        let deadline = almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);

        let data = crate::encode::encode_apng(
            &inputs,
            self.canvas_width,
            self.canvas_height,
            &apng_config,
            metadata_ref,
            &enough::Unstoppable,
            &deadline,
        )?;

        Ok(EncodeOutput::new(data, ImageFormat::Png))
    }
}

// ── PngDecoderConfig ─────────────────────────────────────────────────

/// PNG decoder configuration implementing [`DecoderConfig`](zc::DecoderConfig).
#[derive(Clone, Debug)]
pub struct PngDecoderConfig {
    limits: ResourceLimits,
}

impl PngDecoderConfig {
    /// Create a default PNG decoder config with safe resource limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: ResourceLimits::none()
                .with_max_pixels(PngDecodeConfig::DEFAULT_MAX_PIXELS)
                .with_max_memory(PngDecodeConfig::DEFAULT_MAX_MEMORY),
        }
    }
}

impl PngDecoderConfig {
    /// Convenience: decode in one call (native format).
    pub fn decode(&self, data: &[u8]) -> Result<DecodeOutput, At<PngError>> {
        use zc::decode::{Decode, DecodeJob, DecoderConfig};
        self.job().decoder(data, &[])?.decode()
    }

    /// Convenience: probe image header.
    pub fn probe(&self, data: &[u8]) -> Result<ImageInfo, At<PngError>> {
        use zc::decode::{DecodeJob, DecoderConfig};
        self.job().probe(data)
    }

    /// Convenience: probe header (alias for backwards compatibility).
    pub fn probe_header(&self, data: &[u8]) -> Result<ImageInfo, At<PngError>> {
        self.probe(data)
    }

    /// Convenience: decode into an RGB8 target buffer.
    pub fn decode_into_rgb8(
        &self,
        data: &[u8],
        dst: imgref::ImgRefMut<'_, Rgb<u8>>,
    ) -> Result<ImageInfo, At<PngError>> {
        let mut dst: PixelSliceMut<'_> = PixelSliceMut::from(dst).erase();
        let output = self.decode(data)?;
        let info = output.info().clone();
        let pixels = output.into_buffer();
        let src = to_rgb8(pixels);
        copy_rows_u8(&src, &mut dst);
        Ok(info)
    }

    /// Convenience: decode into an RGB16 target buffer.
    pub fn decode_into_rgb16(
        &self,
        data: &[u8],
        dst: imgref::ImgRefMut<'_, Rgb<u16>>,
    ) -> Result<ImageInfo, At<PngError>> {
        let mut dst: PixelSliceMut<'_> = PixelSliceMut::from(dst).erase();
        let output = self.decode(data)?;
        let info = output.info().clone();
        let pixels = output.into_buffer();
        decode_into_rgb16(pixels, &mut dst);
        Ok(info)
    }

    /// Convenience: decode into an RGB F32 target buffer.
    pub fn decode_into_rgb_f32(
        &self,
        data: &[u8],
        dst: imgref::ImgRefMut<'_, Rgb<f32>>,
    ) -> Result<ImageInfo, At<PngError>> {
        let mut dst: PixelSliceMut<'_> = PixelSliceMut::from(dst).erase();
        let output = self.decode(data)?;
        let info = output.info().clone();
        let pixels = output.into_buffer();
        decode_into_rgb_f32(pixels, &mut dst);
        Ok(info)
    }

    /// Convenience: decode into an RGBA F32 target buffer.
    pub fn decode_into_rgba_f32(
        &self,
        data: &[u8],
        dst: imgref::ImgRefMut<'_, Rgba<f32>>,
    ) -> Result<ImageInfo, At<PngError>> {
        let mut dst: PixelSliceMut<'_> = PixelSliceMut::from(dst).erase();
        let output = self.decode(data)?;
        let info = output.info().clone();
        let pixels = output.into_buffer();
        decode_into_rgba_f32(pixels, &mut dst);
        Ok(info)
    }

    /// Convenience: decode into a Gray F32 target buffer.
    pub fn decode_into_gray_f32(
        &self,
        data: &[u8],
        dst: imgref::ImgRefMut<'_, Gray<f32>>,
    ) -> Result<ImageInfo, At<PngError>> {
        let mut dst: PixelSliceMut<'_> = PixelSliceMut::from(dst).erase();
        let output = self.decode(data)?;
        let info = output.info().clone();
        let pixels = output.into_buffer();
        decode_into_gray_f32(pixels, &mut dst);
        Ok(info)
    }
}

impl Default for PngDecoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

static PNG_DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_icc(true)
    .with_exif(true)
    .with_xmp(true)
    .with_cicp(true)
    .with_cancel(true)
    .with_animation(true)
    .with_cheap_probe(true)
    .with_native_gray(true)
    .with_native_16bit(true)
    .with_native_alpha(true)
    .with_enforces_max_pixels(true)
    .with_enforces_max_memory(true);

impl zc::decode::DecoderConfig for PngDecoderConfig {
    type Error = At<PngError>;
    type Job<'a> = PngDecodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Png
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        DECODE_DESCRIPTORS
    }

    fn capabilities() -> &'static DecodeCapabilities {
        &PNG_DECODE_CAPS
    }

    fn job(&self) -> PngDecodeJob<'_> {
        PngDecodeJob {
            config: self,
            stop: None,
            limits: None,
        }
    }
}

// ── PngDecodeJob ─────────────────────────────────────────────────────

/// Per-operation PNG decode job.
pub struct PngDecodeJob<'a> {
    config: &'a PngDecoderConfig,
    stop: Option<&'a dyn enough::Stop>,
    limits: Option<ResourceLimits>,
}

impl<'a> zc::decode::DecodeJob<'a> for PngDecodeJob<'a> {
    type Error = At<PngError>;
    type Dec = PngDecoder<'a>;
    type StreamDec = zc::Unsupported<At<PngError>>;
    type FrameDec = PngFrameDecoder;

    fn with_stop(mut self, stop: &'a dyn enough::Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, At<PngError>> {
        let info = crate::decode::probe(data)?;
        Ok(convert_info(&info))
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, At<PngError>> {
        let info = crate::decode::probe(data)?;
        let has_alpha = info.has_alpha;
        let is_16bit = info.bit_depth == 16;
        let native_format = match (has_alpha, is_16bit) {
            (true, true) => PixelDescriptor::RGBA16_SRGB,
            (true, false) => PixelDescriptor::RGBA8_SRGB,
            (false, true) => PixelDescriptor::RGB16_SRGB,
            (false, false) => PixelDescriptor::RGB8_SRGB,
        };
        Ok(OutputInfo::full_decode(info.width, info.height, native_format).with_alpha(has_alpha))
    }

    fn decoder(
        self,
        data: &'a [u8],
        preferred: &[PixelDescriptor],
    ) -> Result<PngDecoder<'a>, At<PngError>> {
        Ok(PngDecoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
            data,
            preferred: preferred.to_vec(),
        })
    }

    fn streaming_decoder(
        self,
        _data: &'a [u8],
        _preferred: &[PixelDescriptor],
    ) -> Result<zc::Unsupported<At<PngError>>, At<PngError>> {
        Err(PngError::from(zc::UnsupportedOperation::RowLevelDecode).start_at())
    }

    fn frame_decoder(
        self,
        data: &'a [u8],
        _preferred: &[PixelDescriptor],
    ) -> Result<PngFrameDecoder, At<PngError>> {
        PngFrameDecoder::new(data, self.config, self.stop).map_err(ErrorAtExt::start_at)
    }
}

// ── PngDecoder ───────────────────────────────────────────────────────

/// Single-image PNG decoder.
pub struct PngDecoder<'a> {
    config: &'a PngDecoderConfig,
    stop: Option<&'a dyn enough::Stop>,
    limits: Option<ResourceLimits>,
    data: &'a [u8],
    preferred: Vec<PixelDescriptor>,
}

impl PngDecoder<'_> {
    fn effective_config(&self) -> PngDecodeConfig {
        let limits = self.limits.as_ref().unwrap_or(&self.config.limits);
        PngDecodeConfig {
            max_pixels: limits.max_pixels,
            max_memory_bytes: limits.max_memory_bytes,
            skip_decompression_checksum: true,
            skip_critical_chunk_crc: true,
        }
    }
}

impl zc::decode::Decode for PngDecoder<'_> {
    type Error = At<PngError>;

    fn decode(self) -> Result<DecodeOutput, At<PngError>> {
        let cancel: &dyn enough::Stop = self.stop.unwrap_or(&enough::Unstoppable);
        cancel.check().map_err(PngError::from)?;
        let png_config = self.effective_config();
        let result = crate::decode::decode(self.data, &png_config, cancel)?;
        let info = convert_info(&result.info);
        let pixels = if self.preferred.is_empty() {
            result.pixels
        } else {
            negotiate_and_convert(result.pixels, &self.preferred)
        };
        Ok(DecodeOutput::new(pixels, info))
    }
}

// ── PngFrameDecoder ──────────────────────────────────────────────────

/// APNG frame-by-frame decoder implementing [`FrameDecode`](zc::FrameDecode).
///
/// Yields raw (non-composited) subframes with blend/disposal metadata.
/// The caller is responsible for compositing if desired.
pub struct PngFrameDecoder {
    /// Owned copy of the PNG file data.
    file_data: Vec<u8>,
    /// Shared image info for all frames.
    info: std::sync::Arc<ImageInfo>,
    /// Saved decoder state for O(1) resumption between frames.
    decoder_state: crate::decoder::apng::ApngDecoderState,
}

impl PngFrameDecoder {
    fn new(
        data: &[u8],
        config: &PngDecoderConfig,
        _stop: Option<&dyn enough::Stop>,
    ) -> Result<Self, PngError> {
        let probe_info = crate::decode::probe(data)?;
        let image_info = convert_info(&probe_info);

        let decode_config = PngDecodeConfig {
            max_pixels: config.limits.max_pixels,
            max_memory_bytes: config.limits.max_memory_bytes,
            skip_decompression_checksum: true,
            skip_critical_chunk_crc: true,
        };

        // Create ApngDecoder once and save its state for O(1) resumption.
        let decoder = crate::decoder::apng::ApngDecoder::new(data, &decode_config)?;
        let decoder_state = decoder.save_state();

        Ok(Self {
            file_data: data.to_vec(),
            info: std::sync::Arc::new(image_info),
            decoder_state,
        })
    }
}

impl zc::decode::FrameDecode for PngFrameDecoder {
    type Error = At<PngError>;

    fn frame_count(&self) -> Option<u32> {
        Some(self.decoder_state.num_frames)
    }

    fn loop_count(&self) -> Option<u32> {
        Some(self.decoder_state.num_plays)
    }

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, At<PngError>> {
        // Restore decoder from saved state (O(1), no re-scanning)
        let mut decoder = crate::decoder::apng::ApngDecoder::from_state(
            &self.file_data,
            self.decoder_state.clone(),
        );

        let raw = match decoder.next_frame(&enough::Unstoppable)? {
            Some(f) => f,
            None => return Ok(None),
        };

        // Save updated state (chunk_pos / current_frame advanced)
        let idx = self.decoder_state.current_frame;
        self.decoder_state = decoder.save_state();

        let fctl = &raw.fctl;
        let blend = match fctl.blend_op {
            0 => zc::FrameBlend::Source,
            _ => zc::FrameBlend::Over,
        };
        let disposal = match fctl.dispose_op {
            0 => zc::FrameDisposal::None,
            1 => zc::FrameDisposal::RestoreBackground,
            _ => zc::FrameDisposal::RestorePrevious,
        };
        let delay_ms = fctl.delay_ms();
        let frame_rect = [fctl.x_offset, fctl.y_offset, fctl.width, fctl.height];

        let frame = DecodeFrame::new(raw.pixels, self.info.clone(), delay_ms, idx)
            .with_blend(blend)
            .with_disposal(disposal)
            .with_frame_rect(frame_rect);

        Ok(Some(frame))
    }
}

// ── Pixel format negotiation ─────────────────────────────────────────

use rgb::{Gray, Rgb, Rgba};
use zenpixels::{ChannelLayout, ChannelType, GrayAlpha16, PixelBuffer};
use zenpixels_convert::PixelBufferConvertExt as _;

/// Negotiate output format from the caller's preference list and convert.
///
/// Uses `negotiate_pixel_format` to pick the best match, then converts
/// if the native format doesn't already match.
fn negotiate_and_convert(pixels: PixelBuffer, preferred: &[PixelDescriptor]) -> PixelBuffer {
    use zenpixels::PixelFormat;

    let native_desc = pixels.descriptor();
    let target = zc::decode::negotiate_pixel_format(preferred, DECODE_DESCRIPTORS);

    // Already in the target format — no conversion needed
    if native_desc.pixel_format() == target.pixel_format() {
        return pixels;
    }

    // Convert to the negotiated format (8-bit conversions available)
    match target.pixel_format() {
        PixelFormat::Rgb8 => pixels.to_rgb8().erase(),
        PixelFormat::Rgba8 => pixels.to_rgba8().erase(),
        PixelFormat::Gray8 => pixels.to_gray8().erase(),
        PixelFormat::Bgra8 => pixels.to_bgra8().erase(),
        // For 16-bit and F32 formats, zenpixels-convert doesn't have
        // conversion functions yet — return native format unchanged
        _ => pixels,
    }
}

// ── Pixel conversion helpers ─────────────────────────────────────────
//
// PNG decoder produces Rgb8, Rgba8, Gray8, Rgb16, Rgba16, Gray16,
// GrayAlpha16. These helpers convert to any requested target format.

/// Convert native PNG pixel data to Rgb8. Delegates to `PixelBuffer::to_rgb8()`.
fn to_rgb8(pixels: PixelBuffer) -> imgref::ImgVec<Rgb<u8>> {
    let converted = pixels.to_rgb8();
    let w = converted.width() as usize;
    let h = converted.height() as usize;
    let img = converted.as_imgref();
    let buf: Vec<Rgb<u8>> = img.pixels().collect();
    imgref::ImgVec::new(buf, w, h)
}

/// Convert native PNG pixel data to Rgba8. Delegates to `PixelBuffer::to_rgba8()`.
fn to_rgba8(pixels: PixelBuffer) -> imgref::ImgVec<Rgba<u8>> {
    let converted = pixels.to_rgba8();
    let w = converted.width() as usize;
    let h = converted.height() as usize;
    let img = converted.as_imgref();
    let buf: Vec<Rgba<u8>> = img.pixels().collect();
    imgref::ImgVec::new(buf, w, h)
}

/// Convert native PNG pixel data to Gray8. Delegates to `PixelBuffer::to_gray8()`.
fn to_gray8(pixels: PixelBuffer) -> imgref::ImgVec<Gray<u8>> {
    let converted = pixels.to_gray8();
    let w = converted.width() as usize;
    let h = converted.height() as usize;
    let img = converted.as_imgref();
    let buf: Vec<Gray<u8>> = img.pixels().collect();
    imgref::ImgVec::new(buf, w, h)
}

/// Convert native PNG pixel data to Bgra8. Delegates to `PixelBuffer::to_bgra8()`.
fn to_bgra8(pixels: PixelBuffer) -> imgref::ImgVec<rgb::alt::BGRA<u8>> {
    let converted = pixels.to_bgra8();
    let w = converted.width() as usize;
    let h = converted.height() as usize;
    let img = converted.as_imgref();
    let buf: Vec<rgb::alt::BGRA<u8>> = img.pixels().collect();
    imgref::ImgVec::new(buf, w, h)
}

// ── Helpers ──────────────────────────────────────────────────────────

fn convert_info(info: &crate::decode::PngInfo) -> ImageInfo {
    let mut zi = ImageInfo::new(info.width, info.height, ImageFormat::Png);
    if info.has_alpha {
        zi = zi.with_alpha(true);
    }
    if info.has_animation {
        zi = zi.with_animation(true);
    }
    zi = zi.with_frame_count(info.frame_count);
    if let Some(ref icc) = info.icc_profile {
        zi = zi.with_icc_profile(icc.clone());
    }
    if let Some(ref exif) = info.exif {
        zi = zi.with_exif(exif.clone());
    }
    if let Some(ref xmp) = info.xmp {
        zi = zi.with_xmp(xmp.clone());
    }
    if let Some(cicp) = info.cicp {
        zi = zi.with_cicp(cicp);
    }
    if let Some(clli) = info.content_light_level {
        zi = zi.with_content_light_level(clli);
    }
    if let Some(mdcv) = info.mastering_display {
        zi = zi.with_mastering_display(mdcv);
    }
    zi
}

/// Get contiguous pixel bytes from a PixelSlice, borrowing when possible.
///
/// Returns `Cow::Borrowed` when rows are tightly packed (zero-copy),
/// `Cow::Owned` when stride padding must be stripped.
fn contiguous_bytes<'a>(pixels: &'a PixelSlice<'a>) -> alloc::borrow::Cow<'a, [u8]> {
    pixels.contiguous_bytes()
}

/// Copy rows from a typed ImgVec into a PixelSliceMut via byte reinterpretation.
fn copy_rows_u8<P: Copy>(src: &imgref::ImgVec<P>, dst: &mut PixelSliceMut<'_>)
where
    [P]: rgb::ComponentBytes<u8>,
{
    use rgb::ComponentBytes;
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let src_bytes = src_row.as_bytes();
        let dst_row = dst.row_mut(y as u32);
        let n = src_bytes.len().min(dst_row.len());
        dst_row[..n].copy_from_slice(&src_bytes[..n]);
    }
}

/// Decode into linear RGB f32.
fn decode_into_rgb_f32(pixels: PixelBuffer, dst: &mut PixelSliceMut<'_>) {
    use linear_srgb::default::srgb_u8_to_linear;

    let src = to_rgb8(pixels);
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let dst_row = dst.row_mut(y as u32);
        for (i, s) in src_row.iter().enumerate() {
            let offset = i * 12;
            if offset + 12 > dst_row.len() {
                break;
            }
            let r = srgb_u8_to_linear(s.r);
            let g = srgb_u8_to_linear(s.g);
            let b = srgb_u8_to_linear(s.b);
            dst_row[offset..offset + 4].copy_from_slice(&r.to_ne_bytes());
            dst_row[offset + 4..offset + 8].copy_from_slice(&g.to_ne_bytes());
            dst_row[offset + 8..offset + 12].copy_from_slice(&b.to_ne_bytes());
        }
    }
}

/// Decode into linear RGBA f32.
fn decode_into_rgba_f32(pixels: PixelBuffer, dst: &mut PixelSliceMut<'_>) {
    use linear_srgb::default::srgb_u8_to_linear;

    let src = to_rgba8(pixels);
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let dst_row = dst.row_mut(y as u32);
        for (i, s) in src_row.iter().enumerate() {
            let offset = i * 16;
            if offset + 16 > dst_row.len() {
                break;
            }
            let r = srgb_u8_to_linear(s.r);
            let g = srgb_u8_to_linear(s.g);
            let b = srgb_u8_to_linear(s.b);
            dst_row[offset..offset + 4].copy_from_slice(&r.to_ne_bytes());
            dst_row[offset + 4..offset + 8].copy_from_slice(&g.to_ne_bytes());
            dst_row[offset + 8..offset + 12].copy_from_slice(&b.to_ne_bytes());
            dst_row[offset + 12..offset + 16].copy_from_slice(&(s.a as f32 / 255.0).to_ne_bytes());
        }
    }
}

/// Decode into linear Gray f32.
fn decode_into_gray_f32(pixels: PixelBuffer, dst: &mut PixelSliceMut<'_>) {
    use linear_srgb::default::srgb_u8_to_linear;

    let src = to_gray8(pixels);
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let dst_row = dst.row_mut(y as u32);
        for (i, s) in src_row.iter().enumerate() {
            let offset = i * 4;
            if offset + 4 > dst_row.len() {
                break;
            }
            let v = srgb_u8_to_linear(s.value());
            dst_row[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
        }
    }
}

// ── U16 conversion helpers ───────────────────────────────────────────

/// Byte-swap native-endian u16 samples to big-endian for PNG.
fn native_to_be_16(native: &[u8]) -> Vec<u8> {
    if cfg!(target_endian = "big") {
        return native.to_vec();
    }
    let mut out = native.to_vec();
    for chunk in out.chunks_exact_mut(2) {
        chunk.swap(0, 1);
    }
    out
}

/// Convert any PixelBuffer to Rgb<u16>. Upscales 8-bit by v*257.
fn to_rgb16(pixels: PixelBuffer) -> imgref::ImgVec<Rgb<u16>> {
    let desc = pixels.descriptor();
    let w = pixels.width() as usize;
    let h = pixels.height() as usize;
    match (desc.channel_type(), desc.layout()) {
        (ChannelType::U16, ChannelLayout::Rgb) => {
            let img = pixels.try_as_imgref::<Rgb<u16>>().unwrap();
            let buf: Vec<Rgb<u16>> = img.pixels().collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::Rgba) => {
            let img = pixels.try_as_imgref::<Rgba<u16>>().unwrap();
            let buf: Vec<Rgb<u16>> = img
                .pixels()
                .map(|p| Rgb {
                    r: p.r,
                    g: p.g,
                    b: p.b,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::Gray) => {
            let img = pixels.try_as_imgref::<Gray<u16>>().unwrap();
            let buf: Vec<Rgb<u16>> = img
                .pixels()
                .map(|p| {
                    let v = p.value();
                    Rgb { r: v, g: v, b: v }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::GrayAlpha) => {
            let img = pixels.try_as_imgref::<GrayAlpha16>().unwrap();
            let buf: Vec<Rgb<u16>> = img
                .pixels()
                .map(|p| {
                    let v = p.v;
                    Rgb { r: v, g: v, b: v }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        _ => {
            // Upscale 8-bit via to_rgb8
            let rgb8 = to_rgb8(pixels);
            let buf: Vec<Rgb<u16>> = rgb8
                .into_buf()
                .into_iter()
                .map(|p| Rgb {
                    r: p.r as u16 * 257,
                    g: p.g as u16 * 257,
                    b: p.b as u16 * 257,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
    }
}

/// Convert any PixelBuffer to Rgba<u16>. Upscales 8-bit by v*257.
fn to_rgba16(pixels: PixelBuffer) -> imgref::ImgVec<Rgba<u16>> {
    let desc = pixels.descriptor();
    let w = pixels.width() as usize;
    let h = pixels.height() as usize;
    match (desc.channel_type(), desc.layout()) {
        (ChannelType::U16, ChannelLayout::Rgba) => {
            let img = pixels.try_as_imgref::<Rgba<u16>>().unwrap();
            let buf: Vec<Rgba<u16>> = img.pixels().collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::Rgb) => {
            let img = pixels.try_as_imgref::<Rgb<u16>>().unwrap();
            let buf: Vec<Rgba<u16>> = img
                .pixels()
                .map(|p| Rgba {
                    r: p.r,
                    g: p.g,
                    b: p.b,
                    a: 65535,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::Gray) => {
            let img = pixels.try_as_imgref::<Gray<u16>>().unwrap();
            let buf: Vec<Rgba<u16>> = img
                .pixels()
                .map(|p| {
                    let v = p.value();
                    Rgba {
                        r: v,
                        g: v,
                        b: v,
                        a: 65535,
                    }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::GrayAlpha) => {
            let img = pixels.try_as_imgref::<GrayAlpha16>().unwrap();
            let buf: Vec<Rgba<u16>> = img
                .pixels()
                .map(|p| Rgba {
                    r: p.v,
                    g: p.v,
                    b: p.v,
                    a: p.a,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        _ => {
            // Upscale 8-bit via to_rgba8
            let rgba8 = to_rgba8(pixels);
            let buf: Vec<Rgba<u16>> = rgba8
                .into_buf()
                .into_iter()
                .map(|p| Rgba {
                    r: p.r as u16 * 257,
                    g: p.g as u16 * 257,
                    b: p.b as u16 * 257,
                    a: p.a as u16 * 257,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
    }
}

/// Convert any PixelBuffer to Gray<u16>. Upscales 8-bit by v*257.
fn to_gray16(pixels: PixelBuffer) -> imgref::ImgVec<Gray<u16>> {
    let desc = pixels.descriptor();
    let w = pixels.width() as usize;
    let h = pixels.height() as usize;
    match (desc.channel_type(), desc.layout()) {
        (ChannelType::U16, ChannelLayout::Gray) => {
            let img = pixels.try_as_imgref::<Gray<u16>>().unwrap();
            let buf: Vec<Gray<u16>> = img.pixels().collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::GrayAlpha) => {
            let img = pixels.try_as_imgref::<GrayAlpha16>().unwrap();
            let buf: Vec<Gray<u16>> = img.pixels().map(|p| Gray(p.v)).collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::Rgb) => {
            let img = pixels.try_as_imgref::<Rgb<u16>>().unwrap();
            let buf: Vec<Gray<u16>> = img
                .pixels()
                .map(|p| {
                    let luma =
                        ((p.r as u32 * 77 + p.g as u32 * 150 + p.b as u32 * 29 + 128) >> 8) as u16;
                    Gray(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        (ChannelType::U16, ChannelLayout::Rgba) => {
            let img = pixels.try_as_imgref::<Rgba<u16>>().unwrap();
            let buf: Vec<Gray<u16>> = img
                .pixels()
                .map(|p| {
                    let luma =
                        ((p.r as u32 * 77 + p.g as u32 * 150 + p.b as u32 * 29 + 128) >> 8) as u16;
                    Gray(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        _ => {
            // Upscale 8-bit via to_gray8
            let gray8 = to_gray8(pixels);
            let buf: Vec<Gray<u16>> = gray8
                .into_buf()
                .into_iter()
                .map(|p| Gray(p.value() as u16 * 257))
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
    }
}

/// Decode into Rgb<u16> target buffer.
fn decode_into_rgb16(pixels: PixelBuffer, dst: &mut PixelSliceMut<'_>) {
    let src = to_rgb16(pixels);
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let dst_row = dst.row_mut(y as u32);
        for (i, s) in src_row.iter().enumerate() {
            let offset = i * 6;
            if offset + 6 > dst_row.len() {
                break;
            }
            dst_row[offset..offset + 2].copy_from_slice(&s.r.to_ne_bytes());
            dst_row[offset + 2..offset + 4].copy_from_slice(&s.g.to_ne_bytes());
            dst_row[offset + 4..offset + 6].copy_from_slice(&s.b.to_ne_bytes());
        }
    }
}

/// Decode into Rgba<u16> target buffer.
fn decode_into_rgba16(pixels: PixelBuffer, dst: &mut PixelSliceMut<'_>) {
    let src = to_rgba16(pixels);
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let dst_row = dst.row_mut(y as u32);
        for (i, s) in src_row.iter().enumerate() {
            let offset = i * 8;
            if offset + 8 > dst_row.len() {
                break;
            }
            dst_row[offset..offset + 2].copy_from_slice(&s.r.to_ne_bytes());
            dst_row[offset + 2..offset + 4].copy_from_slice(&s.g.to_ne_bytes());
            dst_row[offset + 4..offset + 6].copy_from_slice(&s.b.to_ne_bytes());
            dst_row[offset + 6..offset + 8].copy_from_slice(&s.a.to_ne_bytes());
        }
    }
}

/// Decode into Gray<u16> target buffer.
fn decode_into_gray16(pixels: PixelBuffer, dst: &mut PixelSliceMut<'_>) {
    let src = to_gray16(pixels);
    for y in 0..src.height().min(dst.rows() as usize) {
        let src_row = &src.buf()[y * src.stride()..][..src.width()];
        let dst_row = dst.row_mut(y as u32);
        for (i, s) in src_row.iter().enumerate() {
            let offset = i * 2;
            if offset + 2 > dst_row.len() {
                break;
            }
            dst_row[offset..offset + 2].copy_from_slice(&s.value().to_ne_bytes());
        }
    }
}

// ── decode_rows helpers ──────────────────────────────────────────────

/// Get the PixelDescriptor for decoded PixelBuffer.
fn pixel_descriptor_for_data(pixels: &PixelBuffer) -> PixelDescriptor {
    pixels.descriptor()
}

/// Get raw pixel bytes from PixelBuffer (copies to contiguous bytes).
fn pixel_data_bytes(pixels: &PixelBuffer) -> Vec<u8> {
    pixels.copy_to_contiguous_bytes()
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use imgref::Img;
    use rgb::{Gray, Rgb, Rgba};
    use zc::decode::{Decode, DecodeJob, DecoderConfig};
    use zc::encode::{EncodeJob, EncoderConfig};

    #[test]
    fn encoding_rgb8() {
        let enc = PngEncoderConfig::new();
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 128,
                g: 64,
                b: 32
            };
            64
        ];
        let img = Img::new(pixels, 8, 8);
        let output = enc.encode_rgb8(img.as_ref()).unwrap();
        assert!(!output.data().is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
        assert_eq!(
            &output.data()[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn encoding_rgba8() {
        let enc = PngEncoderConfig::new();
        let pixels: Vec<Rgba<u8>> = vec![
            Rgba {
                r: 100,
                g: 150,
                b: 200,
                a: 128,
            };
            64
        ];
        let img = Img::new(pixels, 8, 8);
        let output = enc.encode_rgba8(img.as_ref()).unwrap();
        assert!(!output.data().is_empty());
    }

    #[test]
    fn encoding_gray8() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![Gray::new(128u8); 64];
        let img = Img::new(pixels, 8, 8);
        let output = enc.encode_gray8(img.as_ref()).unwrap();
        assert!(!output.data().is_empty());
    }

    #[test]
    fn decode_roundtrip() {
        let enc = PngEncoderConfig::new();
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 200,
                g: 100,
                b: 50
            };
            64
        ];
        let img = Img::new(pixels, 8, 8);
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let output = dec.decode(encoded.data()).unwrap();
        assert_eq!(output.info().width, 8);
        assert_eq!(output.info().height, 8);
        assert_eq!(output.info().format, ImageFormat::Png);
    }

    #[test]
    fn probe_header_info() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![Rgb { r: 0u8, g: 0, b: 0 }; 100];
        let img = Img::new(pixels, 10, 10);
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let info = dec.probe_header(encoded.data()).unwrap();
        assert_eq!(info.width, 10);
        assert_eq!(info.height, 10);
        assert_eq!(info.format, ImageFormat::Png);
    }

    #[test]
    fn decode_into_rgb8_roundtrip() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![
            Rgb {
                r: 128u8,
                g: 64,
                b: 32
            };
            64
        ];
        let img = Img::new(pixels.clone(), 8, 8);
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let mut buf = vec![Rgb { r: 0u8, g: 0, b: 0 }; 64];
        let mut dst = imgref::ImgVec::new(buf.clone(), 8, 8);
        let info = dec.decode_into_rgb8(encoded.data(), dst.as_mut()).unwrap();
        assert_eq!(info.width, 8);
        assert_eq!(info.height, 8);
        buf = dst.into_buf();
        assert_eq!(buf[0], pixels[0]);
    }

    #[test]
    fn encode_bgra8_roundtrip() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![
            rgb::alt::BGRA {
                b: 0,
                g: 0,
                r: 255,
                a: 255,
            },
            rgb::alt::BGRA {
                b: 0,
                g: 255,
                r: 0,
                a: 200,
            },
            rgb::alt::BGRA {
                b: 255,
                g: 0,
                r: 0,
                a: 128,
            },
            rgb::alt::BGRA {
                b: 128,
                g: 128,
                r: 128,
                a: 255,
            },
        ];
        let img = Img::new(pixels, 2, 2);
        let output = enc.encode_bgra8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let decoded = dec.decode(output.data()).unwrap();
        let rgba = to_rgba8(decoded.into_buffer());
        let buf = rgba.buf();
        assert_eq!(
            buf[0],
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        );
        assert_eq!(
            buf[1],
            Rgba {
                r: 0,
                g: 255,
                b: 0,
                a: 200
            }
        );
    }

    #[test]
    fn f32_conversion_all_simd_tiers() {
        use archmage::testing::{CompileTimePolicy, for_each_token_permutation};
        use linear_srgb::default::{linear_to_srgb_u8, srgb_u8_to_linear};

        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            // Encode linear f32 → PNG (sRGB u8) → decode to linear f32
            let pixels = vec![
                Rgb {
                    r: 0.0f32,
                    g: 0.5,
                    b: 1.0,
                },
                Rgb {
                    r: 0.25,
                    g: 0.75,
                    b: 0.1,
                },
                Rgb {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                },
                Rgb {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                },
            ];
            let img = Img::new(pixels.clone(), 2, 2);
            let enc = PngEncoderConfig::new();
            let output = enc.encode_rgb_f32(img.as_ref()).unwrap();

            let dec = PngDecoderConfig::new();
            let mut buf = vec![
                Rgb {
                    r: 0.0f32,
                    g: 0.0,
                    b: 0.0
                };
                4
            ];
            let mut dst = imgref::ImgVec::new(buf.clone(), 2, 2);
            dec.decode_into_rgb_f32(output.data(), dst.as_mut())
                .unwrap();
            buf = dst.into_buf();

            // Roundtrip tolerance: linear→sRGB u8→linear introduces quantization
            for (orig, decoded) in pixels.iter().zip(buf.iter()) {
                let expected_r = srgb_u8_to_linear(linear_to_srgb_u8(orig.r.clamp(0.0, 1.0)));
                let expected_g = srgb_u8_to_linear(linear_to_srgb_u8(orig.g.clamp(0.0, 1.0)));
                let expected_b = srgb_u8_to_linear(linear_to_srgb_u8(orig.b.clamp(0.0, 1.0)));
                assert!(
                    (decoded.r - expected_r).abs() < 1e-5,
                    "r mismatch: {} vs {}",
                    decoded.r,
                    expected_r
                );
                assert!(
                    (decoded.g - expected_g).abs() < 1e-5,
                    "g mismatch: {} vs {}",
                    decoded.g,
                    expected_g
                );
                assert!(
                    (decoded.b - expected_b).abs() < 1e-5,
                    "b mismatch: {} vs {}",
                    decoded.b,
                    expected_b
                );
            }
        });
        assert!(report.permutations_run >= 1);
    }

    #[test]
    fn f32_rgba_roundtrip() {
        use linear_srgb::default::{linear_to_srgb_u8, srgb_u8_to_linear};

        let pixels = vec![
            Rgba {
                r: 0.0f32,
                g: 0.5,
                b: 1.0,
                a: 1.0,
            },
            Rgba {
                r: 0.25,
                g: 0.75,
                b: 0.1,
                a: 0.5,
            },
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        ];
        let img = Img::new(pixels.clone(), 2, 2);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_rgba_f32(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let mut dst = imgref::ImgVec::new(
            vec![
                Rgba {
                    r: 0.0f32,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0
                };
                4
            ],
            2,
            2,
        );
        dec.decode_into_rgba_f32(output.data(), dst.as_mut())
            .unwrap();

        for (orig, decoded) in pixels.iter().zip(dst.buf().iter()) {
            let expected_r = srgb_u8_to_linear(linear_to_srgb_u8(orig.r.clamp(0.0, 1.0)));
            let expected_g = srgb_u8_to_linear(linear_to_srgb_u8(orig.g.clamp(0.0, 1.0)));
            let expected_b = srgb_u8_to_linear(linear_to_srgb_u8(orig.b.clamp(0.0, 1.0)));
            let expected_a = (orig.a * 255.0).round() / 255.0;
            assert!((decoded.r - expected_r).abs() < 1e-5, "r mismatch");
            assert!((decoded.g - expected_g).abs() < 1e-5, "g mismatch");
            assert!((decoded.b - expected_b).abs() < 1e-5, "b mismatch");
            assert!(
                (decoded.a - expected_a).abs() < 1e-2,
                "a mismatch: {} vs {}",
                decoded.a,
                expected_a
            );
        }
    }

    #[test]
    fn f32_gray_roundtrip() {
        use linear_srgb::default::{linear_to_srgb_u8, srgb_u8_to_linear};
        use rgb::Gray;

        let pixels = vec![Gray(0.0f32), Gray(0.18), Gray(0.5), Gray(1.0)];
        let img = Img::new(pixels.clone(), 2, 2);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_gray_f32(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let mut dst = imgref::ImgVec::new(vec![Gray(0.0f32); 4], 2, 2);
        dec.decode_into_gray_f32(output.data(), dst.as_mut())
            .unwrap();

        for (orig, decoded) in pixels.iter().zip(dst.buf().iter()) {
            let expected = srgb_u8_to_linear(linear_to_srgb_u8(orig.value().clamp(0.0, 1.0)));
            assert!(
                (decoded.value() - expected).abs() < 1e-5,
                "gray mismatch: {} vs {}",
                decoded.value(),
                expected
            );
        }
    }

    #[test]
    fn f32_known_srgb_values() {
        use linear_srgb::default::srgb_u8_to_linear;

        // Encode known sRGB u8 values, decode to linear f32, verify conversion
        let pixels = vec![
            Rgb { r: 0u8, g: 0, b: 0 },
            Rgb {
                r: 128,
                g: 128,
                b: 128,
            },
            Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            Rgb { r: 255, g: 0, b: 0 },
        ];
        let img = Img::new(pixels, 2, 2);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let mut dst = imgref::ImgVec::new(
            vec![
                Rgb {
                    r: 0.0f32,
                    g: 0.0,
                    b: 0.0
                };
                4
            ],
            2,
            2,
        );
        dec.decode_into_rgb_f32(output.data(), dst.as_mut())
            .unwrap();

        let buf = dst.buf();
        // Black → 0.0 linear
        assert!(buf[0].r.abs() < 1e-6);
        assert!(buf[0].g.abs() < 1e-6);
        assert!(buf[0].b.abs() < 1e-6);
        // sRGB 128 → known linear value
        let expected_128 = srgb_u8_to_linear(128);
        assert!((buf[1].r - expected_128).abs() < 1e-5);
        // White → 1.0 linear
        assert!((buf[2].r - 1.0).abs() < 1e-6);
        assert!((buf[2].g - 1.0).abs() < 1e-6);
        assert!((buf[2].b - 1.0).abs() < 1e-6);
        // Pure red
        assert!((buf[3].r - 1.0).abs() < 1e-6);
        assert!(buf[3].g.abs() < 1e-6);
        assert!(buf[3].b.abs() < 1e-6);
    }

    #[test]
    fn format_is_correct() {
        assert_eq!(
            <PngEncoderConfig as EncoderConfig>::format(),
            ImageFormat::Png
        );
        assert_eq!(
            <PngDecoderConfig as DecoderConfig>::format(),
            ImageFormat::Png
        );
    }

    #[test]
    fn effort_getter_setter() {
        // Default (no effort set) → lossless
        let enc = PngEncoderConfig::new();
        assert_eq!(enc.generic_effort(), None);
        assert_eq!(enc.is_lossless(), Some(true));

        // effort 0 → None (store, no compression)
        let enc = PngEncoderConfig::new().with_generic_effort(0);
        assert_eq!(enc.generic_effort(), Some(0));

        // effort 1 → Fastest
        let enc = PngEncoderConfig::new().with_generic_effort(1);
        assert_eq!(enc.generic_effort(), Some(1));

        // effort 5 → Thorough
        let enc = PngEncoderConfig::new().with_generic_effort(5);
        assert_eq!(enc.generic_effort(), Some(5));
        assert_eq!(enc.is_lossless(), Some(true));

        // effort 9 → Crush
        let enc = PngEncoderConfig::new().with_generic_effort(9);
        assert_eq!(enc.generic_effort(), Some(9));

        // effort 10 → Maniac
        let enc = PngEncoderConfig::new().with_generic_effort(10);
        assert_eq!(enc.generic_effort(), Some(10));

        // effort 11 → Brag
        let enc = PngEncoderConfig::new().with_generic_effort(11);
        assert_eq!(enc.generic_effort(), Some(11));

        // effort 12+ → Minutes
        let enc = PngEncoderConfig::new().with_generic_effort(12);
        assert_eq!(enc.generic_effort(), Some(12));
    }

    #[test]
    fn output_info_matches_decode() {
        let pixels = vec![Rgb { r: 1u8, g: 2, b: 3 }; 6];
        let img = Img::new(pixels, 3, 2);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let info = dec.job().output_info(output.data()).unwrap();
        assert_eq!(info.width, 3);
        assert_eq!(info.height, 2);

        let decoded = dec.decode(output.data()).unwrap();
        assert_eq!(decoded.width(), info.width);
        assert_eq!(decoded.height(), info.height);
    }

    #[test]
    fn four_layer_encode_flow() {
        let pixels = vec![
            Rgb::<u8> { r: 255, g: 0, b: 0 },
            Rgb { r: 0, g: 255, b: 0 },
            Rgb { r: 0, g: 0, b: 255 },
            Rgb {
                r: 128,
                g: 128,
                b: 128,
            },
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);
        let config = PngEncoderConfig::new();

        use zc::encode::Encoder;
        let slice = PixelSlice::from(img.as_ref());
        let output = config
            .job()
            .encoder()
            .unwrap()
            .encode(slice.erase())
            .unwrap();
        assert_eq!(output.format(), ImageFormat::Png);
        assert!(!output.data().is_empty());
    }

    #[test]
    fn four_layer_decode_flow() {
        let pixels = vec![
            Rgb {
                r: 100u8,
                g: 200,
                b: 50
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);
        let enc = PngEncoderConfig::new();
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();

        let config = PngDecoderConfig::new();
        let decoded = config
            .job()
            .decoder(encoded.data(), &[])
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
    }

    #[test]
    fn encoding_clone_send_sync() {
        fn assert_traits<T: Clone + Send + Sync>() {}
        assert_traits::<PngEncoderConfig>();
    }

    #[test]
    fn decoding_clone_send_sync() {
        fn assert_traits<T: Clone + Send + Sync>() {}
        assert_traits::<PngDecoderConfig>();
    }

    #[test]
    fn rgb16_roundtrip() {
        let pixels = vec![
            Rgb::<u16> {
                r: 0,
                g: 32768,
                b: 65535,
            },
            Rgb {
                r: 1000,
                g: 50000,
                b: 12345,
            },
            Rgb {
                r: 65535,
                g: 0,
                b: 0,
            },
            Rgb {
                r: 0,
                g: 65535,
                b: 0,
            },
        ];
        let img = imgref::ImgVec::new(pixels.clone(), 2, 2);

        let encoded = crate::encode::encode_rgb16(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        // Verify PNG signature
        assert_eq!(
            &encoded[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );

        // Decode and verify exact values
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        assert_eq!(decoded.info.width, 2);
        assert_eq!(decoded.info.height, 2);
        assert_eq!(decoded.info.bit_depth, 16);

        let img = decoded
            .pixels
            .try_as_imgref::<Rgb<u16>>()
            .expect("expected Rgb16");
        for (i, (orig, dec)) in pixels.iter().zip(img.pixels()).enumerate() {
            assert_eq!(
                *orig, dec,
                "pixel {i} mismatch: expected {orig:?}, got {dec:?}"
            );
        }
    }

    #[test]
    fn rgba16_roundtrip() {
        let pixels = vec![
            Rgba::<u16> {
                r: 0x0102,
                g: 0x0304,
                b: 0x0506,
                a: 0xFFFF,
            },
            Rgba {
                r: 65535,
                g: 0,
                b: 0,
                a: 32768,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            Rgba {
                r: 65535,
                g: 65535,
                b: 65535,
                a: 65535,
            },
        ];
        let img = imgref::ImgVec::new(pixels.clone(), 2, 2);

        let encoded = crate::encode::encode_rgba16(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        assert_eq!(decoded.info.bit_depth, 16);

        let img = decoded
            .pixels
            .try_as_imgref::<Rgba<u16>>()
            .expect("expected Rgba16");
        for (i, (orig, dec)) in pixels.iter().zip(img.pixels()).enumerate() {
            assert_eq!(
                *orig, dec,
                "pixel {i} mismatch: expected {orig:?}, got {dec:?}"
            );
        }
    }

    #[test]
    fn gray16_roundtrip() {
        let pixels = vec![Gray::<u16>(0), Gray(1000), Gray(32768), Gray(65535)];
        let img = imgref::ImgVec::new(pixels.clone(), 2, 2);

        let encoded = crate::encode::encode_gray16(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        assert_eq!(decoded.info.bit_depth, 16);

        let img = decoded
            .pixels
            .try_as_imgref::<Gray<u16>>()
            .expect("expected Gray16");
        for (i, (orig, dec)) in pixels.iter().zip(img.pixels()).enumerate() {
            assert_eq!(
                *orig, dec,
                "pixel {i} mismatch: expected {orig:?}, got {dec:?}"
            );
        }
    }

    #[test]
    fn rgb16_metadata_roundtrip() {
        let pixels = vec![
            Rgb::<u16> {
                r: 100,
                g: 200,
                b: 300
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let fake_icc = vec![0x42u8; 200];
        let exif_data = b"Exif\0\0test_exif";
        let xmp_data = b"<x:xmpmeta>test</x:xmpmeta>";
        let meta = MetadataView::none()
            .with_icc(&fake_icc)
            .with_exif(exif_data)
            .with_xmp(xmp_data);

        let encoded = crate::encode::encode_rgb16(
            img.as_ref(),
            Some(&meta),
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        assert_eq!(
            decoded.info.icc_profile.as_deref(),
            Some(fake_icc.as_slice())
        );
        assert_eq!(decoded.info.exif.as_deref(), Some(exif_data.as_slice()));
        assert_eq!(decoded.info.xmp.as_deref(), Some(xmp_data.as_slice()));
    }

    #[test]
    fn truecolor_zenflate_rgb8_roundtrip() {
        // Verify that 8-bit truecolor now goes through zenflate (not flate2)
        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32,
            },
            Rgb { r: 0, g: 255, b: 0 },
            Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            Rgb { r: 0, g: 0, b: 0 },
        ];
        let img = imgref::ImgVec::new(pixels.clone(), 2, 2);

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        assert_eq!(decoded.info.width, 2);
        assert_eq!(decoded.info.height, 2);

        let img = decoded
            .pixels
            .try_as_imgref::<Rgb<u8>>()
            .expect("expected Rgb8");
        for (orig, dec) in pixels.iter().zip(img.pixels()) {
            assert_eq!(*orig, dec);
        }
    }

    #[test]
    fn truecolor_zenflate_rgba8_roundtrip() {
        let pixels = vec![
            Rgba::<u8> {
                r: 100,
                g: 150,
                b: 200,
                a: 128,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            Rgba {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            Rgba {
                r: 1,
                g: 2,
                b: 3,
                a: 4,
            },
        ];
        let img = imgref::ImgVec::new(pixels.clone(), 2, 2);

        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        let img = decoded
            .pixels
            .try_as_imgref::<Rgba<u8>>()
            .expect("expected Rgba8");
        for (orig, dec) in pixels.iter().zip(img.pixels()) {
            assert_eq!(*orig, dec);
        }
    }

    #[test]
    fn truecolor_zenflate_gray8_roundtrip() {
        let pixels = vec![Gray(0u8), Gray(128), Gray(255), Gray(1)];
        let img = imgref::ImgVec::new(pixels.clone(), 2, 2);

        let encoded = crate::encode::encode_gray8(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        let img = decoded
            .pixels
            .try_as_imgref::<Gray<u8>>()
            .expect("expected Gray8");
        for (orig, dec) in pixels.iter().zip(img.pixels()) {
            assert_eq!(*orig, dec);
        }
    }

    #[test]
    fn subbyte_gray_1bit_roundtrip() {
        // 4x2 RGBA image: black and white only → should encode as 1-bit gray
        let mut pixels = Vec::new();
        for i in 0..8 {
            let v = if i % 2 == 0 { 0u8 } else { 255u8 };
            pixels.push(Rgba {
                r: v,
                g: v,
                b: v,
                a: 255,
            });
        }
        let img = imgref::ImgVec::new(pixels.clone(), 4, 2);

        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        // Verify pixel-exact roundtrip (decoded may be Gray or RGBA)
        let decoded_rgba = decoded.pixels.to_rgba8();
        let decoded_img = decoded_rgba.as_imgref();
        for (i, (orig, dec)) in pixels.iter().zip(decoded_img.pixels()).enumerate() {
            assert_eq!(
                (orig.r, orig.g, orig.b, orig.a),
                (dec.r, dec.g, dec.b, dec.a),
                "pixel {i} mismatch"
            );
        }
    }

    #[test]
    fn subbyte_gray_4bit_roundtrip() {
        // Gray values: 0, 17, 34, ..., 255 (all divisible by 17) → 4-bit gray
        let mut pixels = Vec::new();
        for i in 0..16 {
            let v = (i * 17) as u8;
            pixels.push(Rgba {
                r: v,
                g: v,
                b: v,
                a: 255,
            });
        }
        let img = imgref::ImgVec::new(pixels.clone(), 4, 4);

        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        let decoded_rgba = decoded.pixels.to_rgba8();
        let decoded_img = decoded_rgba.as_imgref();
        for (i, (orig, dec)) in pixels.iter().zip(decoded_img.pixels()).enumerate() {
            assert_eq!(
                (orig.r, orig.g, orig.b, orig.a),
                (dec.r, dec.g, dec.b, dec.a),
                "pixel {i} mismatch"
            );
        }
    }

    #[test]
    fn rgba_trns_gray_roundtrip() {
        // 10x10 grayscale RGBA with one transparent color
        // Transparent gray=0 (unique — doesn't appear opaque)
        let mut pixels = Vec::new();
        for i in 0..100 {
            if i == 0 {
                pixels.push(Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 0,
                }); // transparent
            } else {
                let v = ((i % 15) * 17 + 17) as u8; // 17..255, avoids 0
                pixels.push(Rgba {
                    r: v,
                    g: v,
                    b: v,
                    a: 255,
                });
            }
        }
        let img = imgref::ImgVec::new(pixels.clone(), 10, 10);

        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        let decoded_rgba = decoded.pixels.to_rgba8();
        let decoded_img = decoded_rgba.as_imgref();
        for (i, (orig, dec)) in pixels.iter().zip(decoded_img.pixels()).enumerate() {
            if orig.a == 0 && dec.a == 0 {
                continue; // both transparent, RGB may differ
            }
            assert_eq!(
                (orig.r, orig.g, orig.b, orig.a),
                (dec.r, dec.g, dec.b, dec.a),
                "pixel {i} mismatch"
            );
        }
    }

    #[test]
    fn rgba_trns_rgb_roundtrip() {
        // >256 unique colors with one transparent color → should use RGB+tRNS
        let mut pixels = Vec::new();
        for r in 0..20u8 {
            for g in 0..21u8 {
                let b = 128u8;
                pixels.push(Rgba {
                    r: r.wrapping_mul(13),
                    g: g.wrapping_mul(12),
                    b,
                    a: 255,
                });
            }
        }
        // Make first pixel transparent with unique RGB
        pixels[0] = Rgba {
            r: 3,
            g: 7,
            b: 11,
            a: 0,
        };
        let w = 20;
        let h = pixels.len() / w;
        let img = imgref::ImgVec::new(pixels.clone(), w, h);

        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &EncodeConfig::default(),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        let decoded_rgba = decoded.pixels.to_rgba8();
        let decoded_img = decoded_rgba.as_imgref();
        for (i, (orig, dec)) in pixels.iter().zip(decoded_img.pixels()).enumerate() {
            if orig.a == 0 && dec.a == 0 {
                continue;
            }
            assert_eq!(
                (orig.r, orig.g, orig.b, orig.a),
                (dec.r, dec.g, dec.b, dec.a),
                "pixel {i} mismatch"
            );
        }
    }

    #[test]
    fn zencodec_u16_encode_decode() {
        // Test the zencodec trait path for U16
        let pixels = vec![
            Rgb::<u16> {
                r: 100,
                g: 200,
                b: 300,
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels.clone(), 2, 2);

        let enc = PngEncoderConfig::new();
        let slice = PixelSlice::from(img.as_ref());
        use zc::encode::Encoder;
        let output = enc.job().encoder().unwrap().encode(slice.erase()).unwrap();
        assert_eq!(output.format(), ImageFormat::Png);

        // Decode back into U16
        let dec = PngDecoderConfig::new();
        let mut dst = imgref::ImgVec::new(vec![Rgb::<u16> { r: 0, g: 0, b: 0 }; 4], 2, 2);
        dec.decode_into_rgb16(output.data(), dst.as_mut()).unwrap();
        for (orig, dec) in pixels.iter().zip(dst.buf().iter()) {
            assert_eq!(orig, dec);
        }
    }

    #[test]
    fn srgb_suppresses_gama_chrm() {
        // PNGv3 precedence: sRGB suppresses gAMA and cHRM in output
        use crate::decode::PngChromaticities;

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let chrm = PngChromaticities {
            white_x: 31270,
            white_y: 32900,
            red_x: 64000,
            red_y: 33000,
            green_x: 30000,
            green_y: 60000,
            blue_x: 15000,
            blue_y: 6000,
        };

        let config = crate::encode::EncodeConfig {
            source_gamma: Some(45455),
            srgb_intent: Some(0), // Perceptual
            chromaticities: Some(chrm),
            ..Default::default()
        };

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        // sRGB is written, gAMA and cHRM are suppressed
        assert_eq!(decoded.info.srgb_intent, Some(0));
        assert_eq!(decoded.info.source_gamma, None);
        assert!(decoded.info.chromaticities.is_none());
    }

    #[test]
    fn gama_chrm_roundtrip_without_srgb() {
        // gAMA + cHRM round-trip when no higher-priority chunk is present
        use crate::decode::PngChromaticities;

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let chrm = PngChromaticities {
            white_x: 31270,
            white_y: 32900,
            red_x: 64000,
            red_y: 33000,
            green_x: 30000,
            green_y: 60000,
            blue_x: 15000,
            blue_y: 6000,
        };

        let config = crate::encode::EncodeConfig {
            source_gamma: Some(45455),
            chromaticities: Some(chrm),
            ..Default::default()
        };

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        assert_eq!(decoded.info.source_gamma, Some(45455));
        assert!(decoded.info.srgb_intent.is_none());
        let dc = decoded.info.chromaticities.expect("cHRM missing");
        assert_eq!(dc.white_x, 31270);
        assert_eq!(dc.white_y, 32900);
        assert_eq!(dc.red_x, 64000);
        assert_eq!(dc.red_y, 33000);
        assert_eq!(dc.green_x, 30000);
        assert_eq!(dc.green_y, 60000);
        assert_eq!(dc.blue_x, 15000);
        assert_eq!(dc.blue_y, 6000);
    }

    #[test]
    fn cicp_suppresses_srgb_gama_chrm() {
        // PNGv3 precedence: cICP suppresses sRGB, gAMA, cHRM
        use crate::decode::PngChromaticities;
        use zc::{Cicp, MetadataView};

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let cicp = Cicp::new(9, 16, 0, true);
        let meta = MetadataView::none().with_cicp(cicp);

        let chrm = PngChromaticities {
            white_x: 31270,
            white_y: 32900,
            red_x: 64000,
            red_y: 33000,
            green_x: 30000,
            green_y: 60000,
            blue_x: 15000,
            blue_y: 6000,
        };

        let config = crate::encode::EncodeConfig {
            source_gamma: Some(45455),
            srgb_intent: Some(0),
            chromaticities: Some(chrm),
            ..Default::default()
        };

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            Some(&meta),
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        // cICP present, everything else suppressed
        assert!(decoded.info.cicp.is_some());
        assert_eq!(decoded.info.srgb_intent, None);
        assert_eq!(decoded.info.source_gamma, None);
        assert!(decoded.info.chromaticities.is_none());
    }

    #[test]
    fn iccp_suppresses_srgb_gama_chrm() {
        // PNGv3 precedence: iCCP suppresses sRGB, gAMA, cHRM
        use crate::decode::PngChromaticities;
        use zc::MetadataView;

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        // Minimal valid ICC profile (just needs to be non-empty for the test)
        // Use a real sRGB profile header so iCCP decompression works
        // Minimal byte sequence — just needs to survive zlib round-trip
        let srgb_icc: &[u8] = &[0u8; 128];
        let meta = MetadataView::none().with_icc(srgb_icc);

        let chrm = PngChromaticities {
            white_x: 31270,
            white_y: 32900,
            red_x: 64000,
            red_y: 33000,
            green_x: 30000,
            green_y: 60000,
            blue_x: 15000,
            blue_y: 6000,
        };

        let config = crate::encode::EncodeConfig {
            source_gamma: Some(45455),
            srgb_intent: Some(0),
            chromaticities: Some(chrm),
            ..Default::default()
        };

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            Some(&meta),
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        // iCCP present, sRGB/gAMA/cHRM suppressed
        assert!(decoded.info.icc_profile.is_some());
        assert_eq!(decoded.info.srgb_intent, None);
        assert_eq!(decoded.info.source_gamma, None);
        assert!(decoded.info.chromaticities.is_none());
    }

    #[test]
    fn cicp_with_iccp_fallback() {
        // PNGv3: cICP + iCCP both written (iCCP as fallback for limited cICP support)
        use zc::{Cicp, MetadataView};

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let cicp = Cicp::new(1, 13, 0, true);
        // Minimal byte sequence — just needs to survive zlib round-trip
        let srgb_icc: &[u8] = &[0u8; 128];
        let meta = MetadataView::none().with_cicp(cicp).with_icc(srgb_icc);

        let config = crate::encode::EncodeConfig::default();

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            Some(&meta),
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        // Both cICP and iCCP present
        assert!(decoded.info.cicp.is_some());
        assert!(decoded.info.icc_profile.is_some());
        // sRGB/gAMA/cHRM suppressed
        assert_eq!(decoded.info.srgb_intent, None);
        assert_eq!(decoded.info.source_gamma, None);
        assert!(decoded.info.chromaticities.is_none());
    }

    #[test]
    fn hdr_metadata_always_written_with_cicp() {
        // mDCV and cLLi always written alongside cICP
        use zc::{Cicp, ContentLightLevel, MasteringDisplay, MetadataView};

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let cicp = Cicp::new(9, 16, 0, true);
        let clli = ContentLightLevel::new(1000, 400);
        let mdcv = MasteringDisplay::new(
            [[35400, 14600], [8500, 39850], [6550, 2300]],
            [15635, 16450],
            10000000,
            50,
        );
        let meta = MetadataView::none()
            .with_cicp(cicp)
            .with_content_light_level(clli)
            .with_mastering_display(mdcv);

        let config = crate::encode::EncodeConfig {
            source_gamma: Some(45455),
            srgb_intent: Some(0),
            ..Default::default()
        };

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            Some(&meta),
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        // cICP + HDR metadata present
        assert!(decoded.info.cicp.is_some());
        assert!(decoded.info.content_light_level.is_some());
        assert!(decoded.info.mastering_display.is_some());
        // sRGB/gAMA suppressed by cICP
        assert_eq!(decoded.info.srgb_intent, None);
        assert_eq!(decoded.info.source_gamma, None);
    }

    #[test]
    fn chrm_negative_values_roundtrip() {
        // Wide-gamut spaces can have negative chromaticity values
        use crate::decode::PngChromaticities;

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        // ACES AP1 has a negative blue_x primary
        let chrm = PngChromaticities {
            white_x: 32168,
            white_y: 33767,
            red_x: 71300,
            red_y: 29300,
            green_x: 16500,
            green_y: 83000,
            blue_x: -12800, // negative — imaginary primary
            blue_y: 4400,
        };

        let config = crate::encode::EncodeConfig {
            source_gamma: Some(100000), // linear
            chromaticities: Some(chrm),
            ..Default::default()
        };

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        let dc = decoded.info.chromaticities.expect("cHRM missing");
        assert_eq!(dc.blue_x, -12800);
        assert_eq!(dc.blue_y, 4400);
        assert_eq!(dc.white_x, 32168);
    }

    #[test]
    fn cicp_roundtrip() {
        use zc::{Cicp, MetadataView};

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let cicp = Cicp::new(9, 16, 0, true); // BT.2020 primaries, PQ transfer, Identity matrix (PNG requirement)
        let meta = MetadataView::none().with_cicp(cicp);
        let config = crate::encode::EncodeConfig::default();

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            Some(&meta),
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        let dc = decoded.info.cicp.expect("cICP missing");
        assert_eq!(dc.color_primaries, 9);
        assert_eq!(dc.transfer_characteristics, 16);
        assert_eq!(dc.matrix_coefficients, 0);
        assert!(dc.full_range);
    }

    #[test]
    fn clli_mdcv_roundtrip() {
        use zc::{ContentLightLevel, MasteringDisplay, MetadataView};

        let pixels = vec![
            Rgb::<u8> {
                r: 128,
                g: 64,
                b: 32
            };
            4
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        let clli = ContentLightLevel::new(1000, 400);
        let mdcv = MasteringDisplay::new(
            [[35400, 14600], [8500, 39850], [6550, 2300]], // R, G, B primaries
            [15635, 16450],                                // white point
            10000000,                                      // 1000 cd/m²
            50,                                            // 0.005 cd/m²
        );
        let meta = MetadataView::none()
            .with_content_light_level(clli)
            .with_mastering_display(mdcv);
        let config = crate::encode::EncodeConfig::default();

        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            Some(&meta),
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();

        let dc = decoded.info.content_light_level.expect("cLLi missing");
        assert_eq!(dc.max_content_light_level, 1000);
        assert_eq!(dc.max_frame_average_light_level, 400);

        let dm = decoded.info.mastering_display.expect("mDCV missing");
        assert_eq!(dm.primaries, [[35400, 14600], [8500, 39850], [6550, 2300]]);
        assert_eq!(dm.white_point, [15635, 16450]);
        assert_eq!(dm.max_luminance, 10000000);
        assert_eq!(dm.min_luminance, 50);
    }

    // ── Real-file roundtrip tests ────────────────────────────────────

    /// Decode a real PNG with gAMA+cHRM (no sRGB), re-encode preserving
    /// the color metadata, decode again, and verify exact roundtrip.
    #[test]
    fn real_file_gama_chrm_roundtrip() {
        let corpus = std::env::var("CODEC_CORPUS_DIR")
            .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string());
        let path = format!("{corpus}/imageflow/test_inputs/frymire.png");
        let data = std::fs::read(&path).expect("frymire.png not found");

        // Decode original
        let orig =
            crate::decode::decode(&data, &PngDecodeConfig::none(), &enough::Unstoppable).unwrap();
        let gamma = orig
            .info
            .source_gamma
            .expect("frymire.png should have gAMA");
        let chrm = orig
            .info
            .chromaticities
            .expect("frymire.png should have cHRM");
        assert!(
            orig.info.srgb_intent.is_none(),
            "frymire.png should NOT have sRGB"
        );

        // gamma=0.45454 (raw u32=45454)
        assert_eq!(gamma, 45454);

        // Re-encode with same metadata
        let config = crate::encode::EncodeConfig {
            source_gamma: Some(gamma),
            chromaticities: Some(chrm),
            ..Default::default()
        };
        let pixels = orig
            .pixels
            .try_as_imgref::<Rgb<u8>>()
            .expect("frymire.png should decode as RGB8");
        let encoded = crate::encode::encode_rgb8(
            pixels,
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        // Decode re-encoded
        let rt = crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
            .unwrap();
        assert_eq!(rt.info.source_gamma, Some(gamma));
        let rt_chrm = rt.info.chromaticities.expect("re-encoded should have cHRM");
        assert_eq!(rt_chrm, chrm);
        assert!(rt.info.srgb_intent.is_none());
    }

    /// Decode a real PNG with sRGB+gAMA+cHRM, re-encode, verify roundtrip.
    /// PNGv3 precedence: sRGB suppresses gAMA/cHRM in output.
    #[test]
    fn real_file_srgb_roundtrip() {
        let corpus = std::env::var("CODEC_CORPUS_DIR")
            .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string());
        let path = format!("{corpus}/imageflow/test_inputs/red-night.png");
        let data = std::fs::read(&path).expect("red-night.png not found");

        let orig =
            crate::decode::decode(&data, &PngDecodeConfig::none(), &enough::Unstoppable).unwrap();
        let intent = orig
            .info
            .srgb_intent
            .expect("red-night.png should have sRGB");
        // Original file has gAMA+cHRM alongside sRGB (pre-PNGv3 practice)
        assert!(orig.info.source_gamma.is_some());
        assert!(orig.info.chromaticities.is_some());

        assert_eq!(intent, 0); // Perceptual

        // Re-encode with all color metadata
        let config = crate::encode::EncodeConfig {
            source_gamma: orig.info.source_gamma,
            srgb_intent: Some(intent),
            chromaticities: orig.info.chromaticities,
            ..Default::default()
        };
        let pixels = orig
            .pixels
            .try_as_imgref::<Rgba<u8>>()
            .expect("red-night.png should decode as RGBA8");
        let encoded = crate::encode::encode_rgba8(
            pixels,
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let rt = crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
            .unwrap();
        // PNGv3: sRGB present → gAMA/cHRM suppressed in output
        assert_eq!(rt.info.srgb_intent, Some(0));
        assert_eq!(rt.info.source_gamma, None);
        assert!(rt.info.chromaticities.is_none());
    }

    /// Decode a real PNG with iCCP (Adobe RGB), re-encode preserving the
    /// ICC profile, verify the profile roundtrips.
    #[test]
    fn real_file_icc_roundtrip() {
        let corpus = std::env::var("CODEC_CORPUS_DIR")
            .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string());
        let path = format!("{corpus}/imageflow/test_inputs/shirt_transparent.png");
        let data = std::fs::read(&path).expect("shirt_transparent.png not found");

        let orig =
            crate::decode::decode(&data, &PngDecodeConfig::none(), &enough::Unstoppable).unwrap();
        let icc = orig
            .info
            .icc_profile
            .as_ref()
            .expect("shirt_transparent.png should have iCCP");
        assert!(!icc.is_empty());

        // Re-encode with ICC profile
        let meta = zc::MetadataView::none().with_icc(icc);
        let config = crate::encode::EncodeConfig::default();
        let pixels = orig
            .pixels
            .try_as_imgref::<Rgba<u8>>()
            .expect("shirt_transparent.png should decode as RGBA8");
        let encoded = crate::encode::encode_rgba8(
            pixels,
            Some(&meta),
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let rt = crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
            .unwrap();
        let rt_icc = rt
            .info
            .icc_profile
            .as_ref()
            .expect("re-encoded should have iCCP");
        assert_eq!(icc, rt_icc);
    }

    #[test]
    fn test_clic_0d154_roundtrip() {
        // This 1024x1024 RGB image triggered a zenflate compression bug
        // at L4 with adaptive MinSum filtering (corrupt deflate stream).
        // The decompression verification in compress_filtered now catches this.
        let corpus = std::env::var("CODEC_CORPUS_DIR")
            .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string());
        let path = format!(
            "{corpus}/clic2025-1024/0d154749c7771f58e89ad343653ec4e20d6f037da829f47f5598e5d0a4ab61f0.png"
        );
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => return, // skip if corpus not available
        };
        let decoded =
            crate::decode::decode(&data, &PngDecodeConfig::none(), &enough::Unstoppable).unwrap();
        let info = &decoded.info;
        let rgb_pixels = decoded
            .pixels
            .try_as_imgref::<Rgb<u8>>()
            .unwrap_or_else(|| panic!("expected Rgb8, got {:?}", decoded.pixels.descriptor()));

        for (name, comp) in [
            ("Fastest", crate::Compression::Fastest),
            ("Fast", crate::Compression::Fast),
            ("Balanced", crate::Compression::Balanced),
            ("Thorough", crate::Compression::Thorough),
            ("High", crate::Compression::High),
            ("Aggressive", crate::Compression::Aggressive),
        ] {
            let config = crate::EncodeConfig {
                source_gamma: info.source_gamma,
                srgb_intent: info.srgb_intent,
                chromaticities: info.chromaticities,
                compression: comp,
                ..Default::default()
            };
            let encoded = crate::encode::encode_rgb8(
                rgb_pixels,
                None,
                &config,
                &enough::Unstoppable,
                &enough::Unstoppable,
            )
            .unwrap();
            match crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable) {
                Ok(_) => {}
                Err(e) => panic!("{name}: full PNG re-decode failed: {e}"),
            }
        }
    }

    #[test]
    fn test_large_rgb8_roundtrip() {
        // Test with a 1024x1024 gradient image to catch compression bugs at ~3MiB
        let width = 1024usize;
        let height = 1024usize;
        let pixels: Vec<rgb::Rgb<u8>> = (0..width * height)
            .map(|i| {
                let x = i % width;
                let y = i / width;
                rgb::Rgb {
                    r: (x & 0xFF) as u8,
                    g: (y & 0xFF) as u8,
                    b: ((x + y) & 0xFF) as u8,
                }
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, width, height);
        let config = crate::EncodeConfig::default();
        let encoded = crate::encode::encode_rgb8(
            img.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        let dec_img = decoded
            .pixels
            .try_as_imgref::<Rgb<u8>>()
            .unwrap_or_else(|| panic!("expected Rgb8, got {:?}", decoded.pixels.descriptor()));
        assert_eq!(dec_img.width(), width);
        assert_eq!(dec_img.height(), height);
    }

    #[test]
    fn immediate_cancel_encode_returns_stopped() {
        use enough::{Stop, StopReason};

        /// A Stop that always says "stop now".
        struct AlreadyCancelled;
        impl Stop for AlreadyCancelled {
            fn check(&self) -> Result<(), StopReason> {
                Err(StopReason::Cancelled)
            }
        }

        let config = PngEncoderConfig::new();
        let job = EncoderConfig::job(&config);
        let job = EncodeJob::with_stop(job, &AlreadyCancelled);
        let encoder = EncodeJob::encoder(job).unwrap();

        // Create a small 2x2 RGB8 image
        let pixels = vec![Rgb { r: 0u8, g: 0, b: 0 }; 4];
        let img = Img::new(pixels, 2, 2);
        let slice = PixelSlice::from(img.as_ref());

        use zc::encode::Encoder;
        let result = encoder.encode(slice.erase());
        assert!(result.is_err());
        match result.unwrap_err().into_inner() {
            PngError::Stopped(reason) => {
                assert_eq!(reason, StopReason::Cancelled);
            }
            other => panic!("expected PngError::Stopped, got: {other}"),
        }
    }

    #[test]
    fn zero_deadline_encode_still_succeeds() {
        // An immediately-expired deadline should still succeed by returning a fast result
        // (at least one strategy is always tried before checking the deadline)
        let config = crate::EncodeConfig::default();
        let deadline =
            almost_enough::time::WithTimeout::new(enough::Unstoppable, std::time::Duration::ZERO);

        let pixels = vec![
            rgb::Rgba {
                r: 255u8,
                g: 0,
                b: 0,
                a: 255,
            };
            16
        ];
        let img = imgref::ImgVec::new(pixels, 4, 4);
        let result = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &deadline,
        );
        assert!(result.is_ok(), "zero-deadline encode should still succeed");

        // Verify it decodes
        let encoded = result.unwrap();
        let decoded =
            crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
                .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
    }

    #[test]
    fn immediate_cancel_decode_returns_stopped() {
        use enough::{Stop, StopReason};

        struct AlreadyCancelled;
        impl Stop for AlreadyCancelled {
            fn check(&self) -> Result<(), StopReason> {
                Err(StopReason::Cancelled)
            }
        }

        // First encode a valid PNG
        let pixels = vec![
            rgb::Rgba {
                r: 128u8,
                g: 64,
                b: 32,
                a: 255,
            };
            16
        ];
        let img = imgref::ImgVec::new(pixels, 4, 4);
        let config = crate::EncodeConfig::default();
        let encoded = crate::encode::encode_rgba8(
            img.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        // Now try to decode with immediate cancel
        let result = crate::decode::decode(&encoded, &PngDecodeConfig::none(), &AlreadyCancelled);
        assert!(result.is_err());
        match result.unwrap_err() {
            PngError::Stopped(reason) => {
                assert_eq!(reason, StopReason::Cancelled);
            }
            other => panic!("expected PngError::Stopped, got: {other}"),
        }
    }

    #[test]
    fn quality_getter_setter() {
        // Default → no quality set, lossless
        let enc = PngEncoderConfig::new();
        assert_eq!(enc.generic_quality(), None);
        assert_eq!(enc.is_lossless(), Some(true));

        // quality 100.0 → still lossless
        let enc = PngEncoderConfig::new().with_generic_quality(100.0);
        assert_eq!(enc.generic_quality(), Some(100.0));
        assert_eq!(enc.is_lossless(), Some(true));

        // quality < 100.0 → lossy
        let enc = PngEncoderConfig::new().with_generic_quality(90.0);
        assert_eq!(enc.generic_quality(), Some(90.0));
        assert_eq!(enc.is_lossless(), Some(false));

        // quality 0.0 → lossy
        let enc = PngEncoderConfig::new().with_generic_quality(0.0);
        assert_eq!(enc.generic_quality(), Some(0.0));
        assert_eq!(enc.is_lossless(), Some(false));

        // effort + quality compose independently
        let enc = PngEncoderConfig::new()
            .with_generic_effort(5)
            .with_generic_quality(75.0);
        assert_eq!(enc.generic_effort(), Some(5));
        assert_eq!(enc.generic_quality(), Some(75.0));
        assert_eq!(enc.is_lossless(), Some(false));
    }

    #[test]
    fn quality_to_mpe_curve() {
        // Exact table points
        assert_eq!(quality_to_mpe(100.0), 0.0);
        assert_eq!(quality_to_mpe(99.0), 0.001);
        assert_eq!(quality_to_mpe(0.0), 0.100);

        // JPEG-equivalent calibration points
        let mpe_95 = quality_to_mpe(95.0);
        assert!((mpe_95 - 0.008).abs() < 0.001, "q95 mpe={mpe_95}");

        let mpe_90 = quality_to_mpe(90.0);
        assert!((mpe_90 - 0.012).abs() < 0.001, "q90 mpe={mpe_90}");

        let mpe_75 = quality_to_mpe(75.0);
        assert!((mpe_75 - 0.020).abs() < 0.001, "q75 mpe={mpe_75}");

        let mpe_50 = quality_to_mpe(50.0);
        assert!((mpe_50 - 0.028).abs() < 0.001, "q50 mpe={mpe_50}");

        // Interpolated mid-point: q97 between q99 (0.001) and q95 (0.008)
        let mpe_97 = quality_to_mpe(97.0);
        assert!(mpe_97 > 0.001 && mpe_97 < 0.008, "q97 mpe={mpe_97}");

        // Monotonicity: lower quality → higher MPE (more tolerant)
        assert!(quality_to_mpe(90.0) > quality_to_mpe(99.0));
        assert!(quality_to_mpe(75.0) > quality_to_mpe(90.0));
        assert!(quality_to_mpe(50.0) > quality_to_mpe(75.0));
        assert!(quality_to_mpe(0.0) > quality_to_mpe(50.0));

        // Clamping: values outside 0-100 are clamped
        assert_eq!(quality_to_mpe(-10.0), quality_to_mpe(0.0));
        assert_eq!(quality_to_mpe(200.0), quality_to_mpe(100.0));
    }

    #[cfg(feature = "quantize")]
    #[test]
    fn quality_auto_indexed_rgba8() {
        // Create a simple image with few unique colors — should always quantize
        let pixels: Vec<Rgba<u8>> = vec![
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
            Rgba {
                r: 255,
                g: 255,
                b: 0,
                a: 255,
            },
        ];
        let img = imgref::ImgVec::new(pixels, 2, 2);

        // Lossless encode (no quality set)
        let enc_lossless = PngEncoderConfig::new();
        let out_lossless = enc_lossless.encode_rgba8(img.as_ref()).unwrap();

        // Lossy encode (quality 90 → auto-indexed)
        let enc_lossy = PngEncoderConfig::new().with_generic_quality(90.0);
        let out_lossy = enc_lossy.encode_rgba8(img.as_ref()).unwrap();

        // Both should produce valid PNG
        assert_eq!(
            &out_lossless.data()[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert_eq!(
            &out_lossy.data()[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );

        // With only 4 colors, auto-indexed should produce a PLTE chunk
        // (indexed PNG is smaller than truecolor for few-color images)
        let has_plte = out_lossy.data().windows(4).any(|w| w == b"PLTE");
        assert!(
            has_plte,
            "expected indexed PNG with PLTE chunk for 4-color image"
        );

        // Both should decode correctly
        let dec = PngDecoderConfig::new();
        let d_lossless = dec.decode(out_lossless.data()).unwrap();
        let d_lossy = dec.decode(out_lossy.data()).unwrap();
        assert_eq!(d_lossless.width(), 2);
        assert_eq!(d_lossy.width(), 2);
    }

    // ── Builder methods ──

    #[test]
    fn with_compression_sets_config() {
        let enc = PngEncoderConfig::new().with_compression(crate::Compression::Turbo);
        let pixels = vec![Rgb { r: 0, g: 0, b: 0 }; 4];
        let img = Img::new(pixels, 2, 2);
        let out = enc.encode_rgb8(img.as_ref()).unwrap();
        assert!(!out.data().is_empty());
    }

    #[test]
    fn with_filter_sets_config() {
        let enc = PngEncoderConfig::new().with_filter(crate::Filter::Auto);
        let pixels = vec![Rgb { r: 0, g: 0, b: 0 }; 4];
        let img = Img::new(pixels, 2, 2);
        let out = enc.encode_rgb8(img.as_ref()).unwrap();
        assert!(!out.data().is_empty());
    }

    #[test]
    fn default_encoder_config() {
        let enc: PngEncoderConfig = Default::default();
        assert!(enc.generic_effort().is_none());
        assert!(enc.generic_quality().is_none());
    }

    // ── 16-bit encode convenience methods ──

    #[test]
    fn encode_rgb16_roundtrip() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![
            Rgb {
                r: 1000u16,
                g: 2000,
                b: 3000
            };
            4
        ];
        let img = Img::new(pixels, 2, 2);
        let out = enc.encode_rgb16(img.as_ref()).unwrap();
        assert!(!out.data().is_empty());
        assert_eq!(
            &out.data()[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn encode_rgba16_roundtrip() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![
            Rgba {
                r: 1000u16,
                g: 2000,
                b: 3000,
                a: 65535
            };
            4
        ];
        let img = Img::new(pixels, 2, 2);
        let out = enc.encode_rgba16(img.as_ref()).unwrap();
        assert!(!out.data().is_empty());
    }

    #[test]
    fn encode_gray16_roundtrip() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![Gray::new(30000u16); 4];
        let img = Img::new(pixels, 2, 2);
        let out = enc.encode_gray16(img.as_ref()).unwrap();
        assert!(!out.data().is_empty());
    }

    // ── effort_to_compression coverage ──

    #[test]
    fn effort_to_compression_all_levels() {
        use crate::Compression;
        // Cover all branches including the uncovered ones (2-8, 12+)
        assert!(matches!(effort_to_compression(-1), Compression::None));
        assert!(matches!(effort_to_compression(0), Compression::None));
        assert!(matches!(effort_to_compression(1), Compression::Fastest));
        assert!(matches!(effort_to_compression(2), Compression::Turbo));
        assert!(matches!(effort_to_compression(3), Compression::Fast));
        assert!(matches!(effort_to_compression(4), Compression::Balanced));
        assert!(matches!(effort_to_compression(5), Compression::Thorough));
        assert!(matches!(effort_to_compression(6), Compression::High));
        assert!(matches!(effort_to_compression(7), Compression::Aggressive));
        assert!(matches!(effort_to_compression(8), Compression::Intense));
        assert!(matches!(effort_to_compression(9), Compression::Crush));
        assert!(matches!(effort_to_compression(10), Compression::Maniac));
        assert!(matches!(effort_to_compression(11), Compression::Brag));
        assert!(matches!(effort_to_compression(12), Compression::Minutes));
        assert!(matches!(effort_to_compression(100), Compression::Minutes));
    }

    // ── quality_to_mpe coverage ──

    #[test]
    fn quality_to_mpe_endpoints() {
        // q=100 → mpe=0.0
        assert_eq!(quality_to_mpe(100.0), 0.0);
        // q=0 → mpe=0.1
        assert_eq!(quality_to_mpe(0.0), 0.1);
        // q > 100 → clamped to 100 → 0.0
        assert_eq!(quality_to_mpe(150.0), 0.0);
        // q < 0 → clamped to 0 → 0.1
        assert_eq!(quality_to_mpe(-10.0), 0.1);
    }

    #[test]
    fn quality_to_mpe_interpolation() {
        // q=95 should be 0.008
        assert!((quality_to_mpe(95.0) - 0.008).abs() < 0.0001);
        // q=50 should be 0.028
        assert!((quality_to_mpe(50.0) - 0.028).abs() < 0.0001);
        // Interpolated value between 95 and 99
        let mid = quality_to_mpe(97.0);
        assert!(mid > 0.001 && mid < 0.008);
    }

    // ── EncodeJob trait methods ──

    #[test]
    fn encode_job_with_stop() {
        let enc = PngEncoderConfig::new();
        let stop = enough::Unstoppable;
        let job = enc.job().with_stop(&stop);
        let encoder = job.encoder().unwrap();
        let pixels = vec![Rgb { r: 0u8, g: 0, b: 0 }; 4];
        let img = Img::new(pixels, 2, 2);
        let out = encoder.do_encode(
            bytemuck::cast_slice(img.buf()),
            2,
            2,
            crate::encode::ColorType::Rgb,
        );
        assert!(out.is_ok());
    }

    #[test]
    fn encode_job_with_limits() {
        let enc = PngEncoderConfig::new();
        let limits = ResourceLimits::default();
        let job = enc.job().with_limits(limits);
        let encoder = job.encoder().unwrap();
        let pixels = vec![Rgb { r: 0u8, g: 0, b: 0 }; 4];
        let img = Img::new(pixels, 2, 2);
        let out = encoder.do_encode(
            bytemuck::cast_slice(img.buf()),
            2,
            2,
            crate::encode::ColorType::Rgb,
        );
        assert!(out.is_ok());
    }

    #[test]
    fn encode_job_with_metadata() {
        let enc = PngEncoderConfig::new();
        let meta = MetadataView::default();
        let job = enc.job().with_metadata(&meta);
        let encoder = job.encoder().unwrap();
        let pixels = vec![Rgb { r: 0u8, g: 0, b: 0 }; 4];
        let img = Img::new(pixels, 2, 2);
        let out = encoder.do_encode(
            bytemuck::cast_slice(img.buf()),
            2,
            2,
            crate::encode::ColorType::Rgb,
        );
        assert!(out.is_ok());
    }

    #[test]
    fn encode_job_frame_encoder() {
        let enc = PngEncoderConfig::new();
        let job = enc.job().with_canvas_size(8, 8).with_loop_count(Some(0));
        let frame_enc = job.frame_encoder();
        assert!(frame_enc.is_ok());
    }

    // ── EncoderConfig trait ──

    #[test]
    fn encoder_supported_descriptors() {
        let descs = <PngEncoderConfig as EncoderConfig>::supported_descriptors();
        assert!(!descs.is_empty());
        assert!(descs.contains(&PixelDescriptor::RGB8_SRGB));
        assert!(descs.contains(&PixelDescriptor::RGBA8_SRGB));
    }

    #[test]
    fn encoder_is_lossless() {
        let enc = PngEncoderConfig::new();
        assert_eq!(enc.is_lossless(), Some(true));
        let enc_lossy = enc.with_generic_quality(90.0);
        assert_eq!(enc_lossy.is_lossless(), Some(false));
    }

    // ── Encoder trait roundtrip tests ──

    #[test]
    fn encoder_trait_rgb8() {
        use zc::encode::Encoder;
        let pixels: Vec<Rgb<u8>> = (0..16 * 16)
            .map(|i| Rgb {
                r: (i % 256) as u8,
                g: ((i * 3) % 256) as u8,
                b: ((i * 7) % 256) as u8,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgba8() {
        use zc::encode::Encoder;
        let pixels: Vec<Rgba<u8>> = (0..16 * 16)
            .map(|i| Rgba {
                r: (i % 256) as u8,
                g: ((i * 3) % 256) as u8,
                b: ((i * 7) % 256) as u8,
                a: 255,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_gray8() {
        use zc::encode::Encoder;
        let pixels: Vec<Gray<u8>> = (0..16 * 16).map(|i| Gray((i % 256) as u8)).collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgb16() {
        use zc::encode::Encoder;
        let pixels: Vec<Rgb<u16>> = (0..16 * 16)
            .map(|i| Rgb {
                r: (i * 256) as u16,
                g: ((i * 3 * 256) % 65536) as u16,
                b: 0,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgba16() {
        use zc::encode::Encoder;
        let pixels: Vec<Rgba<u16>> = (0..16 * 16)
            .map(|i| Rgba {
                r: (i * 256) as u16,
                g: ((i * 3 * 256) % 65536) as u16,
                b: 0,
                a: 65535,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_gray16() {
        use zc::encode::Encoder;
        let pixels: Vec<Gray<u16>> = (0..16 * 16).map(|i| Gray((i * 256) as u16)).collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgb_f32() {
        use zc::encode::Encoder;
        let pixels: Vec<Rgb<f32>> = (0..16 * 16)
            .map(|i| Rgb {
                r: (i % 256) as f32 / 255.0,
                g: ((i * 3) % 256) as f32 / 255.0,
                b: ((i * 7) % 256) as f32 / 255.0,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgba_f32() {
        use zc::encode::Encoder;
        let pixels: Vec<Rgba<f32>> = (0..16 * 16)
            .map(|i| Rgba {
                r: (i % 256) as f32 / 255.0,
                g: ((i * 3) % 256) as f32 / 255.0,
                b: ((i * 7) % 256) as f32 / 255.0,
                a: 1.0,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_gray_f32() {
        use zc::encode::Encoder;
        let pixels: Vec<Gray<f32>> = (0..16 * 16)
            .map(|i| Gray((i % 256) as f32 / 255.0))
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_bgra8() {
        use zc::encode::Encoder;
        let pixels: Vec<rgb::alt::BGRA<u8>> = (0..16 * 16)
            .map(|i| rgb::alt::BGRA {
                b: (i % 256) as u8,
                g: 128,
                r: 64,
                a: 255,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_dyn_encoder() {
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 100,
                g: 150,
                b: 200,
            };
            32 * 32
        ];
        let img = imgref::ImgVec::new(pixels, 32, 32);
        let config = PngEncoderConfig::new();
        let dyn_enc = config.job().dyn_encoder().unwrap();
        let output = dyn_enc
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }
}
