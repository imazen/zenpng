//! Indexed (palette) PNG encoding with pluggable quantizer backends.
//!
//! All public functions accept `&dyn Quantizer` for runtime backend selection.
//! Use [`default_quantizer()`](crate::default_quantizer) to get the best available backend.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::Rgba;
use std::collections::HashMap;

use zencodec::Metadata;

use enough::Stop;

use crate::encode::{self, EncodeConfig};
use crate::encoder::PngWriteMetadata;
use whereat::at;

use crate::error::PngError;
use crate::quantize::{QuantizeOutput, Quantizer};

/// Quality gate for auto-encode: indexed (smaller) vs truecolor (lossless).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum QualityGate {
    /// Mean OKLab ΔE threshold (lower = stricter).
    /// 0.0 = lossless only, 0.02 = good, 0.05 = moderate.
    MaxDeltaE(f64),
    /// Maximum MPE score (lower = stricter).
    /// 0.008 ≈ JPEG q95, 0.02 ≈ JPEG q75, 0.028 ≈ JPEG q50.
    MaxMpe(f32),
    /// Minimum SSIMULACRA2 score (higher = stricter, 0-100).
    /// 85+ ≈ near-lossless, 75+ ≈ good, 65+ ≈ moderate.
    MinSsim2(f32),
}

impl QualityGate {
    /// Whether this gate requires zenquant's `with_compute_quality_metric`.
    #[must_use]
    pub fn needs_metric(&self) -> bool {
        matches!(self, QualityGate::MaxMpe(_) | QualityGate::MinSsim2(_))
    }

    /// Check whether a [`QuantizeOutput`] passes this quality gate.
    ///
    /// For `MaxDeltaE`, uses the externally-computed `delta_e`.
    /// For `MaxMpe`/`MinSsim2`, reads optional metrics from the output.
    /// If the required metric is `None` (backend doesn't support it), the gate fails
    /// conservatively (falls back to truecolor).
    #[must_use]
    pub fn check(&self, output: &QuantizeOutput, delta_e: f64) -> bool {
        match *self {
            QualityGate::MaxDeltaE(max) => delta_e <= max,
            QualityGate::MaxMpe(max) => output.mpe_score.is_some_and(|mpe| mpe <= max),
            QualityGate::MinSsim2(min) => output.ssim2_estimate.is_some_and(|ss2| ss2 >= min),
        }
    }
}

/// Palette split into RGB and alpha arrays for PNG PLTE/tRNS chunks.
struct SplitPalette {
    rgb: Vec<u8>,
    alpha: Vec<u8>,
    has_transparency: bool,
}

/// Split a `&[[u8; 4]]` RGBA palette into separate RGB and alpha arrays.
fn split_palette(palette_rgba: &[[u8; 4]]) -> SplitPalette {
    let mut rgb = Vec::with_capacity(palette_rgba.len() * 3);
    let mut alpha = Vec::with_capacity(palette_rgba.len());
    let mut has_transparency = false;

    for entry in palette_rgba {
        rgb.push(entry[0]);
        rgb.push(entry[1]);
        rgb.push(entry[2]);
        alpha.push(entry[3]);
        if entry[3] < 255 {
            has_transparency = true;
        }
    }

    SplitPalette {
        rgb,
        alpha,
        has_transparency,
    }
}

/// Result of exact-palette detection: the palette and per-frame index buffers.
struct ExactPalette {
    palette_rgba: Vec<[u8; 4]>,
    frame_indices: Vec<Vec<u8>>,
}

/// Scan all frame pixel buffers for unique RGBA colors. If there are ≤256 unique
/// colors across all frames, build an exact palette and per-frame index buffers.
/// Returns `None` if more than 256 unique colors are found (early exit).
///
/// Each entry in `frame_pixels` is a flat `&[u8]` of RGBA8 pixels (4 bytes each).
fn try_build_exact_palette(
    frame_pixels: &[&[u8]],
    pixels_per_frame: usize,
) -> Option<ExactPalette> {
    let mut color_to_index: HashMap<[u8; 4], u8> = HashMap::with_capacity(257);
    let mut palette_rgba: Vec<[u8; 4]> = Vec::with_capacity(256);

    // First pass: collect unique colors across all frames
    for frame in frame_pixels {
        let rgba: &[[u8; 4]] = bytemuck::cast_slice(&frame[..pixels_per_frame * 4]);
        for &color in rgba {
            if let std::collections::hash_map::Entry::Vacant(e) = color_to_index.entry(color) {
                if palette_rgba.len() >= 256 {
                    return None; // >256 unique colors
                }
                let idx = palette_rgba.len() as u8;
                e.insert(idx);
                palette_rgba.push(color);
            }
        }
    }

    // Second pass: build index buffers
    let mut frame_indices = Vec::with_capacity(frame_pixels.len());
    for frame in frame_pixels {
        let rgba: &[[u8; 4]] = bytemuck::cast_slice(&frame[..pixels_per_frame * 4]);
        let indices: Vec<u8> = rgba.iter().map(|color| color_to_index[color]).collect();
        frame_indices.push(indices);
    }

    Some(ExactPalette {
        palette_rgba,
        frame_indices,
    })
}

