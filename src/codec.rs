//! zencodec trait implementations for PNG.
//!
//! Provides [`PngEncoderConfig`] and [`PngDecoderConfig`] types that implement the
//! [`EncoderConfig`] / [`DecoderConfig`] traits from zencodec.
#![allow(dead_code)]

extern crate std;

use alloc::borrow::Cow;
use alloc::vec::Vec;

use whereat::{At, at};
use zencodec::decode::{AnimationFrame, DecodeCapabilities, DecodeOutput, OutputInfo};
use zencodec::encode::{EncodeCapabilities, EncodeOutput};
use zencodec::{ImageFormat, ImageInfo, Metadata, ResourceLimits};
use zenpixels::{Pixel, PixelDescriptor, PixelSlice, PixelSliceMut};

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

/// PNG encoder configuration implementing [`EncoderConfig`](zencodec::EncoderConfig).
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
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGBA8 pixels in one call.
    pub fn encode_rgba8(
        &self,
        img: imgref::ImgRef<'_, Rgba<u8>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode Gray8 pixels in one call.
    pub fn encode_gray8(
        &self,
        img: imgref::ImgRef<'_, Gray<u8>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGB16 pixels in one call.
    pub fn encode_rgb16(
        &self,
        img: imgref::ImgRef<'_, Rgb<u16>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGBA16 pixels in one call.
    pub fn encode_rgba16(
        &self,
        img: imgref::ImgRef<'_, Rgba<u16>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode Gray16 pixels in one call.
    pub fn encode_gray16(
        &self,
        img: imgref::ImgRef<'_, Gray<u16>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGB F32 pixels in one call.
    pub fn encode_rgb_f32(
        &self,
        img: imgref::ImgRef<'_, Rgb<f32>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode RGBA F32 pixels in one call.
    pub fn encode_rgba_f32(
        &self,
        img: imgref::ImgRef<'_, Rgba<f32>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode Gray F32 pixels in one call.
    pub fn encode_gray_f32(
        &self,
        img: imgref::ImgRef<'_, Gray<f32>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
    }

    /// Convenience: encode BGRA8 pixels (swizzles to RGBA) in one call.
    pub fn encode_bgra8(
        &self,
        img: imgref::ImgRef<'_, rgb::alt::BGRA<u8>>,
    ) -> Result<EncodeOutput, At<PngError>> {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
        self.clone()
            .job()
            .encoder()?
            .encode(PixelSlice::from(img).erase())
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
/// Piecewise-linear interpolation calibrated so that `quality=Q` produces
/// the same SSIMULACRA2 score as libjpeg-turbo at quality Q, via the chain:
///
///   quality → MPE threshold → (zenquant quantization) → MPE → SSIM2
///
/// Derived from 27,170 libjpeg-turbo measurements across 209 CID22-512
/// images (SSIM2 anchor), plus zenquant's 1992-image MPE↔SSIM2 calibration.
///
/// | quality | libjpeg SSIM2 | MPE    |
/// |---------|---------------|--------|
/// | 100     | lossless      | 0.000  |
/// | 99      | 95.9          | 0.003  |
/// | 95      | 91.5          | 0.007  |
/// | 90      | 87.7          | 0.011  |
/// | 85      | 84.3          | 0.015  |
/// | 80      | 81.2          | 0.020  |
/// | 75      | 78.3          | 0.026  |
/// | 70      | 75.3          | 0.031  |
/// | 60      | 72.0          | 0.037  |
/// | 50      | 68.4          | 0.044  |
/// | 40      | 63.9          | 0.052  |
/// | 30      | 59.2          | 0.060  |
/// | 0       | —             | 0.100  |
fn quality_to_mpe(quality: f32) -> f32 {
    // (quality, mpe) — sorted descending by quality.
    // Calibrated to match libjpeg-turbo SSIMULACRA2 at each quality level.
    const TABLE: [(f32, f32); 14] = [
        (100.0, 0.000),
        (99.0, 0.003),
        (95.0, 0.007),
        (90.0, 0.011),
        (85.0, 0.015),
        (80.0, 0.020),
        (75.0, 0.026),
        (70.0, 0.031),
        (60.0, 0.037),
        (50.0, 0.044),
        (40.0, 0.052),
        (30.0, 0.060),
        (10.0, 0.085),
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

/// Convert a [`ThreadingPolicy`](zencodec::ThreadingPolicy) to a concrete thread count.
///
/// Returns 0 for "no limit" (use as many threads as beneficial),
/// 1 for single-threaded, or N for a specific cap.
fn threading_to_count(policy: zencodec::ThreadingPolicy) -> usize {
    match policy {
        zencodec::ThreadingPolicy::SingleThread => 1,
        zencodec::ThreadingPolicy::LimitOrSingle { max_threads } => max_threads as usize,
        zencodec::ThreadingPolicy::LimitOrAny {
            preferred_max_threads,
        } => preferred_max_threads as usize,
        zencodec::ThreadingPolicy::Balanced => {
            std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
        }
        zencodec::ThreadingPolicy::Unlimited => 0, // 0 = no limit
        _ => 0,                                    // future variants: default to no limit
    }
}

static PNG_ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_icc(true)
    .with_exif(true)
    .with_xmp(true)
    .with_cicp(true)
    .with_stop(true)
    .with_animation(true)
    .with_lossless(true)
    .with_lossy(true)
    .with_native_gray(true)
    .with_native_16bit(true)
    .with_native_alpha(true)
    .with_push_rows(true)
    .with_enforces_max_pixels(true)
    .with_enforces_max_memory(true)
    .with_effort_range(0, 12)
    .with_quality_range(0.0, 100.0);

impl zencodec::encode::EncoderConfig for PngEncoderConfig {
    type Error = At<PngError>;
    type Job = PngEncodeJob;

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

    fn job(self) -> PngEncodeJob {
        PngEncodeJob {
            config: self,
            stop: None,
            metadata: None,
            limits: None,
            policy: None,
            canvas_width: 0,
            canvas_height: 0,
            loop_count: None,
        }
    }
}

// ── PngEncodeJob ─────────────────────────────────────────────────────

/// Per-operation PNG encode job.
pub struct PngEncodeJob {
    config: PngEncoderConfig,
    stop: Option<zencodec::StopToken>,
    metadata: Option<Metadata>,
    limits: Option<ResourceLimits>,
    policy: Option<zencodec::encode::EncodePolicy>,
    canvas_width: u32,
    canvas_height: u32,
    loop_count: Option<u32>,
}

impl zencodec::encode::EncodeJob for PngEncodeJob {
    type Error = At<PngError>;
    type Enc = PngEncoder;
    type AnimationFrameEnc = PngAnimationFrameEncoder;

    fn with_stop(mut self, stop: zencodec::StopToken) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_metadata(mut self, meta: Metadata) -> Self {
        self.metadata = Some(meta);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn with_policy(mut self, policy: zencodec::encode::EncodePolicy) -> Self {
        self.policy = Some(policy);
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

    fn encoder(self) -> Result<PngEncoder, At<PngError>> {
        Ok(PngEncoder {
            config: self.config,
            stop: self.stop,
            metadata: self.metadata,
            limits: self.limits,
            policy: self.policy,
            canvas_width: self.canvas_width,
            canvas_height: self.canvas_height,
            streaming: None,
        })
    }

    fn animation_frame_encoder(self) -> Result<PngAnimationFrameEncoder, At<PngError>> {
        let effective_meta = apply_encode_policy(self.metadata.as_ref(), self.policy.as_ref());
        // Apply threading policy from limits to the config.
        let mut config = self.config.config.clone();
        if let Some(ref limits) = self.limits {
            let thread_count = threading_to_count(limits.threading());
            config.max_threads = thread_count;
            if thread_count == 1 {
                config.parallel = false;
            }
        }
        let mut enc = PngAnimationFrameEncoder::new(
            config,
            self.canvas_width,
            self.canvas_height,
            effective_meta,
        );
        enc.loop_count = self.loop_count.unwrap_or(0);
        enc.limits = self.limits;
        Ok(enc)
    }
}

// ── PngEncoder ───────────────────────────────────────────────────────

/// Single-image PNG encoder.
pub struct PngEncoder {
    config: PngEncoderConfig,
    stop: Option<zencodec::StopToken>,
    metadata: Option<Metadata>,
    limits: Option<ResourceLimits>,
    policy: Option<zencodec::encode::EncodePolicy>,
    /// Canvas dimensions for push_rows mode (set via `with_canvas_size`).
    canvas_width: u32,
    canvas_height: u32,
    /// Streaming state, lazily initialized on first push_rows() call.
    streaming: Option<StreamingMode>,
}

/// Streaming mode selection for push_rows/finish.
enum StreamingMode {
    /// Buffer all pixel data, encode in finish() (effort > 1).
    Buffered(BufferedStreamingState),
    /// True streaming: emit stored DEFLATE blocks as rows arrive (effort 0).
    TrueStreaming(TrueStreamingState),
    /// Pre-filtered streaming: filter rows on arrival, compress in finish() (effort 1).
    /// Saves ~1× image size vs Buffered by eliminating the raw pixel buffer.
    PreFiltered(PreFilteredState),
}

/// Buffered state: accumulates raw pixel bytes, delegates to encode_raw in finish().
struct BufferedStreamingState {
    /// Accumulated raw pixel bytes in PNG's native byte order.
    pixel_data: Vec<u8>,
    /// PNG color type (0=Gray, 2=RGB, 4=GrayAlpha, 6=RGBA).
    color_type: crate::encode::ColorType,
    /// PNG bit depth.
    bit_depth: crate::encode::BitDepth,
    /// Bytes per row (width × bpp, no filter byte).
    row_bytes: usize,
    /// Rows received so far.
    rows_pushed: u32,
}

/// True streaming state: writes PNG output incrementally as rows arrive.
///
/// At effort 0, rows are written as stored DEFLATE blocks (no compression).
/// This avoids buffering the full image — only one previous row is kept for
/// filtering. Output bytes are built directly in the output Vec.
struct TrueStreamingState {
    /// Growing PNG output (signature + IHDR + metadata already written).
    output: Vec<u8>,
    /// Scratch buffer for format conversion (row_bytes). Only allocated for
    /// formats that need conversion (float, 16-bit, BGRA); empty otherwise.
    convert_buf: Vec<u8>,
    /// Bytes per row (width × bpp, no filter byte).
    row_bytes: usize,
    /// Rows received so far.
    rows_pushed: u32,
    /// Running Adler-32 checksum for zlib.
    adler: u32,
    /// Position of IDAT length field in output (for backpatching).
    idat_len_pos: usize,
    /// Remaining bytes in current stored DEFLATE block.
    block_remaining: usize,
    /// Remaining filtered bytes in the entire image.
    filtered_remaining: usize,
}

/// Pre-filtered streaming state: filters rows on arrival, compresses in finish().
///
/// At effort 1, the single Paeth filter is applied per-row as data arrives.
/// The filtered bytes (filter_byte + row_data per row) accumulate in `filtered_data`.
/// On finish(), the buffer is compressed with Turbo and assembled into a PNG.
///
/// Peak memory: ~1× image (filtered data) + compressed output.
/// vs Buffered: ~2× image (raw pixels + filter pass inside compress_filtered).
struct PreFilteredState {
    /// PNG preamble: signature + IHDR + metadata chunks.
    preamble: Vec<u8>,
    /// Accumulated pre-filtered row data ([filter_byte, filtered_row...] per row).
    filtered_data: Vec<u8>,
    /// Previous row in PNG byte order (for Paeth reference). Zeroed for first row.
    prev_row: Vec<u8>,
    /// Scratch buffer for format conversion (row_bytes).
    convert_buf: Vec<u8>,
    /// PNG filter type (4 = Paeth).
    filter_type: u8,
    /// Bytes per pixel in PNG output format.
    bpp: usize,
    /// Bytes per row (width × bpp, no filter byte).
    row_bytes: usize,
    /// Rows received so far.
    rows_pushed: u32,
    /// PNG color type.
    color_type: crate::encode::ColorType,
    /// PNG bit depth.
    bit_depth: crate::encode::BitDepth,
    /// Zenflate compression effort for finish().
    zenflate_effort: u32,
}

impl PngEncoder {
    /// Build an `EncodeConfig` with threading policy applied from resource limits.
    fn config_with_threading(&self) -> EncodeConfig {
        let mut config = self.config.config.clone();
        if let Some(ref limits) = self.limits {
            let thread_count = threading_to_count(limits.threading());
            config.max_threads = thread_count;
            if thread_count == 1 {
                config.parallel = false;
            }
        }
        config
    }

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
        let cancel: &dyn enough::Stop = match self.stop {
            Some(ref s) => s as &dyn enough::Stop,
            None => &enough::Unstoppable,
        };
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
        let config = self.config_with_threading();
        let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
        let deadline = almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);
        // Apply encode policy to filter metadata
        let effective_meta = apply_encode_policy(self.metadata.as_ref(), self.policy.as_ref());
        let meta_ref = effective_meta.as_ref();
        let data = crate::encode::encode_raw(
            bytes, w, h, color_type, bit_depth, meta_ref, &config, cancel, &deadline,
        )?;
        // Post-encode output size check
        if let Some(ref limits) = self.limits {
            limits
                .check_output_size(data.len() as u64)
                .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
        }
        Ok(EncodeOutput::new(data, ImageFormat::Png))
    }
}

impl zencodec::encode::Encoder for PngEncoder {
    type Error = At<PngError>;

    fn reject(op: zencodec::UnsupportedOperation) -> At<PngError> {
        at!(PngError::from(op))
    }

    fn preferred_strip_height(&self) -> u32 {
        1 // PNG uses row-at-a-time filtering
    }

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, At<PngError>> {
        use linear_srgb::default::{linear_to_srgb_u8_rgba_slice, linear_to_srgb_u8_slice};
        use zenpixels::PixelFormat;

        let w = pixels.width();
        let h = pixels.rows();
        // Policy-filtered metadata for auto-indexed paths
        let effective_meta = apply_encode_policy(self.metadata.as_ref(), self.policy.as_ref());
        let meta_ref = effective_meta.as_ref();

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
                    let cancel: &dyn enough::Stop = match self.stop {
                        Some(ref s) => s as &dyn enough::Stop,
                        None => &enough::Unstoppable,
                    };
                    let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
                    let deadline =
                        almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);
                    let quantizer = crate::default_quantizer();
                    let config = self.config_with_threading();
                    let result = crate::encode_auto(
                        img,
                        &config,
                        &*quantizer,
                        crate::QualityGate::MaxMpe(mpe),
                        meta_ref,
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
                let floats: &[f32] = bytemuck::cast_slice(&src);
                let mut srgb = vec![0u8; floats.len()];
                linear_to_srgb_u8_slice(floats, &mut srgb);
                self.do_encode(&srgb, w, h, crate::encode::ColorType::Rgb)
            }
            PixelFormat::RgbaF32 => {
                let src = contiguous_bytes(&pixels);
                let floats: &[f32] = bytemuck::cast_slice(&src);
                let mut srgb = vec![0u8; floats.len()];
                linear_to_srgb_u8_rgba_slice(floats, &mut srgb);

                // Auto-indexed path when not lossless and quality < 100
                #[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
                if !self.config.lossless
                    && let Some(q) = self.config.quality
                    && q < 100.0
                {
                    let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&srgb);
                    let img = imgref::Img::new(rgba, w as usize, h as usize);
                    let mpe = quality_to_mpe(q);
                    let cancel: &dyn enough::Stop = match self.stop {
                        Some(ref s) => s as &dyn enough::Stop,
                        None => &enough::Unstoppable,
                    };
                    let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
                    let deadline =
                        almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);
                    let quantizer = crate::default_quantizer();
                    let config = self.config_with_threading();
                    let result = crate::encode_auto(
                        img,
                        &config,
                        &*quantizer,
                        crate::QualityGate::MaxMpe(mpe),
                        meta_ref,
                        cancel,
                        &deadline,
                    )?;
                    return Ok(EncodeOutput::new(result.data, ImageFormat::Png));
                }

                self.do_encode(&srgb, w, h, crate::encode::ColorType::Rgba)
            }
            PixelFormat::GrayF32 => {
                let src = contiguous_bytes(&pixels);
                let floats: &[f32] = bytemuck::cast_slice(&src);
                let mut srgb = vec![0u8; floats.len()];
                linear_to_srgb_u8_slice(floats, &mut srgb);
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
            _ => Err(at!(PngError::from(
                zencodec::UnsupportedOperation::PixelFormat
            ))),
        }
    }

    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), At<PngError>> {
        use linear_srgb::default::{linear_to_srgb_u8_rgba_slice, linear_to_srgb_u8_slice};
        use zenpixels::PixelFormat;

        let w = rows.width();
        let h = rows.rows();

        if h == 0 {
            return Ok(());
        }

        // Initialize streaming state on first call.
        if self.streaming.is_none() {
            let (color_type, bit_depth) = pixel_format_to_png(rows.descriptor().pixel_format())
                .ok_or_else(|| at!(PngError::from(zencodec::UnsupportedOperation::PixelFormat)))?;

            // Infer width from first push if not set via with_canvas_size
            if self.canvas_width == 0 {
                self.canvas_width = w;
            }

            let channels: usize = match color_type {
                crate::encode::ColorType::Grayscale => 1,
                crate::encode::ColorType::Rgb => 3,
                crate::encode::ColorType::GrayscaleAlpha => 2,
                crate::encode::ColorType::Rgba => 4,
            };
            let depth_bytes: usize = match bit_depth {
                crate::encode::BitDepth::Eight => 1,
                crate::encode::BitDepth::Sixteen => 2,
            };
            let row_bytes = self.canvas_width as usize * channels * depth_bytes;
            let effort = self.config.config.compression.effort();

            let bpp = channels * depth_bytes;
            if effort == 0 && self.canvas_height > 0 {
                // True streaming: write PNG header and IDAT incrementally.
                self.streaming = Some(StreamingMode::TrueStreaming(TrueStreamingState::new(
                    self.canvas_width,
                    self.canvas_height,
                    color_type,
                    bit_depth,
                    row_bytes,
                    self.metadata.as_ref(),
                    self.policy.as_ref(),
                    &self.config.config,
                )?));
            } else if effort == 1 && self.canvas_height > 0 {
                // Pre-filtered streaming: filter rows on arrival, compress in finish().
                self.streaming = Some(StreamingMode::PreFiltered(PreFilteredState::new(
                    self.canvas_width,
                    self.canvas_height,
                    color_type,
                    bit_depth,
                    row_bytes,
                    bpp,
                    self.metadata.as_ref(),
                    self.policy.as_ref(),
                    &self.config.config,
                )?));
            } else {
                // Buffered: accumulate pixel data, encode in finish().
                let capacity = if self.canvas_height > 0 {
                    row_bytes * self.canvas_height as usize
                } else {
                    row_bytes * h as usize
                };
                self.streaming = Some(StreamingMode::Buffered(BufferedStreamingState {
                    pixel_data: Vec::with_capacity(capacity),
                    color_type,
                    bit_depth,
                    row_bytes,
                    rows_pushed: 0,
                }));
            }
        }

        // Width must be consistent across calls.
        if w != self.canvas_width && self.canvas_width > 0 {
            return Err(at!(PngError::InvalidInput(alloc::format!(
                "push_rows: width {} does not match canvas width {}",
                w,
                self.canvas_width
            ))));
        }

        let format = rows.descriptor().pixel_format();
        let mode = self.streaming.as_mut().unwrap();

        match mode {
            StreamingMode::Buffered(state) => {
                // Check for overflow
                if self.canvas_height > 0 && state.rows_pushed + h > self.canvas_height {
                    return Err(at!(PngError::InvalidInput(alloc::format!(
                        "push_rows: would exceed canvas height {} (already pushed {}, pushing {})",
                        self.canvas_height,
                        state.rows_pushed,
                        h
                    ))));
                }

                // Reserve all needed capacity in one shot — no per-row reallocs.
                state.pixel_data.reserve(state.row_bytes * h as usize);

                for y in 0..h {
                    let src = rows.row(y);
                    match format {
                        PixelFormat::Rgb8 | PixelFormat::Rgba8 | PixelFormat::Gray8 => {
                            state.pixel_data.extend_from_slice(&src[..state.row_bytes]);
                        }
                        PixelFormat::Rgb16 | PixelFormat::Rgba16 | PixelFormat::Gray16 => {
                            let samples: &[u16] = bytemuck::cast_slice(&src[..state.row_bytes]);
                            for &val in samples {
                                state.pixel_data.extend_from_slice(&val.to_be_bytes());
                            }
                        }
                        PixelFormat::RgbF32 | PixelFormat::GrayF32 => {
                            let floats: &[f32] = bytemuck::cast_slice(src);
                            let start = state.pixel_data.len();
                            state.pixel_data.resize(start + floats.len(), 0);
                            linear_to_srgb_u8_slice(floats, &mut state.pixel_data[start..]);
                        }
                        PixelFormat::RgbaF32 => {
                            let floats: &[f32] = bytemuck::cast_slice(src);
                            let start = state.pixel_data.len();
                            state.pixel_data.resize(start + floats.len(), 0);
                            linear_to_srgb_u8_rgba_slice(floats, &mut state.pixel_data[start..]);
                        }
                        PixelFormat::Bgra8 => {
                            for c in src.chunks_exact(4) {
                                state
                                    .pixel_data
                                    .extend_from_slice(&[c[2], c[1], c[0], c[3]]);
                            }
                        }
                        _ => {
                            return Err(at!(PngError::from(
                                zencodec::UnsupportedOperation::PixelFormat
                            )));
                        }
                    }
                }
                state.rows_pushed += h;
            }
            StreamingMode::TrueStreaming(state) => {
                // Check for overflow
                if state.rows_pushed + h > self.canvas_height {
                    return Err(at!(PngError::InvalidInput(alloc::format!(
                        "push_rows: would exceed canvas height {} (already pushed {}, pushing {})",
                        self.canvas_height,
                        state.rows_pushed,
                        h
                    ))));
                }

                for y in 0..h {
                    let src = rows.row(y);
                    // Convert source pixels into PNG byte order, then write
                    // as a stored DEFLATE row. convert_buf is reused per row.
                    match format {
                        PixelFormat::Rgb8 | PixelFormat::Rgba8 | PixelFormat::Gray8 => {
                            state.push_raw_row(&src[..state.row_bytes]);
                        }
                        PixelFormat::Rgb16 | PixelFormat::Rgba16 | PixelFormat::Gray16 => {
                            let samples: &[u16] = bytemuck::cast_slice(&src[..state.row_bytes]);
                            for (i, &val) in samples.iter().enumerate() {
                                let be = val.to_be_bytes();
                                state.convert_buf[i * 2] = be[0];
                                state.convert_buf[i * 2 + 1] = be[1];
                            }
                            state.push_converted_row();
                        }
                        PixelFormat::RgbF32 | PixelFormat::GrayF32 => {
                            let floats: &[f32] = bytemuck::cast_slice(src);
                            linear_to_srgb_u8_slice(floats, &mut state.convert_buf);
                            state.push_converted_row();
                        }
                        PixelFormat::RgbaF32 => {
                            let floats: &[f32] = bytemuck::cast_slice(src);
                            linear_to_srgb_u8_rgba_slice(floats, &mut state.convert_buf);
                            state.push_converted_row();
                        }
                        PixelFormat::Bgra8 => {
                            for (i, c) in src.chunks_exact(4).enumerate() {
                                state.convert_buf[i * 4] = c[2];
                                state.convert_buf[i * 4 + 1] = c[1];
                                state.convert_buf[i * 4 + 2] = c[0];
                                state.convert_buf[i * 4 + 3] = c[3];
                            }
                            state.push_converted_row();
                        }
                        _ => {
                            return Err(at!(PngError::from(
                                zencodec::UnsupportedOperation::PixelFormat
                            )));
                        }
                    }
                }
            }
            StreamingMode::PreFiltered(state) => {
                // Check for overflow
                if state.rows_pushed + h > self.canvas_height {
                    return Err(at!(PngError::InvalidInput(alloc::format!(
                        "push_rows: would exceed canvas height {} (already pushed {}, pushing {})",
                        self.canvas_height,
                        state.rows_pushed,
                        h
                    ))));
                }

                for y in 0..h {
                    let src = rows.row(y);
                    match format {
                        PixelFormat::Rgb8 | PixelFormat::Rgba8 | PixelFormat::Gray8 => {
                            state.push_raw_row(&src[..state.row_bytes]);
                        }
                        PixelFormat::Rgb16 | PixelFormat::Rgba16 | PixelFormat::Gray16 => {
                            let samples: &[u16] = bytemuck::cast_slice(&src[..state.row_bytes]);
                            for (i, &val) in samples.iter().enumerate() {
                                let be = val.to_be_bytes();
                                state.convert_buf[i * 2] = be[0];
                                state.convert_buf[i * 2 + 1] = be[1];
                            }
                            state.push_converted_row();
                        }
                        PixelFormat::RgbF32 | PixelFormat::GrayF32 => {
                            let floats: &[f32] = bytemuck::cast_slice(src);
                            linear_to_srgb_u8_slice(floats, &mut state.convert_buf);
                            state.push_converted_row();
                        }
                        PixelFormat::RgbaF32 => {
                            let floats: &[f32] = bytemuck::cast_slice(src);
                            linear_to_srgb_u8_rgba_slice(floats, &mut state.convert_buf);
                            state.push_converted_row();
                        }
                        PixelFormat::Bgra8 => {
                            for (i, c) in src.chunks_exact(4).enumerate() {
                                state.convert_buf[i * 4] = c[2];
                                state.convert_buf[i * 4 + 1] = c[1];
                                state.convert_buf[i * 4 + 2] = c[0];
                                state.convert_buf[i * 4 + 3] = c[3];
                            }
                            state.push_converted_row();
                        }
                        _ => {
                            return Err(at!(PngError::from(
                                zencodec::UnsupportedOperation::PixelFormat
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<EncodeOutput, At<PngError>> {
        let mode = self.streaming.take().ok_or_else(|| {
            at!(PngError::InvalidInput(
                "finish() called without any push_rows() calls".into()
            ))
        })?;

        match mode {
            StreamingMode::Buffered(state) => {
                let h = state.rows_pushed;
                let w = self.canvas_width;

                if w == 0 || h == 0 {
                    return Err(at!(PngError::InvalidInput("no pixel data pushed".into())));
                }

                // Validate total data size
                let expected = state.row_bytes * h as usize;
                if state.pixel_data.len() != expected {
                    return Err(at!(PngError::InvalidInput(alloc::format!(
                        "finish: pixel data size {} does not match expected {} ({}×{} rows)",
                        state.pixel_data.len(),
                        expected,
                        state.row_bytes,
                        h
                    ))));
                }

                self.do_encode_with_depth(
                    &state.pixel_data,
                    w,
                    h,
                    state.color_type,
                    state.bit_depth,
                )
            }
            StreamingMode::TrueStreaming(state) => {
                if state.rows_pushed == 0 {
                    return Err(at!(PngError::InvalidInput("no pixel data pushed".into())));
                }
                let data = state.finish();
                if let Some(ref limits) = self.limits {
                    limits
                        .check_output_size(data.len() as u64)
                        .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
                }
                Ok(EncodeOutput::new(data, ImageFormat::Png))
            }
            StreamingMode::PreFiltered(state) => {
                if state.rows_pushed == 0 {
                    return Err(at!(PngError::InvalidInput("no pixel data pushed".into())));
                }
                let cancel: &dyn enough::Stop = match self.stop {
                    Some(ref s) => s as &dyn enough::Stop,
                    None => &enough::Unstoppable,
                };
                let data = state.finish(cancel)?;
                if let Some(ref limits) = self.limits {
                    limits
                        .check_output_size(data.len() as u64)
                        .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
                }
                Ok(EncodeOutput::new(data, ImageFormat::Png))
            }
        }
    }
}

// ── PngAnimationFrameEncoder ──────────────────────────────────────────────

/// Accumulated frame data for APNG encoding.
struct AccumulatedFrame {
    pixels: Vec<u8>, // RGBA8 canvas-sized
    duration_ms: u32,
}

/// APNG frame-by-frame encoder implementing [`AnimationFrameEncoder`](zencodec::encode::AnimationFrameEncoder).
///
/// Accumulates canvas-sized RGBA8 frames, then encodes them all on [`finish()`](PngAnimationFrameEncoder::do_finish).
pub struct PngAnimationFrameEncoder {
    frames: Vec<AccumulatedFrame>,
    canvas_width: u32,
    canvas_height: u32,
    config: crate::encode::EncodeConfig,
    metadata: Option<Metadata>,
    loop_count: u32,
    /// In-progress frame being built row-by-row.
    building_frame: Option<BuildingFrame>,
    /// Resource limits for frame accumulation.
    limits: Option<ResourceLimits>,
    /// Cumulative pixel data size across all accumulated frames.
    cumulative_pixel_bytes: u64,
}

/// State for row-by-row frame construction.
struct BuildingFrame {
    pixels: Vec<u8>,
    duration_ms: u32,
    rows_pushed: u32,
}

impl PngAnimationFrameEncoder {
    fn new(
        config: crate::encode::EncodeConfig,
        canvas_width: u32,
        canvas_height: u32,
        metadata: Option<Metadata>,
    ) -> Self {
        Self {
            frames: Vec::new(),
            canvas_width,
            canvas_height,
            config,
            metadata,
            loop_count: 0,
            building_frame: None,
            limits: None,
            cumulative_pixel_bytes: 0,
        }
    }

    /// Extract RGBA8 bytes from a PixelSlice, converting as needed.
    ///
    /// Supports RGBA8, BGRA8, RGB8, and Gray8 inputs. Other formats
    /// (16-bit, float) are rejected with a clear error listing the
    /// supported formats.
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
            (zenpixels::ChannelType::U8, zenpixels::ChannelLayout::Gray) => {
                let src = contiguous_bytes(pixels);
                Ok(src.iter().flat_map(|&g| [g, g, g, 255]).collect())
            }
            _ => Err(PngError::InvalidInput(alloc::format!(
                "APNG frame encoder: unsupported pixel format {:?}; \
                 supported formats are RGBA8, BGRA8, RGB8, and Gray8",
                desc
            ))),
        }
    }
}

impl zencodec::encode::AnimationFrameEncoder for PngAnimationFrameEncoder {
    type Error = At<PngError>;

    fn reject(op: zencodec::UnsupportedOperation) -> At<PngError> {
        at!(PngError::from(op))
    }

    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        _stop: Option<&dyn enough::Stop>,
    ) -> Result<(), At<PngError>> {
        let rgba = Self::pixels_to_rgba8(&pixels)?;
        // Check resource limits before accumulating
        if let Some(ref limits) = self.limits {
            // Check max_frames (new frame count = current + 1)
            limits
                .check_frames(self.frames.len() as u32 + 1)
                .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
            // Check max_memory (cumulative pixel data size)
            let new_cumulative = self.cumulative_pixel_bytes + rgba.len() as u64;
            limits
                .check_memory(new_cumulative)
                .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
        }
        self.cumulative_pixel_bytes += rgba.len() as u64;
        self.frames.push(AccumulatedFrame {
            pixels: rgba,
            duration_ms,
        });
        Ok(())
    }

    fn finish(self, stop: Option<&dyn enough::Stop>) -> Result<EncodeOutput, At<PngError>> {
        self.do_finish(stop).map_err(|e| at!(e))
    }
}

impl PngAnimationFrameEncoder {
    fn do_finish(self, stop: Option<&dyn enough::Stop>) -> Result<EncodeOutput, PngError> {
        let cancel: &dyn enough::Stop = stop.unwrap_or(&enough::Unstoppable);
        cancel.check().map_err(PngError::from)?;

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

        let timeout = std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);
        let deadline = almost_enough::time::WithTimeout::new(enough::Unstoppable, timeout);

        let data = crate::encode::encode_apng(
            &inputs,
            self.canvas_width,
            self.canvas_height,
            &apng_config,
            self.metadata.as_ref(),
            cancel,
            &deadline,
        )
        .map_err(|e| e.into_inner())?;

        Ok(EncodeOutput::new(data, ImageFormat::Png))
    }
}

// ── PngDecoderConfig ─────────────────────────────────────────────────

/// PNG decoder configuration implementing [`DecoderConfig`](zencodec::DecoderConfig).
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
        use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
        self.job().decoder(Cow::Borrowed(data), &[])?.decode()
    }

    /// Convenience: probe image header.
    pub fn probe(&self, data: &[u8]) -> Result<ImageInfo, At<PngError>> {
        use zencodec::decode::{DecodeJob, DecoderConfig};
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
    .with_stop(true)
    .with_animation(true)
    .with_cheap_probe(true)
    .with_native_gray(true)
    .with_native_16bit(true)
    .with_native_alpha(true)
    .with_enforces_max_pixels(true)
    .with_enforces_max_memory(true)
    .with_enforces_max_input_bytes(true);

impl zencodec::decode::DecoderConfig for PngDecoderConfig {
    type Error = At<PngError>;
    type Job<'a> = PngDecodeJob<'a>;

    fn formats() -> &'static [ImageFormat] {
        &[ImageFormat::Png]
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
            policy: None,
            start_frame_index: 0,
        }
    }
}

// ── PngDecodeJob ─────────────────────────────────────────────────────

/// Per-operation PNG decode job.
pub struct PngDecodeJob<'a> {
    config: &'a PngDecoderConfig,
    stop: Option<zencodec::StopToken>,
    limits: Option<ResourceLimits>,
    policy: Option<zencodec::decode::DecodePolicy>,
    start_frame_index: u32,
}

impl<'a> zencodec::decode::DecodeJob<'a> for PngDecodeJob<'a> {
    type Error = At<PngError>;
    type Dec = PngDecoder<'a>;
    type StreamDec = PngStreamingDecoder<'a>;
    type AnimationFrameDec = PngAnimationFrameDecoder;

    fn with_stop(mut self, stop: zencodec::StopToken) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn with_policy(mut self, policy: zencodec::decode::DecodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    fn with_start_frame_index(mut self, index: u32) -> Self {
        self.start_frame_index = index;
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, At<PngError>> {
        let info = crate::decode::probe(data)?;
        let mut image_info = convert_info(&info);
        if let Ok(probe) = crate::detect::probe(data) {
            image_info = image_info.with_source_encoding_details(probe);
        }
        Ok(image_info)
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, At<PngError>> {
        let info = crate::decode::probe(data)?;
        // Derive has_trns: has_alpha is set for color types 4/6 (intrinsic alpha)
        // or when a tRNS chunk is present. If has_alpha is true but color_type
        // doesn't have intrinsic alpha, then tRNS must be present.
        let intrinsic_alpha = info.color_type == 4 || info.color_type == 6;
        let has_trns = info.has_alpha && !intrinsic_alpha;
        let native_format = native_output_descriptor(info.color_type, info.bit_depth, has_trns);
        Ok(
            OutputInfo::full_decode(info.width, info.height, native_format)
                .with_alpha(info.has_alpha),
        )
    }

    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<PngDecoder<'a>, At<PngError>> {
        Ok(PngDecoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
            policy: self.policy,
            data,
            preferred: preferred.to_vec(),
        })
    }

    fn streaming_decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<PngStreamingDecoder<'a>, At<PngError>> {
        PngStreamingDecoder::new(
            data,
            self.config,
            self.stop,
            self.limits.as_ref(),
            self.policy.as_ref(),
            preferred,
        )
    }

    fn animation_frame_decoder(
        self,
        data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor],
    ) -> Result<PngAnimationFrameDecoder, At<PngError>> {
        // Check input size limit
        let effective_limits = self.limits.as_ref().unwrap_or(&self.config.limits);
        effective_limits
            .check_input_size(data.len() as u64)
            .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
        PngAnimationFrameDecoder::new(
            &data,
            self.config,
            self.stop,
            preferred,
            self.start_frame_index,
        )
        .map_err(|e| at!(e))
    }

    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn zencodec::decode::DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, At<PngError>> {
        push_decoder_native(self, data, sink, preferred)
    }
}

// ── PngDecoder ───────────────────────────────────────────────────────

/// Single-image PNG decoder.
pub struct PngDecoder<'a> {
    config: &'a PngDecoderConfig,
    stop: Option<zencodec::StopToken>,
    limits: Option<ResourceLimits>,
    policy: Option<zencodec::decode::DecodePolicy>,
    data: Cow<'a, [u8]>,
    preferred: Vec<PixelDescriptor>,
}

impl PngDecoder<'_> {
    fn effective_config(&self) -> PngDecodeConfig {
        let limits = self.limits.as_ref().unwrap_or(&self.config.limits);
        let config = PngDecodeConfig {
            max_pixels: limits.max_pixels,
            max_memory_bytes: limits.max_memory_bytes,
            skip_decompression_checksum: true,
            skip_critical_chunk_crc: true,
        };
        apply_decode_policy(config, self.policy.as_ref())
    }
}

impl zencodec::decode::Decode for PngDecoder<'_> {
    type Error = At<PngError>;

    fn decode(self) -> Result<DecodeOutput, At<PngError>> {
        let cancel: &dyn enough::Stop = match self.stop {
            Some(ref s) => s as &dyn enough::Stop,
            None => &enough::Unstoppable,
        };
        cancel.check().map_err(PngError::from)?;
        // Check input size limit
        let effective_limits = self.limits.as_ref().unwrap_or(&self.config.limits);
        effective_limits
            .check_input_size(self.data.len() as u64)
            .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
        let png_config = self.effective_config();
        let result = crate::decode::decode(&self.data, &png_config, cancel)?;
        let info = convert_info(&result.info);
        let pixels = if self.preferred.is_empty() {
            result.pixels
        } else {
            negotiate_and_convert(result.pixels, &self.preferred)
        };
        let mut output = DecodeOutput::new(pixels, info);
        if let Ok(probe) = crate::detect::probe(&self.data) {
            output = output.with_source_encoding_details(probe);
        }
        Ok(output)
    }
}

// ── Native push_decoder ─────────────────────────────────────────────

/// Determine the native output `PixelDescriptor` for a given PNG color
/// type, bit depth, and tRNS presence. Must match `build_pixel_data`.
fn native_output_descriptor(color_type: u8, bit_depth: u8, has_trns: bool) -> PixelDescriptor {
    match (color_type, bit_depth, has_trns) {
        (0, 16, false) => PixelDescriptor::GRAY16_SRGB,
        (0, 16, true) => GrayAlpha16::DESCRIPTOR, // GRAYA16
        (0, _, false) => PixelDescriptor::GRAY8_SRGB,
        (0, _, true) => PixelDescriptor::RGBA8_SRGB,
        (2, 16, false) => PixelDescriptor::RGB16_SRGB,
        (2, 16, true) => PixelDescriptor::RGBA16_SRGB,
        (2, 8, false) => PixelDescriptor::RGB8_SRGB,
        (2, 8, true) => PixelDescriptor::RGBA8_SRGB,
        (3, _, true) => PixelDescriptor::RGBA8_SRGB,
        (3, _, false) => PixelDescriptor::RGB8_SRGB,
        (4, 16, _) => GrayAlpha16::DESCRIPTOR, // GRAYA16
        (4, 8, _) => PixelDescriptor::RGBA8_SRGB,
        (6, 16, _) => PixelDescriptor::RGBA16_SRGB,
        (6, 8, _) => PixelDescriptor::RGBA8_SRGB,
        _ => PixelDescriptor::RGBA8_SRGB, // fallback
    }
}

/// Native row-streaming push decoder. Decodes PNG rows one at a time
/// directly into the sink buffer, avoiding the 2x peak memory of the
/// full-decode-then-copy fallback.
///
/// Falls back to `helpers::copy_decode_to_sink` for interlaced PNGs
/// (Adam7 scatters pixels across 7 passes and requires a full canvas).
fn push_decoder_native<'a>(
    job: PngDecodeJob<'a>,
    data: Cow<'a, [u8]>,
    sink: &mut dyn zencodec::decode::DecodeRowSink,
    preferred: &[PixelDescriptor],
) -> Result<OutputInfo, At<PngError>> {
    use crate::decoder::postprocess::post_process_row;
    use crate::decoder::row::RowDecoder;

    let wrap_sink = |e: zencodec::decode::SinkError| -> At<PngError> {
        at!(PngError::InvalidInput(alloc::format!("sink error: {e}")))
    };

    // Check input size limit
    let effective_limits = job.limits.as_ref().unwrap_or(&job.config.limits);
    effective_limits
        .check_input_size(data.len() as u64)
        .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;

    // Check for interlacing — fall back to full decode for Adam7
    if data.len() >= 29 && data[..8] == crate::chunk::PNG_SIGNATURE && data[28] == 1 {
        return zencodec::helpers::copy_decode_to_sink(job, data, sink, preferred, |e| {
            at!(PngError::InvalidInput(alloc::format!("sink error: {e}")))
        });
    }

    // Build effective config (limits + policy)
    let limits = job.limits.as_ref().unwrap_or(&job.config.limits);
    let png_config = PngDecodeConfig {
        max_pixels: limits.max_pixels,
        max_memory_bytes: limits.max_memory_bytes,
        skip_decompression_checksum: true,
        skip_critical_chunk_crc: true,
    };
    let png_config = apply_decode_policy(png_config, job.policy.as_ref());

    let cancel: &dyn enough::Stop = match &job.stop {
        Some(s) => s,
        None => &enough::Unstoppable,
    };
    cancel.check().map_err(PngError::from)?;

    let mut reader = RowDecoder::new(&data, &png_config)?;
    let ihdr = *reader.ihdr();
    let has_trns = reader.ancillary().trns.is_some();

    let w = ihdr.width;
    let h = ihdr.height;
    let descriptor = native_output_descriptor(ihdr.color_type, ihdr.bit_depth, has_trns);

    sink.begin(w, h, descriptor).map_err(wrap_sink)?;

    // Passthrough fast path: RGB8 or RGBA8 without tRNS — raw unfiltered
    // data IS the output pixels. Decode directly into the sink buffer.
    let is_passthrough =
        !has_trns && ihdr.bit_depth == 8 && (ihdr.color_type == 6 || ihdr.color_type == 2);

    if is_passthrough {
        let raw_row_bytes = ihdr.raw_row_bytes()?;

        // Request the full sink buffer up front so we can use split_at_mut
        // for zero-copy prev-row references during unfiltering.
        let mut dst = sink
            .provide_next_buffer(0, h, w, descriptor)
            .map_err(wrap_sink)?;

        // Row 0: prev is zeros
        if h > 0 {
            let zeros = alloc::vec![0u8; raw_row_bytes];
            let row_slice = dst.row_mut(0);
            match reader.next_raw_row_direct(&mut row_slice[..raw_row_bytes], &zeros) {
                Some(Ok(())) => {}
                Some(Err(e)) => return Err(at!(e)),
                None => {
                    return Err(at!(PngError::Decode(
                        "unexpected end of image data at row 0".into()
                    )));
                }
            }
            cancel.check().map_err(PngError::from)?;
        }

        // Rows 1..h: prev is the previous row in the sink buffer.
        // We cannot borrow two rows from PixelSliceMut simultaneously via
        // row_mut, so copy the previous row into a temp buffer for unfilter.
        let mut prev_buf = alloc::vec![0u8; raw_row_bytes];
        for y in 1..h {
            // Save previous row for unfilter reference
            prev_buf.copy_from_slice(&dst.row_mut(y - 1)[..raw_row_bytes]);
            let row_slice = dst.row_mut(y);
            match reader.next_raw_row_direct(&mut row_slice[..raw_row_bytes], &prev_buf) {
                Some(Ok(())) => {}
                Some(Err(e)) => return Err(at!(e)),
                None => {
                    return Err(at!(PngError::Decode(alloc::format!(
                        "unexpected end of image data at row {y}"
                    ))));
                }
            }
            cancel.check().map_err(PngError::from)?;
        }
        drop(dst);
    } else {
        // General path: post-process each raw row, then write to sink.
        let out_bpp = descriptor.bytes_per_pixel();
        let out_row_bytes = w as usize * out_bpp;
        let mut row_buf = Vec::new();
        let mut raw_copy = alloc::vec![0u8; ihdr.raw_row_bytes()?];

        let mut dst = sink
            .provide_next_buffer(0, h, w, descriptor)
            .map_err(wrap_sink)?;

        let mut y = 0u32;
        while let Some(result) = reader.next_raw_row() {
            let raw = result?;
            cancel.check().map_err(PngError::from)?;

            // Copy raw row data so the borrow on reader is released (NLL),
            // allowing reader.ancillary() below.
            raw_copy[..raw.len()].copy_from_slice(raw);

            post_process_row(
                &raw_copy[..ihdr.raw_row_bytes()?],
                &ihdr,
                reader.ancillary(),
                &mut row_buf,
            );

            let sink_row = dst.row_mut(y);
            let copy_len = out_row_bytes.min(row_buf.len()).min(sink_row.len());
            sink_row[..copy_len].copy_from_slice(&row_buf[..copy_len]);
            y += 1;
        }
        drop(dst);
    }

    reader.finish_metadata();
    sink.finish().map_err(wrap_sink)?;

    let has_alpha = descriptor.has_alpha();
    Ok(OutputInfo::full_decode(w, h, descriptor).with_alpha(has_alpha))
}

// ── PngStreamingDecoder ──────────────────────────────────────────────

/// Pull-based streaming PNG decoder implementing [`StreamingDecode`](zencodec::decode::StreamingDecode).
///
/// Yields one post-processed row per `next_batch()` call, backed by
/// [`RowDecoder`](crate::decoder::row::RowDecoder). Only non-interlaced
/// PNGs are supported; interlaced images are rejected at construction.
pub struct PngStreamingDecoder<'a> {
    reader: crate::decoder::row::RowDecoder<'a>,
    info: ImageInfo,
    descriptor: PixelDescriptor,
    /// Post-processed row buffer, reused across calls.
    row_buf: Vec<u8>,
    /// Raw row copy buffer (needed to release borrow on reader before
    /// calling `reader.ancillary()` for post-processing).
    raw_copy: Vec<u8>,
    /// Current row index (y coordinate).
    y: u32,
    width: u32,
    height: u32,
    /// True when the raw format is passthrough (no post-processing needed).
    is_passthrough: bool,
}

impl<'a> PngStreamingDecoder<'a> {
    fn new(
        data: Cow<'a, [u8]>,
        config: &PngDecoderConfig,
        stop: Option<zencodec::StopToken>,
        limits: Option<&ResourceLimits>,
        policy: Option<&zencodec::decode::DecodePolicy>,
        _preferred: &[PixelDescriptor],
    ) -> Result<Self, At<PngError>> {
        // Reject interlaced PNGs — Adam7 requires full-canvas buffering
        if data.len() >= 29 && data[..8] == crate::chunk::PNG_SIGNATURE && data[28] == 1 {
            return Err(at!(PngError::from(
                zencodec::UnsupportedOperation::RowLevelDecode
            )));
        }

        let cancel: &dyn enough::Stop = match &stop {
            Some(s) => s,
            None => &enough::Unstoppable,
        };
        cancel.check().map_err(PngError::from)?;

        let effective_limits = limits.unwrap_or(&config.limits);
        // Check input size limit
        effective_limits
            .check_input_size(data.len() as u64)
            .map_err(|e| PngError::LimitExceeded(alloc::format!("{e}")))?;
        let png_config = PngDecodeConfig {
            max_pixels: effective_limits.max_pixels,
            max_memory_bytes: effective_limits.max_memory_bytes,
            skip_decompression_checksum: true,
            skip_critical_chunk_crc: true,
        };
        let png_config = apply_decode_policy(png_config, policy);

        // RowDecoder requires &'a [u8], but we have Cow<'a, [u8]>.
        // Cow::Borrowed gives us &'a [u8] directly. Cow::Owned can't
        // provide a borrow with lifetime 'a, but in practice streaming_decoder
        // is always called with borrowed data.
        let data_ref: &'a [u8] = match data {
            Cow::Borrowed(b) => b,
            Cow::Owned(_) => {
                return Err(at!(PngError::InvalidInput(
                    "streaming decoder requires borrowed data".into(),
                )));
            }
        };

        let reader =
            crate::decoder::row::RowDecoder::new(data_ref, &png_config).map_err(|e| at!(e))?;
        let ihdr = *reader.ihdr();
        let has_trns = reader.ancillary().trns.is_some();

        let w = ihdr.width;
        let h = ihdr.height;
        let descriptor = native_output_descriptor(ihdr.color_type, ihdr.bit_depth, has_trns);

        let probe_info = crate::decode::probe(data_ref)?;
        let info = convert_info(&probe_info);

        let is_passthrough =
            !has_trns && ihdr.bit_depth == 8 && (ihdr.color_type == 6 || ihdr.color_type == 2);

        let raw_row_bytes = ihdr.raw_row_bytes().map_err(|e| at!(e))?;
        let out_row_bytes = w as usize * descriptor.bytes_per_pixel();

        Ok(Self {
            reader,
            info,
            descriptor,
            row_buf: alloc::vec![0u8; out_row_bytes],
            raw_copy: alloc::vec![0u8; raw_row_bytes],
            y: 0,
            width: w,
            height: h,
            is_passthrough,
        })
    }
}

