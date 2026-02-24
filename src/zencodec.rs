//! zencodec-types trait implementations for PNG.
//!
//! Provides [`PngEncoderConfig`] and [`PngDecoderConfig`] types that implement the
//! [`EncoderConfig`] / [`DecoderConfig`] traits from zencodec-types.

extern crate std;

use alloc::vec::Vec;

use zencodec_types::{
    CodecCapabilities, DecodeFrame, DecodeOutput, EncodeOutput, ImageFormat, ImageInfo,
    MetadataView, OutputInfo, PixelDescriptor, PixelSlice, PixelSliceMut, ResourceLimits, Stop,
};

use crate::decode::PngDecodeConfig;
use crate::encode::EncodeConfig;
use crate::error::PngError;

/// Default encode timeout: 120 seconds.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

// ── Capabilities ─────────────────────────────────────────────────────

static ENCODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_encode_icc(true)
    .with_encode_exif(true)
    .with_encode_xmp(true)
    .with_native_gray(true)
    .with_cheap_probe(true)
    .with_lossless(true)
    .with_effort_range(0, 10);

static DECODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_decode_icc(true)
    .with_decode_exif(true)
    .with_decode_xmp(true)
    .with_native_gray(true)
    .with_cheap_probe(true);

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

/// PNG encoder configuration implementing [`EncoderConfig`](zencodec_types::EncoderConfig).
///
/// PNG is lossless — quality is not applicable.
/// Use [`with_effort`](PngEncoderConfig::with_effort) to control compression level.
#[derive(Clone, Debug)]
pub struct PngEncoderConfig {
    config: EncodeConfig,
    effort: Option<i32>,
}

impl PngEncoderConfig {
    /// Create a default PNG encoder config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: EncodeConfig::default(),
            effort: None,
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
}

impl Default for PngEncoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

fn effort_to_compression(effort: i32) -> crate::Compression {
    use crate::Compression;
    match effort {
        ..=1 => Compression::Fast,
        2..=3 => Compression::Balanced,
        4..=5 => Compression::Thorough,
        6..=7 => Compression::High,
        8 => Compression::Aggressive,
        _ => Compression::Best,
    }
}

impl zencodec_types::EncoderConfig for PngEncoderConfig {
    type Error = PngError;
    type Job<'a> = PngEncodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Png
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ENCODE_DESCRIPTORS
    }

    fn capabilities() -> &'static CodecCapabilities {
        &ENCODE_CAPS
    }

    fn with_effort(mut self, effort: i32) -> Self {
        self.effort = Some(effort);
        self.config.compression = effort_to_compression(effort);
        self
    }

    fn effort(&self) -> Option<i32> {
        self.effort
    }

    fn is_lossless(&self) -> Option<bool> {
        Some(true)
    }

    fn job(&self) -> PngEncodeJob<'_> {
        PngEncodeJob {
            config: self,
            stop: None,
            metadata: None,
            limits: None,
            canvas_width: 0,
            canvas_height: 0,
        }
    }
}

// ── PngEncodeJob ─────────────────────────────────────────────────────

/// Per-operation PNG encode job.
pub struct PngEncodeJob<'a> {
    config: &'a PngEncoderConfig,
    stop: Option<&'a dyn Stop>,
    metadata: Option<&'a MetadataView<'a>>,
    limits: Option<ResourceLimits>,
    canvas_width: u32,
    canvas_height: u32,
}

impl<'a> zencodec_types::EncodeJob<'a> for PngEncodeJob<'a> {
    type Error = PngError;
    type Encoder = PngEncoder<'a>;
    type FrameEncoder = PngFrameEncoder;

    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
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

    fn encoder(self) -> PngEncoder<'a> {
        PngEncoder {
            config: self.config,
            stop: self.stop,
            metadata: self.metadata,
            limits: self.limits,
        }
    }

    fn frame_encoder(self) -> Result<PngFrameEncoder, PngError> {
        let owned_meta = self.metadata.map(OwnedMetadata::from_metadata);
        Ok(PngFrameEncoder::new(
            self.config.config.clone(),
            self.canvas_width,
            self.canvas_height,
            owned_meta,
        ))
    }
}

// ── PngEncoder ───────────────────────────────────────────────────────

