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

    fn encode_bgra8(
        self,
        img: ImgRef<'_, rgb::alt::BGRA<u8>>,
    ) -> Result<ZEncodeOutput, Self::Error> {
        let (buf, w, h) = img.to_contiguous_buf();
        let rgba: Vec<Rgba<u8>> = buf
            .iter()
            .map(|p| Rgba {
                r: p.r,
                g: p.g,
                b: p.b,
                a: p.a,
            })
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&rgba);
        self.do_encode(bytes, w as u32, h as u32, png::ColorType::Rgba)
    }

    fn encode_bgrx8(
        self,
        img: ImgRef<'_, rgb::alt::BGRA<u8>>,
    ) -> Result<ZEncodeOutput, Self::Error> {
        let (buf, w, h) = img.to_contiguous_buf();
        let rgb_pixels: Vec<Rgb<u8>> = buf
            .iter()
            .map(|p| Rgb {
                r: p.r,
                g: p.g,
                b: p.b,
            })
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&rgb_pixels);
        self.do_encode(bytes, w as u32, h as u32, png::ColorType::Rgb)
    }

    fn encode_rgb_f32(self, img: ImgRef<'_, Rgb<f32>>) -> Result<ZEncodeOutput, Self::Error> {
        use linear_srgb::default::linear_to_srgb_u8;
        let (buf, w, h) = img.to_contiguous_buf();
        let srgb: Vec<Rgb<u8>> = buf
            .iter()
            .map(|p| Rgb {
                r: linear_to_srgb_u8(p.r.clamp(0.0, 1.0)),
                g: linear_to_srgb_u8(p.g.clamp(0.0, 1.0)),
                b: linear_to_srgb_u8(p.b.clamp(0.0, 1.0)),
            })
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&srgb);
        self.do_encode(bytes, w as u32, h as u32, png::ColorType::Rgb)
    }

    fn encode_rgba_f32(self, img: ImgRef<'_, Rgba<f32>>) -> Result<ZEncodeOutput, Self::Error> {
        use linear_srgb::default::linear_to_srgb_u8;
        let (buf, w, h) = img.to_contiguous_buf();
        let srgb: Vec<Rgba<u8>> = buf
            .iter()
            .map(|p| Rgba {
                r: linear_to_srgb_u8(p.r.clamp(0.0, 1.0)),
                g: linear_to_srgb_u8(p.g.clamp(0.0, 1.0)),
                b: linear_to_srgb_u8(p.b.clamp(0.0, 1.0)),
                a: (p.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            })
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&srgb);
        self.do_encode(bytes, w as u32, h as u32, png::ColorType::Rgba)
    }

    fn encode_gray_f32(self, img: ImgRef<'_, Gray<f32>>) -> Result<ZEncodeOutput, Self::Error> {
        use linear_srgb::default::linear_to_srgb_u8;
        let (buf, w, h) = img.to_contiguous_buf();
        let bytes: Vec<u8> = buf
            .iter()
            .map(|g| linear_to_srgb_u8(g.value().clamp(0.0, 1.0)))
            .collect();
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

    fn decode_into_rgb8(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, Rgb<u8>>,
    ) -> Result<ZImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_rgb8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_rgba8(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, Rgba<u8>>,
    ) -> Result<ZImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_rgba8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_gray8(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, Gray<u8>>,
    ) -> Result<ZImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_gray8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_bgra8(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, rgb::alt::BGRA<u8>>,
    ) -> Result<ZImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_bgra8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            let n = src_row.len().min(dst_row.len());
            dst_row[..n].copy_from_slice(&src_row[..n]);
        }
        Ok(info)
    }

    fn decode_into_bgrx8(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, rgb::alt::BGRA<u8>>,
    ) -> Result<ZImageInfo, Self::Error> {
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_bgra8();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = rgb::alt::BGRA {
                    b: s.b,
                    g: s.g,
                    r: s.r,
                    a: 255,
                };
            }
        }
        Ok(info)
    }

    fn decode_into_rgb_f32(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, Rgb<f32>>,
    ) -> Result<ZImageInfo, Self::Error> {
        use linear_srgb::default::srgb_to_linear_fast;
        let output = self.decode(data)?;
        let info = output.info().clone();
        // Use into_rgb_f32() to preserve full source precision (u16 → f32 via /65535)
        // then linearize. This avoids truncating 16-bit PNG to 8-bit.
        let src = output.into_rgb_f32();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = Rgb {
                    r: srgb_to_linear_fast(s.r),
                    g: srgb_to_linear_fast(s.g),
                    b: srgb_to_linear_fast(s.b),
                };
            }
        }
        Ok(info)
    }

    fn decode_into_rgba_f32(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, Rgba<f32>>,
    ) -> Result<ZImageInfo, Self::Error> {
        use linear_srgb::default::srgb_to_linear_fast;
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_rgba_f32();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = Rgba {
                    r: srgb_to_linear_fast(s.r),
                    g: srgb_to_linear_fast(s.g),
                    b: srgb_to_linear_fast(s.b),
                    a: s.a,
                };
            }
        }
        Ok(info)
    }

    fn decode_into_gray_f32(
        self,
        data: &[u8],
        mut dst: imgref::ImgRefMut<'_, Gray<f32>>,
    ) -> Result<ZImageInfo, Self::Error> {
        use linear_srgb::default::srgb_to_linear_fast;
        let output = self.decode(data)?;
        let info = output.info().clone();
        let src = output.into_gray_f32();
        for (src_row, dst_row) in src.as_ref().rows().zip(dst.rows_mut()) {
            for (s, d) in src_row.iter().zip(dst_row.iter_mut()) {
                *d = Gray::new(srgb_to_linear_fast(s.value()));
            }
        }
        Ok(info)
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

    #[test]
    fn decode_into_rgb8_roundtrip() {
        let enc = PngEncoding::new();
        let pixels = vec![Rgb { r: 128, g: 64, b: 32 }; 64];
        let img = Img::new(pixels.clone(), 8, 8);
        let encoded = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoding::new();
        let mut buf = vec![Rgb { r: 0, g: 0, b: 0 }; 64];
        let mut dst = imgref::ImgVec::new(buf.clone(), 8, 8);
        let info = dec
            .decode_into_rgb8(encoded.bytes(), dst.as_mut())
            .unwrap();
        assert_eq!(info.width, 8);
        assert_eq!(info.height, 8);
        buf = dst.into_buf();
        assert_eq!(buf[0], pixels[0]);
    }

    #[test]
    fn encode_bgra8_roundtrip() {
        let enc = PngEncoding::new();
        let pixels = vec![
            rgb::alt::BGRA { b: 0, g: 0, r: 255, a: 255 },
            rgb::alt::BGRA { b: 0, g: 255, r: 0, a: 200 },
            rgb::alt::BGRA { b: 255, g: 0, r: 0, a: 128 },
            rgb::alt::BGRA { b: 128, g: 128, r: 128, a: 255 },
        ];
        let img = Img::new(pixels, 2, 2);
        let output = enc.encode_bgra8(img.as_ref()).unwrap();

        let dec = PngDecoding::new();
        let decoded = dec.decode(output.bytes()).unwrap();
        let rgba = decoded.into_rgba8();
        let buf = rgba.buf();
        assert_eq!(buf[0], Rgba { r: 255, g: 0, b: 0, a: 255 });
        assert_eq!(buf[1], Rgba { r: 0, g: 255, b: 0, a: 200 });
    }

    #[test]
    fn f32_conversion_all_simd_tiers() {
        use archmage::testing::{for_each_token_permutation, CompileTimePolicy};
        use linear_srgb::default::{linear_to_srgb_u8, srgb_u8_to_linear};

        let report = for_each_token_permutation(CompileTimePolicy::Warn, |_perm| {
            // Encode linear f32 → PNG (sRGB u8) → decode to linear f32
            let pixels = vec![
                Rgb { r: 0.0f32, g: 0.5, b: 1.0 },
                Rgb { r: 0.25, g: 0.75, b: 0.1 },
                Rgb { r: 0.0, g: 0.0, b: 0.0 },
                Rgb { r: 1.0, g: 1.0, b: 1.0 },
            ];
            let img = Img::new(pixels.clone(), 2, 2);
            let enc = PngEncoding::new();
            let output = enc.encode_rgb_f32(img.as_ref()).unwrap();

            let dec = PngDecoding::new();
            let mut buf = vec![Rgb { r: 0.0f32, g: 0.0, b: 0.0 }; 4];
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
            Rgba { r: 0.0f32, g: 0.5, b: 1.0, a: 1.0 },
            Rgba { r: 0.25, g: 0.75, b: 0.1, a: 0.5 },
            Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
            Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
        ];
        let img = Img::new(pixels.clone(), 2, 2);
        let enc = PngEncoding::new();
        let output = enc.encode_rgba_f32(img.as_ref()).unwrap();

        let dec = PngDecoding::new();
        let mut dst = imgref::ImgVec::new(
            vec![Rgba { r: 0.0f32, g: 0.0, b: 0.0, a: 0.0 }; 4],
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
            assert!((decoded.a - expected_a).abs() < 1e-2, "a mismatch: {} vs {}", decoded.a, expected_a);
        }
    }

    #[test]
    fn f32_gray_roundtrip() {
        use linear_srgb::default::{linear_to_srgb_u8, srgb_u8_to_linear};
        use zencodec_types::Gray;

        let pixels = vec![
            Gray(0.0f32),
            Gray(0.18),
            Gray(0.5),
            Gray(1.0),
        ];
        let img = Img::new(pixels.clone(), 2, 2);
        let enc = PngEncoding::new();
        let output = enc.encode_gray_f32(img.as_ref()).unwrap();

        let dec = PngDecoding::new();
        let mut dst = imgref::ImgVec::new(vec![Gray(0.0f32); 4], 2, 2);
        dec.decode_into_gray_f32(output.bytes(), dst.as_mut())
            .unwrap();

        for (orig, decoded) in pixels.iter().zip(dst.buf().iter()) {
            let expected = srgb_u8_to_linear(linear_to_srgb_u8(orig.0.clamp(0.0, 1.0)));
            assert!(
                (decoded.0 - expected).abs() < 1e-5,
                "gray mismatch: {} vs {}",
                decoded.0,
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
            Rgb { r: 128, g: 128, b: 128 },
            Rgb { r: 255, g: 255, b: 255 },
            Rgb { r: 255, g: 0, b: 0 },
        ];
        let img = Img::new(pixels, 2, 2);
        let enc = PngEncoding::new();
        let output = enc.encode_rgb8(img.as_ref()).unwrap();

        let dec = PngDecoding::new();
        let mut dst = imgref::ImgVec::new(
            vec![Rgb { r: 0.0f32, g: 0.0, b: 0.0 }; 4],
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
}