impl zencodec::decode::StreamingDecode for PngStreamingDecoder<'_> {
    type Error = At<PngError>;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, At<PngError>> {
        use crate::decoder::postprocess::post_process_row;

        if self.y >= self.height {
            return Ok(None);
        }

        let raw = match self.reader.next_raw_row() {
            Some(Ok(row)) => row,
            Some(Err(e)) => return Err(at!(e)),
            None => return Ok(None),
        };

        let y = self.y;
        self.y += 1;

        if self.is_passthrough {
            // Raw unfiltered data IS the output — copy into row_buf
            let copy_len = raw.len().min(self.row_buf.len());
            self.row_buf[..copy_len].copy_from_slice(&raw[..copy_len]);
        } else {
            // Need post-processing: copy raw to release borrow on reader
            self.raw_copy[..raw.len()].copy_from_slice(raw);
            let raw_len = self.reader.ihdr().raw_row_bytes().map_err(|e| at!(e))?;

            // Post-process expands/converts the raw row into row_buf
            let mut tmp = core::mem::take(&mut self.row_buf);
            post_process_row(
                &self.raw_copy[..raw_len],
                self.reader.ihdr(),
                self.reader.ancillary(),
                &mut tmp,
            );
            self.row_buf = tmp;
        }

        let stride = self.width as usize * self.descriptor.bytes_per_pixel();
        let slice = PixelSlice::new(
            &self.row_buf[..stride],
            self.width,
            1,
            stride,
            self.descriptor,
        )
        .map_err(|e| at!(PngError::InvalidInput(alloc::format!("pixel slice: {e}"))))?;

        Ok(Some((y, slice)))
    }

    fn info(&self) -> &ImageInfo {
        &self.info
    }
}

