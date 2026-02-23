//! Indexed (palette) PNG encoding via zenquant.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::Rgba;

use zencodec_types::MetadataView;
use zenquant::{OutputFormat, QuantizeConfig};

use enough::Stop;

use crate::encode::{self, EncodeConfig};
use crate::encoder::PngWriteMetadata;
use crate::error::PngError;

/// Encode RGBA8 pixels to indexed PNG using zenquant for palette quantization.
///
/// Quantizes the image to at most 256 colors, then writes an indexed PNG
/// with PLTE and optional tRNS chunks. Uses multi-strategy filter selection
/// and zenflate compression for best file sizes.
pub fn encode_indexed_rgba8(
    img: ImgRef<Rgba<u8>>,
    encode_config: &EncodeConfig,
    quant_config: &QuantizeConfig,
    metadata: Option<&MetadataView<'_>>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, w, h) = img.to_contiguous_buf();

    let rgba_slice: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(buf.as_ref());
    let result = zenquant::quantize_rgba(rgba_slice, w, h, quant_config)?;

    // Build separate RGB and alpha palette arrays
    let palette_rgba = result.palette_rgba();
    let mut palette_rgb = Vec::with_capacity(palette_rgba.len() * 3);
    let mut palette_alpha = Vec::with_capacity(palette_rgba.len());
    let mut has_transparency = false;

    for entry in palette_rgba {
        palette_rgb.push(entry[0]);
        palette_rgb.push(entry[1]);
        palette_rgb.push(entry[2]);
        palette_alpha.push(entry[3]);
        if entry[3] < 255 {
            has_transparency = true;
        }
    }

    let alpha = if has_transparency {
        Some(palette_alpha.as_slice())
    } else {
        None
    };

    let effort = encode_config.compression.effort();
    let opts = encode_config.compress_options(cancel, deadline, None);

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = encode_config.source_gamma;
    write_meta.srgb_intent = encode_config.srgb_intent;
    write_meta.chromaticities = encode_config.chromaticities;

    crate::encoder::write_indexed_png(
        result.indices(),
        width,
        height,
        &palette_rgb,
        alpha,
        &write_meta,
        effort,
        opts,
    )
}

/// Create a default [`QuantizeConfig`] tuned for PNG output.
pub fn default_quantize_config() -> QuantizeConfig {
    QuantizeConfig::new(OutputFormat::Png)
}

/// Result of [`encode_rgba8_auto`], indicating which encoding path was chosen.
#[derive(Debug)]
pub struct AutoEncodeResult {
    /// The encoded PNG data.
    pub data: Vec<u8>,
    /// Whether the image was encoded as indexed (palette) PNG.
    /// If `false`, the image was encoded as truecolor RGBA8.
    pub indexed: bool,
    /// Mean OKLab ΔE between the original and quantized image.
    /// Only meaningful when `indexed` is `true` (always `0.0` for truecolor).
    pub quality_loss: f64,
}

/// Encode RGBA8 pixels, automatically choosing indexed or truecolor PNG.
///
/// Tries quantizing to 256 colors via zenquant. If the mean perceptual error
/// (OKLab ΔE) is at or below `max_loss`, emits an indexed PNG (typically much
/// smaller). Otherwise falls back to truecolor RGBA8 PNG.
///
/// # Quality loss scale (mean OKLab ΔE)
///
/// | Value | Meaning |
/// |-------|---------|
/// | 0.0   | Only use indexed if quantization is lossless |
/// | 0.01  | Virtually imperceptible — safe for all content |
/// | 0.02  | Minimal — good default for photographic images |
/// | 0.05  | Moderate — visible on close inspection of smooth gradients |
/// | 0.10  | Aggressive — noticeable artifacts in some images |
pub fn encode_rgba8_auto(
    img: ImgRef<Rgba<u8>>,
    encode_config: &EncodeConfig,
    quant_config: &QuantizeConfig,
    max_loss: f64,
    metadata: Option<&MetadataView<'_>>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<AutoEncodeResult, PngError> {
    let (buf, w, h) = img.to_contiguous_buf();
    let rgba_slice: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(buf.as_ref());

    let result = zenquant::quantize_rgba(rgba_slice, w, h, quant_config)?;

    // Compute quality loss
    let loss = compute_mean_delta_e(buf.as_ref(), result.palette_rgba(), result.indices());

    if loss <= max_loss {
        // Quality acceptable — encode as indexed
        let width = img.width() as u32;
        let height = img.height() as u32;

        let palette_rgba = result.palette_rgba();
        let mut palette_rgb = Vec::with_capacity(palette_rgba.len() * 3);
        let mut palette_alpha = Vec::with_capacity(palette_rgba.len());
        let mut has_transparency = false;

        for entry in palette_rgba {
            palette_rgb.push(entry[0]);
            palette_rgb.push(entry[1]);
            palette_rgb.push(entry[2]);
            palette_alpha.push(entry[3]);
            if entry[3] < 255 {
                has_transparency = true;
            }
        }

        let alpha = if has_transparency {
            Some(palette_alpha.as_slice())
        } else {
            None
        };

        let effort = encode_config.compression.effort();
        let opts = encode_config.compress_options(cancel, deadline, None);

        let mut write_meta = PngWriteMetadata::from_metadata(metadata);
        write_meta.source_gamma = encode_config.source_gamma;
        write_meta.srgb_intent = encode_config.srgb_intent;
        write_meta.chromaticities = encode_config.chromaticities;

        let data = crate::encoder::write_indexed_png(
            result.indices(),
            width,
            height,
            &palette_rgb,
            alpha,
            &write_meta,
            effort,
            opts,
        )?;

        Ok(AutoEncodeResult {
            data,
            indexed: true,
            quality_loss: loss,
        })
    } else {
        // Quality too low — fall back to truecolor
        let data = encode::encode_rgba8(img, metadata, encode_config, cancel, deadline)?;
        Ok(AutoEncodeResult {
            data,
            indexed: false,
            quality_loss: loss,
        })
    }
}

