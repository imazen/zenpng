//! Quantizer backend abstraction for palette generation.
//!
//! Provides a [`Quantizer`] trait with implementations for multiple backends:
//! - [`ZenquantQuantizer`] (feature `quantize`, default) — perceptual quality, metrics, joint optimization
//! - [`ImagequantQuantizer`] (feature `imagequant`) — libimagequant
//! - [`QuantetteQuantizer`] (feature `quantette`) — fast k-means / Wu
//!
//! # Runtime selection
//!
//! All backends implement the same [`Quantizer`] trait, enabling runtime dispatch:
//! ```ignore
//! let quantizer: Box<dyn Quantizer> = match name {
//!     "zenquant" => Box::new(ZenquantQuantizer::default()),
//!     "imagequant" => Box::new(ImagequantQuantizer::default()),
//!     "quantette" => Box::new(QuantetteQuantizer::default()),
//!     _ => return Err(..),
//! };
//! let output = quantizer.quantize_rgba(pixels, width, height)?;
//! ```

use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::PngError;

/// Output of a single-image quantization.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QuantizeOutput {
    /// RGBA palette entries (2–256).
    pub palette_rgba: Vec<[u8; 4]>,
    /// Per-pixel palette index (row-major, length = width × height).
    pub indices: Vec<u8>,
    /// MPE quality score (lower = better). Only zenquant provides this.
    pub mpe_score: Option<f32>,
    /// Estimated SSIMULACRA2 score (0–100, higher = better). Only zenquant provides this.
    pub ssim2_estimate: Option<f32>,
    /// Estimated butteraugli distance (lower = better). Only zenquant provides this.
    pub butteraugli_estimate: Option<f32>,
}

/// Output of multi-frame quantization with a shared palette.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MultiFrameOutput {
    /// Shared RGBA palette across all frames.
    pub palette_rgba: Vec<[u8; 4]>,
    /// Per-frame index buffers (parallel to input frames).
    pub frame_indices: Vec<Vec<u8>>,
    /// Per-frame MPE scores. Only zenquant provides these.
    pub mpe_scores: Vec<Option<f32>>,
    /// Per-frame SSIMULACRA2 estimates. Only zenquant provides these.
    pub ssim2_estimates: Vec<Option<f32>>,
    /// Per-frame butteraugli estimates. Only zenquant provides these.
    pub butteraugli_estimates: Vec<Option<f32>>,
}

/// A color quantizer that reduces RGBA8 images to ≤256 indexed colors.
///
/// Built-in implementations:
/// - [`ZenquantQuantizer`] — perceptual median-cut, quality metrics, APNG temporal consistency
/// - [`ImagequantQuantizer`] — libimagequant, high-quality dithering
/// - [`QuantetteQuantizer`] — fast k-means or Wu quantization
pub trait Quantizer: Send + Sync {
    /// Quantize RGBA8 pixels to a palette + index map.
    ///
    /// `pixels` must contain exactly `width × height` entries in row-major order.
    fn quantize_rgba(
        &self,
        pixels: &[[u8; 4]],
        width: usize,
        height: usize,
    ) -> Result<QuantizeOutput, PngError>;

    /// Quantize multiple frames with a shared palette (for APNG).
    ///
    /// Each frame in `frames` must contain exactly `width × height` RGBA pixels.
    ///
    /// Default implementation concatenates all frames vertically, quantizes as
    /// one image, then splits indices per frame. Backends with native multi-frame
    /// support (zenquant) override this for temporal consistency.
    fn quantize_multi_frame(
        &self,
        frames: &[&[[u8; 4]]],
        width: usize,
        height: usize,
    ) -> Result<MultiFrameOutput, PngError> {
        let pixels_per_frame = width * height;
        let mut concat: Vec<[u8; 4]> = Vec::with_capacity(pixels_per_frame * frames.len());
        for frame in frames {
            if frame.len() < pixels_per_frame {
                return Err(PngError::InvalidInput(format!(
                    "frame has {} pixels, expected {}",
                    frame.len(),
                    pixels_per_frame
                )));
            }
            concat.extend_from_slice(&frame[..pixels_per_frame]);
        }
        let total_height = height * frames.len();
        let result = self.quantize_rgba(&concat, width, total_height)?;
        let n = frames.len();
        let mut frame_indices = Vec::with_capacity(n);
        for i in 0..n {
            let start = i * pixels_per_frame;
            frame_indices.push(result.indices[start..start + pixels_per_frame].to_vec());
        }
        Ok(MultiFrameOutput {
            palette_rgba: result.palette_rgba,
            frame_indices,
            mpe_scores: vec![None; n],
            ssim2_estimates: vec![None; n],
            butteraugli_estimates: vec![None; n],
        })
    }