/// Single-image PNG encoder.
pub struct PngEncoder<'a> {
    config: &'a PngEncoderConfig,
    stop: Option<&'a dyn Stop>,
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
    ) -> Result<EncodeOutput, PngError> {
        self.do_encode_with_depth(bytes, w, h, color_type, crate::encode::BitDepth::Eight)
    }

    fn do_encode_with_depth(
        &self,
        bytes: &[u8],
        w: u32,
        h: u32,
        color_type: crate::encode::ColorType,
        bit_depth: crate::encode::BitDepth,
    ) -> Result<EncodeOutput, PngError> {
        let cancel: &dyn Stop = self.stop.unwrap_or(&enough::Unstoppable);
        // Pre-flight stop check
        cancel.check()?;
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

impl zencodec_types::Encoder for PngEncoder<'_> {
    type Error = PngError;

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, PngError> {
        let desc = pixels.descriptor();
        let w = pixels.width();
        let h = pixels.rows();

        match (desc.channel_type, desc.layout) {
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgb) => {
                let bytes = collect_contiguous_bytes(&pixels);
                self.do_encode(&bytes, w, h, crate::encode::ColorType::Rgb)
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgba) => {
                let bytes = collect_contiguous_bytes(&pixels);
                self.do_encode(&bytes, w, h, crate::encode::ColorType::Rgba)
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Gray) => {
                // Gray<u8> needs .value() extraction (not byte-contiguous via ComponentBytes)
                let bytes = collect_gray8_bytes(&pixels);
                self.do_encode(&bytes, w, h, crate::encode::ColorType::Grayscale)
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Bgra) => {
                // BGRA → RGBA swizzle
                let src = collect_contiguous_bytes(&pixels);
                let rgba: Vec<u8> = src
                    .chunks_exact(4)
                    .flat_map(|c| [c[2], c[1], c[0], c[3]])
                    .collect();
                self.do_encode(&rgba, w, h, crate::encode::ColorType::Rgba)
            }
            (zencodec_types::ChannelType::U16, zencodec_types::ChannelLayout::Rgb) => {
                let bytes = collect_contiguous_bytes(&pixels);
                let be = native_to_be_16(&bytes);
                self.do_encode_with_depth(
                    &be,
                    w,
                    h,
                    crate::encode::ColorType::Rgb,
                    crate::encode::BitDepth::Sixteen,
                )
            }
            (zencodec_types::ChannelType::U16, zencodec_types::ChannelLayout::Rgba) => {
                let bytes = collect_contiguous_bytes(&pixels);
                let be = native_to_be_16(&bytes);
                self.do_encode_with_depth(
                    &be,
                    w,
                    h,
                    crate::encode::ColorType::Rgba,
                    crate::encode::BitDepth::Sixteen,
                )
            }
            (zencodec_types::ChannelType::U16, zencodec_types::ChannelLayout::Gray) => {
                let bytes = collect_contiguous_bytes(&pixels);
                let be = native_to_be_16(&bytes);
                self.do_encode_with_depth(
                    &be,
                    w,
                    h,
                    crate::encode::ColorType::Grayscale,
                    crate::encode::BitDepth::Sixteen,
                )
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgb) => {
                use linear_srgb::default::linear_to_srgb_u8;
                let src = collect_contiguous_bytes(&pixels);
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
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgba) => {
                use linear_srgb::default::linear_to_srgb_u8;
                let src = collect_contiguous_bytes(&pixels);
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
                self.do_encode(&srgb, w, h, crate::encode::ColorType::Rgba)
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Gray) => {
                use linear_srgb::default::linear_to_srgb_u8;
                let src = collect_contiguous_bytes(&pixels);
                let srgb: Vec<u8> = src
                    .chunks_exact(4)
                    .map(|c| {
                        let v = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                        linear_to_srgb_u8(v.clamp(0.0, 1.0))
                    })
                    .collect();
                self.do_encode(&srgb, w, h, crate::encode::ColorType::Grayscale)
            }
            _ => Err(PngError::InvalidInput(alloc::format!(
                "unsupported pixel format: {:?}",
                desc
            ))),
        }
    }

    fn push_rows(&mut self, _rows: PixelSlice<'_>) -> Result<(), PngError> {
        Err(PngError::InvalidInput(
            "PNG does not support incremental encoding".into(),
        ))
    }

    fn finish(self) -> Result<EncodeOutput, PngError> {
        Err(PngError::InvalidInput(
            "PNG does not support incremental encoding".into(),
        ))
    }

    fn encode_from(
        self,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, PngError> {
        Err(PngError::InvalidInput(
            "PNG does not support pull-based encoding".into(),
        ))
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
    cicp: Option<zencodec_types::Cicp>,
    content_light_level: Option<zencodec_types::ContentLightLevel>,
    mastering_display: Option<zencodec_types::MasteringDisplay>,
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

/// APNG frame-by-frame encoder implementing [`FrameEncoder`](zencodec_types::FrameEncoder).
///
/// Accumulates canvas-sized RGBA8 frames, then encodes them all on [`finish()`](PngFrameEncoder::finish).
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
        match (desc.channel_type, desc.layout) {
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgba) => {
                Ok(collect_contiguous_bytes(pixels))
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Bgra) => {
                let src = collect_contiguous_bytes(pixels);
                Ok(src
                    .chunks_exact(4)
                    .flat_map(|c| [c[2], c[1], c[0], c[3]])
                    .collect())
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgb) => {
                let src = collect_contiguous_bytes(pixels);
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

impl zencodec_types::FrameEncoder for PngFrameEncoder {
    type Error = PngError;

    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32) -> Result<(), PngError> {
        let rgba = Self::pixels_to_rgba8(&pixels)?;
        self.frames.push(AccumulatedFrame {
            pixels: rgba,
            duration_ms,
        });
        Ok(())
    }

    fn begin_frame(&mut self, duration_ms: u32) -> Result<(), PngError> {
        if self.building_frame.is_some() {
            return Err(PngError::InvalidInput(
                "begin_frame called while a frame is already in progress".into(),
            ));
        }
        let expected = self.canvas_width as usize * self.canvas_height as usize * 4;
        self.building_frame = Some(BuildingFrame {
            pixels: Vec::with_capacity(expected),
            duration_ms,
            rows_pushed: 0,
        });
        Ok(())
    }

    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), PngError> {
        let frame = self
            .building_frame
            .as_mut()
            .ok_or_else(|| PngError::InvalidInput("push_rows called without begin_frame".into()))?;

        let rgba = Self::pixels_to_rgba8(&rows)?;
        let row_count = rows.rows();
        frame.pixels.extend_from_slice(&rgba);
        frame.rows_pushed += row_count;
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), PngError> {
        let frame = self
            .building_frame
            .take()
            .ok_or_else(|| PngError::InvalidInput("end_frame called without begin_frame".into()))?;

        self.frames.push(AccumulatedFrame {
            pixels: frame.pixels,
            duration_ms: frame.duration_ms,
        });
        Ok(())
    }

    fn pull_frame(
        &mut self,
        _duration_ms: u32,
        _source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<(), PngError> {
        Err(PngError::InvalidInput(
            "APNG frame encoder: pull_frame not yet implemented".into(),
        ))
    }

    fn with_loop_count(&mut self, count: Option<u32>) {
        self.loop_count = count.unwrap_or(0);
    }

    fn finish(self) -> Result<EncodeOutput, PngError> {
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

/// PNG decoder configuration implementing [`DecoderConfig`](zencodec_types::DecoderConfig).
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

impl Default for PngDecoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl zencodec_types::DecoderConfig for PngDecoderConfig {
    type Error = PngError;
    type Job<'a> = PngDecodeJob<'a>;

    fn format() -> ImageFormat {
        ImageFormat::Png
    }

    fn supported_descriptors() -> &'static [PixelDescriptor] {
        DECODE_DESCRIPTORS
    }

    fn capabilities() -> &'static CodecCapabilities {
        &DECODE_CAPS
    }

    fn job(&self) -> PngDecodeJob<'_> {
        PngDecodeJob {
            config: self,
            stop: None,
            limits: None,
        }
    }

    fn probe_header(&self, data: &[u8]) -> Result<ImageInfo, Self::Error> {
        let info = crate::decode::probe(data)?;
        Ok(convert_info(&info))
    }
}

// ── PngDecodeJob ─────────────────────────────────────────────────────

/// Per-operation PNG decode job.
pub struct PngDecodeJob<'a> {
    config: &'a PngDecoderConfig,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
}

