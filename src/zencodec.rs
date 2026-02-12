//! zencodec-types trait implementations for PNG.
//!
//! Provides [`PngEncoding`] and [`PngDecoding`] types that implement the
//! [`Encoding`] / [`Decoding`] traits from zencodec-types.

extern crate std;

use alloc::vec::Vec;

use imgref::ImgRef;
use rgb::{Gray, Rgb, Rgba};

use zencodec_types::{
    CodecCapabilities, DecodeOutput as ZDecodeOutput, Decoding, DecodingJob,
    EncodeOutput as ZEncodeOutput, Encoding, EncodingJob, ImageFormat as ZImageFormat,
    ImageInfo as ZImageInfo, ImageMetadata as ZImageMetadata, ResourceLimits, Stop,
};

use crate::decode::PngLimits;
use crate::encode::EncodeConfig;
use crate::error::PngError;

// ── Encoding ────────────────────────────────────────────────────────────────

/// PNG encoder configuration implementing [`Encoding`].
///
/// PNG is lossless — quality and alpha_quality are not applicable.
/// Use [`with_effort`](PngEncoding::with_effort) to control compression level.
#[derive(Clone, Debug)]
pub struct PngEncoding {
    config: EncodeConfig,
    limits: ResourceLimits,
}

impl PngEncoding {
    /// Create a default PNG encoder config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: EncodeConfig::default(),
            limits: ResourceLimits::none(),
        }
    }

    /// Set compression effort (0–10).
    ///
    /// Maps to PNG compression levels:
    /// - 0–2: Fast
    /// - 3–7: Balanced
    /// - 8–10: High
    #[must_use]
    pub fn with_effort(mut self, effort: u32) -> Self {
        self.config.compression = match effort {
            0..=2 => png::Compression::Fast,
            3..=7 => png::Compression::Balanced,
            _ => png::Compression::High,
        };
        self
    }

    /// Set PNG compression level directly.
    #[must_use]
    pub fn with_compression(mut self, compression: png::Compression) -> Self {
        self.config.compression = compression;
        self
    }

    /// Set PNG row filter type directly.
    #[must_use]
    pub fn with_filter(mut self, filter: png::Filter) -> Self {
        self.config.filter = filter;
        self
    }
}

impl Default for PngEncoding {
    fn default() -> Self {
        Self::new()
    }
}

static ENCODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_encode_icc(true)
    .with_encode_exif(true)
    .with_encode_xmp(true)
    .with_native_gray(true)
    .with_cheap_probe(true);

impl Encoding for PngEncoding {
    type Error = PngError;
    type Job<'a> = PngEncodeJob<'a>;

    fn capabilities() -> &'static CodecCapabilities {
        &ENCODE_CAPS
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn job(&self) -> PngEncodeJob<'_> {
        PngEncodeJob {
            config: self,
            icc: None,
            exif: None,
            xmp: None,
            limits: self.limits,
        }
    }
}

// ── Encode job ──────────────────────────────────────────────────────────────

/// Per-operation PNG encode job.
pub struct PngEncodeJob<'a> {
    config: &'a PngEncoding,
    icc: Option<&'a [u8]>,
    exif: Option<&'a [u8]>,
    xmp: Option<&'a [u8]>,
    limits: ResourceLimits,
}

impl<'a> PngEncodeJob<'a> {
    /// Set ICC color profile for this encode operation.
    #[must_use]
    pub fn with_icc(mut self, icc: &'a [u8]) -> Self {
        self.icc = Some(icc);
        self
    }

    /// Set EXIF metadata for this encode operation.
    #[must_use]
    pub fn with_exif(mut self, exif: &'a [u8]) -> Self {
        self.exif = Some(exif);
        self
    }

    /// Set XMP metadata for this encode operation.
    #[must_use]
    pub fn with_xmp(mut self, xmp: &'a [u8]) -> Self {
        self.xmp = Some(xmp);
        self
    }

    fn build_metadata(&self) -> Option<ZImageMetadata<'a>> {
        if self.icc.is_none() && self.exif.is_none() && self.xmp.is_none() {
            return None;
        }
        let mut meta = ZImageMetadata::none();
        if let Some(icc) = self.icc {
            meta = meta.with_icc(icc);
        }
        if let Some(exif) = self.exif {
            meta = meta.with_exif(exif);
        }
        if let Some(xmp) = self.xmp {
            meta = meta.with_xmp(xmp);
        }
        Some(meta)
    }

    fn do_encode(
        self,
        bytes: &[u8],
        w: u32,
        h: u32,
        color_type: png::ColorType,
    ) -> Result<ZEncodeOutput, PngError> {
        let meta = self.build_metadata();
        let data =
            crate::encode::encode_raw(bytes, w, h, color_type, meta.as_ref(), &self.config.config)?;
        Ok(ZEncodeOutput::new(data, ZImageFormat::Png))
    }
}