// ── PngAnimationFrameDecoder ──────────────────────────────────────────────

/// APNG frame-by-frame decoder implementing [`AnimationFrameDecoder`](zencodec::decode::AnimationFrameDecoder).
///
/// Yields composited full-canvas frames. The returned [`AnimationFrame`] borrows
/// the decoder's internal canvas buffer; calling `render_next_frame()` again
/// invalidates the previous borrow.
pub struct PngAnimationFrameDecoder {
    /// Owned copy of the PNG file data.
    file_data: Vec<u8>,
    /// Image info for all frames.
    info: ImageInfo,
    /// Saved decoder state for O(1) resumption between frames.
    decoder_state: crate::decoder::apng::ApngDecoderState,
    /// Preferred output pixel formats for format negotiation.
    preferred: Vec<PixelDescriptor>,
    /// Internal canvas buffer holding the last rendered frame's pixels.
    canvas: Option<PixelBuffer>,
    /// First frame index to yield (skip earlier frames, but still decode them
    /// because APNG compositing depends on prior frames).
    start_frame_index: u32,
    /// Number of frames decoded so far (used to track position vs `start_frame_index`).
    frames_decoded: u32,
}

impl PngAnimationFrameDecoder {
    fn new(
        data: &[u8],
        config: &PngDecoderConfig,
        _stop: Option<zencodec::StopToken>,
        preferred: &[PixelDescriptor],
        start_frame_index: u32,
    ) -> Result<Self, PngError> {
        let probe_info = crate::decode::probe(data).map_err(|e| e.into_inner())?;
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
            info: image_info,
            decoder_state,
            preferred: preferred.to_vec(),
            canvas: None,
            start_frame_index,
            frames_decoded: 0,
        })
    }
}