    /// Return a version of this quantizer with quality metric computation enabled.
    ///
    /// Called by [`encode_auto`](crate::encode_auto) and [`encode_apng_auto`](crate::encode_apng_auto)
    /// when the quality gate requires metrics (`MaxMpe`, `MinSsim2`).
    ///
    /// Backends that support quality metrics (zenquant) should return a copy
    /// with metrics enabled. Returns `None` if not supported (default).
    fn with_quality_metrics(&self) -> Option<Box<dyn Quantizer>> {
        None
    }

    /// Backend name for diagnostics and logging.
    fn name(&self) -> &str;
}

/// Create a default quantizer (zenquant if available, else first enabled backend).
///
/// Quality metrics are enabled for zenquant (needed for [`QualityGate::MaxMpe`] / [`MinSsim2`]).
#[cfg(any(feature = "quantize", feature = "imagequant", feature = "quantette"))]
pub fn default_quantizer() -> Box<dyn Quantizer> {
    #[cfg(feature = "quantize")]
    {
        Box::new(ZenquantQuantizer::new().with_compute_quality_metric(true))
    }
    #[cfg(all(not(feature = "quantize"), feature = "imagequant"))]
    {
        Box::new(ImagequantQuantizer::default())
    }
    #[cfg(all(
        not(feature = "quantize"),
        not(feature = "imagequant"),
        feature = "quantette"
    ))]
    {
        Box::new(QuantetteQuantizer::default())
    }
}

/// Create a quantizer by name at runtime.
///
/// Returns an error if the named backend is not enabled via feature flags.
/// Valid names: `"zenquant"`, `"imagequant"`, `"quantette"`.
pub fn quantizer_by_name(name: &str) -> Result<Box<dyn Quantizer>, PngError> {
    match name {
        #[cfg(feature = "quantize")]
        "zenquant" => Ok(Box::new(
            ZenquantQuantizer::new().with_compute_quality_metric(true),
        )),
        #[cfg(feature = "imagequant")]
        "imagequant" => Ok(Box::new(ImagequantQuantizer::default())),
        #[cfg(feature = "quantette")]
        "quantette" => Ok(Box::new(QuantetteQuantizer::default())),
        _ => Err(PngError::InvalidInput(format!(
            "unknown or disabled quantizer backend: {name:?}. Available: {:?}",
            available_backends()
        ))),
    }
}

/// List available quantizer backend names (based on enabled features).
pub fn available_backends() -> &'static [&'static str] {
    &[
        #[cfg(feature = "quantize")]
        "zenquant",
        #[cfg(feature = "imagequant")]
        "imagequant",
        #[cfg(feature = "quantette")]
        "quantette",
    ]
}

// ── Zenquant backend ──────────────────────────────────────────────

#[cfg(feature = "quantize")]
pub use self::zenquant_backend::ZenquantQuantizer;

#[cfg(feature = "quantize")]
mod zenquant_backend {
    use super::*;

    /// zenquant quantizer — perceptual median-cut with dithering and quality metrics.
    ///
    /// Wraps a [`zenquant::QuantizeConfig`] with builder methods for common settings.
    /// For full control, use [`config_mut()`](Self::config_mut) to access the inner config.
    ///
    /// Supports joint optimization (feature `joint`) and native multi-frame APNG
    /// palette building with temporal consistency.
    #[derive(Debug, Clone)]
    pub struct ZenquantQuantizer {
        config: zenquant::QuantizeConfig,
    }