impl<'a> zencodec_types::DecodeJob<'a> for PngDecodeJob<'a> {
    type Error = PngError;
    type Decoder = PngDecoder<'a>;
    type FrameDecoder = PngFrameDecoder;
    fn with_stop(mut self, stop: &'a dyn Stop) -> Self {
        self.stop = Some(stop);
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, PngError> {
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

    fn decoder(self) -> PngDecoder<'a> {
        PngDecoder {
            config: self.config,
            stop: self.stop,
            limits: self.limits,
        }
    }

    fn frame_decoder(self, data: &[u8]) -> Result<PngFrameDecoder, PngError> {
        PngFrameDecoder::new(data, self.config, self.stop)
    }
}

// ── PngDecoder ───────────────────────────────────────────────────────

/// Single-image PNG decoder.
pub struct PngDecoder<'a> {
    config: &'a PngDecoderConfig,
    stop: Option<&'a dyn Stop>,
    limits: Option<ResourceLimits>,
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

impl zencodec_types::Decoder for PngDecoder<'_> {
    type Error = PngError;

    fn decode(self, data: &[u8]) -> Result<DecodeOutput, PngError> {
        let cancel: &dyn Stop = self.stop.unwrap_or(&enough::Unstoppable);
        cancel.check()?;
        let png_config = self.effective_config();
        let result = crate::decode::decode(data, &png_config, cancel)?;
        let info = convert_info(&result.info);
        Ok(DecodeOutput::new(result.pixels, info))
    }

    fn decode_into(self, data: &[u8], mut dst: PixelSliceMut<'_>) -> Result<ImageInfo, PngError> {
        let desc = dst.descriptor();
        let output = self.decode(data)?;
        let info = output.info().clone();
        let pixels = output.into_pixels();

        match (desc.channel_type, desc.layout) {
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgb) => {
                let src = to_rgb8(pixels);
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Rgba) => {
                let src = to_rgba8(pixels);
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Gray) => {
                let src = to_gray8(pixels);
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::U8, zencodec_types::ChannelLayout::Bgra) => {
                let src = to_bgra8(pixels);
                copy_rows_u8(&src, &mut dst);
            }
            (zencodec_types::ChannelType::U16, zencodec_types::ChannelLayout::Rgb) => {
                decode_into_rgb16(pixels, &mut dst);
            }
            (zencodec_types::ChannelType::U16, zencodec_types::ChannelLayout::Rgba) => {
                decode_into_rgba16(pixels, &mut dst);
            }
            (zencodec_types::ChannelType::U16, zencodec_types::ChannelLayout::Gray) => {
                decode_into_gray16(pixels, &mut dst);
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgb) => {
                decode_into_rgb_f32(pixels, &mut dst);
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Rgba) => {
                decode_into_rgba_f32(pixels, &mut dst);
            }
            (zencodec_types::ChannelType::F32, zencodec_types::ChannelLayout::Gray) => {
                decode_into_gray_f32(pixels, &mut dst);
            }
            _ => {
                return Err(PngError::InvalidInput(alloc::format!(
                    "unsupported decode_into format: {:?}",
                    desc
                )));
            }
        }

        Ok(info)
    }

    fn decode_rows(
        self,
        data: &[u8],
        sink: &mut dyn zencodec_types::DecodeRowSink,
    ) -> Result<ImageInfo, PngError> {
        let cancel: &dyn Stop = self.stop.unwrap_or(&enough::Unstoppable);
        cancel.check()?;
        let png_config = self.effective_config();

        // Full decode, then write rows to sink
        let result = crate::decode::decode(data, &png_config, cancel)?;
        let info = convert_info(&result.info);
        let w = result.info.width;
        let h = result.info.height;

        let descriptor = pixel_descriptor_for_data(&result.pixels);
        let bpp = descriptor.bytes_per_pixel();
        let row_bytes = w as usize * bpp;

        let pixel_bytes = pixel_data_bytes(&result.pixels);
        for y in 0..h {
            let src_start = y as usize * row_bytes;
            let src_end = src_start + row_bytes;
            if src_end <= pixel_bytes.len() {
                let (buf, _stride) = sink.demand(y, 1, w, bpp);
                buf[..row_bytes].copy_from_slice(&pixel_bytes[src_start..src_end]);
            }
        }

        Ok(info)
    }
}