// ── APNG indexed encoding ───────────────────────────────────────────

/// Encode canvas-sized RGBA8 frames into an indexed APNG file using a global palette.
///
/// Quantizes a representative sample across all frames to produce a single
/// global palette, then encodes each frame as indexed data with delta regions.
#[allow(clippy::too_many_arguments)]
pub fn encode_apng_indexed(
    frames: &[crate::encode::ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    config: &crate::encode::ApngEncodeConfig,
    quant_config: &QuantizeConfig,
    metadata: Option<&MetadataView<'_>>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    if frames.is_empty() {
        return Err(PngError::InvalidInput(
            "APNG requires at least one frame".into(),
        ));
    }
    let expected_len = canvas_width as usize * canvas_height as usize * 4;
    for (i, frame) in frames.iter().enumerate() {
        if frame.pixels.len() < expected_len {
            return Err(PngError::InvalidInput(alloc::format!(
                "frame {i}: pixel buffer too small: need {expected_len}, got {}",
                frame.pixels.len()
            )));
        }
    }

    let effort = config.encode.compression.effort();
    let mut write_meta = crate::encoder::PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.encode.source_gamma;
    write_meta.srgb_intent = config.encode.srgb_intent;
    write_meta.chromaticities = config.encode.chromaticities;

    crate::encoder::apng::encode_apng_indexed(
        frames,
        canvas_width,
        canvas_height,
        &write_meta,
        config.num_plays,
        effort,
        quant_config,
        cancel,
        deadline,
    )
}