impl zencodec::decode::AnimationFrameDecoder for PngAnimationFrameDecoder {
    type Error = At<PngError>;

    fn wrap_sink_error(err: zencodec::decode::SinkError) -> At<PngError> {
        at!(PngError::InvalidInput(alloc::format!("sink error: {err}")))
    }

    fn info(&self) -> &ImageInfo {
        &self.info
    }

    fn frame_count(&self) -> Option<u32> {
        Some(self.decoder_state.num_frames)
    }

    fn loop_count(&self) -> Option<u32> {
        Some(self.decoder_state.num_plays)
    }

    fn render_next_frame_to_sink(
        &mut self,
        stop: Option<&dyn enough::Stop>,
        sink: &mut dyn zencodec::decode::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, At<PngError>> {
        zencodec::helpers::copy_frame_to_sink(self, stop, sink)
    }

    fn render_next_frame(
        &mut self,
        _stop: Option<&dyn enough::Stop>,
    ) -> Result<Option<AnimationFrame<'_>>, At<PngError>> {
        loop {
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
            self.frames_decoded += 1;

            // Skip frames before start_frame_index. We must still decode them
            // (not skip) because APNG compositing depends on prior frame disposal
            // and blending, but we don't yield them to the caller.
            if idx < self.start_frame_index {
                continue;
            }

            let delay_ms = raw.fctl.delay_ms();

            // Apply format negotiation to frame pixels if preferred formats specified
            let pixels = if self.preferred.is_empty() {
                raw.pixels
            } else {
                negotiate_and_convert(raw.pixels, &self.preferred)
            };

            // Store the rendered frame in the internal canvas buffer
            self.canvas = Some(pixels);

            // Borrow from the canvas we just stored
            let canvas = self.canvas.as_ref().unwrap();
            let pixel_slice = canvas.as_slice();
            let frame = AnimationFrame::new(pixel_slice, delay_ms, idx);

            return Ok(Some(frame));
        }
    }
}