// ── PngFrameDecoder ──────────────────────────────────────────────────

/// APNG frame-by-frame decoder implementing [`FrameDecoder`](zencodec_types::FrameDecoder).
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
        _stop: Option<&dyn Stop>,
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

impl zencodec_types::FrameDecoder for PngFrameDecoder {
    type Error = PngError;

    fn frame_count(&self) -> Option<u32> {
        Some(self.decoder_state.num_frames)
    }

    fn loop_count(&self) -> Option<u32> {
        Some(self.decoder_state.num_plays)
    }

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, PngError> {
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
            0 => zencodec_types::FrameBlend::Source,
            _ => zencodec_types::FrameBlend::Over,
        };
        let disposal = match fctl.dispose_op {
            0 => zencodec_types::FrameDisposal::None,
            1 => zencodec_types::FrameDisposal::RestoreBackground,
            _ => zencodec_types::FrameDisposal::RestorePrevious,
        };
        let delay_ms = fctl.delay_ms();
        let frame_rect = [fctl.x_offset, fctl.y_offset, fctl.width, fctl.height];

        let frame = DecodeFrame::new(raw.pixels, self.info.clone(), delay_ms, idx)
            .with_blend(blend)
            .with_disposal(disposal)
            .with_frame_rect(frame_rect);

        Ok(Some(frame))
    }

    fn next_frame_into(
        &mut self,
        _dst: PixelSliceMut<'_>,
        _prior_frame: Option<u32>,
    ) -> Result<Option<ImageInfo>, PngError> {
        // Not implemented yet — callers can use next_frame() and copy
        Err(PngError::InvalidInput(
            "PNG frame_decoder: next_frame_into not yet implemented".into(),
        ))
    }
}

// ── Pixel conversion helpers ─────────────────────────────────────────
//
// PNG decoder produces Rgb8, Rgba8, Gray8, Rgb16, Rgba16, Gray16,
// GrayAlpha16. These helpers convert to any requested target format.

use rgb::{Gray, Rgb, Rgba};
use zencodec_types::PixelData;