/// Result of [`encode_auto`], indicating which encoding path was chosen.
#[derive(Debug)]
#[non_exhaustive]
pub struct AutoEncodeResult {
    /// The encoded PNG data.
    pub data: Vec<u8>,
    /// Whether the image was encoded as indexed (palette) PNG.
    /// If `false`, the image was encoded as truecolor RGBA8.
    pub indexed: bool,
    /// Mean OKLab ΔE between the original and quantized image.
    /// Only meaningful when `indexed` is `true` (always `0.0` for truecolor).
    pub quality_loss: f64,
    /// MPE quality score (lower = better). `None` unless `QualityGate::MaxMpe` used.
    pub mpe_score: Option<f32>,
    /// Estimated SSIMULACRA2 score (0-100, higher = better).
    /// `None` unless `QualityGate::MaxMpe` or `QualityGate::MinSsim2` used.
    pub ssim2_estimate: Option<f32>,
    /// Estimated butteraugli distance (lower = better).
    /// `None` unless `QualityGate::MaxMpe` or `QualityGate::MinSsim2` used.
    pub butteraugli_estimate: Option<f32>,
}

/// Encode RGBA8 pixels to indexed PNG using any [`Quantizer`] backend.
///
/// Quantizes the image to at most 256 colors, then writes an indexed PNG
/// with PLTE and optional tRNS chunks.
pub fn encode_indexed(
    img: ImgRef<Rgba<u8>>,
    encode_config: &EncodeConfig,
    quantizer: &dyn Quantizer,
    metadata: Option<&Metadata>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, w, h) = img.to_contiguous_buf();
    let rgba: &[[u8; 4]] = bytemuck::cast_slice(buf.as_ref());

    let result = quantizer.quantize_rgba(rgba, w, h)?;
    encode_from_quantize_output(
        &result,
        width,
        height,
        encode_config,
        metadata,
        cancel,
        deadline,
    )
}

/// Encode RGBA8 pixels, automatically choosing indexed or truecolor PNG.
///
/// Tries quantizing to ≤256 colors. If the quality gate passes, emits an
/// indexed PNG (typically much smaller). Otherwise falls back to truecolor RGBA8.
///
/// **Quality gates with metrics** (`MaxMpe`, `MinSsim2`) require a backend that
/// provides those metrics (currently only zenquant with `compute_quality_metric`
/// enabled). With other backends, these gates fail conservatively → truecolor fallback.
/// Use `MaxDeltaE` for backend-agnostic quality gating.
///
/// # Quality gates
///
/// | Gate | Scale | Good default | Meaning |
/// |------|-------|-------------|---------|
/// | `MaxDeltaE(0.02)` | 0.0 – ∞ | 0.02 | Mean OKLab ΔE (lower = stricter) |
/// | `MaxMpe(0.008)` | 0.0 – ∞ | 0.008 | Masked perceptual error (lower = stricter) |
/// | `MinSsim2(85.0)` | 0 – 100 | 85.0 | SSIMULACRA2 estimate (higher = stricter) |
pub fn encode_auto(
    img: ImgRef<Rgba<u8>>,
    encode_config: &EncodeConfig,
    quantizer: &dyn Quantizer,
    gate: QualityGate,
    metadata: Option<&Metadata>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<AutoEncodeResult> {
    let (buf, w, _h) = img.to_contiguous_buf();
    let width = img.width() as u32;
    let height = img.height() as u32;
    let pixel_bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());

    // Fast path: if ≤256 unique colors, use exact palette (zero quality loss)
    if let Some(exact) = try_build_exact_palette(&[pixel_bytes], w * _h) {
        return encode_exact_palette_result(
            &exact,
            0,
            width,
            height,
            encode_config,
            metadata,
            cancel,
            deadline,
        );
    }

    // Enable quality metrics if the gate needs them
    let adjusted;
    let quantizer: &dyn Quantizer = if gate.needs_metric() {
        match quantizer.with_quality_metrics() {
            Some(q) => {
                adjusted = q;
                &*adjusted
            }
            None => quantizer,
        }
    } else {
        quantizer
    };

    // Quantization path
    let rgba: &[[u8; 4]] = bytemuck::cast_slice(buf.as_ref());
    let result = quantizer.quantize_rgba(rgba, w, _h)?;

    let original: &[Rgba<u8>] = bytemuck::cast_slice(buf.as_ref());
    let loss = compute_mean_delta_e(original, &result.palette_rgba, &result.indices);

    if gate.check(&result, loss) {
        let data = encode_from_quantize_output(
            &result,
            width,
            height,
            encode_config,
            metadata,
            cancel,
            deadline,
        )?;
        Ok(AutoEncodeResult {
            data,
            indexed: true,
            quality_loss: loss,
            mpe_score: result.mpe_score,
            ssim2_estimate: result.ssim2_estimate,
            butteraugli_estimate: result.butteraugli_estimate,
        })
    } else {
        let data = encode::encode_rgba8(img, metadata, encode_config, cancel, deadline)?;
        Ok(AutoEncodeResult {
            data,
            indexed: false,
            quality_loss: loss,
            mpe_score: result.mpe_score,
            ssim2_estimate: result.ssim2_estimate,
            butteraugli_estimate: result.butteraugli_estimate,
        })
    }
}

/// Internal: encode from a QuantizeOutput.
fn encode_from_quantize_output(
    result: &QuantizeOutput,
    width: u32,
    height: u32,
    encode_config: &EncodeConfig,
    metadata: Option<&Metadata>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let sp = split_palette(&result.palette_rgba);
    let alpha = if sp.has_transparency {
        Some(sp.alpha.as_slice())
    } else {
        None
    };

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = encode_config.source_gamma;
    write_meta.srgb_intent = encode_config.srgb_intent;
    write_meta.chromaticities = encode_config.chromaticities;

    let effort = encode_config.compression.effort();
    let opts = encode_config.compress_options(cancel, deadline, None);

    Ok(crate::encoder::write_indexed_png(
        &result.indices,
        width,
        height,
        &sp.rgb,
        alpha,
        &write_meta,
        effort,
        opts,
    )?)
}

