//! Zennode encode node definition for zenpng.
//!
//! Provides [`EncodePng`], a self-documenting pipeline node that bridges
//! zennode's parameter system with [`PngEncoderConfig`].
//!
//! Feature-gated behind `feature = "zennode"`.

use zennode::*;

use crate::PngEncoderConfig;

/// Direct PNG encoder configuration (zennode node).
///
/// Controls PNG-specific encoding parameters: compression effort, quality
/// (lossy via palette quantization), near-lossless bit rounding, and
/// lossless mode.
///
/// **RIAPI**: `?png.e=4&png.q=90&png.lossless=false`
/// **JSON**: `{ "effort": 4, "quality": 90.0, "lossless": false }`
///
/// Convert to [`PngEncoderConfig`] via [`to_encoder_config()`](EncodePng::to_encoder_config).
#[derive(Node, Clone, Debug)]
#[node(id = "zenpng.encode", group = Encode, role = Encode)]
#[node(tags("png", "encode", "lossless", "compression"))]
pub struct EncodePng {
    /// Compression effort (0 = store, 4 = default, 12 = maximum).
    ///
    /// Higher effort means slower encoding but smaller files.
    /// Maps to PNG compression strategy via `with_generic_effort()`.
    #[param(range(0..=12), default = 4, step = 1)]
    #[param(section = "Main", label = "Effort")]
    #[kv("png.effort", "png.e")]
    pub effort: i32,

    /// Quality target (0-100). Only used when `lossless` is false.
    ///
    /// Controls palette quantization quality for lossy PNG encoding.
    /// 100.0 = best quality, 0.0 = smallest file.
    #[param(range(0.0..=100.0), default = 100.0, identity = 100.0, step = 1.0)]
    #[param(section = "Main", label = "Quality")]
    #[kv("png.quality", "png.q")]
    pub quality: f32,

    /// Near-lossless bit rounding (0-4 bits).
    ///
    /// Rounds least-significant bits of each 8-bit sample to improve
    /// compression. 0 = fully lossless, 1-2 = imperceptible,
    /// 3-4 = minor visible reduction with significant size savings.
    #[param(range(0..=4), default = 0, identity = 0, step = 1)]
    #[param(unit = "bits", section = "Advanced", label = "Near-Lossless Bits")]
    #[kv("png.nlbits", "png.near_lossless")]
    pub near_lossless_bits: i32,

    /// Lossless mode. When true, quality is ignored and output is lossless.
    #[param(default = true)]
    #[param(section = "Main", label = "Lossless")]
    #[kv("png.lossless")]
    pub lossless: bool,
}

impl Default for EncodePng {
    fn default() -> Self {
        Self {
            effort: 4,
            quality: 100.0,
            near_lossless_bits: 0,
            lossless: true,
        }
    }
}

impl EncodePng {
    /// Convert this node into a [`PngEncoderConfig`] for encoding.
    pub fn to_encoder_config(&self) -> PngEncoderConfig {
        use zencodec::encode::EncoderConfig;

        let mut config = PngEncoderConfig::new()
            .with_generic_effort(self.effort)
            .with_lossless(self.lossless);

        if !self.lossless {
            config = config.with_generic_quality(self.quality);
        }

        if self.near_lossless_bits > 0 {
            config = config.with_near_lossless_bits(self.near_lossless_bits as u8);
        }

        config
    }
}