impl<'a> EncodingJob<'a> for PngEncodeJob<'a> {
    type Error = PngError;

    fn with_stop(self, _stop: &'a dyn Stop) -> Self {
        self // PNG encoding is not cancellable
    }

    fn with_metadata(mut self, meta: &'a ZImageMetadata<'a>) -> Self {
        if let Some(icc) = meta.icc_profile {
            self.icc = Some(icc);
        }
        if let Some(exif) = meta.exif {
            self.exif = Some(exif);
        }
        if let Some(xmp) = meta.xmp {
            self.xmp = Some(xmp);
        }
        self
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn encode_rgb8(self, img: ImgRef<'_, Rgb<u8>>) -> Result<ZEncodeOutput, Self::Error> {
        let (buf, w, h) = img.to_contiguous_buf();
        let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
        self.do_encode(bytes, w as u32, h as u32, png::ColorType::Rgb)
    }

    fn encode_rgba8(self, img: ImgRef<'_, Rgba<u8>>) -> Result<ZEncodeOutput, Self::Error> {
        let (buf, w, h) = img.to_contiguous_buf();
        let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
        self.do_encode(bytes, w as u32, h as u32, png::ColorType::Rgba)
    }

    fn encode_gray8(self, img: ImgRef<'_, Gray<u8>>) -> Result<ZEncodeOutput, Self::Error> {
        let (buf, w, h) = img.to_contiguous_buf();
        let bytes: Vec<u8> = buf.iter().map(|g| g.value()).collect();
        self.do_encode(&bytes, w as u32, h as u32, png::ColorType::Grayscale)
    }
}

// ── Decoding ────────────────────────────────────────────────────────────────

/// PNG decoder configuration implementing [`Decoding`].
#[derive(Clone, Debug)]
pub struct PngDecoding {
    limits: ResourceLimits,
}

impl PngDecoding {
    /// Create a default PNG decoder config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: ResourceLimits::none(),
        }
    }
}

impl Default for PngDecoding {
    fn default() -> Self {
        Self::new()
    }
}

static DECODE_CAPS: CodecCapabilities = CodecCapabilities::new()
    .with_decode_icc(true)
    .with_decode_exif(true)
    .with_decode_xmp(true)
    .with_native_gray(true)
    .with_cheap_probe(true);

impl Decoding for PngDecoding {
    type Error = PngError;
    type Job<'a> = PngDecodeJob<'a>;

    fn capabilities() -> &'static CodecCapabilities {
        &DECODE_CAPS
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn job(&self) -> PngDecodeJob<'_> {
        PngDecodeJob {
            config: self,
            limits: self.limits,
        }
    }

    fn probe_header(&self, data: &[u8]) -> Result<ZImageInfo, Self::Error> {
        let info = crate::decode::probe(data)?;
        Ok(convert_info(&info))
    }
}

// ── Decode job ──────────────────────────────────────────────────────────────

/// Per-operation PNG decode job.
pub struct PngDecodeJob<'a> {
    config: &'a PngDecoding,
    limits: ResourceLimits,
}

impl<'a> DecodingJob<'a> for PngDecodeJob<'a> {
    type Error = PngError;

    fn with_stop(self, _stop: &'a dyn Stop) -> Self {
        self // PNG decoding is not cancellable
    }

    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    fn decode(self, data: &[u8]) -> Result<ZDecodeOutput, Self::Error> {
        // Merge job-level limits with config-level limits (job takes precedence).
        let max_pixels = self.limits.max_pixels.or(self.config.limits.max_pixels);
        let max_memory = self
            .limits
            .max_memory_bytes
            .or(self.config.limits.max_memory_bytes);

        let png_limits = if max_pixels.is_some() || max_memory.is_some() {
            Some(PngLimits {
                max_pixels,
                max_memory_bytes: max_memory,
            })
        } else {
            None
        };

        let result = crate::decode::decode(data, png_limits.as_ref())?;
        let info = convert_info(&result.info);

        Ok(ZDecodeOutput::new(result.pixels, info))
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn convert_info(info: &crate::decode::PngInfo) -> ZImageInfo {
    let mut zi = ZImageInfo::new(info.width, info.height, ZImageFormat::Png);
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
    zi
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use imgref::Img;
    use zencodec_types::{Decoding, Encoding};

    #[test]
    fn encoding_rgb8() {
        let enc = PngEncoding::new();
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
        assert_eq!(output.format(), ZImageFormat::Png);
        assert_eq!(
            &output.bytes()[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn encoding_rgba8() {
        let enc = PngEncoding::new();
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
        let enc = PngEncoding::new();
        let pixels = vec![Gray::new(128u8); 64];
        let img = Img::new(pixels, 8, 8);
        let output = enc.encode_gray8(img.as_ref()).unwrap();
        assert!(!output.bytes().is_empty());
    }

    #[test]
    fn decode_roundtrip() {
        let enc = PngEncoding::new();
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

        let dec = PngDecoding::new();
        let output = dec.decode(encoded.bytes()).unwrap();
        assert_eq!(output.info().width, 8);
        assert_eq!(output.info().height, 8);
        assert_eq!(output.info().format, ZImageFormat::Png);
    }

    #[test]
    fn probe_header_info() {
        let enc = PngEncoding::new();
        let pixels = vec![Rgb { r: 0, g: 0, b: 0 }; 100];
        let img = Img::new(pixels, 10, 10);
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoding::new();
        let info = dec.probe_header(encoded.bytes()).unwrap();
        assert_eq!(info.width, 10);
        assert_eq!(info.height, 10);
        assert_eq!(info.format, ZImageFormat::Png);
    }
}