/// Convert native PNG pixel data to Rgb8. Downscales 16-bit by >>8.
fn to_rgb8(pixels: PixelData) -> imgref::ImgVec<Rgb<u8>> {
    match pixels {
        PixelData::Rgb8(img) => img,
        PixelData::Rgba8(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgb {
                    r: p.r,
                    g: p.g,
                    b: p.b,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Gray8(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let v = p.value();
                    Rgb { r: v, g: v, b: v }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgb16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgb {
                    r: (p.r >> 8) as u8,
                    g: (p.g >> 8) as u8,
                    b: (p.b >> 8) as u8,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgba16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgb {
                    r: (p.r >> 8) as u8,
                    g: (p.g >> 8) as u8,
                    b: (p.b >> 8) as u8,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Gray16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let v = (p.value() >> 8) as u8;
                    Rgb { r: v, g: v, b: v }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::GrayAlpha16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let v = (p.v >> 8) as u8;
                    Rgb { r: v, g: v, b: v }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        other => unreachable!("PNG decoder produced unexpected format: {other:?}"),
    }
}

/// Convert native PNG pixel data to Rgba8. Downscales 16-bit by >>8.
fn to_rgba8(pixels: PixelData) -> imgref::ImgVec<Rgba<u8>> {
    match pixels {
        PixelData::Rgba8(img) => img,
        PixelData::Rgb8(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgba {
                    r: p.r,
                    g: p.g,
                    b: p.b,
                    a: 255,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Gray8(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let v = p.value();
                    Rgba {
                        r: v,
                        g: v,
                        b: v,
                        a: 255,
                    }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgba16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgba {
                    r: (p.r >> 8) as u8,
                    g: (p.g >> 8) as u8,
                    b: (p.b >> 8) as u8,
                    a: (p.a >> 8) as u8,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgb16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgba {
                    r: (p.r >> 8) as u8,
                    g: (p.g >> 8) as u8,
                    b: (p.b >> 8) as u8,
                    a: 255,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Gray16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let v = (p.value() >> 8) as u8;
                    Rgba {
                        r: v,
                        g: v,
                        b: v,
                        a: 255,
                    }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::GrayAlpha16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let v = (p.v >> 8) as u8;
                    Rgba {
                        r: v,
                        g: v,
                        b: v,
                        a: (p.a >> 8) as u8,
                    }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        other => unreachable!("PNG decoder produced unexpected format: {other:?}"),
    }
}

/// Convert native PNG pixel data to Gray8. Downscales 16-bit by >>8.
fn to_gray8(pixels: PixelData) -> imgref::ImgVec<Gray<u8>> {
    match pixels {
        PixelData::Gray8(img) => img,
        PixelData::Rgb8(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let luma = ((p.r as u16 * 77 + p.g as u16 * 150 + p.b as u16 * 29) >> 8) as u8;
                    Gray::new(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgba8(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let luma = ((p.r as u16 * 77 + p.g as u16 * 150 + p.b as u16 * 29) >> 8) as u8;
                    Gray::new(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Gray16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Gray::new((p.value() >> 8) as u8))
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::GrayAlpha16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| Gray::new((p.v >> 8) as u8))
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgb16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let luma = ((p.r as u32 * 77 + p.g as u32 * 150 + p.b as u32 * 29) >> 16) as u8;
                    Gray::new(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgba16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u8>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let luma = ((p.r as u32 * 77 + p.g as u32 * 150 + p.b as u32 * 29) >> 16) as u8;
                    Gray::new(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        other => unreachable!("PNG decoder produced unexpected format: {other:?}"),
    }
}

/// Convert native PNG pixel data to Bgra8. Downscales 16-bit by >>8.
fn to_bgra8(pixels: PixelData) -> imgref::ImgVec<rgb::alt::BGRA<u8>> {
    // Convert to Rgba8 first (handles all formats including 16-bit),
    // then swizzle to BGRA.
    let rgba = to_rgba8(pixels);
    let w = rgba.width();
    let h = rgba.height();
    let buf: Vec<rgb::alt::BGRA<u8>> = rgba
        .into_buf()
        .into_iter()
        .map(|p| rgb::alt::BGRA {
            b: p.b,
            g: p.g,
            r: p.r,
            a: p.a,
        })
        .collect();
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

/// Collect contiguous bytes from a PixelSlice (handles stride).
fn collect_contiguous_bytes(pixels: &PixelSlice<'_>) -> Vec<u8> {
    let h = pixels.rows();
    let w = pixels.width();
    let bpp = pixels.descriptor().bytes_per_pixel();
    let row_bytes = w as usize * bpp;

    let mut out = Vec::with_capacity(row_bytes * h as usize);
    for y in 0..h {
        out.extend_from_slice(&pixels.row(y)[..row_bytes]);
    }
    out
}

/// Collect Gray8 pixel bytes (Gray<u8> has padding so we can't use ComponentBytes).
fn collect_gray8_bytes(pixels: &PixelSlice<'_>) -> Vec<u8> {
    // Gray<u8> is 1 byte per pixel, so contiguous bytes are already correct
    collect_contiguous_bytes(pixels)
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
fn decode_into_rgb_f32(pixels: PixelData, dst: &mut PixelSliceMut<'_>) {
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
fn decode_into_rgba_f32(pixels: PixelData, dst: &mut PixelSliceMut<'_>) {
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
fn decode_into_gray_f32(pixels: PixelData, dst: &mut PixelSliceMut<'_>) {
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

/// Convert any PixelData to Rgb<u16>. Upscales 8-bit by v*257.
fn to_rgb16(pixels: PixelData) -> imgref::ImgVec<Rgb<u16>> {
    match pixels {
        PixelData::Rgb16(img) => img,
        PixelData::Rgba16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u16>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgb {
                    r: p.r,
                    g: p.g,
                    b: p.b,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Gray16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u16>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let v = p.value();
                    Rgb { r: v, g: v, b: v }
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::GrayAlpha16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgb<u16>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgb {
                    r: p.v,
                    g: p.v,
                    b: p.v,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        other => {
            // Upscale 8-bit
            let rgb8 = to_rgb8(other);
            let w = rgb8.width();
            let h = rgb8.height();
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

/// Convert any PixelData to Rgba<u16>. Upscales 8-bit by v*257.
fn to_rgba16(pixels: PixelData) -> imgref::ImgVec<Rgba<u16>> {
    match pixels {
        PixelData::Rgba16(img) => img,
        PixelData::Rgb16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u16>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgba {
                    r: p.r,
                    g: p.g,
                    b: p.b,
                    a: 65535,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Gray16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u16>> = img
                .into_buf()
                .into_iter()
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
        PixelData::GrayAlpha16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Rgba<u16>> = img
                .into_buf()
                .into_iter()
                .map(|p| Rgba {
                    r: p.v,
                    g: p.v,
                    b: p.v,
                    a: p.a,
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        other => {
            // Upscale 8-bit
            let rgba8 = to_rgba8(other);
            let w = rgba8.width();
            let h = rgba8.height();
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

/// Convert any PixelData to Gray<u16>. Upscales 8-bit by v*257.
fn to_gray16(pixels: PixelData) -> imgref::ImgVec<Gray<u16>> {
    match pixels {
        PixelData::Gray16(img) => img,
        PixelData::GrayAlpha16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u16>> = img.into_buf().into_iter().map(|p| Gray(p.v)).collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgb16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u16>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let luma = ((p.r as u32 * 77 + p.g as u32 * 150 + p.b as u32 * 29) >> 8) as u16;
                    Gray(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        PixelData::Rgba16(img) => {
            let w = img.width();
            let h = img.height();
            let buf: Vec<Gray<u16>> = img
                .into_buf()
                .into_iter()
                .map(|p| {
                    let luma = ((p.r as u32 * 77 + p.g as u32 * 150 + p.b as u32 * 29) >> 8) as u16;
                    Gray(luma)
                })
                .collect();
            imgref::ImgVec::new(buf, w, h)
        }
        other => {
            // Upscale 8-bit
            let gray8 = to_gray8(other);
            let w = gray8.width();
            let h = gray8.height();
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
fn decode_into_rgb16(pixels: PixelData, dst: &mut PixelSliceMut<'_>) {
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
fn decode_into_rgba16(pixels: PixelData, dst: &mut PixelSliceMut<'_>) {
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
fn decode_into_gray16(pixels: PixelData, dst: &mut PixelSliceMut<'_>) {
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

/// Get the PixelDescriptor for decoded PixelData.
fn pixel_descriptor_for_data(pixels: &PixelData) -> PixelDescriptor {
    match pixels {
        PixelData::Rgb8(_) => PixelDescriptor::RGB8_SRGB,
        PixelData::Rgba8(_) => PixelDescriptor::RGBA8_SRGB,
        PixelData::Gray8(_) => PixelDescriptor::GRAY8_SRGB,
        PixelData::Rgb16(_) => PixelDescriptor::RGB16_SRGB,
        PixelData::Rgba16(_) => PixelDescriptor::RGBA16_SRGB,
        PixelData::Gray16(_) => PixelDescriptor::GRAY16_SRGB,
        PixelData::GrayAlpha16(_) => PixelDescriptor::new(
            zencodec_types::ChannelType::U16,
            zencodec_types::ChannelLayout::GrayAlpha,
            zencodec_types::AlphaMode::Straight,
            zencodec_types::TransferFunction::Srgb,
        ),
        _ => PixelDescriptor::RGBA8_SRGB, // fallback
    }
}

/// Get raw pixel bytes from PixelData (borrows the underlying buffer).
/// For GrayAlpha16, returns an empty slice (caller must handle separately).
fn pixel_data_bytes(pixels: &PixelData) -> Vec<u8> {
    use rgb::ComponentBytes;
    match pixels {
        PixelData::Rgb8(img) => img.buf().as_bytes().to_vec(),
        PixelData::Rgba8(img) => img.buf().as_bytes().to_vec(),
        PixelData::Gray8(img) => img.buf().as_bytes().to_vec(),
        PixelData::Rgb16(img) => bytemuck::cast_slice::<Rgb<u16>, u8>(img.buf()).to_vec(),
        PixelData::Rgba16(img) => bytemuck::cast_slice::<Rgba<u16>, u8>(img.buf()).to_vec(),
        PixelData::Gray16(img) => bytemuck::cast_slice::<Gray<u16>, u8>(img.buf()).to_vec(),
        PixelData::GrayAlpha16(img) => {
            // GrayAlpha<u16> is not Pod, manually serialize
            let mut bytes = Vec::with_capacity(img.buf().len() * 4);
            for ga in img.buf() {
                bytes.extend_from_slice(&ga.v.to_ne_bytes());
                bytes.extend_from_slice(&ga.a.to_ne_bytes());
            }
            bytes
        }
        _ => Vec::new(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use imgref::Img;
    use rgb::{Gray, Rgb, Rgba};
    use zencodec_types::{DecodeJob, Decoder, DecoderConfig, EncodeJob, Encoder, EncoderConfig};

    #[test]
    fn encoding_rgb8() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![
            Rgb {
                r: 128,
                g: 64,
                b: 32
            };
            64
        ];
        let img = Img::new(pixels, 8, 8);
        let output = enc.encode_rgb8(img.as_ref()).unwrap();
        assert!(!output.bytes().is_empty());
        assert_eq!(output.format(), ImageFormat::Png);
        assert_eq!(
            &output.bytes()[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn encoding_rgba8() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![
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
        assert!(!output.bytes().is_empty());
    }

    #[test]
    fn encoding_gray8() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![Gray::new(128u8); 64];
        let img = Img::new(pixels, 8, 8);
        let output = enc.encode_gray8(img.as_ref()).unwrap();
        assert!(!output.bytes().is_empty());
    }

    #[test]
    fn decode_roundtrip() {
        let enc = PngEncoderConfig::new();
        let pixels = vec![
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
        let output = dec.decode(encoded.bytes()).unwrap();
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
        let info = dec.probe_header(encoded.bytes()).unwrap();
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
        let info = dec.decode_into_rgb8(encoded.bytes(), dst.as_mut()).unwrap();
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
        let decoded = dec.decode(output.bytes()).unwrap();
        let rgba = to_rgba8(decoded.into_pixels());
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
            dec.decode_into_rgb_f32(output.bytes(), dst.as_mut())
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
        dec.decode_into_rgba_f32(output.bytes(), dst.as_mut())
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
        use zencodec_types::Gray;

        let pixels = vec![Gray(0.0f32), Gray(0.18), Gray(0.5), Gray(1.0)];
        let img = Img::new(pixels.clone(), 2, 2);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_gray_f32(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let mut dst = imgref::ImgVec::new(vec![Gray(0.0f32); 4], 2, 2);
        dec.decode_into_gray_f32(output.bytes(), dst.as_mut())
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
        dec.decode_into_rgb_f32(output.bytes(), dst.as_mut())
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
    fn capabilities_are_correct() {
        let caps = PngEncoderConfig::capabilities();
        assert!(caps.encode_icc());
        assert!(caps.encode_exif());
        assert!(caps.encode_xmp());
        assert!(caps.lossless());
        assert_eq!(caps.effort_range(), Some([0, 10]));
        assert_eq!(caps.quality_range(), None);
    }

    #[test]
    fn effort_getter_setter() {
        let enc = PngEncoderConfig::new().with_effort(5);
        assert_eq!(enc.effort(), Some(5));
        assert_eq!(enc.is_lossless(), Some(true));
    }

    #[test]
    fn output_info_matches_decode() {
        let pixels = vec![Rgb { r: 1u8, g: 2, b: 3 }; 6];
        let img = Img::new(pixels, 3, 2);
        let enc = PngEncoderConfig::new();
        let output = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoderConfig::new();
        let info = dec.job().output_info(output.bytes()).unwrap();
        assert_eq!(info.width, 3);
        assert_eq!(info.height, 2);

        let decoded = dec.decode(output.bytes()).unwrap();
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

        let slice = PixelSlice::from(img.as_ref());
        let output = config.job().encoder().encode(slice).unwrap();
        assert_eq!(output.format(), ImageFormat::Png);
        assert!(!output.bytes().is_empty());
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
        let decoded = config.job().decoder().decode(encoded.bytes()).unwrap();
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

        match &decoded.pixels {
            PixelData::Rgb16(img) => {
                let buf = img.buf();
                for (i, (orig, dec)) in pixels.iter().zip(buf.iter()).enumerate() {
                    assert_eq!(
                        orig, dec,
                        "pixel {i} mismatch: expected {orig:?}, got {dec:?}"
                    );
                }
            }
            other => panic!("expected Rgb16, got {other:?}"),
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

        match &decoded.pixels {
            PixelData::Rgba16(img) => {
                let buf = img.buf();
                for (i, (orig, dec)) in pixels.iter().zip(buf.iter()).enumerate() {
                    assert_eq!(
                        orig, dec,
                        "pixel {i} mismatch: expected {orig:?}, got {dec:?}"
                    );
                }
            }
            other => panic!("expected Rgba16, got {other:?}"),
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

        match &decoded.pixels {
            PixelData::Gray16(img) => {
                let buf = img.buf();
                for (i, (orig, dec)) in pixels.iter().zip(buf.iter()).enumerate() {
                    assert_eq!(
                        orig, dec,
                        "pixel {i} mismatch: expected {orig:?}, got {dec:?}"
                    );
                }
            }
            other => panic!("expected Gray16, got {other:?}"),
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

        match &decoded.pixels {
            PixelData::Rgb8(img) => {
                let buf = img.buf();
                for (orig, dec) in pixels.iter().zip(buf.iter()) {
                    assert_eq!(orig, dec);
                }
            }
            other => panic!("expected Rgb8, got {other:?}"),
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
        match &decoded.pixels {
            PixelData::Rgba8(img) => {
                let buf = img.buf();
                for (orig, dec) in pixels.iter().zip(buf.iter()) {
                    assert_eq!(orig, dec);
                }
            }
            other => panic!("expected Rgba8, got {other:?}"),
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
        match &decoded.pixels {
            PixelData::Gray8(img) => {
                let buf = img.buf();
                for (orig, dec) in pixels.iter().zip(buf.iter()) {
                    assert_eq!(orig, dec);
                }
            }
            other => panic!("expected Gray8, got {other:?}"),
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
        for (i, (orig, dec)) in pixels.iter().zip(decoded_rgba.buf().iter()).enumerate() {
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
        for (i, (orig, dec)) in pixels.iter().zip(decoded_rgba.buf().iter()).enumerate() {
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
        for (i, (orig, dec)) in pixels.iter().zip(decoded_rgba.buf().iter()).enumerate() {
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
        for (i, (orig, dec)) in pixels.iter().zip(decoded_rgba.buf().iter()).enumerate() {
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
        let output = enc.job().encoder().encode(slice).unwrap();
        assert_eq!(output.format(), ImageFormat::Png);

        // Decode back into U16
        let dec = PngDecoderConfig::new();
        let mut dst = imgref::ImgVec::new(vec![Rgb::<u16> { r: 0, g: 0, b: 0 }; 4], 2, 2);
        dec.decode_into_rgb16(output.bytes(), dst.as_mut()).unwrap();
        for (orig, dec) in pixels.iter().zip(dst.buf().iter()) {
            assert_eq!(orig, dec);
        }
    }

    #[test]
    fn gama_srgb_chrm_roundtrip() {
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

        // sRGB standard chromaticities
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

        assert_eq!(decoded.info.source_gamma, Some(45455));
        assert_eq!(decoded.info.srgb_intent, Some(0));
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
    fn cicp_roundtrip() {
        use zencodec_types::{Cicp, MetadataView};

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
        use zencodec_types::{ContentLightLevel, MasteringDisplay, MetadataView};

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
        use zencodec_types::PixelData;

        let path = "/home/lilith/work/codec-corpus/imageflow/test_inputs/frymire.png";
        let data = std::fs::read(path).expect("frymire.png not found");

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
        let PixelData::Rgb8(ref pixels) = orig.pixels else {
            panic!("frymire.png should decode as RGB8");
        };
        let encoded = crate::encode::encode_rgb8(
            pixels.as_ref(),
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
    #[test]
    fn real_file_srgb_roundtrip() {
        use zencodec_types::PixelData;

        let path = "/home/lilith/work/codec-corpus/imageflow/test_inputs/red-night.png";
        let data = std::fs::read(path).expect("red-night.png not found");

        let orig =
            crate::decode::decode(&data, &PngDecodeConfig::none(), &enough::Unstoppable).unwrap();
        let intent = orig
            .info
            .srgb_intent
            .expect("red-night.png should have sRGB");
        let gamma = orig
            .info
            .source_gamma
            .expect("red-night.png should have gAMA");
        let chrm = orig
            .info
            .chromaticities
            .expect("red-night.png should have cHRM");

        assert_eq!(intent, 0); // Perceptual

        // Re-encode with all color metadata
        let config = crate::encode::EncodeConfig {
            source_gamma: Some(gamma),
            srgb_intent: Some(intent),
            chromaticities: Some(chrm),
            ..Default::default()
        };
        let PixelData::Rgba8(ref pixels) = orig.pixels else {
            panic!("red-night.png should decode as RGBA8");
        };
        let encoded = crate::encode::encode_rgba8(
            pixels.as_ref(),
            None,
            &config,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let rt = crate::decode::decode(&encoded, &PngDecodeConfig::none(), &enough::Unstoppable)
            .unwrap();
        assert_eq!(rt.info.srgb_intent, Some(0));
        assert_eq!(rt.info.source_gamma, Some(gamma));
        assert_eq!(rt.info.chromaticities, Some(chrm));
    }

    /// Decode a real PNG with iCCP (Adobe RGB), re-encode preserving the
    /// ICC profile, verify the profile roundtrips.
    #[test]
    fn real_file_icc_roundtrip() {
        use zencodec_types::PixelData;

        let path = "/home/lilith/work/codec-corpus/imageflow/test_inputs/shirt_transparent.png";
        let data = std::fs::read(path).expect("shirt_transparent.png not found");

        let orig =
            crate::decode::decode(&data, &PngDecodeConfig::none(), &enough::Unstoppable).unwrap();
        let icc = orig
            .info
            .icc_profile
            .as_ref()
            .expect("shirt_transparent.png should have iCCP");
        assert!(!icc.is_empty());

        // Re-encode with ICC profile
        let meta = zencodec_types::MetadataView::none().with_icc(icc);
        let config = crate::encode::EncodeConfig::default();
        let PixelData::Rgba8(ref pixels) = orig.pixels else {
            panic!("shirt_transparent.png should decode as RGBA8");
        };
        let encoded = crate::encode::encode_rgba8(
            pixels.as_ref(),
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
        let path = "/home/lilith/work/codec-corpus/clic2025-1024/0d154749c7771f58e89ad343653ec4e20d6f037da829f47f5598e5d0a4ab61f0.png";
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => return, // skip if corpus not available
        };
        let decoded =
            crate::decode::decode(&data, &PngDecodeConfig::none(), &enough::Unstoppable).unwrap();
        let info = &decoded.info;
        let rgb_pixels = match &decoded.pixels {
            zencodec_types::PixelData::Rgb8(img) => img,
            other => panic!("expected Rgb8, got {:?}", other.descriptor()),
        };

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
                rgb_pixels.as_ref(),
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
        match &decoded.pixels {
            zencodec_types::PixelData::Rgb8(dec_img) => {
                assert_eq!(dec_img.width(), width);
                assert_eq!(dec_img.height(), height);
            }
            other => panic!("expected Rgb8, got {:?}", other.descriptor()),
        }
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
        let job = zencodec_types::EncoderConfig::job(&config);
        let job = zencodec_types::EncodeJob::with_stop(job, &AlreadyCancelled);
        let encoder = zencodec_types::EncodeJob::encoder(job);

        // Create a small 2x2 RGB8 image
        let pixels = [0u8; 12]; // 2x2 RGB
        let desc = PixelDescriptor::RGB8_SRGB;
        let slice = PixelSlice::new(&pixels, 2, 2, 6, desc).unwrap();

        let result = zencodec_types::Encoder::encode(encoder, slice);
        assert!(result.is_err());
        match result.unwrap_err() {
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
}