/// Internal: encode result from an exact palette (≤256 unique colors, zero loss).
#[allow(clippy::too_many_arguments)]
fn encode_exact_palette_result(
    exact: &ExactPalette,
    frame_idx: usize,
    width: u32,
    height: u32,
    encode_config: &EncodeConfig,
    metadata: Option<&Metadata>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<AutoEncodeResult> {
    let sp = split_palette(&exact.palette_rgba);
    let alpha = if sp.has_transparency {
        Some(sp.alpha.as_slice())
    } else {
        None
    };

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = encode_config.source_gamma;
    write_meta.srgb_intent = encode_config.srgb_intent;
    write_meta.chromaticities = encode_config.chromaticities;

    let effort = encode_config.compression.effort();
    let opts = encode_config.compress_options(cancel, deadline, None);

    let data = crate::encoder::write_indexed_png(
        &exact.frame_indices[frame_idx],
        width,
        height,
        &sp.rgb,
        alpha,
        &write_meta,
        effort,
        opts,
    )?;

    Ok(AutoEncodeResult {
        data,
        indexed: true,
        quality_loss: 0.0,
        mpe_score: Some(0.0),
        ssim2_estimate: Some(100.0),
        butteraugli_estimate: Some(0.0),
    })
}

// ── APNG API ───────────────────────────────────────────────────────

/// Bundled parameters for APNG encoding with any [`Quantizer`] backend.
pub struct ApngEncodeParams<'a> {
    /// Canvas-sized RGBA8 frames to encode.
    pub frames: &'a [crate::encode::ApngFrameInput<'a>],
    /// Canvas width in pixels.
    pub canvas_width: u32,
    /// Canvas height in pixels.
    pub canvas_height: u32,
    /// APNG encoding configuration (compression, num_plays, etc.).
    pub config: &'a crate::encode::ApngEncodeConfig,
    /// Quantizer backend.
    pub quantizer: &'a dyn Quantizer,
    /// Optional PNG metadata (gAMA, sRGB, cHRM, iCCP, etc.).
    pub metadata: Option<&'a Metadata>,
    /// Cancellation token.
    pub cancel: &'a dyn Stop,
    /// Deadline/timeout token.
    pub deadline: &'a dyn Stop,
}

/// Encode canvas-sized RGBA8 frames into an indexed APNG with any [`Quantizer`].
///
/// Builds a shared palette via [`Quantizer::quantize_multi_frame`]. Zenquant
/// provides temporal consistency (identical pixels across frames get the same
/// index); other backends use concatenated quantization.
pub fn encode_apng_indexed(params: &ApngEncodeParams<'_>) -> crate::error::Result<Vec<u8>> {
    let frames = params.frames;
    let w = params.canvas_width as usize;
    let h = params.canvas_height as usize;
    let config = params.config;
    let cancel = params.cancel;
    let deadline = params.deadline;

    validate_apng_frames(frames, w, h)?;

    let expected_len = w * h * 4;
    let pixels_per_frame = w * h;

    // Fast path: ≤256 unique colors across all frames
    let frame_pixel_slices: Vec<&[u8]> = frames.iter().map(|f| &f.pixels[..expected_len]).collect();
    if let Some(exact) = try_build_exact_palette(&frame_pixel_slices, pixels_per_frame) {
        return encode_apng_from_palette(
            frames,
            &exact.palette_rgba,
            &exact.frame_indices,
            params.canvas_width,
            params.canvas_height,
            config,
            params.metadata,
            cancel,
            deadline,
        );
    }

    // Multi-frame quantization via the trait
    let frame_rgba: Vec<&[[u8; 4]]> = frames
        .iter()
        .map(|f| {
            let pixels: &[[u8; 4]] = bytemuck::cast_slice(&f.pixels[..expected_len]);
            pixels
        })
        .collect();

    let mf = params.quantizer.quantize_multi_frame(&frame_rgba, w, h)?;

    encode_apng_from_palette(
        frames,
        &mf.palette_rgba,
        &mf.frame_indices,
        params.canvas_width,
        params.canvas_height,
        config,
        params.metadata,
        cancel,
        deadline,
    )
}