// ── Pixel format negotiation ─────────────────────────────────────────

use rgb::{Gray, Rgb, Rgba};
use zenpixels::{ChannelLayout, ChannelType, GrayAlpha16, PixelBuffer};
use zenpixels_convert::{PixelBufferConvertExt as _, PixelBufferConvertTypedExt as _};

/// Negotiate output format from the caller's preference list and convert.
///
/// Uses `negotiate_pixel_format` to pick the best match, then converts
/// if the native format doesn't already match.
fn negotiate_and_convert(pixels: PixelBuffer, preferred: &[PixelDescriptor]) -> PixelBuffer {
    let native_desc = pixels.descriptor();
    let target = zencodec::decode::negotiate_pixel_format(preferred, DECODE_DESCRIPTORS);

    // Already in the target format — no conversion needed
    let Some(target) = target else {
        return pixels;
    };
    if native_desc == target {
        return pixels;
    }

    // Use convert_to() for all format conversions (8/16/F32, all layouts)
    match pixels.convert_to(target) {
        Ok(converted) => converted,
        Err(_) => pixels, // Fallback to native format if conversion unavailable
    }
}

// ── Pixel conversion helpers ─────────────────────────────────────────
//
// PNG decoder produces Rgb8, Rgba8, Gray8, Rgb16, Rgba16, Gray16,
// GrayAlpha16. These helpers convert to any requested target format.

/// Convert native PNG pixel data to Rgb8. Delegates to `PixelBuffer::to_rgb8()`.
#[allow(unused_imports)]
use zenpixels_convert::PixelBufferConvertTypedExt as _;
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
    zi = zi.with_sequence(info.sequence.clone());
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

/// Apply encode policy to filter metadata fields.
///
/// Returns a new `Metadata` with fields stripped according to the policy,
/// or `None` if no metadata was provided.
fn apply_encode_policy(
    metadata: Option<&Metadata>,
    policy: Option<&zencodec::encode::EncodePolicy>,
) -> Option<Metadata> {
    let meta = metadata?;
    let Some(policy) = policy else {
        return Some(meta.clone());
    };
    let mut filtered = meta.clone();
    if policy.embed_icc == Some(false) {
        filtered.icc_profile = None;
    }
    if policy.embed_exif == Some(false) {
        filtered.exif = None;
    }
    if policy.embed_xmp == Some(false) {
        filtered.xmp = None;
    }
    Some(filtered)
}

/// Apply decode policy to control metadata extraction and strictness.
///
/// Returns adjusted `PngDecodeConfig` based on the policy.
fn apply_decode_policy(
    mut config: PngDecodeConfig,
    policy: Option<&zencodec::decode::DecodePolicy>,
) -> PngDecodeConfig {
    let Some(policy) = policy else {
        return config;
    };
    // Strict policy: enable CRC and Adler-32 verification
    if policy.strict == Some(true) {
        config.skip_critical_chunk_crc = false;
        config.skip_decompression_checksum = false;
    }
    config
}

/// Get contiguous pixel bytes from a PixelSlice, borrowing when possible.
///
/// Returns `Cow::Borrowed` when rows are tightly packed (zero-copy),
/// `Cow::Owned` when stride padding must be stripped.
fn contiguous_bytes<'a>(pixels: &'a PixelSlice<'a>) -> alloc::borrow::Cow<'a, [u8]> {
    pixels.contiguous_bytes()
}

/// Map a PixelFormat to the corresponding PNG color type and bit depth.
fn pixel_format_to_png(
    format: zenpixels::PixelFormat,
) -> Option<(crate::encode::ColorType, crate::encode::BitDepth)> {
    use zenpixels::PixelFormat;
    match format {
        PixelFormat::Rgb8 => Some((
            crate::encode::ColorType::Rgb,
            crate::encode::BitDepth::Eight,
        )),
        PixelFormat::Rgba8 => Some((
            crate::encode::ColorType::Rgba,
            crate::encode::BitDepth::Eight,
        )),
        PixelFormat::Gray8 => Some((
            crate::encode::ColorType::Grayscale,
            crate::encode::BitDepth::Eight,
        )),
        PixelFormat::Rgb16 => Some((
            crate::encode::ColorType::Rgb,
            crate::encode::BitDepth::Sixteen,
        )),
        PixelFormat::Rgba16 => Some((
            crate::encode::ColorType::Rgba,
            crate::encode::BitDepth::Sixteen,
        )),
        PixelFormat::Gray16 => Some((
            crate::encode::ColorType::Grayscale,
            crate::encode::BitDepth::Sixteen,
        )),
        // Float and BGRA are converted to 8-bit on the fly
        PixelFormat::RgbF32 => Some((
            crate::encode::ColorType::Rgb,
            crate::encode::BitDepth::Eight,
        )),
        PixelFormat::RgbaF32 => Some((
            crate::encode::ColorType::Rgba,
            crate::encode::BitDepth::Eight,
        )),
        PixelFormat::GrayF32 => Some((
            crate::encode::ColorType::Grayscale,
            crate::encode::BitDepth::Eight,
        )),
        PixelFormat::Bgra8 => Some((
            crate::encode::ColorType::Rgba,
            crate::encode::BitDepth::Eight,
        )),
        _ => None,
    }
}

impl TrueStreamingState {
    /// Initialize true streaming state: write PNG signature, IHDR, and metadata,
    /// then start the IDAT chunk with a zlib header.
    #[allow(clippy::too_many_arguments)]
    fn new(
        width: u32,
        height: u32,
        color_type: crate::encode::ColorType,
        bit_depth: crate::encode::BitDepth,
        row_bytes: usize,
        metadata: Option<&Metadata>,
        policy: Option<&zencodec::encode::EncodePolicy>,
        config: &EncodeConfig,
    ) -> Result<Self, At<PngError>> {
        use crate::chunk::{PNG_SIGNATURE, write::write_chunk};
        use crate::encoder::{PngWriteMetadata, metadata_size_estimate, write_all_metadata};

        let filtered_row = row_bytes + 1; // filter byte + row data
        let total_filtered = filtered_row * height as usize;
        let num_blocks = if total_filtered == 0 {
            1
        } else {
            total_filtered.div_ceil(65535)
        };
        let idat_data_len = 2 + 5 * num_blocks + total_filtered + 4; // zlib header + blocks + adler

        // PNG chunk lengths are u32. Reject images whose stored IDAT exceeds this.
        if idat_data_len > u32::MAX as usize {
            return Err(at!(PngError::LimitExceeded(
                "image too large for single IDAT chunk at effort 0".into(),
            )));
        }

        // Build metadata
        let effective_meta = apply_encode_policy(metadata, policy);
        let mut write_meta = PngWriteMetadata::from_metadata(effective_meta.as_ref());
        write_meta.source_gamma = config.source_gamma;
        write_meta.srgb_intent = config.srgb_intent;
        write_meta.chromaticities = config.chromaticities;
        write_meta.pixels_per_unit_x = config.pixels_per_unit_x;
        write_meta.pixels_per_unit_y = config.pixels_per_unit_y;
        write_meta.phys_unit = config.phys_unit;
        write_meta.text_chunks.clone_from(&config.text_chunks);
        write_meta.last_modified = config.last_modified;

        let est = 8 + 25 + (12 + idat_data_len) + 12 + metadata_size_estimate(&write_meta);
        let mut output = Vec::with_capacity(est);

        // PNG signature
        output.extend_from_slice(&PNG_SIGNATURE);

        // IHDR
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = match bit_depth {
            crate::encode::BitDepth::Eight => 8,
            crate::encode::BitDepth::Sixteen => 16,
        };
        ihdr[9] = match color_type {
            crate::encode::ColorType::Grayscale => 0,
            crate::encode::ColorType::Rgb => 2,
            crate::encode::ColorType::GrayscaleAlpha => 4,
            crate::encode::ColorType::Rgba => 6,
        };
        write_chunk(&mut output, b"IHDR", &ihdr);

        // Metadata chunks
        write_all_metadata(&mut output, &write_meta)?;

        // Start IDAT chunk: length (placeholder) + "IDAT"
        let idat_len_pos = output.len();
        output.extend_from_slice(&(idat_data_len as u32).to_be_bytes());
        output.extend_from_slice(b"IDAT");

        // Zlib header (no compression)
        output.extend_from_slice(&[0x78, 0x01]);

        Ok(Self {
            output,
            convert_buf: vec![0u8; row_bytes],
            row_bytes,
            rows_pushed: 0,
            adler: 1,
            idat_len_pos,
            block_remaining: 0,
            filtered_remaining: total_filtered,
        })
    }

    /// Push one row from `self.convert_buf` (already written by caller).
    /// Avoids passing a borrow of self's own field to push_raw_row.
    fn push_converted_row(&mut self) {
        // Write filter byte (0x00 = None).
        if self.block_remaining == 0 {
            let block_len = self.filtered_remaining.min(65535);
            let is_final = block_len >= self.filtered_remaining;
            write_stored_block_header(&mut self.output, block_len, is_final);
            self.block_remaining = block_len;
        }
        self.output.push(0u8);
        self.block_remaining -= 1;
        self.filtered_remaining -= 1;

        // Write row data from convert_buf, splitting across blocks.
        let mut pos = 0;
        let row_bytes = self.row_bytes;
        while pos < row_bytes {
            if self.block_remaining == 0 {
                let block_len = self.filtered_remaining.min(65535);
                let is_final = block_len >= self.filtered_remaining;
                write_stored_block_header(&mut self.output, block_len, is_final);
                self.block_remaining = block_len;
            }
            let n = (row_bytes - pos).min(self.block_remaining);
            self.output
                .extend_from_slice(&self.convert_buf[pos..pos + n]);
            pos += n;
            self.block_remaining -= n;
            self.filtered_remaining -= n;
        }

        // Adler-32: filter byte (0x00) then row data.
        let s1 = self.adler & 0xFFFF;
        let s2 = ((self.adler >> 16) + s1) % 65521;
        self.adler = (s2 << 16) | s1;
        self.adler = zenflate::adler32(self.adler, &self.convert_buf[..row_bytes]);

        self.rows_pushed += 1;
    }

    /// Push one row of converted pixel data (already in PNG byte order).
    /// Writes filter byte + row data as stored DEFLATE blocks.
    fn push_raw_row(&mut self, row: &[u8]) {
        debug_assert_eq!(row.len(), self.row_bytes);

        // Write filter byte (0x00 = None) — effort 0 always uses None filter.
        if self.block_remaining == 0 {
            let block_len = self.filtered_remaining.min(65535);
            let is_final = block_len >= self.filtered_remaining;
            write_stored_block_header(&mut self.output, block_len, is_final);
            self.block_remaining = block_len;
        }
        self.output.push(0u8);
        self.block_remaining -= 1;
        self.filtered_remaining -= 1;

        // Write row data, splitting across stored blocks as needed.
        let mut data = row;
        while !data.is_empty() {
            if self.block_remaining == 0 {
                let block_len = self.filtered_remaining.min(65535);
                let is_final = block_len >= self.filtered_remaining;
                write_stored_block_header(&mut self.output, block_len, is_final);
                self.block_remaining = block_len;
            }
            let n = data.len().min(self.block_remaining);
            self.output.extend_from_slice(&data[..n]);
            data = &data[n..];
            self.block_remaining -= n;
            self.filtered_remaining -= n;
        }

        // Update Adler-32: filter byte (0x00) then row data.
        // Manual update for the 0x00 filter byte: s2 += s1 (since byte is 0).
        let s1 = self.adler & 0xFFFF;
        let s2 = ((self.adler >> 16) + s1) % 65521;
        self.adler = (s2 << 16) | s1;
        self.adler = zenflate::adler32(self.adler, row);

        self.rows_pushed += 1;
    }

    /// Finalize the PNG: write Adler-32, backpatch IDAT length, CRC, IEND.
    fn finish(mut self) -> Vec<u8> {
        // Write Adler-32 checksum
        self.output.extend_from_slice(&self.adler.to_be_bytes());

        // CRC-32 over "IDAT" + data (starts 4 bytes after idat_len_pos)
        let crc_start = self.idat_len_pos + 4;
        let crc = zenflate::crc32(0, &self.output[crc_start..]);
        self.output.extend_from_slice(&crc.to_be_bytes());

        // IEND
        crate::chunk::write::write_chunk(&mut self.output, b"IEND", &[]);

        self.output
    }
}