    impl Default for ZenquantQuantizer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ZenquantQuantizer {
        /// Create with default PNG-tuned settings.
        #[must_use]
        pub fn new() -> Self {
            Self {
                config: zenquant::QuantizeConfig::new(zenquant::OutputFormat::Png),
            }
        }

        /// Create for a specific output format (Png, Gif, PngJoint).
        #[must_use]
        pub fn with_format(format: zenquant::OutputFormat) -> Self {
            Self {
                config: zenquant::QuantizeConfig::new(format),
            }
        }

        /// Create from an existing zenquant config (for full control).
        #[must_use]
        pub fn from_config(config: zenquant::QuantizeConfig) -> Self {
            Self { config }
        }

        /// Set quality preset (Fast, Balanced, Best).
        #[must_use]
        pub fn with_quality(mut self, quality: zenquant::Quality) -> Self {
            self.config = self.config.with_quality(quality);
            self
        }

        /// Set maximum palette colors (2–256).
        #[must_use]
        pub fn with_max_colors(mut self, n: u16) -> Self {
            self.config = self.config.with_max_colors(n.into());
            self
        }

        /// Enable quality metric computation (MPE, SSIM2, butteraugli estimates).
        ///
        /// Required for [`QualityGate::MaxMpe`] and [`QualityGate::MinSsim2`].
        #[must_use]
        pub fn with_compute_quality_metric(mut self, compute: bool) -> Self {
            self.config = self.config.with_compute_quality_metric(compute);
            self
        }

        /// Access the underlying zenquant config.
        pub fn config(&self) -> &zenquant::QuantizeConfig {
            &self.config
        }

        /// Mutably access the underlying zenquant config for fine-tuning.
        pub fn config_mut(&mut self) -> &mut zenquant::QuantizeConfig {
            &mut self.config
        }
    }

    impl Quantizer for ZenquantQuantizer {
        fn quantize_rgba(
            &self,
            pixels: &[[u8; 4]],
            width: usize,
            height: usize,
        ) -> Result<QuantizeOutput, PngError> {
            let rgba: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(pixels);
            let result = zenquant::quantize_rgba(rgba, width, height, &self.config)?;
            Ok(QuantizeOutput {
                palette_rgba: result.palette_rgba().to_vec(),
                indices: result.indices().to_vec(),
                mpe_score: result.mpe_score(),
                ssim2_estimate: result.ssimulacra2_estimate(),
                butteraugli_estimate: result.butteraugli_estimate(),
            })
        }

        fn quantize_multi_frame(
            &self,
            frames: &[&[[u8; 4]]],
            width: usize,
            height: usize,
        ) -> Result<MultiFrameOutput, PngError> {
            use imgref::ImgRef;

            let pixels_per_frame = width * height;
            let frame_refs: Vec<ImgRef<'_, zenquant::RGBA<u8>>> = frames
                .iter()
                .map(|f| {
                    let pixels: &[zenquant::RGBA<u8>] =
                        bytemuck::cast_slice(&f[..pixels_per_frame]);
                    ImgRef::new(pixels, width, height)
                })
                .collect();

            let palette_result = zenquant::build_palette_rgba(&frame_refs, &self.config)?;
            let palette_rgba = palette_result.palette_rgba().to_vec();

            let mut frame_indices = Vec::with_capacity(frames.len());
            let mut mpe_scores = Vec::with_capacity(frames.len());
            let mut ssim2_estimates = Vec::with_capacity(frames.len());
            let mut butteraugli_estimates = Vec::with_capacity(frames.len());
            let mut prev_indices: Option<Vec<u8>> = None;

            for frame_ref in &frame_refs {
                let (frame_buf, fw, fh) = frame_ref.to_contiguous_buf();
                let remap_result = if let Some(prev) = &prev_indices {
                    palette_result.remap_rgba_with_prev(
                        frame_buf.as_ref(),
                        fw,
                        fh,
                        &self.config,
                        prev,
                    )?
                } else {
                    palette_result.remap_rgba(frame_buf.as_ref(), fw, fh, &self.config)?
                };

                mpe_scores.push(remap_result.mpe_score());
                ssim2_estimates.push(remap_result.ssimulacra2_estimate());
                butteraugli_estimates.push(remap_result.butteraugli_estimate());
                let indices = remap_result.indices().to_vec();
                prev_indices = Some(indices.clone());
                frame_indices.push(indices);
            }

            Ok(MultiFrameOutput {
                palette_rgba,
                frame_indices,
                mpe_scores,
                ssim2_estimates,
                butteraugli_estimates,
            })
        }