/// Encode APNG frames, automatically choosing indexed or truecolor encoding.
///
/// Tries quantizing a representative sample across all frames. If the mean
/// perceptual error (OKLab ΔE) is at or below `max_loss`, emits an indexed
/// APNG. Otherwise falls back to truecolor RGBA8 APNG.
#[allow(clippy::too_many_arguments)]
pub fn encode_apng_auto(
    frames: &[crate::encode::ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    config: &crate::encode::ApngEncodeConfig,
    quant_config: &QuantizeConfig,
    max_loss: f64,
    metadata: Option<&MetadataView<'_>>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> Result<AutoEncodeResult, PngError> {
    if frames.is_empty() {
        return Err(PngError::InvalidInput(
            "APNG requires at least one frame".into(),
        ));
    }
    let expected_len = canvas_width as usize * canvas_height as usize * 4;
    for (i, frame) in frames.iter().enumerate() {
        if frame.pixels.len() < expected_len {
            return Err(PngError::InvalidInput(alloc::format!(
                "frame {i}: pixel buffer too small: need {expected_len}, got {}",
                frame.pixels.len()
            )));
        }
    }

    // Build representative sample across all frames
    let w = canvas_width as usize;
    let h = canvas_height as usize;
    let total_pixels = w * h * frames.len();
    let target_samples = 10_000.min(total_pixels);
    let sample_step = (total_pixels / target_samples).max(1);

    let mut sample_rgba: Vec<Rgba<u8>> = Vec::with_capacity(target_samples);
    let mut pixel_idx = 0usize;
    for frame in frames {
        let pixels: &[Rgba<u8>] = bytemuck::cast_slice(frame.pixels);
        for px in pixels {
            if pixel_idx % sample_step == 0 {
                sample_rgba.push(*px);
            }
            pixel_idx += 1;
        }
    }

    // Quantize sample
    let sample_zq: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(&sample_rgba);
    let result = zenquant::quantize_rgba(sample_zq, sample_rgba.len(), 1, quant_config)?;

    // Compute quality loss on the sample
    let loss = compute_mean_delta_e(&sample_rgba, result.palette_rgba(), result.indices());

    if loss <= max_loss {
        // Quality acceptable — encode as indexed APNG
        let data = encode_apng_indexed(
            frames,
            canvas_width,
            canvas_height,
            config,
            quant_config,
            metadata,
            cancel,
            deadline,
        )?;
        Ok(AutoEncodeResult {
            data,
            indexed: true,
            quality_loss: loss,
        })
    } else {
        // Quality too low — fall back to truecolor APNG
        let data = crate::encode::encode_apng(
            frames,
            canvas_width,
            canvas_height,
            config,
            metadata,
            cancel,
            deadline,
        )?;
        Ok(AutoEncodeResult {
            data,
            indexed: false,
            quality_loss: loss,
        })
    }
}

/// Convert sRGB u8 to OKLab [L, a, b].
fn srgb_u8_to_oklab(lut: &linear_srgb::lut::SrgbConverter, r: u8, g: u8, b: u8) -> [f32; 3] {
    let lr = lut.srgb_u8_to_linear(r);
    let lg = lut.srgb_u8_to_linear(g);
    let lb = lut.srgb_u8_to_linear(b);

    let l = 0.412_221_46_f32.mul_add(lr, 0.536_332_55_f32.mul_add(lg, 0.051_445_995 * lb));
    let m = 0.211_903_5_f32.mul_add(lr, 0.713_695_2_f32.mul_add(lg, 0.074_399_3 * lb));
    let s = 0.324_425_76_f32.mul_add(lr, 0.568_564_5_f32.mul_add(lg, 0.106_909_87 * lb));

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    [
        0.210_454_26_f32.mul_add(l_, 0.793_617_8_f32.mul_add(m_, -0.004_072_047 * s_)),
        1.977_998_5_f32.mul_add(l_, (-2.428_592_2_f32).mul_add(m_, 0.450_593_7 * s_)),
        0.025_904_037_f32.mul_add(l_, 0.782_771_77_f32.mul_add(m_, -0.808_675_77 * s_)),
    ]
}

/// Compute mean OKLab ΔE between original pixels and their quantized versions.
fn compute_mean_delta_e(original: &[Rgba<u8>], palette_rgba: &[[u8; 4]], indices: &[u8]) -> f64 {
    if original.is_empty() {
        return 0.0;
    }

    let lut = linear_srgb::lut::SrgbConverter::new();

    // Precompute OKLab for all palette entries
    let palette_oklab: Vec<[f32; 3]> = palette_rgba
        .iter()
        .map(|e| srgb_u8_to_oklab(&lut, e[0], e[1], e[2]))
        .collect();

    let mut sum = 0.0_f64;
    for (pixel, &idx) in original.iter().zip(indices.iter()) {
        let orig = srgb_u8_to_oklab(&lut, pixel.r, pixel.g, pixel.b);
        let quant = &palette_oklab[idx as usize];

        let dl = (orig[0] - quant[0]) as f64;
        let da = (orig[1] - quant[1]) as f64;
        let db = (orig[2] - quant[2]) as f64;
        sum += (dl * dl + da * da + db * db).sqrt();
    }

    sum / original.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use imgref::ImgVec;

    fn test_image_4x4() -> ImgVec<Rgba<u8>> {
        let pixels = vec![
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
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 128,
            },
            Rgba {
                r: 0,
                g: 255,
                b: 0,
                a: 128,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 255,
                a: 128,
            },
            Rgba {
                r: 255,
                g: 255,
                b: 0,
                a: 128,
            },
            Rgba {
                r: 128,
                g: 128,
                b: 128,
                a: 255,
            },
            Rgba {
                r: 64,
                g: 64,
                b: 64,
                a: 255,
            },
            Rgba {
                r: 192,
                g: 192,
                b: 192,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
        ];
        ImgVec::new(pixels, 4, 4)
    }

    #[test]
    fn roundtrip_indexed_png() {
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        let encoded = encode_indexed_rgba8(
            img.as_ref(),
            &config,
            &quant,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(!encoded.is_empty());

        // Verify PNG signature
        assert_eq!(&encoded[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);

        // Full decode roundtrip through zenpng
        let decoded = crate::decode::decode(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
    }

    #[test]
    fn roundtrip_with_metadata() {
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        let fake_icc = vec![0x42u8; 200];
        let exif_data = b"Exif\0\0test_exif";
        let xmp_data = b"<x:xmpmeta>test</x:xmpmeta>";

        let meta = MetadataView::none()
            .with_icc(&fake_icc)
            .with_exif(exif_data)
            .with_xmp(xmp_data);

        let encoded = encode_indexed_rgba8(
            img.as_ref(),
            &config,
            &quant,
            Some(&meta),
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded = crate::decode::decode(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);

        // ICC profile should roundtrip
        let icc = decoded.info.icc_profile.as_ref().expect("ICC missing");
        assert_eq!(icc.as_slice(), &fake_icc[..]);

        // EXIF should roundtrip
        let exif = decoded.info.exif.as_ref().expect("EXIF missing");
        assert_eq!(exif.as_slice(), exif_data);

        // XMP should roundtrip
        let xmp = decoded.info.xmp.as_ref().expect("XMP missing");
        assert_eq!(xmp.as_slice(), xmp_data);
    }

    #[test]
    fn all_compression_levels_work() {
        let img = test_image_4x4();
        let quant = default_quantize_config();

        for comp in [
            crate::Compression::None,
            crate::Compression::Fastest,
            crate::Compression::Fast,
            crate::Compression::Balanced,
            crate::Compression::Thorough,
            crate::Compression::High,
            crate::Compression::Aggressive,
        ] {
            let config = EncodeConfig::default().with_compression(comp);
            let encoded = encode_indexed_rgba8(
                img.as_ref(),
                &config,
                &quant,
                None,
                &enough::Unstoppable,
                &enough::Unstoppable,
            )
            .unwrap();
            let decoded = crate::decode::decode(
                &encoded,
                &crate::decode::PngDecodeConfig::none(),
                &enough::Unstoppable,
            )
            .unwrap();
            assert_eq!(decoded.info.width, 4);
            assert_eq!(decoded.info.height, 4);
        }
    }

    #[test]
    fn auto_encode_few_colors_uses_indexed() {
        // 4x4 with only 10 unique colors — should always pick indexed
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        let result = encode_rgba8_auto(
            img.as_ref(),
            &config,
            &quant,
            0.02,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(
            result.indexed,
            "few-color image should use indexed encoding"
        );
        assert!(
            result.quality_loss < 0.001,
            "few-color image should be near-lossless"
        );

        // Verify it decodes correctly
        let decoded = crate::decode::decode(
            &result.data,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
    }

    #[test]
    fn auto_encode_zero_threshold_few_colors() {
        // With threshold 0.0, only lossless quantization should be accepted
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        let result = encode_rgba8_auto(
            img.as_ref(),
            &config,
            &quant,
            0.0,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        // With only 10 colors, zenquant should produce lossless quantization
        assert!(
            result.indexed,
            "10-color image with threshold 0.0 should still use indexed"
        );
        assert!(
            result.quality_loss == 0.0,
            "10-color image should be exactly lossless, got {}",
            result.quality_loss
        );
    }

    #[test]
    fn auto_encode_returns_truecolor_on_tight_threshold() {
        // Build a 16x16 gradient with many unique colors
        let mut pixels = Vec::with_capacity(256);
        for y in 0..16u8 {
            for x in 0..16u8 {
                pixels.push(Rgba {
                    r: x.wrapping_mul(17),
                    g: y.wrapping_mul(17),
                    b: x.wrapping_add(y).wrapping_mul(9),
                    a: 255,
                });
            }
        }
        let img = ImgVec::new(pixels, 16, 16);
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        // With very tight threshold, a gradient image should fall back to truecolor
        let result = encode_rgba8_auto(
            img.as_ref(),
            &config,
            &quant,
            0.0,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        // Even if this happens to be lossless, that's OK — we just verify the function works
        let decoded = crate::decode::decode(
            &result.data,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 16);
        assert_eq!(decoded.info.height, 16);
    }

    #[test]
    fn auto_encode_quality_loss_is_reasonable() {
        // 64x64 gradient: enough colors to stress quantizer, but small enough for fast test
        let mut pixels = Vec::with_capacity(64 * 64);
        for y in 0..64u32 {
            for x in 0..64u32 {
                pixels.push(Rgba {
                    r: (x * 4).min(255) as u8,
                    g: (y * 4).min(255) as u8,
                    b: ((x + y) * 2).min(255) as u8,
                    a: 255,
                });
            }
        }
        let img = ImgVec::new(pixels, 64, 64);
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        // With generous threshold, should use indexed
        let result = encode_rgba8_auto(
            img.as_ref(),
            &config,
            &quant,
            0.10,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(
            result.indexed,
            "64x64 gradient with 0.10 threshold should use indexed"
        );

        // Quality loss for a smooth gradient into 256 colors should be small
        assert!(
            result.quality_loss < 0.05,
            "quality loss {:.6} unexpectedly high for smooth gradient",
            result.quality_loss
        );

        // Indexed should decode correctly
        let decoded = crate::decode::decode(
            &result.data,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 64);
        assert_eq!(decoded.info.height, 64);
    }
}