/// Encode APNG frames, auto-choosing indexed or truecolor, with any [`Quantizer`].
///
/// Quantizes via [`Quantizer::quantize_multi_frame`], checks quality gate
/// per frame. Falls back to truecolor if any frame fails the gate.
///
/// **Note:** `MaxMpe`/`MinSsim2` gates require per-frame metrics, which only
/// zenquant provides. With other backends these gates fail → truecolor fallback.
/// Use `MaxDeltaE` for backend-agnostic gating.
pub fn encode_apng_auto(
    params: &ApngEncodeParams<'_>,
    gate: QualityGate,
) -> crate::error::Result<AutoEncodeResult> {
    let frames = params.frames;
    let w = params.canvas_width as usize;
    let h = params.canvas_height as usize;
    let config = params.config;
    let cancel = params.cancel;
    let deadline = params.deadline;

    validate_apng_frames(frames, w, h)?;

    let expected_len = w * h * 4;
    let pixels_per_frame = w * h;

    // Fast path: ≤256 unique colors (zero loss)
    let frame_pixel_slices: Vec<&[u8]> = frames.iter().map(|f| &f.pixels[..expected_len]).collect();
    if let Some(exact) = try_build_exact_palette(&frame_pixel_slices, pixels_per_frame) {
        let data = encode_apng_from_palette(
            frames,
            &exact.palette_rgba,
            &exact.frame_indices,
            params.canvas_width,
            params.canvas_height,
            config,
            params.metadata,
            cancel,
            deadline,
        )?;
        return Ok(AutoEncodeResult {
            data,
            indexed: true,
            quality_loss: 0.0,
            mpe_score: Some(0.0),
            ssim2_estimate: Some(100.0),
            butteraugli_estimate: Some(0.0),
        });
    }

    // Enable quality metrics if the gate needs them
    let adjusted;
    let quantizer: &dyn Quantizer = if gate.needs_metric() {
        match params.quantizer.with_quality_metrics() {
            Some(q) => {
                adjusted = q;
                &*adjusted
            }
            None => params.quantizer,
        }
    } else {
        params.quantizer
    };

    // Multi-frame quantization
    let frame_rgba: Vec<&[[u8; 4]]> = frames
        .iter()
        .map(|f| {
            let pixels: &[[u8; 4]] = bytemuck::cast_slice(&f.pixels[..expected_len]);
            pixels
        })
        .collect();

    let mf = quantizer.quantize_multi_frame(&frame_rgba, w, h)?;

    // Per-frame quality gate check
    let mut worst_loss = 0.0_f64;
    let mut worst_mpe: Option<f32> = None;
    let mut worst_ssim2: Option<f32> = None;
    let mut worst_ba: Option<f32> = None;

    for (i, indices) in mf.frame_indices.iter().enumerate() {
        cancel.check().map_err(PngError::from)?;

        let frame_pixels: &[Rgba<u8>] = bytemuck::cast_slice(&frames[i].pixels[..expected_len]);
        let frame_loss = compute_mean_delta_e(frame_pixels, &mf.palette_rgba, indices);

        let frame_output = QuantizeOutput {
            palette_rgba: mf.palette_rgba.clone(),
            indices: indices.clone(),
            mpe_score: mf.mpe_scores[i],
            ssim2_estimate: mf.ssim2_estimates[i],
            butteraugli_estimate: mf.butteraugli_estimates[i],
        };

        if !gate.check(&frame_output, frame_loss) {
            // Frame failed — bail to truecolor
            let data = crate::encode::encode_apng(
                frames,
                params.canvas_width,
                params.canvas_height,
                config,
                params.metadata,
                cancel,
                deadline,
            )?;
            return Ok(AutoEncodeResult {
                data,
                indexed: false,
                quality_loss: frame_loss,
                mpe_score: frame_output.mpe_score,
                ssim2_estimate: frame_output.ssim2_estimate,
                butteraugli_estimate: frame_output.butteraugli_estimate,
            });
        }

        worst_loss = worst_loss.max(frame_loss);
        if let Some(mpe) = mf.mpe_scores[i] {
            worst_mpe = Some(worst_mpe.map_or(mpe, |prev: f32| prev.max(mpe)));
        }
        if let Some(ss2) = mf.ssim2_estimates[i] {
            worst_ssim2 = Some(worst_ssim2.map_or(ss2, |prev: f32| prev.min(ss2)));
        }
        if let Some(ba) = mf.butteraugli_estimates[i] {
            worst_ba = Some(worst_ba.map_or(ba, |prev: f32| prev.max(ba)));
        }
    }

    // All frames passed
    let data = encode_apng_from_palette(
        frames,
        &mf.palette_rgba,
        &mf.frame_indices,
        params.canvas_width,
        params.canvas_height,
        config,
        params.metadata,
        cancel,
        deadline,
    )?;

    Ok(AutoEncodeResult {
        data,
        indexed: true,
        quality_loss: worst_loss,
        mpe_score: worst_mpe,
        ssim2_estimate: worst_ssim2,
        butteraugli_estimate: worst_ba,
    })
}