        fn with_quality_metrics(&self) -> Option<Box<dyn Quantizer>> {
            Some(Box::new(self.clone().with_compute_quality_metric(true)))
        }

        fn name(&self) -> &str {
            "zenquant"
        }
    }
}

// ── Imagequant backend ────────────────────────────────────────────

#[cfg(feature = "imagequant")]
pub use self::imagequant_backend::ImagequantQuantizer;

#[cfg(feature = "imagequant")]
mod imagequant_backend {
    use super::*;

    /// imagequant quantizer — libimagequant with high-quality dithering.
    ///
    /// Pure Rust port of pngquant's quantization engine.
    #[derive(Debug, Clone)]
    pub struct ImagequantQuantizer {
        /// Processing speed (1 = slowest/best, 10 = fastest). Default: 4.
        pub speed: i32,
        /// Maximum quality (0–100). Default: 100.
        pub max_quality: u8,
        /// Dithering level (0.0 = none, 1.0 = full). Default: 1.0.
        pub dithering: f32,
        /// Maximum palette colors (2–256). Default: 256.
        pub max_colors: u16,
    }

    impl Default for ImagequantQuantizer {
        fn default() -> Self {
            Self {
                speed: 4,
                max_quality: 100,
                dithering: 1.0,
                max_colors: 256,
            }
        }
    }

    impl ImagequantQuantizer {
        #[must_use]
        pub fn with_speed(mut self, speed: i32) -> Self {
            self.speed = speed;
            self
        }

        #[must_use]
        pub fn with_max_quality(mut self, q: u8) -> Self {
            self.max_quality = q;
            self
        }

        #[must_use]
        pub fn with_dithering(mut self, d: f32) -> Self {
            self.dithering = d;
            self
        }

        #[must_use]
        pub fn with_max_colors(mut self, n: u16) -> Self {
            self.max_colors = n;
            self
        }
    }

    impl Quantizer for ImagequantQuantizer {
        fn quantize_rgba(
            &self,
            pixels: &[[u8; 4]],
            width: usize,
            height: usize,
        ) -> Result<QuantizeOutput, PngError> {
            let mut attr = imagequant::Attributes::new();
            attr.set_quality(0, self.max_quality)
                .map_err(|e| PngError::InvalidInput(format!("imagequant quality: {e}")))?;
            attr.set_speed(self.speed)
                .map_err(|e| PngError::InvalidInput(format!("imagequant speed: {e}")))?;
            attr.set_max_colors(self.max_colors as u32)
                .map_err(|e| PngError::InvalidInput(format!("imagequant max_colors: {e}")))?;

            let rgba_pixels: Vec<imagequant::RGBA> = pixels
                .iter()
                .map(|p| imagequant::RGBA::new(p[0], p[1], p[2], p[3]))
                .collect();

            let mut img = attr
                .new_image(rgba_pixels, width, height, 0.0)
                .map_err(|e| PngError::InvalidInput(format!("imagequant image: {e}")))?;
            let mut result = attr
                .quantize(&mut img)
                .map_err(|e| PngError::InvalidInput(format!("imagequant quantize: {e}")))?;
            result
                .set_dithering_level(self.dithering)
                .map_err(|e| PngError::InvalidInput(format!("imagequant dithering: {e}")))?;

            let (palette, indices) = result
                .remapped(&mut img)
                .map_err(|e| PngError::InvalidInput(format!("imagequant remap: {e}")))?;

            let palette_rgba: Vec<[u8; 4]> = palette.iter().map(|c| [c.r, c.g, c.b, c.a]).collect();

            Ok(QuantizeOutput {
                palette_rgba,
                indices,
                mpe_score: None,
                ssim2_estimate: None,
                butteraugli_estimate: None,
            })
        }