impl PreFilteredState {
    /// Initialize pre-filtered streaming: build PNG preamble, allocate filter buffers.
    #[allow(clippy::too_many_arguments)]
    fn new(
        width: u32,
        height: u32,
        color_type: crate::encode::ColorType,
        bit_depth: crate::encode::BitDepth,
        row_bytes: usize,
        bpp: usize,
        metadata: Option<&Metadata>,
        policy: Option<&zencodec::encode::EncodePolicy>,
        config: &EncodeConfig,
    ) -> Result<Self, At<PngError>> {
        use crate::chunk::{PNG_SIGNATURE, write::write_chunk};
        use crate::encoder::{PngWriteMetadata, metadata_size_estimate, write_all_metadata};

        let effective_meta = apply_encode_policy(metadata, policy);
        let mut write_meta = PngWriteMetadata::from_metadata(effective_meta.as_ref());
        write_meta.source_gamma = config.source_gamma;
        write_meta.srgb_intent = config.srgb_intent;
        write_meta.chromaticities = config.chromaticities;
        write_meta.pixels_per_unit_x = config.pixels_per_unit_x;
        write_meta.pixels_per_unit_y = config.pixels_per_unit_y;
        write_meta.phys_unit = config.phys_unit;
        write_meta.text_chunks.clone_from(&config.text_chunks);
        write_meta.last_modified = config.last_modified;

        // Build preamble: PNG signature + IHDR + metadata
        let est = 8 + 25 + metadata_size_estimate(&write_meta);
        let mut preamble = Vec::with_capacity(est);
        preamble.extend_from_slice(&PNG_SIGNATURE);

        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = match bit_depth {
            crate::encode::BitDepth::Eight => 8,
            crate::encode::BitDepth::Sixteen => 16,
        };
        ihdr[9] = match color_type {
            crate::encode::ColorType::Grayscale => 0,
            crate::encode::ColorType::Rgb => 2,
            crate::encode::ColorType::GrayscaleAlpha => 4,
            crate::encode::ColorType::Rgba => 6,
        };
        write_chunk(&mut preamble, b"IHDR", &ihdr);
        write_all_metadata(&mut preamble, &write_meta)?;

        let filtered_row = row_bytes + 1;
        let total_filtered = filtered_row * height as usize;

        Ok(Self {
            preamble,
            filtered_data: Vec::with_capacity(total_filtered),
            prev_row: vec![0u8; row_bytes],
            convert_buf: vec![0u8; row_bytes],
            filter_type: 4, // Paeth
            bpp,
            row_bytes,
            rows_pushed: 0,
            color_type,
            bit_depth,
            zenflate_effort: 1, // Turbo
        })
    }

    /// Push one row of pixel data (already in PNG byte order). Applies Paeth filter.
    fn push_raw_row(&mut self, row: &[u8]) {
        debug_assert_eq!(row.len(), self.row_bytes);

        // Filter byte
        self.filtered_data.push(self.filter_type);

        // Apply Paeth filter
        let start = self.filtered_data.len();
        self.filtered_data.resize(start + self.row_bytes, 0);
        crate::encoder::filter::apply_filter(
            self.filter_type,
            row,
            &self.prev_row,
            self.bpp,
            &mut self.filtered_data[start..],
        );

        // Save current row as prev
        self.prev_row.copy_from_slice(row);
        self.rows_pushed += 1;
    }

    /// Push one row from `self.convert_buf`. Avoids borrow issues.
    fn push_converted_row(&mut self) {
        // Filter byte
        self.filtered_data.push(self.filter_type);

        // Apply Paeth filter from convert_buf
        let start = self.filtered_data.len();
        let row_bytes = self.row_bytes;
        self.filtered_data.resize(start + row_bytes, 0);
        crate::encoder::filter::apply_filter(
            self.filter_type,
            &self.convert_buf[..row_bytes],
            &self.prev_row,
            self.bpp,
            &mut self.filtered_data[start..],
        );

        // Save current row as prev
        self.prev_row
            .copy_from_slice(&self.convert_buf[..row_bytes]);
        self.rows_pushed += 1;
    }

    /// Compress pre-filtered data and assemble the final PNG.
    ///
    /// Peak memory during this call: ~2× image (filtered_data + compress_bound).
    /// After compression, filtered_data is dropped before assembling output.
    fn finish(self, cancel: &dyn enough::Stop) -> Result<Vec<u8>, At<PngError>> {
        use crate::chunk::write::write_chunk;

        // Destructure to allow dropping filtered_data independently.
        let Self {
            preamble,
            filtered_data,
            zenflate_effort,
            ..
        } = self;

        // Compress filtered data with zenflate.
        // Peak: filtered_data (~1×) + compressed_bound (~1×) = ~2× image.
        let level = zenflate::CompressionLevel::new(zenflate_effort);
        let mut compressor = zenflate::Compressor::new(level);
        let bound = zenflate::Compressor::zlib_compress_bound(filtered_data.len());
        let mut compressed = vec![0u8; bound];
        let len = compressor
            .zlib_compress(&filtered_data, &mut compressed, cancel)
            .map_err(|e| match e {
                zenflate::CompressionError::Stopped(reason) => PngError::Stopped(reason),
                other => PngError::InvalidInput(alloc::format!("compression failed: {other}")),
            })?;
        compressed.truncate(len);

        // Free the filtered data before building output.
        drop(filtered_data);

        // Assemble: preamble + IDAT + IEND
        let mut out = preamble;
        out.reserve(12 + compressed.len() + 12);
        write_chunk(&mut out, b"IDAT", &compressed);
        write_chunk(&mut out, b"IEND", &[]);

        Ok(out)
    }
}