/// Validate APNG frame dimensions and buffer sizes.
fn validate_apng_frames(
    frames: &[crate::encode::ApngFrameInput<'_>],
    w: usize,
    h: usize,
) -> crate::error::Result<()> {
    if frames.is_empty() {
        return Err(at!(PngError::InvalidInput(
            "APNG requires at least one frame".into(),
        )));
    }
    let expected_len = w * h * 4;
    for (i, frame) in frames.iter().enumerate() {
        if frame.pixels.len() < expected_len {
            return Err(at!(PngError::InvalidInput(alloc::format!(
                "frame {i}: pixel buffer too small: need {expected_len}, got {}",
                frame.pixels.len()
            ))));
        }
    }
    Ok(())
}

/// Shared APNG encoding from palette + per-frame indices.
#[allow(clippy::too_many_arguments)]
fn encode_apng_from_palette(
    frames: &[crate::encode::ApngFrameInput<'_>],
    palette_rgba: &[[u8; 4]],
    all_indices: &[Vec<u8>],
    canvas_width: u32,
    canvas_height: u32,
    config: &crate::encode::ApngEncodeConfig,
    metadata: Option<&Metadata>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let effort = config.encode.compression.effort();
    let mut write_meta = crate::encoder::PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.encode.source_gamma;
    write_meta.srgb_intent = config.encode.srgb_intent;
    write_meta.chromaticities = config.encode.chromaticities;

    Ok(crate::encoder::apng::encode_apng_indexed_from_indices(
        frames,
        palette_rgba,
        all_indices,
        canvas_width,
        canvas_height,
        &write_meta,
        config.num_plays,
        effort,
        cancel,
        deadline,
    )?)
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
/// Mean OKLab ΔE between original and quantized pixels, alpha-aware via dual-background
/// compositing: each pixel is composited against black and white, and the max ΔE of the
/// two is used. This makes alpha mismatches visible as color differences.
fn compute_mean_delta_e(original: &[Rgba<u8>], palette_rgba: &[[u8; 4]], indices: &[u8]) -> f64 {
    if original.is_empty() {
        return 0.0;
    }

    let lut = linear_srgb::lut::SrgbConverter::new();

    // Precompute OKLab for all palette entries composited against black and white
    let palette_on_black: Vec<[f32; 3]> = palette_rgba
        .iter()
        .map(|e| {
            let (r, g, b) = composite_over_black(e[0], e[1], e[2], e[3]);
            srgb_u8_to_oklab(&lut, r, g, b)
        })
        .collect();
    let palette_on_white: Vec<[f32; 3]> = palette_rgba
        .iter()
        .map(|e| {
            let (r, g, b) = composite_over_white(e[0], e[1], e[2], e[3]);
            srgb_u8_to_oklab(&lut, r, g, b)
        })
        .collect();

    let mut sum = 0.0_f64;
    let mut count = 0usize;
    for (pixel, &idx) in original.iter().zip(indices.iter()) {
        let idx = idx as usize;

        let (ob_r, ob_g, ob_b) = composite_over_black(pixel.r, pixel.g, pixel.b, pixel.a);
        let orig_black = srgb_u8_to_oklab(&lut, ob_r, ob_g, ob_b);
        let quant_black = &palette_on_black[idx];
        let de_black = oklab_delta_e(&orig_black, quant_black);

        let (ow_r, ow_g, ow_b) = composite_over_white(pixel.r, pixel.g, pixel.b, pixel.a);
        let orig_white = srgb_u8_to_oklab(&lut, ow_r, ow_g, ow_b);
        let quant_white = &palette_on_white[idx];
        let de_white = oklab_delta_e(&orig_white, quant_white);

        sum += if de_black > de_white {
            de_black
        } else {
            de_white
        };
        count += 1;
    }

    if count == 0 {
        return 0.0;
    }
    sum / count as f64
}

/// Composite RGBA over black background → resulting sRGB u8.
#[inline]
fn composite_over_black(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    let af = a as u16;
    (
        ((r as u16 * af + 127) / 255) as u8,
        ((g as u16 * af + 127) / 255) as u8,
        ((b as u16 * af + 127) / 255) as u8,
    )
}

/// Composite RGBA over white background → resulting sRGB u8.
#[inline]
fn composite_over_white(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    let af = a as u16;
    let inv = 255 - af;
    (
        ((r as u16 * af + 255 * inv + 127) / 255) as u8,
        ((g as u16 * af + 255 * inv + 127) / 255) as u8,
        ((b as u16 * af + 255 * inv + 127) / 255) as u8,
    )
}

/// OKLab Euclidean distance (ΔE).
#[inline]
fn oklab_delta_e(a: &[f32; 3], b: &[f32; 3]) -> f64 {
    let dl = (a[0] - b[0]) as f64;
    let da = (a[1] - b[1]) as f64;
    let db = (a[2] - b[2]) as f64;
    (dl * dl + da * da + db * db).sqrt()
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

    fn default_quantizer() -> Box<dyn Quantizer> {
        crate::quantize::default_quantizer()
    }

    #[test]
    fn roundtrip_indexed_png() {
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quantizer = default_quantizer();

        let encoded = encode_indexed(
            img.as_ref(),
            &config,
            &*quantizer,
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
        let quantizer = default_quantizer();

        let fake_icc = vec![0x42u8; 200];
        let exif_data = b"Exif\0\0test_exif";
        let xmp_data = b"<x:xmpmeta>test</x:xmpmeta>";

        let meta = Metadata::none()
            .with_icc(fake_icc.as_slice())
            .with_exif(exif_data.as_slice())
            .with_xmp(xmp_data.as_slice());

        let encoded = encode_indexed(
            img.as_ref(),
            &config,
            &*quantizer,
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
        let quantizer = default_quantizer();

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
            let encoded = encode_indexed(
                img.as_ref(),
                &config,
                &*quantizer,
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
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quantizer = default_quantizer();

        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxDeltaE(0.02),
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
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quantizer = default_quantizer();

        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxDeltaE(0.0),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
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
        let quantizer = default_quantizer();

        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxDeltaE(0.0),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
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
        let quantizer = default_quantizer();

        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxDeltaE(0.10),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(
            result.indexed,
            "64x64 gradient with 0.10 threshold should use indexed"
        );

        assert!(
            result.quality_loss < 0.05,
            "quality loss {:.6} unexpectedly high for smooth gradient",
            result.quality_loss
        );

        let decoded = crate::decode::decode(
            &result.data,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 64);
        assert_eq!(decoded.info.height, 64);
    }

    #[cfg(feature = "joint")]
    #[test]
    fn roundtrip_joint_indexed_png() {
        use crate::quantize::ZenquantQuantizer;

        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quantizer = ZenquantQuantizer::with_format(zenquant::OutputFormat::PngJoint);

        let encoded = encode_indexed(
            img.as_ref(),
            &config,
            &quantizer,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(!encoded.is_empty());

        assert_eq!(&encoded[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);

        let decoded = crate::decode::decode(
            &encoded,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
    }

    #[cfg(feature = "joint")]
    #[test]
    fn joint_produces_smaller_or_equal_output() {
        use crate::quantize::ZenquantQuantizer;

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

        let q_standard = ZenquantQuantizer::new();
        let standard = encode_indexed(
            img.as_ref(),
            &config,
            &q_standard,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let q_joint = ZenquantQuantizer::with_format(zenquant::OutputFormat::PngJoint);
        let joint = encode_indexed(
            img.as_ref(),
            &config,
            &q_joint,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        let dec_standard = crate::decode::decode(
            &standard,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        let dec_joint = crate::decode::decode(
            &joint,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(dec_standard.info.width, 64);
        assert_eq!(dec_joint.info.width, 64);

        let ratio = joint.len() as f64 / standard.len() as f64;
        assert!(
            ratio < 1.05,
            "joint output ({}) much larger than standard ({}) — ratio {:.3}",
            joint.len(),
            standard.len(),
            ratio,
        );
    }

    #[cfg(feature = "joint")]
    #[test]
    fn joint_auto_encode_roundtrip() {
        use crate::quantize::ZenquantQuantizer;

        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quantizer = ZenquantQuantizer::with_format(zenquant::OutputFormat::PngJoint);

        let result = encode_auto(
            img.as_ref(),
            &config,
            &quantizer,
            QualityGate::MaxDeltaE(0.02),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(
            result.indexed,
            "few-color image should use indexed encoding"
        );

        let decoded = crate::decode::decode(
            &result.data,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
    }

    #[cfg(feature = "joint")]
    #[test]
    fn joint_compression_comparison() {
        use crate::quantize::ZenquantQuantizer;
        use zenpixels_convert::PixelBufferConvertExt;

        fn compare(name: &str, img: ImgRef<'_, Rgba<u8>>, tolerance: f32) {
            let config = EncodeConfig::default();

            let q_std = ZenquantQuantizer::new();
            let standard = encode_indexed(
                img,
                &config,
                &q_std,
                None,
                &enough::Unstoppable,
                &enough::Unstoppable,
            )
            .unwrap();

            let q_joint = ZenquantQuantizer::from_config(
                zenquant::QuantizeConfig::new(zenquant::OutputFormat::PngJoint)
                    ._with_joint_tolerance(tolerance),
            );
            let joint = encode_indexed(
                img,
                &config,
                &q_joint,
                None,
                &enough::Unstoppable,
                &enough::Unstoppable,
            )
            .unwrap();

            let saving_pct = (1.0 - joint.len() as f64 / standard.len() as f64) * 100.0;
            eprintln!(
                "{:30} tol={:.3} {:>7} -> {:>7} ({:+.1}%)",
                name,
                tolerance,
                standard.len(),
                joint.len(),
                saving_pct,
            );
        }

        // 256x256 smooth gradient
        let mut pixels = Vec::with_capacity(256 * 256);
        for y in 0..256u32 {
            for x in 0..256u32 {
                pixels.push(Rgba {
                    r: x.min(255) as u8,
                    g: y.min(255) as u8,
                    b: ((x + y) / 2).min(255) as u8,
                    a: 255,
                });
            }
        }
        let img = ImgVec::new(pixels, 256, 256);

        // Load real photos
        let corpus = std::env::var("CODEC_CORPUS_DIR")
            .unwrap_or_else(|_| "/home/lilith/work/codec-corpus".to_string());
        let paths: Vec<String> = vec![
            format!("{corpus}/imageflow/test_inputs/dice.png"),
            format!("{corpus}/imageflow/test_inputs/red-night.png"),
            format!("{corpus}/imageflow/test_inputs/rings2.png"),
        ];
        let mut real_images: Vec<(String, ImgVec<Rgba<u8>>)> = Vec::new();
        for path in &paths {
            if std::path::Path::new(path.as_str()).exists() {
                let data = std::fs::read(path).unwrap();
                let decoded = crate::decode::decode(
                    &data,
                    &crate::decode::PngDecodeConfig::none(),
                    &enough::Unstoppable,
                )
                .unwrap();
                let name = std::path::Path::new(path)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let pb = decoded.pixels.to_rgba8();
                let img = ImgVec::new(
                    pb.as_imgref().pixels().collect(),
                    pb.width() as usize,
                    pb.height() as usize,
                );
                real_images.push((name, img));
            }
        }

        for &tol in &[0.002, 0.005, 0.010, 0.020] {
            eprintln!("--- tolerance {tol} ---");
            compare("256x256 gradient", img.as_ref(), tol);
            for (name, ri) in &real_images {
                compare(name, ri.as_ref(), tol);
            }
        }
    }

    #[test]
    fn exact_palette_pixel_perfect_roundtrip() {
        use zenpixels_convert::{PixelBufferConvertExt, PixelBufferConvertTypedExt};

        let mut pixels = Vec::with_capacity(64);
        for y in 0..8u8 {
            for x in 0..8u8 {
                pixels.push(Rgba {
                    r: x * 32,
                    g: y * 32,
                    b: 128,
                    a: 255,
                });
            }
        }
        let img = ImgVec::new(pixels.clone(), 8, 8);
        let config = EncodeConfig::default();
        let quantizer = default_quantizer();

        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxDeltaE(0.0),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();

        assert!(result.indexed, "≤256 unique colors must use indexed");
        assert_eq!(result.quality_loss, 0.0, "exact palette must be lossless");
        assert_eq!(result.mpe_score, Some(0.0));
        assert_eq!(result.ssim2_estimate, Some(100.0));
        assert_eq!(result.butteraugli_estimate, Some(0.0));

        let decoded = crate::decode::decode(
            &result.data,
            &crate::decode::PngDecodeConfig::none(),
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded_rgba = decoded.pixels.to_rgba8();
        let decoded_img = decoded_rgba.as_imgref();
        let decoded_buf = decoded_img.buf();
        assert_eq!(decoded_buf.len(), pixels.len());
        for (i, (orig, dec)) in pixels.iter().zip(decoded_buf.iter()).enumerate() {
            assert_eq!(
                orig, dec,
                "pixel {i} mismatch: orig {:?} != decoded {:?}",
                orig, dec
            );
        }
    }

    // ── QualityGate unit tests ──────────────────────────────────────

    #[test]
    fn quality_gate_needs_metric() {
        assert!(!QualityGate::MaxDeltaE(0.02).needs_metric());
        assert!(QualityGate::MaxMpe(0.008).needs_metric());
        assert!(QualityGate::MinSsim2(85.0).needs_metric());
    }

    #[test]
    fn quality_gate_check_max_delta_e() {
        let output = QuantizeOutput {
            palette_rgba: vec![],
            indices: vec![],
            mpe_score: None,
            ssim2_estimate: None,
            butteraugli_estimate: None,
        };
        assert!(QualityGate::MaxDeltaE(0.05).check(&output, 0.02));
        assert!(!QualityGate::MaxDeltaE(0.01).check(&output, 0.02));
        assert!(QualityGate::MaxDeltaE(0.02).check(&output, 0.02)); // exact boundary
    }

    #[test]
    fn quality_gate_check_max_mpe() {
        let output_with = QuantizeOutput {
            palette_rgba: vec![],
            indices: vec![],
            mpe_score: Some(0.005),
            ssim2_estimate: None,
            butteraugli_estimate: None,
        };
        assert!(QualityGate::MaxMpe(0.008).check(&output_with, 0.0));
        assert!(!QualityGate::MaxMpe(0.003).check(&output_with, 0.0));

        // Missing metric → gate fails conservatively
        let output_none = QuantizeOutput {
            palette_rgba: vec![],
            indices: vec![],
            mpe_score: None,
            ssim2_estimate: None,
            butteraugli_estimate: None,
        };
        assert!(!QualityGate::MaxMpe(1.0).check(&output_none, 0.0));
    }

    #[test]
    fn quality_gate_check_min_ssim2() {
        let output_with = QuantizeOutput {
            palette_rgba: vec![],
            indices: vec![],
            mpe_score: None,
            ssim2_estimate: Some(90.0),
            butteraugli_estimate: None,
        };
        assert!(QualityGate::MinSsim2(85.0).check(&output_with, 0.0));
        assert!(!QualityGate::MinSsim2(95.0).check(&output_with, 0.0));

        // Missing metric → gate fails conservatively
        let output_none = QuantizeOutput {
            palette_rgba: vec![],
            indices: vec![],
            mpe_score: None,
            ssim2_estimate: None,
            butteraugli_estimate: None,
        };
        assert!(!QualityGate::MinSsim2(0.0).check(&output_none, 0.0));
    }

    // ── MaxMpe / MinSsim2 auto-encode paths ─────────────────────────

    #[test]
    fn auto_encode_max_mpe_gate() {
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quantizer = default_quantizer();

        // Few colors → exact palette → passes any gate
        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxMpe(0.008),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(result.indexed);
        assert_eq!(result.quality_loss, 0.0);
        assert_eq!(result.mpe_score, Some(0.0));
    }

    #[test]
    fn auto_encode_min_ssim2_gate() {
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quantizer = default_quantizer();

        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MinSsim2(85.0),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(result.indexed);
        assert_eq!(result.ssim2_estimate, Some(100.0));
    }

    // ── with_quality_metrics integration ────────────────────────────

    #[test]
    fn auto_encode_mpe_gate_on_many_colors() {
        // >256 colors → quantization required, MaxMpe needs metrics
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
        let quantizer = default_quantizer();

        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxMpe(1.0), // very lenient
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        // Should have MPE metric populated
        assert!(result.mpe_score.is_some());
    }

    // ── Truecolor fallback on strict gate ───────────────────────────

    #[test]
    fn auto_encode_truecolor_fallback_on_strict_mpe() {
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
        let quantizer = default_quantizer();

        // Impossibly strict MPE gate → should fall back to truecolor
        let result = encode_auto(
            img.as_ref(),
            &config,
            &*quantizer,
            QualityGate::MaxMpe(0.0),
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        assert!(!result.indexed, "should fall back to truecolor");
    }

    // ── encode_indexed with high effort ─────────────────────────────

    #[test]
    fn indexed_high_effort_roundtrip() {
        let img = test_image_4x4();
        let config = EncodeConfig::default().with_compression(crate::Compression::High);
        let quantizer = default_quantizer();
        let encoded = encode_indexed(
            img.as_ref(),
            &config,
            &*quantizer,
            None,
            &enough::Unstoppable,
            &enough::Unstoppable,
        )
        .unwrap();
        let decoded = crate::decode::decode(
            &encoded,
            &crate::decode::PngDecodeConfig::strict(),
            &enough::Unstoppable,
        )
        .unwrap();
        assert_eq!(decoded.info.width, 4);
    }

    // ── APNG indexed roundtrip ──────────────────────────────────────

    #[test]
    fn apng_indexed_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let sz = (w * h * 4) as usize;
        // 4 unique colors across both frames
        let frame0: Vec<u8> = (0..sz)
            .map(|i| match (i / 4) % 4 {
                0 => [255u8, 0, 0, 255],
                1 => [0, 255, 0, 255],
                2 => [0, 0, 255, 255],
                _ => [255, 255, 0, 255],
            }[i % 4])
            .collect();
        let frame1: Vec<u8> = (0..sz)
            .map(|i| match (i / 4) % 4 {
                0 => [0u8, 0, 255, 255],
                1 => [255, 255, 0, 255],
                2 => [255, 0, 0, 255],
                _ => [0, 255, 0, 255],
            }[i % 4])
            .collect();

        let frames = [
            crate::encode::ApngFrameInput::new(&frame0, 1, 30),
            crate::encode::ApngFrameInput::new(&frame1, 1, 30),
        ];
        let config = crate::encode::ApngEncodeConfig::default();
        let quantizer = default_quantizer();

        let params = crate::indexed::ApngEncodeParams {
            frames: &frames,
            canvas_width: w,
            canvas_height: h,
            config: &config,
            quantizer: &*quantizer,
            metadata: None,
            cancel: &enough::Unstoppable,
            deadline: &enough::Unstoppable,
        };
        let encoded = encode_apng_indexed(&params).unwrap();
        assert!(!encoded.is_empty());
        assert_eq!(&encoded[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    // ── APNG auto roundtrip ─────────────────────────────────────────

    #[test]
    fn apng_auto_few_colors_uses_indexed() {
        let w = 4u32;
        let h = 4u32;
        let sz = (w * h * 4) as usize;
        // Only 2 unique colors
        let frame0 = vec![255u8, 0, 0, 255].repeat(w as usize * h as usize);
        let frame1 = vec![0u8, 255, 0, 255].repeat(w as usize * h as usize);
        assert!(frame0.len() >= sz);
        assert!(frame1.len() >= sz);

        let frames = [
            crate::encode::ApngFrameInput::new(&frame0, 1, 30),
            crate::encode::ApngFrameInput::new(&frame1, 1, 30),
        ];
        let config = crate::encode::ApngEncodeConfig::default();
        let quantizer = default_quantizer();

        let params = crate::indexed::ApngEncodeParams {
            frames: &frames,
            canvas_width: w,
            canvas_height: h,
            config: &config,
            quantizer: &*quantizer,
            metadata: None,
            cancel: &enough::Unstoppable,
            deadline: &enough::Unstoppable,
        };
        let result = encode_apng_auto(&params, QualityGate::MaxDeltaE(0.02)).unwrap();
        assert!(result.indexed, "2-color APNG should use indexed");
        assert_eq!(result.quality_loss, 0.0);
    }

    #[test]
    fn apng_auto_validates_empty_frames() {
        let config = crate::encode::ApngEncodeConfig::default();
        let quantizer = default_quantizer();
        let frames: &[crate::encode::ApngFrameInput<'_>] = &[];
        let params = crate::indexed::ApngEncodeParams {
            frames,
            canvas_width: 4,
            canvas_height: 4,
            config: &config,
            quantizer: &*quantizer,
            metadata: None,
            cancel: &enough::Unstoppable,
            deadline: &enough::Unstoppable,
        };
        let result = encode_apng_auto(&params, QualityGate::MaxDeltaE(0.02));
        assert!(result.is_err());
    }

    // ── compute_mean_delta_e edge case ──────────────────────────────

    #[test]
    fn delta_e_empty_returns_zero() {
        let result = compute_mean_delta_e(&[], &[], &[]);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn delta_e_exact_match_returns_zero() {
        let pixels = vec![Rgba {
            r: 128,
            g: 64,
            b: 32,
            a: 255,
        }];
        let palette = vec![[128, 64, 32, 255]];
        let indices = vec![0];
        let result = compute_mean_delta_e(&pixels, &palette, &indices);
        assert!(result < 1e-10);
    }

    #[test]
    fn delta_e_detects_alpha_mismatch() {
        // Same RGB, different alpha — old version would return 0.0
        let pixels = vec![Rgba {
            r: 200,
            g: 100,
            b: 50,
            a: 255,
        }];
        let palette = vec![[200, 100, 50, 0]]; // fully transparent
        let indices = vec![0];
        let result = compute_mean_delta_e(&pixels, &palette, &indices);
        // Opaque vs transparent must produce significant ΔE
        assert!(
            result > 0.1,
            "expected large ΔE for alpha mismatch, got {result}"
        );
    }

    #[test]
    fn delta_e_transparent_exact_match_is_zero() {
        // Both fully transparent — composited result is the same regardless of RGB
        let pixels = vec![Rgba {
            r: 255,
            g: 0,
            b: 0,
            a: 0,
        }];
        let palette = vec![[0, 255, 0, 0]]; // different RGB but both alpha=0
        let indices = vec![0];
        let result = compute_mean_delta_e(&pixels, &palette, &indices);
        assert!(
            result < 1e-10,
            "transparent pixels should match regardless of RGB, got {result}"
        );
    }

    #[test]
    fn delta_e_shorter_indices_uses_correct_count() {
        // 3 pixels but only 2 indices — should divide by 2, not 3
        let pixels = vec![
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            Rgba {
                r: 128,
                g: 128,
                b: 128,
                a: 255,
            },
        ];
        let palette = vec![[0, 0, 0, 255], [255, 255, 255, 255]];
        let indices = vec![0, 1]; // only 2 indices for 3 pixels
        let result = compute_mean_delta_e(&pixels, &palette, &indices);
        // Both matched pixels are exact matches → ΔE should be ~0
        assert!(
            result < 1e-10,
            "exact matches should give ~0 ΔE, got {result}"
        );
    }
}