        fn name(&self) -> &str {
            "imagequant"
        }
    }
}

// ── Quantette backend ─────────────────────────────────────────────

#[cfg(feature = "quantette")]
pub use self::quantette_backend::QuantetteQuantizer;

#[cfg(feature = "quantette")]
mod quantette_backend {
    use super::*;

    /// quantette quantizer — fast k-means or Wu quantization.
    ///
    /// Operates on sRGB colors only. Images with transparent pixels (alpha < 255)
    /// will have transparency ignored — all palette entries get alpha = 255.
    /// Use zenquant or imagequant for images requiring alpha transparency.
    #[derive(Debug, Clone)]
    pub struct QuantetteQuantizer {
        /// Use k-means refinement (true) or Wu's method (false). Default: true.
        pub kmeans: bool,
        /// Enable Floyd-Steinberg dithering. Default: true.
        pub dithering: bool,
        /// Maximum palette colors (2–256). Default: 256.
        pub max_colors: u16,
        /// K-means sampling factor. Default: 1.0.
        pub sampling_factor: f32,
    }

    impl Default for QuantetteQuantizer {
        fn default() -> Self {
            Self {
                kmeans: true,
                dithering: true,
                max_colors: 256,
                sampling_factor: 1.0,
            }
        }
    }

    impl QuantetteQuantizer {
        #[must_use]
        pub fn with_kmeans(mut self, kmeans: bool) -> Self {
            self.kmeans = kmeans;
            self
        }

        #[must_use]
        pub fn with_dithering(mut self, dithering: bool) -> Self {
            self.dithering = dithering;
            self
        }

        #[must_use]
        pub fn with_max_colors(mut self, n: u16) -> Self {
            self.max_colors = n;
            self
        }

        #[must_use]
        pub fn with_sampling_factor(mut self, f: f32) -> Self {
            self.sampling_factor = f;
            self
        }
    }

    impl Quantizer for QuantetteQuantizer {
        fn quantize_rgba(
            &self,
            pixels: &[[u8; 4]],
            width: usize,
            height: usize,
        ) -> Result<QuantizeOutput, PngError> {
            use quantette::deps::palette::Srgb;
            use quantette::{ImageBuf, Pipeline, QuantizeMethod};

            // quantette works with RGB only — strip alpha
            let srgb_pixels: Vec<Srgb<u8>> =
                pixels.iter().map(|p| Srgb::new(p[0], p[1], p[2])).collect();

            let image = ImageBuf::new(width as u32, height as u32, srgb_pixels)
                .map_err(|e| PngError::InvalidInput(format!("quantette image: {e}")))?;

            let method = if self.kmeans {
                use quantette::kmeans::KmeansOptions;
                QuantizeMethod::Kmeans(KmeansOptions::new().sampling_factor(self.sampling_factor))
            } else {
                QuantizeMethod::Wu
            };

            let palette_size = self
                .max_colors
                .try_into()
                .map_err(|_| PngError::InvalidInput("max_colors out of range".into()))?;

            let mut pipeline = Pipeline::new()
                .palette_size(palette_size)
                .quantize_method(method);

            if self.dithering {
                use quantette::dither::FloydSteinberg;
                pipeline = pipeline.ditherer(Some(FloydSteinberg::new()));
            }

            let indexed = pipeline
                .input_image(image.as_ref())
                .output_srgb8_indexed_image();

            // Add alpha=255 to all palette entries (quantette doesn't handle alpha)
            let palette_rgba: Vec<[u8; 4]> = indexed
                .palette()
                .iter()
                .map(|c| [c.red, c.green, c.blue, 255])
                .collect();
            let indices = indexed.indices().to_vec();

            Ok(QuantizeOutput {
                palette_rgba,
                indices,
                mpe_score: None,
                ssim2_estimate: None,
                butteraugli_estimate: None,
            })
        }

        fn name(&self) -> &str {
            "quantette"
        }
    }
}