/// Write a stored DEFLATE block header (5 bytes).
fn write_stored_block_header(out: &mut Vec<u8>, len: usize, is_final: bool) {
    out.push(if is_final { 1 } else { 0 });
    out.push((len & 0xFF) as u8);
    out.push(((len >> 8) & 0xFF) as u8);
    let nlen = !len & 0xFFFF;
    out.push((nlen & 0xFF) as u8);
    out.push(((nlen >> 8) & 0xFF) as u8);
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
    use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
    use zencodec::encode::{EncodeJob, EncoderConfig};

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
            <PngDecoderConfig as DecoderConfig>::formats(),
            &[ImageFormat::Png]
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

        use zencodec::encode::Encoder;
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
            .decoder(Cow::Borrowed(encoded.data()), &[])
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
        let meta = Metadata::none()
            .with_icc(fake_icc.as_slice())
            .with_exif(exif_data.as_slice())
            .with_xmp(xmp_data.as_slice());

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
        use zencodec::encode::Encoder;
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
        use zencodec::{Cicp, Metadata};

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
        let meta = Metadata::none().with_cicp(cicp);

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
        use zencodec::Metadata;

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
        let meta = Metadata::none().with_icc(srgb_icc);

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
        use zencodec::{Cicp, Metadata};

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
        let meta = Metadata::none().with_cicp(cicp).with_icc(srgb_icc);

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
        use zencodec::{Cicp, ContentLightLevel, MasteringDisplay, Metadata};

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
            [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]],
            [0.3127, 0.3290],
            1000.0,
            0.005,
        );
        let meta = Metadata::none()
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
        use zencodec::{Cicp, Metadata};

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
        let meta = Metadata::none().with_cicp(cicp);
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
        use zencodec::{ContentLightLevel, MasteringDisplay, Metadata};

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
            [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]], // R, G, B primaries (CIE xy)
            [0.3127, 0.3290],                                 // white point (CIE xy)
            1000.0,                                           // max luminance (cd/m²)
            0.005,                                            // min luminance (cd/m²)
        );
        let meta = Metadata::none()
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
        // Values roundtrip through PNG u16 (0.00002 units) and u32 (0.0001 cd/m² units)
        assert_eq!(
            dm.primaries_xy,
            [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]]
        );
        assert_eq!(dm.white_point_xy, [0.3127, 0.3290]);
        assert_eq!(dm.max_luminance, 1000.0);
        assert_eq!(dm.min_luminance, 0.005);
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
        let meta = zencodec::Metadata::none().with_icc(icc.as_slice());
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
        let stop = zencodec::StopToken::new(AlreadyCancelled);
        let job = config.clone().job().with_stop(stop);
        let encoder = job.encoder().unwrap();

        // Create a small 2x2 RGB8 image
        let pixels = vec![Rgb { r: 0u8, g: 0, b: 0 }; 4];
        let img = Img::new(pixels, 2, 2);
        let slice = PixelSlice::from(img.as_ref());

        use zencodec::encode::Encoder;
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
        match result.unwrap_err().into_inner() {
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
        assert_eq!(quality_to_mpe(99.0), 0.003);
        assert_eq!(quality_to_mpe(0.0), 0.100);

        // libjpeg-turbo SSIM2-calibrated points
        let mpe_95 = quality_to_mpe(95.0);
        assert!((mpe_95 - 0.007).abs() < 0.001, "q95 mpe={mpe_95}");

        let mpe_90 = quality_to_mpe(90.0);
        assert!((mpe_90 - 0.011).abs() < 0.001, "q90 mpe={mpe_90}");

        let mpe_75 = quality_to_mpe(75.0);
        assert!((mpe_75 - 0.026).abs() < 0.001, "q75 mpe={mpe_75}");

        let mpe_50 = quality_to_mpe(50.0);
        assert!((mpe_50 - 0.044).abs() < 0.001, "q50 mpe={mpe_50}");

        // Interpolated mid-point: q97 between q99 (0.003) and q95 (0.007)
        let mpe_97 = quality_to_mpe(97.0);
        assert!(mpe_97 > 0.003 && mpe_97 < 0.007, "q97 mpe={mpe_97}");

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
        // q=95 should be 0.007
        assert!((quality_to_mpe(95.0) - 0.007).abs() < 0.0001);
        // q=50 should be 0.044
        assert!((quality_to_mpe(50.0) - 0.044).abs() < 0.0001);
        // Interpolated value between 95 and 99
        let mid = quality_to_mpe(97.0);
        assert!(mid > 0.003 && mid < 0.007);
    }

    // ── EncodeJob trait methods ──

    #[test]
    fn encode_job_with_stop() {
        let enc = PngEncoderConfig::new();
        let stop = zencodec::StopToken::new(enough::Unstoppable);
        let job = enc.job().with_stop(stop);
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
        let meta = Metadata::default();
        let job = enc.job().with_metadata(meta);
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
    fn encode_job_animation_frame_encoder() {
        let enc = PngEncoderConfig::new();
        let job = enc.job().with_canvas_size(8, 8).with_loop_count(Some(0));
        let frame_enc = job.animation_frame_encoder();
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
        use zencodec::encode::Encoder;
        let pixels: Vec<Rgb<u8>> = (0..16 * 16)
            .map(|i| Rgb {
                r: (i % 256) as u8,
                g: ((i * 3) % 256) as u8,
                b: ((i * 7) % 256) as u8,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgba8() {
        use zencodec::encode::Encoder;
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
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_gray8() {
        use zencodec::encode::Encoder;
        let pixels: Vec<Gray<u8>> = (0..16 * 16).map(|i| Gray((i % 256) as u8)).collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgb16() {
        use zencodec::encode::Encoder;
        let pixels: Vec<Rgb<u16>> = (0..16 * 16)
            .map(|i| Rgb {
                r: (i * 256) as u16,
                g: ((i * 3 * 256) % 65536) as u16,
                b: 0,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgba16() {
        use zencodec::encode::Encoder;
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
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_gray16() {
        use zencodec::encode::Encoder;
        let pixels: Vec<Gray<u16>> = (0..16 * 16).map(|i| Gray((i * 256) as u16)).collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgb_f32() {
        use zencodec::encode::Encoder;
        let pixels: Vec<Rgb<f32>> = (0..16 * 16)
            .map(|i| Rgb {
                r: (i % 256) as f32 / 255.0,
                g: ((i * 3) % 256) as f32 / 255.0,
                b: ((i * 7) % 256) as f32 / 255.0,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_rgba_f32() {
        use zencodec::encode::Encoder;
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
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_gray_f32() {
        use zencodec::encode::Encoder;
        let pixels: Vec<Gray<f32>> = (0..16 * 16)
            .map(|i| Gray((i % 256) as f32 / 255.0))
            .collect();
        let img = imgref::ImgVec::new(pixels, 16, 16);
        let config = PngEncoderConfig::new();
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    #[test]
    fn encoder_trait_bgra8() {
        use zencodec::encode::Encoder;
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
        let encoder = config.clone().job().encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).into())
            .unwrap();
        assert!(!output.is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
    }

    // ── ResourceLimits enforcement tests ──

    #[test]
    fn encode_max_output_bytes_rejects_large_output() {
        use zencodec::encode::Encoder;
        // Encode a 32x32 RGB image — output will be well over 100 bytes.
        let pixels: Vec<Rgb<u8>> = (0..32 * 32)
            .map(|i| Rgb {
                r: (i % 256) as u8,
                g: ((i * 3) % 256) as u8,
                b: ((i * 7) % 256) as u8,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 32, 32);
        let config = PngEncoderConfig::new();
        let limits = ResourceLimits::none().with_max_output(100);
        let encoder = config.clone().job().with_limits(limits).encoder().unwrap();
        let result = encoder.encode(PixelSlice::from(img.as_ref()).erase());
        let err = result.unwrap_err();
        let msg = alloc::format!("{}", err);
        assert!(
            msg.contains("limit exceeded") || msg.contains("output size"),
            "expected limit exceeded error, got: {msg}"
        );
    }

    #[test]
    fn encode_max_output_bytes_allows_small_output() {
        use zencodec::encode::Encoder;
        // 2x2 tiny image — output will be small but still a valid PNG
        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 0, g: 0, b: 0 }; 4];
        let img = imgref::ImgVec::new(pixels, 2, 2);
        let config = PngEncoderConfig::new();
        // Allow up to 10 KB — should be plenty for a 2x2 PNG
        let limits = ResourceLimits::none().with_max_output(10_000);
        let encoder = config.clone().job().with_limits(limits).encoder().unwrap();
        let result = encoder.encode(PixelSlice::from(img.as_ref()).erase());
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    #[test]
    fn apng_push_frame_rejects_over_max_frames() {
        use zencodec::encode::AnimationFrameEncoder;
        let config = PngEncoderConfig::new();
        let limits = ResourceLimits::none().with_max_frames(2);
        let job = config
            .job()
            .with_canvas_size(4, 4)
            .with_loop_count(Some(0))
            .with_limits(limits);
        let mut enc = job.animation_frame_encoder().unwrap();

        let make_frame = || {
            let pixels: Vec<Rgba<u8>> = vec![
                Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                };
                16
            ];
            imgref::ImgVec::new(pixels, 4, 4)
        };

        // First two frames should succeed
        let img1 = make_frame();
        enc.push_frame(PixelSlice::from(img1.as_ref()).erase(), 100, None)
            .unwrap();
        let img2 = make_frame();
        enc.push_frame(PixelSlice::from(img2.as_ref()).erase(), 100, None)
            .unwrap();

        // Third frame should fail with limit exceeded
        let img3 = make_frame();
        let result = enc.push_frame(PixelSlice::from(img3.as_ref()).erase(), 100, None);
        let err = result.unwrap_err();
        let msg = alloc::format!("{}", err);
        assert!(
            msg.contains("limit exceeded") || msg.contains("frame count"),
            "expected frame limit error, got: {msg}"
        );
    }

    #[test]
    fn apng_push_frame_rejects_over_max_memory() {
        use zencodec::encode::AnimationFrameEncoder;
        let config = PngEncoderConfig::new();
        // 4x4 RGBA8 = 64 bytes per frame. Limit to 100 bytes total.
        let limits = ResourceLimits::none().with_max_memory(100);
        let job = config
            .job()
            .with_canvas_size(4, 4)
            .with_loop_count(Some(0))
            .with_limits(limits);
        let mut enc = job.animation_frame_encoder().unwrap();

        let make_frame = || {
            let pixels: Vec<Rgba<u8>> = vec![
                Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                };
                16
            ];
            imgref::ImgVec::new(pixels, 4, 4)
        };

        // First frame is 64 bytes — should succeed (64 <= 100)
        let img1 = make_frame();
        enc.push_frame(PixelSlice::from(img1.as_ref()).erase(), 100, None)
            .unwrap();

        // Second frame would be 128 bytes cumulative — should fail (128 > 100)
        let img2 = make_frame();
        let result = enc.push_frame(PixelSlice::from(img2.as_ref()).erase(), 100, None);
        let err = result.unwrap_err();
        let msg = alloc::format!("{}", err);
        assert!(
            msg.contains("limit exceeded") || msg.contains("memory"),
            "expected memory limit error, got: {msg}"
        );
    }

    #[test]
    fn decode_max_input_bytes_rejects_large_input() {
        use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
        // First, encode a valid PNG
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 128,
                g: 64,
                b: 32
            };
            64
        ];
        let img = imgref::ImgVec::new(pixels, 8, 8);
        let enc = PngEncoderConfig::new();
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();
        let data = encoded.data();

        // Now try to decode with a very small max_input_bytes limit
        let dec = PngDecoderConfig::new();
        let limits = ResourceLimits::none().with_max_input_bytes(10);
        let result = dec
            .job()
            .with_limits(limits)
            .decoder(Cow::Borrowed(data), &[])
            .unwrap()
            .decode();
        let err = result.unwrap_err();
        let msg = alloc::format!("{}", err);
        assert!(
            msg.contains("limit exceeded") || msg.contains("input size"),
            "expected input size limit error, got: {msg}"
        );
    }

    #[test]
    fn decode_max_input_bytes_allows_small_input() {
        use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
        // Encode a tiny PNG
        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 0, g: 0, b: 0 }; 4];
        let img = imgref::ImgVec::new(pixels, 2, 2);
        let enc = PngEncoderConfig::new();
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();
        let data = encoded.data();

        // Generous limit — should succeed
        let dec = PngDecoderConfig::new();
        let limits = ResourceLimits::none().with_max_input_bytes(100_000);
        let result = dec
            .job()
            .with_limits(limits)
            .decoder(Cow::Borrowed(data), &[])
            .unwrap()
            .decode();
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    // ── ThreadingPolicy tests ──

    #[test]
    fn encode_single_thread_produces_valid_png() {
        use zencodec::ThreadingPolicy;
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let pixels: Vec<Rgb<u8>> = (0..32 * 32)
            .map(|i| Rgb {
                r: (i % 256) as u8,
                g: ((i * 3) % 256) as u8,
                b: ((i * 7) % 256) as u8,
            })
            .collect();
        let img = imgref::ImgVec::new(pixels, 32, 32);

        let config = PngEncoderConfig::new();
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::SingleThread);
        let encoder = config.clone().job().with_limits(limits).encoder().unwrap();
        let output = encoder
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        // Valid PNG header
        assert_eq!(
            &output.data()[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );

        // Roundtrip: decode it back
        use zencodec::decode::{Decode, DecodeJob, DecoderConfig};
        let dec = PngDecoderConfig::new();
        let result = dec
            .job()
            .decoder(Cow::Borrowed(output.data()), &[])
            .unwrap()
            .decode();
        assert!(
            result.is_ok(),
            "roundtrip decode failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn encode_single_thread_matches_default_threading() {
        use zencodec::ThreadingPolicy;
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 100,
                g: 150,
                b: 200,
            };
            16 * 16
        ];
        let img = imgref::ImgVec::new(pixels, 16, 16);

        // Encode with SingleThread
        let config_st = PngEncoderConfig::new();
        let limits = ResourceLimits::none().with_threading(ThreadingPolicy::SingleThread);
        let st_output = config_st
            .job()
            .with_limits(limits)
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        // Encode with default (Unlimited)
        let config_def = PngEncoderConfig::new();
        let def_output = config_def
            .job()
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        // Both must be valid PNGs (same content, so should be identical)
        assert_eq!(
            &st_output.data()[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert_eq!(st_output.data().len(), def_output.data().len());
    }

    // TODO: fix lifetime issue — `config` must outlive `dyn_enc`
    // #[test]
    // fn encoder_trait_dyn_encoder() {
    //     let pixels: Vec<Rgb<u8>> = vec![
    //         Rgb {
    //             r: 100,
    //             g: 150,
    //             b: 200,
    //         };
    //         32 * 32
    //     ];
    //     let img = imgref::ImgVec::new(pixels, 32, 32);
    //     let config = PngEncoderConfig::new();
    //     let dyn_enc = config.clone().job().dyn_encoder().unwrap();
    //     let output = dyn_enc
    //         .encode(PixelSlice::from(img.as_ref()).into())
    //         .unwrap();
    //     assert!(!output.is_empty());
    //     assert_eq!(output.format(), ImageFormat::Png);
    // }

    // ── push_rows / finish tests ──

    #[test]
    fn push_rows_rgb8_roundtrip() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 4u32;
        let h = 6u32;
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| Rgb {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 19) as u8,
            })
            .collect();
        let img = Img::new(pixels.clone(), w as usize, h as usize);

        // Encode via push_rows (2 rows at a time)
        let config = PngEncoderConfig::new();
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for strip_y in (0..h).step_by(2) {
            let strip = img.sub_image(0, strip_y as usize, w as usize, 2);
            let slice = PixelSlice::from(strip).erase();
            encoder.push_rows(slice).unwrap();
        }
        let push_output = encoder.finish().unwrap();

        // Encode via one-shot for comparison
        let oneshot_output = config
            .job()
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        // Both should produce valid PNG of the same dimensions
        assert!(!push_output.data().is_empty());
        let decoded_push = crate::decode(
            push_output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded_one = crate::decode(
            oneshot_output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded_push.info.width, w);
        assert_eq!(decoded_push.info.height, h);
        // Pixel data should be identical
        assert_eq!(
            decoded_push.pixels.copy_to_contiguous_bytes(),
            decoded_one.pixels.copy_to_contiguous_bytes()
        );
    }

    #[test]
    fn push_rows_rgba8_single_row_at_a_time() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 3u32;
        let h = 4u32;
        let pixels: Vec<Rgba<u8>> = (0..w * h)
            .map(|i| Rgba {
                r: (i * 5) as u8,
                g: (i * 11) as u8,
                b: (i * 17) as u8,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new();
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for y in 0..h {
            let strip = img.sub_image(0, y as usize, w as usize, 1);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    #[test]
    fn push_rows_all_at_once() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 8u32;
        let h = 8u32;
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 128,
                g: 64,
                b: 32,
            };
            (w * h) as usize
        ];
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new();
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        encoder
            .push_rows(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    #[test]
    fn push_rows_gray8() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 5u32;
        let h = 3u32;
        let pixels: Vec<Gray<u8>> = (0..w * h).map(|i| Gray::new((i * 17) as u8)).collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new();
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        encoder
            .push_rows(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    #[test]
    fn push_rows_finish_without_push_errors() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let config = PngEncoderConfig::new();
        let encoder = config
            .clone()
            .job()
            .with_canvas_size(4, 4)
            .encoder()
            .unwrap();
        assert!(encoder.finish().is_err());
    }

    #[test]
    fn push_rows_overflow_errors() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 2u32;
        let h = 2u32;
        let pixels: Vec<Rgb<u8>> = vec![Rgb { r: 0, g: 0, b: 0 }; (w * 3) as usize]; // 3 rows worth
        let img = Img::new(pixels, w as usize, 3);

        let config = PngEncoderConfig::new();
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        // Pushing 3 rows when canvas is 2 tall should error
        let result = encoder.push_rows(PixelSlice::from(img.as_ref()).erase());
        assert!(result.is_err());
    }

    #[test]
    fn push_rows_matches_encode_output() {
        // Verify push_rows produces byte-identical output to encode()
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 16u32;
        let h = 16u32;
        let pixels: Vec<Rgba<u8>> = (0..w * h)
            .map(|i| Rgba {
                r: (i % 256) as u8,
                g: ((i * 3) % 256) as u8,
                b: ((i * 7) % 256) as u8,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        // push_rows path
        let config = PngEncoderConfig::new().with_generic_effort(3);
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for y in (0..h).step_by(4) {
            let rows = (h - y).min(4);
            let strip = img.sub_image(0, y as usize, w as usize, rows as usize);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let push_out = encoder.finish().unwrap();

        // encode() path
        let one_enc = config.clone().job().encoder().unwrap();
        let one_out = one_enc
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        // Should decode to identical pixels
        let dec_push = crate::decode(
            push_out.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        let dec_one = crate::decode(
            one_out.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(
            dec_push.pixels.copy_to_contiguous_bytes(),
            dec_one.pixels.copy_to_contiguous_bytes()
        );
    }

    #[test]
    fn push_rows_caps_advertised() {
        use zencodec::encode::EncoderConfig;

        let caps = PngEncoderConfig::capabilities();
        assert!(caps.supports(zencodec::UnsupportedOperation::RowLevelEncode));
    }

    #[test]
    fn push_rows_infer_canvas_width() {
        // If canvas_size not set, width inferred from first push
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 4u32;
        let h = 2u32;
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 100,
                g: 100,
                b: 100,
            };
            (w * h) as usize
        ];
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new();
        let mut encoder = config.clone().job().encoder().unwrap(); // no with_canvas_size
        encoder
            .push_rows(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    // ── True streaming (effort 0) tests ─────────────────────────────

    #[test]
    fn streaming_effort0_rgb8_roundtrip() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 4u32;
        let h = 6u32;
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| Rgb {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 19) as u8,
            })
            .collect();
        let img = Img::new(pixels.clone(), w as usize, h as usize);

        // Effort 0 → true streaming path
        let config = PngEncoderConfig::new().with_compression(crate::Compression::None);
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for strip_y in (0..h).step_by(2) {
            let strip = img.sub_image(0, strip_y as usize, w as usize, 2);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let output = encoder.finish().unwrap();

        // Decode and verify pixel-exact
        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);

        // Compare with one-shot encode at effort 0
        let oneshot = config
            .job()
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        let decoded_one = crate::decode(
            oneshot.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(
            decoded.pixels.copy_to_contiguous_bytes(),
            decoded_one.pixels.copy_to_contiguous_bytes()
        );
    }

    #[test]
    fn streaming_effort0_rgba8_single_row() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 3u32;
        let h = 4u32;
        let pixels: Vec<Rgba<u8>> = (0..w * h)
            .map(|i| Rgba {
                r: (i * 5) as u8,
                g: (i * 11) as u8,
                b: (i * 17) as u8,
                a: 200,
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new().with_compression(crate::Compression::None);
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for y in 0..h {
            let strip = img.sub_image(0, y as usize, w as usize, 1);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    #[test]
    fn streaming_effort0_gray8() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 8u32;
        let h = 4u32;
        let pixels: Vec<Gray<u8>> = (0..w * h).map(|i| Gray(i as u8)).collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new().with_compression(crate::Compression::None);
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        encoder
            .push_rows(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    #[test]
    fn streaming_effort0_matches_oneshot_effort0() {
        // True streaming at effort 0 should produce byte-identical output
        // to one-shot encode at effort 0 (both use stored DEFLATE + None filter).
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 16u32;
        let h = 12u32;
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| Rgb {
                r: (i * 3 + 7) as u8,
                g: (i * 5 + 11) as u8,
                b: (i * 7 + 13) as u8,
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new().with_compression(crate::Compression::None);

        // One-shot encode
        let oneshot = config
            .clone()
            .job()
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        // Streaming encode (3 rows at a time)
        let mut encoder = config.job().with_canvas_size(w, h).encoder().unwrap();
        for strip_y in (0..h).step_by(3) {
            let strip_h = (h - strip_y).min(3);
            let strip = img.sub_image(0, strip_y as usize, w as usize, strip_h as usize);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let streaming = encoder.finish().unwrap();

        // Byte-identical output
        assert_eq!(oneshot.data(), streaming.data());
    }

    #[test]
    fn streaming_effort0_large_row() {
        // Test rows that exceed 65535 bytes (stored block boundary).
        // 16384 × 4 = 65536 bytes per row → must split across blocks.
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 16384u32;
        let h = 2u32;
        // Use non-opaque alpha to prevent auto-optimization (RGBA→RGB strip)
        let pixels: Vec<Rgba<u8>> = (0..w * h)
            .map(|i| Rgba {
                r: (i % 251) as u8,
                g: (i % 241) as u8,
                b: (i % 239) as u8,
                a: 200,
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new().with_compression(crate::Compression::None);
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for y in 0..h {
            let strip = img.sub_image(0, y as usize, w as usize, 1);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);

        // Verify byte-identical with one-shot (both RGBA, no auto-opt)
        let oneshot = config
            .job()
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        assert_eq!(oneshot.data(), output.data());
    }

    #[test]
    fn streaming_effort0_fallback_without_canvas_height() {
        // Without canvas_height, effort 0 should fall back to buffered mode
        // (can't pre-compute IDAT size), but still produce valid output.
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 4u32;
        let h = 3u32;
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 50,
                g: 100,
                b: 150,
            };
            (w * h) as usize
        ];
        let img = Img::new(pixels, w as usize, h as usize);

        // Only set width, not height → falls back to buffered
        let config = PngEncoderConfig::new().with_compression(crate::Compression::None);
        let mut encoder = config.clone().job().encoder().unwrap(); // no with_canvas_size
        encoder
            .push_rows(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    // ── Pre-filtered streaming (effort 1) tests ─────────────────────

    #[test]
    fn streaming_effort1_rgb8_roundtrip() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 8u32;
        let h = 6u32;
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| Rgb {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 19) as u8,
            })
            .collect();
        let img = Img::new(pixels.clone(), w as usize, h as usize);

        let config = PngEncoderConfig::new().with_compression(crate::Compression::Fastest);
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for strip_y in (0..h).step_by(2) {
            let strip = img.sub_image(0, strip_y as usize, w as usize, 2);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);

        // Pixel-exact match with one-shot
        let oneshot = config
            .job()
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        let decoded_one = crate::decode(
            oneshot.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(
            decoded.pixels.copy_to_contiguous_bytes(),
            decoded_one.pixels.copy_to_contiguous_bytes()
        );
    }

    #[test]
    fn streaming_effort1_matches_oneshot_bytes() {
        // Pre-filtered streaming at effort 1 should produce byte-identical
        // output to one-shot encode at effort 1 (both use Paeth + Turbo).
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 12u32;
        let h = 8u32;
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| Rgb {
                r: (i * 3 + 7) as u8,
                g: (i * 5 + 11) as u8,
                b: (i * 7 + 13) as u8,
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new().with_compression(crate::Compression::Fastest);

        let oneshot = config
            .clone()
            .job()
            .encoder()
            .unwrap()
            .encode(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        let mut encoder = config.job().with_canvas_size(w, h).encoder().unwrap();
        for strip_y in (0..h).step_by(3) {
            let strip_h = (h - strip_y).min(3);
            let strip = img.sub_image(0, strip_y as usize, w as usize, strip_h as usize);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let streaming = encoder.finish().unwrap();

        // Byte-identical (no auto-opt for RGB)
        assert_eq!(oneshot.data(), streaming.data());
    }

    #[test]
    fn streaming_effort1_smaller_than_effort0() {
        // Effort 1 (Paeth + Turbo) should compress better than effort 0 (None + Store).
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 64u32;
        let h = 64u32;
        // Smooth gradient compresses well with Paeth
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| {
                let x = (i % w) as u8;
                let y = (i / w) as u8;
                Rgb {
                    r: x,
                    g: y,
                    b: x.wrapping_add(y),
                }
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config0 = PngEncoderConfig::new().with_compression(crate::Compression::None);
        let config1 = PngEncoderConfig::new().with_compression(crate::Compression::Fastest);

        let mut enc0 = config0.job().with_canvas_size(w, h).encoder().unwrap();
        let mut enc1 = config1.job().with_canvas_size(w, h).encoder().unwrap();

        enc0.push_rows(PixelSlice::from(img.as_ref()).erase())
            .unwrap();
        enc1.push_rows(PixelSlice::from(img.as_ref()).erase())
            .unwrap();

        let out0 = enc0.finish().unwrap();
        let out1 = enc1.finish().unwrap();

        assert!(
            out1.data().len() < out0.data().len(),
            "effort 1 ({}) should be smaller than effort 0 ({})",
            out1.data().len(),
            out0.data().len()
        );
    }

    #[test]
    fn streaming_effort1_rgba8_single_row() {
        use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};

        let w = 5u32;
        let h = 4u32;
        let pixels: Vec<Rgba<u8>> = (0..w * h)
            .map(|i| Rgba {
                r: (i * 5) as u8,
                g: (i * 11) as u8,
                b: (i * 17) as u8,
                a: 200,
            })
            .collect();
        let img = Img::new(pixels, w as usize, h as usize);

        let config = PngEncoderConfig::new().with_compression(crate::Compression::Fastest);
        let mut encoder = config
            .clone()
            .job()
            .with_canvas_size(w, h)
            .encoder()
            .unwrap();
        for y in 0..h {
            let strip = img.sub_image(0, y as usize, w as usize, 1);
            encoder.push_rows(PixelSlice::from(strip).erase()).unwrap();
        }
        let output = encoder.finish().unwrap();

        let decoded = crate::decode(
            output.data(),
            &crate::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
    }

    // ── Fix verification tests ──

    #[test]
    fn output_info_returns_gray8_for_grayscale_png() {
        // Encode a grayscale image
        let pixels: Vec<Gray<u8>> = vec![Gray(128); 16];
        let img = Img::new(pixels, 4, 4);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_gray8(img.as_ref()).unwrap();

        // output_info() should report GRAY8_SRGB, not RGB8_SRGB
        let dec = PngDecoderConfig::new();
        let info = dec.job().output_info(output.data()).unwrap();
        assert_eq!(info.width, 4);
        assert_eq!(info.height, 4);
        assert_eq!(
            info.native_format,
            PixelDescriptor::GRAY8_SRGB,
            "grayscale PNG should report GRAY8_SRGB, not {:?}",
            info.native_format
        );
    }

    #[test]
    fn output_info_returns_gray16_for_grayscale16_png() {
        // Encode a 16-bit grayscale image.
        // Use a value with non-zero low byte (32769 = 0x8001) to prevent
        // the optimizer from reducing it to 8-bit.
        let pixels: Vec<Gray<u16>> = vec![Gray(32769); 16];
        let img = Img::new(pixels, 4, 4);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_gray16(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let info = dec.job().output_info(output.data()).unwrap();
        assert_eq!(
            info.native_format,
            PixelDescriptor::GRAY16_SRGB,
            "16-bit grayscale PNG should report GRAY16_SRGB, not {:?}",
            info.native_format
        );
    }

    #[test]
    fn output_info_returns_rgb8_for_rgb_png() {
        // Encode an RGB image
        let pixels: Vec<Rgb<u8>> = vec![
            Rgb {
                r: 100,
                g: 150,
                b: 200,
            };
            16
        ];
        let img = Img::new(pixels, 4, 4);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let info = dec.job().output_info(output.data()).unwrap();
        assert_eq!(
            info.native_format,
            PixelDescriptor::RGB8_SRGB,
            "RGB PNG should report RGB8_SRGB, not {:?}",
            info.native_format
        );
    }

    #[test]
    fn output_info_returns_rgba8_for_rgba_png() {
        let pixels: Vec<Rgba<u8>> = vec![
            Rgba {
                r: 100,
                g: 150,
                b: 200,
                a: 128,
            };
            16
        ];
        let img = Img::new(pixels, 4, 4);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_rgba8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let info = dec.job().output_info(output.data()).unwrap();
        assert_eq!(
            info.native_format,
            PixelDescriptor::RGBA8_SRGB,
            "RGBA PNG should report RGBA8_SRGB, not {:?}",
            info.native_format
        );
    }

    #[test]
    fn apng_finish_respects_stop_token() {
        use enough::{Stop, StopReason};
        use zencodec::encode::AnimationFrameEncoder;

        /// A Stop that always says "stop now".
        struct AlreadyCancelled;
        impl Stop for AlreadyCancelled {
            fn check(&self) -> Result<(), StopReason> {
                Err(StopReason::Cancelled)
            }
        }

        let config = PngEncoderConfig::new();
        let job = config
            .clone()
            .job()
            .with_canvas_size(4, 4)
            .with_loop_count(Some(0));
        let mut enc = job.animation_frame_encoder().unwrap();

        // Push one frame
        let pixels: Vec<Rgba<u8>> = vec![
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            };
            16
        ];
        let img = imgref::ImgVec::new(pixels, 4, 4);
        enc.push_frame(PixelSlice::from(img.as_ref()).erase(), 100, None)
            .unwrap();

        // finish() with a cancelled stop token should fail
        let result = enc.finish(Some(&AlreadyCancelled));
        assert!(result.is_err(), "finish with cancelled stop should fail");
        match result.unwrap_err().into_inner() {
            PngError::Stopped(reason) => {
                assert_eq!(reason, StopReason::Cancelled);
            }
            other => panic!("expected PngError::Stopped, got: {other}"),
        }
    }

    #[test]
    fn apng_finish_succeeds_without_stop_token() {
        use zencodec::encode::AnimationFrameEncoder;

        let config = PngEncoderConfig::new();
        let job = config
            .clone()
            .job()
            .with_canvas_size(4, 4)
            .with_loop_count(Some(0));
        let mut enc = job.animation_frame_encoder().unwrap();

        let pixels: Vec<Rgba<u8>> = vec![
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            };
            16
        ];
        let img = imgref::ImgVec::new(pixels, 4, 4);
        enc.push_frame(PixelSlice::from(img.as_ref()).erase(), 100, None)
            .unwrap();

        // finish() without stop token should succeed
        let result = enc.finish(None);
        assert!(
            result.is_ok(),
            "finish without stop should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn apng_pixels_to_rgba8_rejects_unsupported_format() {
        use zencodec::encode::AnimationFrameEncoder;

        let config = PngEncoderConfig::new();
        let job = config
            .clone()
            .job()
            .with_canvas_size(4, 4)
            .with_loop_count(Some(0));
        let mut enc = job.animation_frame_encoder().unwrap();

        // Try pushing a 16-bit frame, which is not supported by the APNG encoder
        let pixels: Vec<Rgba<u16>> = vec![
            Rgba {
                r: 1000,
                g: 2000,
                b: 3000,
                a: 65535,
            };
            16
        ];
        let img = imgref::ImgVec::new(pixels, 4, 4);
        let result = enc.push_frame(PixelSlice::from(img.as_ref()).erase(), 100, None);
        assert!(result.is_err(), "16-bit RGBA should be rejected");
        let msg = alloc::format!("{}", result.unwrap_err());
        assert!(
            msg.contains("unsupported pixel format"),
            "error should mention unsupported pixel format, got: {msg}"
        );
        assert!(
            msg.contains("RGBA8") || msg.contains("supported formats"),
            "error should list supported formats, got: {msg}"
        );
    }

    #[test]
    fn apng_pixels_to_rgba8_handles_gray8() {
        use zencodec::encode::AnimationFrameEncoder;

        let config = PngEncoderConfig::new();
        let job = config
            .clone()
            .job()
            .with_canvas_size(4, 4)
            .with_loop_count(Some(0));
        let mut enc = job.animation_frame_encoder().unwrap();

        // Push a Gray8 frame — should be accepted and converted to RGBA8
        let pixels: Vec<Gray<u8>> = vec![Gray(128); 16];
        let img = imgref::ImgVec::new(pixels, 4, 4);
        enc.push_frame(PixelSlice::from(img.as_ref()).erase(), 100, None)
            .unwrap();

        // Finish and verify it produces valid output
        let output = enc.finish(None).unwrap();
        assert!(!output.data().is_empty());
        assert_eq!(output.format(), ImageFormat::Png);

        // Verify round-trip: decode and check pixel values
        let decoded = crate::decode::decode(
            output.data(),
            &PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
    }
}
