//! PNG encoding.

use alloc::string::String;
use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::{Gray, Rgb, Rgba};

use zencodec::{Cicp, ContentLightLevel, MasteringDisplay, Metadata};

use enough::Stop;

use crate::decode::{PhysUnit, PngChromaticities, PngTime, TextChunk};
use crate::encoder::PngWriteMetadata;
use whereat::at;

use crate::error::PngError;
use crate::types::{Compression, Filter};

/// PNG encode configuration.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct EncodeConfig {
    /// PNG compression level.
    pub compression: Compression,
    /// PNG row filter strategy.
    pub filter: Filter,
    /// Use multi-threaded screening and refinement.
    ///
    /// When true, Phase 1 (strategy screening) and Phase 2 (refinement)
    /// run their independent evaluations in parallel using `std::thread::scope`.
    /// Each thread allocates its own compression buffers (~3× image size),
    /// so memory usage scales with thread count. Default: false.
    pub parallel: bool,
    /// Source gamma for gAMA chunk (scaled by 100000, e.g. 45455 = 1/2.2).
    ///
    /// Suppressed in output when sRGB, iCCP, or cICP is present (PNGv3 precedence).
    pub source_gamma: Option<u32>,
    /// sRGB rendering intent for sRGB chunk (0-3).
    ///
    /// Suppressed in output when iCCP or cICP is present (PNGv3 precedence).
    /// When written, suppresses gAMA and cHRM.
    pub srgb_intent: Option<u8>,
    /// Chromaticities for cHRM chunk.
    ///
    /// Suppressed in output when sRGB, iCCP, or cICP is present (PNGv3 precedence).
    pub chromaticities: Option<PngChromaticities>,
    /// CICP color signaling for the `cICP` chunk (PNG-3). Set to
    /// [`Cicp::BT2100_PQ`] / [`Cicp::BT2100_HLG`] for HDR renditions. Takes
    /// precedence over gAMA/cHRM/sRGB per PNG-3; matrix-coefficients are forced
    /// to 0 (PNG's RGB color model) by the chunk writer.
    pub cicp: Option<Cicp>,
    /// Content light level info for the `cLLI` chunk (PNG-3 HDR: MaxCLL/MaxFALL).
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering display color volume for the `mDCV` chunk (PNG-3 HDR). Written
    /// only alongside a [`cicp`](Self::cicp) chunk (PNG-3 §11.3.2.7).
    pub mastering_display: Option<MasteringDisplay>,
    /// Near-lossless: number of least-significant bits to round (0-4).
    ///
    /// When set to N > 0, each 8-bit sample has its lowest N bits rounded
    /// to the nearest multiple of 2^N. This creates more byte repetition
    /// that PNG filters and DEFLATE exploit for smaller files.
    ///
    /// Quality impact is imperceptible at 1-2 bits. At 3-4 bits, there's
    /// a visible but minor quality reduction, with significant compression
    /// gains (typically 20-50% smaller for photographic content).
    ///
    /// Default: 0 (lossless). Does not affect indexed (palette) encoding.
    pub near_lossless_bits: u8,
    /// Maximum number of threads for compression.
    ///
    /// - `0` means no limit (use as many threads as beneficial).
    /// - `1` forces fully single-threaded operation: no `std::thread::scope`
    ///   calls anywhere in the compression pipeline.
    /// - `N > 1` caps parallelism to at most N threads.
    ///
    /// When set to 1, both the screening/refinement phases and the
    /// recompression phase run sequentially. This is derived from
    /// [`ThreadingPolicy`](zencodec::ThreadingPolicy) when using the zencodec adapter.
    ///
    /// Default: 0 (no limit).
    pub max_threads: usize,
    /// Physical pixel dimensions X for pHYs chunk.
    pub pixels_per_unit_x: Option<u32>,
    /// Physical pixel dimensions Y for pHYs chunk.
    pub pixels_per_unit_y: Option<u32>,
    /// Unit for physical pixel dimensions (pHYs chunk).
    pub phys_unit: Option<PhysUnit>,
    /// Text chunks to embed (tEXt).
    pub text_chunks: Vec<TextChunk>,
    /// Last modification time for tIME chunk.
    pub last_modified: Option<PngTime>,
    /// Lossless color-type and bit-depth downcast options.
    ///
    /// Each flag controls one optimization that detects when the input has
    /// fewer effective bits/channels/colors than its declared format. All
    /// are pure scans of the source pixels; the encoded output is always
    /// bit-exact for the chosen format. Disable individual flags for
    /// debugging or to skip the scan cost on inputs known not to benefit.
    pub downcast: DowncastFlags,
}

/// Lossless downcast knobs applied before filtering.
///
/// These predicates run once over the input. Most early-exit on the first
/// disqualifying pixel — on photographic content they bail in row 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DowncastFlags {
    /// Detect all-opaque RGBA and emit RGB (drops the alpha channel).
    /// Saves 25% of raw pixel bytes before filtering.
    pub rgba_to_rgb: bool,
    /// Detect `R == G == B` for every pixel and emit Grayscale (or
    /// GrayscaleAlpha when alpha is needed). Saves 50–67% of raw bytes.
    pub rgb_to_gray: bool,
    /// Detect every grayscale value being a multiple of 17 / 85 / 255 and
    /// emit 4-/2-/1-bit grayscale via sub-byte packing. Up to 8× reduction
    /// on line art and B&W scans.
    pub sub_byte_gray: bool,
    /// Detect ≤256 unique colors and emit indexed PNG with PLTE/tRNS.
    pub indexed: bool,
    /// Detect binary alpha + a single exclusive transparent color and emit
    /// it via tRNS instead of a full alpha channel.
    pub alpha_to_trns: bool,
    /// Detect 16-bit channels with PNG-lossless bit-replication
    /// (`hi == lo` per pair → `u16 = u8 * 0x0101`) and downcast to 8-bit.
    /// This is the correct PNG-lossless 16→8 condition; use this on inputs
    /// produced by widening 8-bit data via bit-replication.
    pub downcast_16_to_8_replicated: bool,
    /// Detect 16-bit channels whose low byte is always zero and downcast
    /// by dropping the low byte. This is **not** a strict round-trip under
    /// PNG semantics: a decoder reconstructs `0x12 → 0x1212`, not `0x1200`,
    /// so the encoded color drifts. Off by default; enable only when the
    /// caller's pipeline treats u16 channels as `u8 << 8`.
    pub downcast_16_to_8_low_zero: bool,
    /// Detect Display-P3 / Rec.2020 / Adobe-RGB inputs whose pixels all
    /// fall within sRGB primaries, transform values into sRGB, and re-tag.
    /// More expensive than the byte-level predicates (per-pixel matrix
    /// multiply with early-exit) — gated on `compression.effort() >= 7`.
    /// Off by default until the implementation lands.
    pub gamut_downcast: bool,
}

impl Default for DowncastFlags {
    fn default() -> Self {
        Self {
            rgba_to_rgb: true,
            rgb_to_gray: true,
            sub_byte_gray: true,
            indexed: true,
            alpha_to_trns: true,
            downcast_16_to_8_replicated: true,
            downcast_16_to_8_low_zero: false,
            gamut_downcast: false,
        }
    }
}

impl DowncastFlags {
    /// Disable every downcast — write the input format verbatim.
    #[must_use]
    pub fn none() -> Self {
        Self {
            rgba_to_rgb: false,
            rgb_to_gray: false,
            sub_byte_gray: false,
            indexed: false,
            alpha_to_trns: false,
            downcast_16_to_8_replicated: false,
            downcast_16_to_8_low_zero: false,
            gamut_downcast: false,
        }
    }
}

impl EncodeConfig {
    /// Set compression level.
    #[must_use]
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Set row filter strategy.
    #[must_use]
    pub fn with_filter(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    /// Enable multi-threaded screening and refinement.
    #[must_use]
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Set source gamma for gAMA chunk (scaled by 100000, e.g. 45455 = 1/2.2).
    #[must_use]
    pub fn with_source_gamma(mut self, gamma: Option<u32>) -> Self {
        self.source_gamma = gamma;
        self
    }

    /// Set sRGB rendering intent for sRGB chunk (0-3).
    #[must_use]
    pub fn with_srgb_intent(mut self, intent: Option<u8>) -> Self {
        self.srgb_intent = intent;
        self
    }

    /// Set chromaticities for cHRM chunk.
    #[must_use]
    pub fn with_chromaticities(mut self, chrm: Option<PngChromaticities>) -> Self {
        self.chromaticities = chrm;
        self
    }

    /// Set CICP color signaling for the `cICP` chunk (PNG-3). Use
    /// [`Cicp::BT2100_PQ`] / [`Cicp::BT2100_HLG`] for HDR. Per PNG-3 precedence
    /// this suppresses gAMA/cHRM/sRGB in the output.
    #[must_use]
    pub fn with_cicp(mut self, cicp: Option<Cicp>) -> Self {
        self.cicp = cicp;
        self
    }

    /// Set content light level info for the `cLLI` chunk (PNG-3 HDR).
    #[must_use]
    pub fn with_content_light_level(mut self, clli: Option<ContentLightLevel>) -> Self {
        self.content_light_level = clli;
        self
    }

    /// Set mastering display color volume for the `mDCV` chunk (PNG-3 HDR).
    /// Only emitted when [`with_cicp`](Self::with_cicp) is also set.
    #[must_use]
    pub fn with_mastering_display(mut self, mdcv: Option<MasteringDisplay>) -> Self {
        self.mastering_display = mdcv;
        self
    }

    /// Set near-lossless bit rounding (0-4).
    #[must_use]
    pub fn with_near_lossless_bits(mut self, bits: u8) -> Self {
        self.near_lossless_bits = bits;
        self
    }

    /// Set physical pixel dimensions (pHYs chunk).
    ///
    /// Both X and Y must be set for the chunk to be written.
    #[must_use]
    pub fn with_phys(mut self, ppux: u32, ppuy: u32, unit: PhysUnit) -> Self {
        self.pixels_per_unit_x = Some(ppux);
        self.pixels_per_unit_y = Some(ppuy);
        self.phys_unit = Some(unit);
        self
    }

    /// Add a text chunk (tEXt). Can be called multiple times.
    #[must_use]
    pub fn with_text(mut self, keyword: impl Into<String>, text: impl Into<String>) -> Self {
        self.text_chunks.push(TextChunk {
            keyword: keyword.into(),
            text: text.into(),
            compressed: false,
        });
        self
    }

    /// Set last modification time (tIME chunk).
    #[must_use]
    pub fn with_last_modified(mut self, time: PngTime) -> Self {
        self.last_modified = Some(time);
        self
    }

    /// Replace the downcast flag set wholesale.
    #[must_use]
    pub fn with_downcast(mut self, flags: DowncastFlags) -> Self {
        self.downcast = flags;
        self
    }

    /// Build compression options from this config and caller-provided stops.
    pub(crate) fn compress_options<'a>(
        &self,
        cancel: &'a dyn Stop,
        deadline: &'a dyn Stop,
        remaining_ns: Option<&'a dyn Fn() -> Option<u64>>,
    ) -> crate::encoder::CompressOptions<'a> {
        crate::encoder::CompressOptions {
            parallel: self.parallel && self.max_threads != 1,
            cancel,
            deadline,
            remaining_ns,
            max_threads: self.max_threads,
        }
    }
}

/// PNG color type (internal, maps to PNG spec values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // GrayscaleAlpha not yet used in encode, but kept for completeness
pub(crate) enum ColorType {
    Grayscale,
    Rgb,
    GrayscaleAlpha,
    Rgba,
}

impl ColorType {
    /// PNG spec color type byte.
    pub(crate) fn to_png_byte(self) -> u8 {
        match self {
            ColorType::Grayscale => 0,
            ColorType::Rgb => 2,
            ColorType::GrayscaleAlpha => 4,
            ColorType::Rgba => 6,
        }
    }

    /// Number of samples per pixel (color + alpha channels).
    pub(crate) fn channels(self) -> u8 {
        match self {
            ColorType::Grayscale => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
        }
    }
}

/// PNG bit depth (internal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BitDepth {
    Eight,
    Sixteen,
}

impl BitDepth {
    /// PNG spec bit depth value.
    pub(crate) fn to_png_byte(self) -> u8 {
        match self {
            BitDepth::Eight => 8,
            BitDepth::Sixteen => 16,
        }
    }
}

/// Encode RGB8 pixels to PNG.
pub fn encode_rgb8(
    img: ImgRef<Rgb<u8>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw(
        bytes,
        width,
        height,
        ColorType::Rgb,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGBA8 pixels to PNG.
pub fn encode_rgba8(
    img: ImgRef<Rgba<u8>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw(
        bytes,
        width,
        height,
        ColorType::Rgba,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode Gray8 pixels to PNG.
pub fn encode_gray8(
    img: ImgRef<Gray<u8>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: Vec<u8> = buf.iter().map(|g| g.value()).collect();
    encode_raw(
        &bytes,
        width,
        height,
        ColorType::Grayscale,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGB16 pixels to PNG.
///
/// Input samples are native-endian u16. The encoder handles byte-swapping
/// to big-endian as required by the PNG specification.
pub fn encode_rgb16(
    img: ImgRef<Rgb<u16>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    let be = native_to_be_16(bytes);
    encode_raw(
        &be,
        width,
        height,
        ColorType::Rgb,
        BitDepth::Sixteen,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGBA16 pixels to PNG.
///
/// Input samples are native-endian u16. The encoder handles byte-swapping
/// to big-endian as required by the PNG specification.
pub fn encode_rgba16(
    img: ImgRef<Rgba<u16>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    let be = native_to_be_16(bytes);
    encode_raw(
        &be,
        width,
        height,
        ColorType::Rgba,
        BitDepth::Sixteen,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode Gray16 pixels to PNG.
///
/// Input samples are native-endian u16. The encoder handles byte-swapping
/// to big-endian as required by the PNG specification.
pub fn encode_gray16(
    img: ImgRef<Gray<u16>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    let be = native_to_be_16(bytes);
    encode_raw(
        &be,
        width,
        height,
        ColorType::Grayscale,
        BitDepth::Sixteen,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Low-level encode: raw bytes to PNG with metadata and config applied.
///
/// Automatically optimizes color type and bit depth for smallest output:
/// - RGBA → RGB (strip opaque alpha), GrayscaleAlpha, or Grayscale
/// - RGB → Grayscale (when R==G==B)
/// - 16-bit → 8-bit (when all samples fit in 8 bits)
/// - Truecolor → Indexed (when ≤256 unique colors)
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_raw(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: BitDepth,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let effort = config.compression.effort();

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.source_gamma;
    write_meta.srgb_intent = config.srgb_intent;
    write_meta.chromaticities = config.chromaticities;
    // Builder values override the Metadata-derived ones, but only when set —
    // `from_metadata` may already have populated these from `metadata`.
    write_meta.cicp = config.cicp.or(write_meta.cicp);
    write_meta.content_light_level = config
        .content_light_level
        .or(write_meta.content_light_level);
    write_meta.mastering_display = config.mastering_display.or(write_meta.mastering_display);
    write_meta.pixels_per_unit_x = config.pixels_per_unit_x;
    write_meta.pixels_per_unit_y = config.pixels_per_unit_y;
    write_meta.phys_unit = config.phys_unit;
    write_meta.text_chunks.clone_from(&config.text_chunks);
    write_meta.last_modified = config.last_modified;

    let w = width as usize;
    let h = height as usize;

    // Gamut downcast: rewrite a wider-gamut buffer into sRGB primaries
    // when every pixel fits losslessly. Gated on (a) the explicit flag,
    // (b) compression.effort() >= 7 (this pass is more expensive than
    // the byte-level predicates per CLAUDE.md design), (c) 8-bit RGB or
    // RGBA, and (d) a detectable source gamut from cICP. On success the
    // rewritten buffer takes over and wide-gamut metadata is dropped.
    let gamut_bytes;
    let bytes = if config.downcast.gamut_downcast
        && effort >= 7
        && bit_depth == BitDepth::Eight
        && (color_type == ColorType::Rgb || color_type == ColorType::Rgba)
        && let Some(cicp) = write_meta.cicp
        && let Some(src_gamut) = crate::gamut::SourceGamut::from_cicp(cicp)
    {
        let downcast = match color_type {
            ColorType::Rgb => crate::gamut::try_downcast_rgb8_to_srgb(bytes, src_gamut),
            ColorType::Rgba => crate::gamut::try_downcast_rgba8_to_srgb(bytes, src_gamut),
            _ => None,
        };
        match downcast {
            Some(out) => {
                gamut_bytes = out;
                // Re-tag as sRGB: drop wide-gamut signals.
                write_meta.cicp = None;
                write_meta.chromaticities = None;
                write_meta.source_gamma = None;
                if write_meta.srgb_intent.is_none() {
                    // Default to perceptual rendering intent.
                    write_meta.srgb_intent = Some(0);
                }
                &gamut_bytes
            }
            None => bytes,
        }
    } else {
        bytes
    };

    // Near-lossless: quantize LSBs for better compression (8-bit only)
    let nl_bytes;
    let bytes = if config.near_lossless_bits > 0 && bit_depth == BitDepth::Eight {
        let channels: usize = match color_type {
            ColorType::Grayscale => 1,
            ColorType::Rgb => 3,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgba => 4,
        };
        nl_bytes =
            crate::optimize::near_lossless_quantize(bytes, channels, config.near_lossless_bits);
        &nl_bytes
    } else {
        bytes
    };

    // Auto-optimize color type and bit depth (gated on downcast flags).
    let dc = &config.downcast;
    let optimization = match (color_type, bit_depth) {
        (ColorType::Rgba, BitDepth::Eight) => crate::optimize::optimize_rgba8(bytes, w, h, dc),
        (ColorType::Rgb, BitDepth::Eight) => crate::optimize::optimize_rgb8(bytes, w, h, dc),
        (_, BitDepth::Sixteen) => {
            crate::optimize::optimize_16bit(bytes, w, h, color_type.to_png_byte(), dc)
        }
        _ => crate::optimize::OptimalEncoding::Original,
    };

    match optimization {
        crate::optimize::OptimalEncoding::Indexed {
            palette_rgb,
            palette_alpha,
            indices,
        } => {
            let opts = config.compress_options(cancel, deadline, None);
            Ok(crate::encoder::write_indexed_png(
                &indices,
                width,
                height,
                &palette_rgb,
                palette_alpha.as_deref(),
                &write_meta,
                effort,
                opts,
            )?)
        }
        crate::optimize::OptimalEncoding::Truecolor {
            bytes: opt_bytes,
            color_type: opt_ct,
            bit_depth: opt_bd,
            trns,
        } => {
            let opts = config.compress_options(cancel, deadline, None);
            Ok(crate::encoder::write_truecolor_png(
                &opt_bytes,
                width,
                height,
                opt_ct,
                opt_bd,
                trns.as_deref(),
                &write_meta,
                effort,
                opts,
            )?)
        }
        crate::optimize::OptimalEncoding::Original => {
            let opts = config.compress_options(cancel, deadline, None);
            Ok(crate::encoder::write_truecolor_png(
                bytes,
                width,
                height,
                color_type.to_png_byte(),
                bit_depth.to_png_byte(),
                None,
                &write_meta,
                effort,
                opts,
            )?)
        }
    }
}

/// Encode RGB8 pixels to PNG, returning per-phase compression statistics.
#[cfg(feature = "_dev")]
pub fn encode_rgb8_with_stats(
    img: ImgRef<Rgb<u8>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<(Vec<u8>, crate::encoder::PhaseStats)> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw_with_stats(
        bytes,
        width,
        height,
        ColorType::Rgb,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Encode RGBA8 pixels to PNG, returning per-phase compression statistics.
#[cfg(feature = "_dev")]
pub fn encode_rgba8_with_stats(
    img: ImgRef<Rgba<u8>>,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<(Vec<u8>, crate::encoder::PhaseStats)> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, _, _) = img.to_contiguous_buf();
    let bytes: &[u8] = bytemuck::cast_slice(buf.as_ref());
    encode_raw_with_stats(
        bytes,
        width,
        height,
        ColorType::Rgba,
        BitDepth::Eight,
        metadata,
        config,
        cancel,
        deadline,
    )
}

/// Low-level encode with stats: raw bytes to PNG with per-phase timing.
///
/// Applies the same auto-optimization as `encode_raw` (color type reduction,
/// bit depth reduction, auto-indexing).
#[cfg(feature = "_dev")]
#[allow(clippy::too_many_arguments)]
fn encode_raw_with_stats(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: BitDepth,
    metadata: Option<&Metadata>,
    config: &EncodeConfig,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<(Vec<u8>, crate::encoder::PhaseStats)> {
    let effort = config.compression.effort();

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.source_gamma;
    write_meta.srgb_intent = config.srgb_intent;
    write_meta.chromaticities = config.chromaticities;
    // Builder values override the Metadata-derived ones, but only when set —
    // `from_metadata` may already have populated these from `metadata`.
    write_meta.cicp = config.cicp.or(write_meta.cicp);
    write_meta.content_light_level = config
        .content_light_level
        .or(write_meta.content_light_level);
    write_meta.mastering_display = config.mastering_display.or(write_meta.mastering_display);
    write_meta.pixels_per_unit_x = config.pixels_per_unit_x;
    write_meta.pixels_per_unit_y = config.pixels_per_unit_y;
    write_meta.phys_unit = config.phys_unit;
    write_meta.text_chunks.clone_from(&config.text_chunks);
    write_meta.last_modified = config.last_modified;

    let w = width as usize;
    let h = height as usize;

    // Near-lossless: quantize LSBs for better compression (8-bit only)
    let nl_bytes;
    let bytes = if config.near_lossless_bits > 0 && bit_depth == BitDepth::Eight {
        let channels: usize = match color_type {
            ColorType::Grayscale => 1,
            ColorType::Rgb => 3,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgba => 4,
        };
        nl_bytes =
            crate::optimize::near_lossless_quantize(bytes, channels, config.near_lossless_bits);
        &nl_bytes
    } else {
        bytes
    };

    // Determine optimal encoding (same logic as encode_raw)
    let (eff_bytes, eff_ct, eff_bd, eff_trns) = match (color_type, bit_depth) {
        (ColorType::Rgba, BitDepth::Eight) => {
            match crate::optimize::optimize_rgba8(bytes, w, h, &config.downcast) {
                crate::optimize::OptimalEncoding::Truecolor {
                    bytes: ob,
                    color_type: ct,
                    bit_depth: bd,
                    trns,
                } => (Some(ob), ct, bd, trns),
                crate::optimize::OptimalEncoding::Indexed { .. } => {
                    // Stats path doesn't support indexed; fall through to truecolor
                    (
                        None,
                        color_type.to_png_byte(),
                        bit_depth.to_png_byte(),
                        None,
                    )
                }
                crate::optimize::OptimalEncoding::Original => (
                    None,
                    color_type.to_png_byte(),
                    bit_depth.to_png_byte(),
                    None,
                ),
            }
        }
        (ColorType::Rgb, BitDepth::Eight) => {
            match crate::optimize::optimize_rgb8(bytes, w, h, &config.downcast) {
                crate::optimize::OptimalEncoding::Truecolor {
                    bytes: ob,
                    color_type: ct,
                    bit_depth: bd,
                    trns,
                } => (Some(ob), ct, bd, trns),
                _ => (
                    None,
                    color_type.to_png_byte(),
                    bit_depth.to_png_byte(),
                    None,
                ),
            }
        }
        (_, BitDepth::Sixteen) => {
            match crate::optimize::optimize_16bit(
                bytes,
                w,
                h,
                color_type.to_png_byte(),
                &config.downcast,
            ) {
                crate::optimize::OptimalEncoding::Truecolor {
                    bytes: ob,
                    color_type: ct,
                    bit_depth: bd,
                    trns,
                } => (Some(ob), ct, bd, trns),
                _ => (
                    None,
                    color_type.to_png_byte(),
                    bit_depth.to_png_byte(),
                    None,
                ),
            }
        }
        _ => (
            None,
            color_type.to_png_byte(),
            bit_depth.to_png_byte(),
            None,
        ),
    };

    let pixel_data = eff_bytes.as_deref().unwrap_or(bytes);
    let opts = config.compress_options(cancel, deadline, None);

    let mut stats = crate::encoder::PhaseStats::default();
    let png = crate::encoder::write_truecolor_png_with_stats(
        pixel_data,
        width,
        height,
        eff_ct,
        eff_bd,
        eff_trns.as_deref(),
        &write_meta,
        effort,
        opts,
        &mut stats,
    )?;
    Ok((png, stats))
}

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

// ── APNG encoding ───────────────────────────────────────────────────

/// A single frame for APNG encoding.
///
/// Each frame must be canvas-sized RGBA8 pixel data (width * height * 4 bytes).
/// The encoder computes delta regions internally.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ApngFrameInput<'a> {
    /// Canvas-sized RGBA8 pixel data.
    pub pixels: &'a [u8],
    /// Numerator of the frame delay fraction (in seconds).
    pub delay_num: u16,
    /// Denominator of the frame delay fraction.
    /// Per the APNG spec, 0 is treated as 100 (i.e., delay_num/100 seconds).
    pub delay_den: u16,
}

impl<'a> ApngFrameInput<'a> {
    /// Create a new APNG frame input.
    #[must_use]
    pub fn new(pixels: &'a [u8], delay_num: u16, delay_den: u16) -> Self {
        Self {
            pixels,
            delay_num,
            delay_den,
        }
    }
}

/// APNG encode configuration.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ApngEncodeConfig {
    /// PNG compression/filter settings.
    pub encode: EncodeConfig,
    /// Animation loop count. 0 = infinite loop.
    pub num_plays: u32,
}

impl ApngEncodeConfig {
    /// Set the encode configuration.
    #[must_use]
    pub fn with_encode(mut self, encode: EncodeConfig) -> Self {
        self.encode = encode;
        self
    }

    /// Set the loop count (0 = infinite).
    #[must_use]
    pub fn with_num_plays(mut self, num_plays: u32) -> Self {
        self.num_plays = num_plays;
        self
    }
}

/// Encode canvas-sized RGBA8 frames into a truecolor APNG file.
///
/// All frames must be `canvas_width * canvas_height * 4` bytes of RGBA8 data.
/// The encoder computes delta regions between consecutive frames for efficient
/// encoding. At least one frame is required.
pub fn encode_apng(
    frames: &[ApngFrameInput<'_>],
    canvas_width: u32,
    canvas_height: u32,
    config: &ApngEncodeConfig,
    metadata: Option<&Metadata>,
    cancel: &dyn Stop,
    deadline: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    // Validation
    if frames.is_empty() {
        return Err(at!(PngError::InvalidInput(
            "APNG requires at least one frame".into(),
        )));
    }
    let expected_len = canvas_width as usize * canvas_height as usize * 4;
    for (i, frame) in frames.iter().enumerate() {
        if frame.pixels.len() < expected_len {
            return Err(at!(PngError::InvalidInput(alloc::format!(
                "frame {i}: pixel buffer too small: need {expected_len}, got {}",
                frame.pixels.len()
            ))));
        }
    }

    let effort = config.encode.compression.effort();
    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = config.encode.source_gamma;
    write_meta.srgb_intent = config.encode.srgb_intent;
    write_meta.chromaticities = config.encode.chromaticities;
    // Builder values override the Metadata-derived ones, but only when set.
    write_meta.cicp = config.encode.cicp.or(write_meta.cicp);
    write_meta.content_light_level = config
        .encode
        .content_light_level
        .or(write_meta.content_light_level);
    write_meta.mastering_display = config
        .encode
        .mastering_display
        .or(write_meta.mastering_display);
    write_meta.pixels_per_unit_x = config.encode.pixels_per_unit_x;
    write_meta.pixels_per_unit_y = config.encode.pixels_per_unit_y;
    write_meta.phys_unit = config.encode.phys_unit;
    write_meta
        .text_chunks
        .clone_from(&config.encode.text_chunks);
    write_meta.last_modified = config.encode.last_modified;

    crate::encoder::apng::encode_apng_truecolor(
        frames,
        canvas_width,
        canvas_height,
        &write_meta,
        config.num_plays,
        effort,
        cancel,
        deadline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use enough::Unstoppable;
    use imgref::Img;

    // ── EncodeConfig builder tests ──────────────────────────────────

    #[test]
    fn encode_config_defaults() {
        let c = EncodeConfig::default();
        assert_eq!(c.compression, Compression::Balanced);
        assert_eq!(c.filter, Filter::Auto);
        assert!(!c.parallel);
        assert!(c.source_gamma.is_none());
        assert!(c.srgb_intent.is_none());
        assert!(c.chromaticities.is_none());
        assert_eq!(c.near_lossless_bits, 0);
        // Downcast defaults: cheap predicates on, expensive ones off.
        assert!(c.downcast.rgba_to_rgb);
        assert!(c.downcast.rgb_to_gray);
        assert!(c.downcast.sub_byte_gray);
        assert!(c.downcast.indexed);
        assert!(c.downcast.alpha_to_trns);
        assert!(c.downcast.downcast_16_to_8_replicated);
        assert!(!c.downcast.downcast_16_to_8_low_zero);
        assert!(!c.downcast.gamut_downcast);
    }

    #[test]
    fn downcast_flags_none_disables_everything() {
        let f = DowncastFlags::none();
        assert!(!f.rgba_to_rgb);
        assert!(!f.rgb_to_gray);
        assert!(!f.sub_byte_gray);
        assert!(!f.indexed);
        assert!(!f.alpha_to_trns);
        assert!(!f.downcast_16_to_8_replicated);
        assert!(!f.downcast_16_to_8_low_zero);
        assert!(!f.gamut_downcast);
    }

    #[test]
    fn rgba8_with_downcast_flags_none_keeps_rgba() {
        // 4 RGBA pixels, all opaque, all gray — would normally become Gray8.
        // With downcast disabled, must stay RGBA8.
        let pixels: Vec<Rgba<u8>> = (0..4)
            .map(|i| Rgba {
                r: i * 30,
                g: i * 30,
                b: i * 30,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels, 4, 1);
        let config = EncodeConfig::default()
            .with_compression(Compression::Fastest)
            .with_downcast(DowncastFlags::none());
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::default(), &Unstoppable).unwrap();
        // PNG color_type 6 = RGBA, NOT gray.
        assert_eq!(
            decoded.info.color_type, 6,
            "downcast disabled — must stay RGBA"
        );
    }

    #[test]
    fn rgba8_p3_to_srgb_gamut_downcast_at_high_effort() {
        // RGBA8 buffer tagged Display P3 + sRGB transfer, with all pixels
        // landing inside sRGB gamut. With gamut_downcast=true and effort
        // 7+, the encoder should rewrite values into sRGB primaries and
        // emit an sRGB chunk (no cICP).
        use zencodec::{Cicp, Metadata};
        // Pixels chosen to be unambiguously in-sRGB for both interpretations.
        let pixels: Vec<Rgba<u8>> = (0..16)
            .map(|i| Rgba {
                r: 80 + (i * 3) as u8,
                g: 90 + (i * 2) as u8,
                b: 100 + i as u8,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels.clone(), 4, 4);

        let metadata = Metadata::none().with_cicp(Cicp::DISPLAY_P3);

        // Disable indexed/sub-byte to keep the test observation simple.
        let flags = DowncastFlags {
            gamut_downcast: true,
            indexed: false,
            sub_byte_gray: false,
            alpha_to_trns: false,
            rgb_to_gray: false,
            ..Default::default()
        };

        let config = EncodeConfig::default()
            .with_compression(Compression::Fast) // effort 7
            .with_downcast(flags);

        let encoded = encode_rgba8(
            img.as_ref(),
            Some(&metadata),
            &config,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::default(), &Unstoppable).unwrap();
        // After downcast, the file should NOT carry P3 cICP.
        assert!(
            decoded.info.cicp.is_none() || decoded.info.cicp == Some(Cicp::SRGB),
            "expected sRGB tagging after gamut downcast, got cicp={:?}",
            decoded.info.cicp
        );
    }

    #[test]
    fn rgba8_p3_gamut_downcast_disabled_when_effort_too_low() {
        // Same buffer, gamut_downcast on but effort < 7 — must keep P3.
        use zencodec::{Cicp, Metadata};
        let pixels: Vec<Rgba<u8>> = (0..4)
            .map(|i| Rgba {
                r: 100 + i,
                g: 100,
                b: 100,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels, 4, 1);
        let metadata = Metadata::none().with_cicp(Cicp::DISPLAY_P3);
        let flags = DowncastFlags {
            gamut_downcast: true,
            indexed: false,
            sub_byte_gray: false,
            alpha_to_trns: false,
            rgb_to_gray: false,
            ..Default::default()
        };
        let config = EncodeConfig::default()
            .with_compression(Compression::Turbo) // effort 2 < 7
            .with_downcast(flags);
        let encoded = encode_rgba8(
            img.as_ref(),
            Some(&metadata),
            &config,
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::default(), &Unstoppable).unwrap();
        assert_eq!(
            decoded.info.cicp,
            Some(Cicp::DISPLAY_P3),
            "low effort must skip gamut downcast"
        );
    }

    #[test]
    fn rgba8_with_default_flags_downcasts_to_gray() {
        // Same buffer, default flags — should auto-downcast to grayscale.
        let pixels: Vec<Rgba<u8>> = (0..4)
            .map(|i| Rgba {
                r: i * 30,
                g: i * 30,
                b: i * 30,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels, 4, 1);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::default(), &Unstoppable).unwrap();
        // Should be Gray (0) or Indexed (3) — both fine, just not RGBA (6).
        assert!(
            decoded.info.color_type == 0 || decoded.info.color_type == 3,
            "expected gray/indexed downcast, got color_type={}",
            decoded.info.color_type
        );
    }

    #[test]
    fn encode_config_with_filter() {
        let c = EncodeConfig::default().with_filter(Filter::Auto);
        assert_eq!(c.filter, Filter::Auto);
    }

    #[test]
    fn encode_config_with_parallel() {
        let c = EncodeConfig::default().with_parallel(true);
        assert!(c.parallel);
    }

    #[test]
    fn encode_config_with_source_gamma() {
        let c = EncodeConfig::default().with_source_gamma(Some(45455));
        assert_eq!(c.source_gamma, Some(45455));
    }

    #[test]
    fn encode_config_with_srgb_intent() {
        let c = EncodeConfig::default().with_srgb_intent(Some(1));
        assert_eq!(c.srgb_intent, Some(1));
    }

    #[test]
    fn encode_config_with_chromaticities() {
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
        let c = EncodeConfig::default().with_chromaticities(Some(chrm));
        assert_eq!(c.chromaticities.unwrap().red_x, 64000);
    }

    #[test]
    fn encode_config_with_near_lossless_bits() {
        let c = EncodeConfig::default().with_near_lossless_bits(2);
        assert_eq!(c.near_lossless_bits, 2);
    }

    // ── Compression effort mapping ──────────────────────────────────

    #[test]
    fn compression_effort_mapping() {
        assert_eq!(Compression::None.effort(), 0);
        assert_eq!(Compression::Fastest.effort(), 1);
        assert_eq!(Compression::Turbo.effort(), 2);
        assert_eq!(Compression::Fast.effort(), 7);
        assert_eq!(Compression::Balanced.effort(), 13);
        assert_eq!(Compression::Thorough.effort(), 17);
        assert_eq!(Compression::High.effort(), 19);
        assert_eq!(Compression::Aggressive.effort(), 22);
        assert_eq!(Compression::Intense.effort(), 24);
        assert_eq!(Compression::Crush.effort(), 27);
        assert_eq!(Compression::Maniac.effort(), 30);
        assert_eq!(Compression::Brag.effort(), 31);
        assert_eq!(Compression::Minutes.effort(), 200);
        assert_eq!(Compression::Effort(15).effort(), 15);
        assert_eq!(Compression::Effort(500).effort(), 200); // clamped
    }

    // ── Encode roundtrip at various effort levels ───────────────────

    fn small_rgb_image() -> (Vec<Rgb<u8>>, usize, usize) {
        let w = 8;
        let h = 4;
        let pixels: Vec<Rgb<u8>> = (0..w * h)
            .map(|i| Rgb {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 23) as u8,
            })
            .collect();
        (pixels, w, h)
    }

    fn small_rgba_image() -> (Vec<Rgba<u8>>, usize, usize) {
        let w = 8;
        let h = 4;
        let pixels: Vec<Rgba<u8>> = (0..w * h)
            .map(|i| Rgba {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 23) as u8,
                a: if i % 4 == 0 { 0 } else { 255 },
            })
            .collect();
        (pixels, w, h)
    }

    fn roundtrip_rgb(config: &EncodeConfig) {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels.clone(), w, h);
        let encoded = encode_rgb8(img.as_ref(), None, config, &Unstoppable, &Unstoppable).unwrap();
        assert!(encoded[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
        assert_eq!(decoded.info.height, h as u32);
        // Pixel-perfect verification
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_effort_0_store() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::None));
    }

    #[test]
    fn roundtrip_effort_1_fastest() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Fastest));
    }

    #[test]
    fn roundtrip_effort_2_turbo() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Turbo));
    }

    #[test]
    fn roundtrip_effort_7_fast() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Fast));
    }

    #[test]
    fn roundtrip_effort_13_balanced() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Balanced));
    }

    #[test]
    fn roundtrip_effort_17_thorough() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Thorough));
    }

    #[test]
    fn roundtrip_rgba_with_transparency() {
        let (pixels, w, h) = small_rgba_image();
        let img = Img::new(pixels.clone(), w, h);
        let config = EncodeConfig::default().with_compression(Compression::Fast);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
        assert!(decoded.info.has_alpha);
        // Verify pixels: opaque pixels exact, transparent pixels preserve alpha
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        assert_eq!(raw.len(), w * h * 4);
        for (i, px) in pixels.iter().enumerate() {
            let off = i * 4;
            if px.a == 255 {
                assert_eq!(raw[off], px.r, "pixel {i} R");
                assert_eq!(raw[off + 1], px.g, "pixel {i} G");
                assert_eq!(raw[off + 2], px.b, "pixel {i} B");
            }
            assert_eq!(raw[off + 3], px.a, "pixel {i} A");
        }
    }

    #[test]
    fn roundtrip_gray8() {
        let pixels: Vec<Gray<u8>> = (0..32).map(|i| Gray(i * 8)).collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fast);
        let encoded =
            encode_gray8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert!(!decoded.info.has_alpha);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels.iter().map(|p| p.value()).collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_rgb16() {
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| Rgb {
                r: i * 2048 + 1,
                g: i * 1024 + 3,
                b: i * 512 + 7,
            })
            .collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [p.r.to_ne_bytes(), p.g.to_ne_bytes(), p.b.to_ne_bytes()].concat())
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_rgba16() {
        let pixels: Vec<Rgba<u16>> = (0..32)
            .map(|i| Rgba {
                r: i * 2048 + 1,
                g: i * 1024 + 3,
                b: i * 512 + 7,
                a: 65535,
            })
            .collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgba16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| {
                [
                    p.r.to_ne_bytes(),
                    p.g.to_ne_bytes(),
                    p.b.to_ne_bytes(),
                    p.a.to_ne_bytes(),
                ]
                .concat()
            })
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_gray16() {
        let pixels: Vec<Gray<u16>> = (0..32).map(|i| Gray(i * 2048 + 5)).collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_gray16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| p.value().to_ne_bytes())
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn near_lossless_produces_valid_output() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels.clone(), w, h);
        let encoded = encode_rgb8(
            img.as_ref(),
            None,
            &EncodeConfig::default()
                .with_compression(Compression::Fastest)
                .with_near_lossless_bits(3),
            &Unstoppable,
            &Unstoppable,
        )
        .unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
        // With 3 near-lossless bits, max error per sample is 2^2 = 4
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        for (i, px) in pixels.iter().enumerate() {
            let off = i * 3;
            assert!(
                (raw[off] as i16 - px.r as i16).unsigned_abs() <= 4,
                "pixel {i} R"
            );
            assert!(
                (raw[off + 1] as i16 - px.g as i16).unsigned_abs() <= 4,
                "pixel {i} G"
            );
            assert!(
                (raw[off + 2] as i16 - px.b as i16).unsigned_abs() <= 4,
                "pixel {i} B"
            );
        }
    }

    // ── APNG encoding ───────────────────────────────────────────────

    #[test]
    fn apng_frame_input_new() {
        let pixels = vec![0u8; 16 * 16 * 4];
        let frame = ApngFrameInput::new(&pixels, 100, 1000);
        assert_eq!(frame.delay_num, 100);
        assert_eq!(frame.delay_den, 1000);
    }

    #[test]
    fn apng_encode_config_builders() {
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::Fast))
            .with_num_plays(3);
        assert_eq!(config.num_plays, 3);
        assert_eq!(config.encode.compression.effort(), 7);
    }

    #[test]
    fn apng_empty_frames_error() {
        let config = ApngEncodeConfig::default();
        let result = encode_apng(&[], 16, 16, &config, None, &Unstoppable, &Unstoppable);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("at least one frame")
        );
    }

    #[test]
    fn apng_buffer_too_small_error() {
        let config = ApngEncodeConfig::default();
        let small_buf = vec![0u8; 10];
        let frames = [ApngFrameInput::new(&small_buf, 100, 1000)];
        let result = encode_apng(&frames, 16, 16, &config, None, &Unstoppable, &Unstoppable);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));
    }

    #[test]
    fn apng_single_frame_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let pixels = vec![128u8; (w * h * 4) as usize];
        let frames = [ApngFrameInput::new(&pixels, 100, 1000)];
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::Fastest));
        let encoded =
            encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
        assert!(encoded[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        // Decode APNG and verify frame pixels
        let apng =
            crate::decode_apng(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(apng.frames.len(), 1);
        let frame_bytes = apng.frames[0].pixels.copy_to_contiguous_bytes();
        // All pixels are (128, 128, 128, 128)
        for chunk in frame_bytes.chunks(4) {
            assert_eq!(chunk, &[128, 128, 128, 128]);
        }
    }

    // ── Color type and bit depth ────────────────────────────────────

    #[test]
    fn color_type_to_png_byte() {
        assert_eq!(ColorType::Grayscale.to_png_byte(), 0);
        assert_eq!(ColorType::Rgb.to_png_byte(), 2);
        assert_eq!(ColorType::GrayscaleAlpha.to_png_byte(), 4);
        assert_eq!(ColorType::Rgba.to_png_byte(), 6);
    }

    #[test]
    fn bit_depth_to_png_byte() {
        assert_eq!(BitDepth::Eight.to_png_byte(), 8);
        assert_eq!(BitDepth::Sixteen.to_png_byte(), 16);
    }

    // ── Native to BE byte swap ──────────────────────────────────────

    #[test]
    fn native_to_be_16_involution() {
        let original = vec![0x12u8, 0x34, 0x56, 0x78];
        let be = native_to_be_16(&original);
        let back = native_to_be_16(&be);
        assert_eq!(original, back);
    }

    // ── Parallel encoding ───────────────────────────────────────────

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // uses parallel threading
    fn parallel_encoding_produces_valid_png() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default()
            .with_compression(Compression::Balanced)
            .with_parallel(true);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    // ── Additional effort tiers (cover compress.rs branches) ────────

    #[test]
    fn roundtrip_effort_3() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(3)));
    }

    #[test]
    fn roundtrip_effort_4() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(4)));
    }

    #[test]
    fn roundtrip_effort_5() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(5)));
    }

    #[test]
    fn roundtrip_effort_6() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(6)));
    }

    #[test]
    fn roundtrip_effort_8() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(8)));
    }

    #[test]
    fn roundtrip_effort_9() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(9)));
    }

    #[test]
    fn roundtrip_effort_10() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(10)));
    }

    #[test]
    fn roundtrip_effort_11() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(11)));
    }

    #[test]
    fn roundtrip_effort_12() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(12)));
    }

    #[test]
    fn roundtrip_effort_14() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(14)));
    }

    #[test]
    fn roundtrip_effort_15() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(15)));
    }

    #[test]
    fn roundtrip_effort_16() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Effort(16)));
    }

    #[test]
    fn roundtrip_effort_19_high() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::High));
    }

    #[test]
    #[ignore = "high-effort compression; run with --ignored"]
    fn roundtrip_effort_22_aggressive() {
        roundtrip_rgb(&EncodeConfig::default().with_compression(Compression::Aggressive));
    }

    // ── Monotonicity: higher effort must not produce larger output ──

    #[test]
    fn monotonicity_effort_0_through_17() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let mut prev_size = usize::MAX;
        for effort in [0, 1, 2, 7, 13, 17] {
            let config = EncodeConfig::default().with_compression(Compression::Effort(effort));
            let encoded =
                encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
            // At effort 0 (store), output is larger; but from effort 1+, should be monotonic
            if effort > 0 && prev_size < usize::MAX {
                assert!(
                    encoded.len() <= prev_size,
                    "effort {effort} produced {} bytes, worse than previous {} bytes",
                    encoded.len(),
                    prev_size
                );
            }
            if effort > 0 {
                prev_size = encoded.len();
            }
        }
    }

    // ── Near-lossless with RGBA (exercises transparent pixel zeroing) ──

    #[test]
    fn near_lossless_rgba() {
        let (pixels, w, h) = small_rgba_image();
        let img = Img::new(pixels.clone(), w, h);
        let config = EncodeConfig::default()
            .with_compression(Compression::Fast)
            .with_near_lossless_bits(2);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
        // Alpha must be exact; RGB bounded error
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        for (i, px) in pixels.iter().enumerate() {
            let off = i * 4;
            assert_eq!(raw[off + 3], px.a, "pixel {i} alpha must be exact");
            if px.a == 255 {
                assert!(
                    (raw[off] as i16 - px.r as i16).unsigned_abs() <= 4,
                    "pixel {i} R"
                );
                assert!(
                    (raw[off + 1] as i16 - px.g as i16).unsigned_abs() <= 4,
                    "pixel {i} G"
                );
                assert!(
                    (raw[off + 2] as i16 - px.b as i16).unsigned_abs() <= 4,
                    "pixel {i} B"
                );
            }
        }
    }

    // ── Gray near-lossless ──────────────────────────────────────────

    #[test]
    fn near_lossless_gray() {
        let pixels: Vec<Gray<u8>> = (0..32).map(|i| Gray(i * 8)).collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default()
            .with_compression(Compression::Fast)
            .with_near_lossless_bits(2);
        let encoded =
            encode_gray8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        for (i, px) in pixels.iter().enumerate() {
            assert!(
                (raw[i] as i16 - px.value() as i16).unsigned_abs() <= 4,
                "pixel {i}"
            );
        }
    }

    // ── Metadata roundtrip ──────────────────────────────────────────

    #[test]
    fn encode_with_gama_chrm() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
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
        let config = EncodeConfig::default()
            .with_compression(Compression::Fastest)
            .with_source_gamma(Some(45455))
            .with_chromaticities(Some(chrm));
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.source_gamma, Some(45455));
        assert_eq!(decoded.info.chromaticities.unwrap().red_x, 64000);
    }

    #[test]
    fn encode_with_srgb_intent() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default()
            .with_compression(Compression::Fastest)
            .with_srgb_intent(Some(0));
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.srgb_intent, Some(0));
    }

    // ── High effort on tiny data (brute-force, fork, beam paths) ────

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // recompression uses thread::scope
    #[ignore = "high-effort compression; run with --ignored"]
    fn roundtrip_effort_24_intense_rgba() {
        let (pixels, w, h) = small_rgba_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Intense);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
        assert_eq!(decoded.info.height, h as u32);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // recompression uses thread::scope
    #[ignore = "high-effort compression; run with --ignored"]
    fn roundtrip_effort_27_crush() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Crush);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // recompression uses thread::scope
    #[ignore = "high-effort compression; run with --ignored"]
    fn roundtrip_effort_30_maniac() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Maniac);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))] // recompression uses thread::scope
    #[ignore = "high-effort compression; run with --ignored"]
    fn roundtrip_effort_31_brag() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Brag);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, w as u32);
    }

    // ── 16-bit encode/decode roundtrips with pixel verification ─────

    #[test]
    fn roundtrip_rgb16_pixel_perfect() {
        // Use values where high_byte != low_byte so optimizer won't reduce to 8-bit
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| Rgb {
                r: i * 2048 + 1, // low byte nonzero → stays 16-bit
                g: i * 1024 + 3,
                b: i * 512 + 7,
            })
            .collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Balanced);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 16);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [p.r.to_ne_bytes(), p.g.to_ne_bytes(), p.b.to_ne_bytes()].concat())
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_rgba16_pixel_perfect() {
        let pixels: Vec<Rgba<u16>> = (0..32)
            .map(|i| Rgba {
                r: i * 2048 + 1,
                g: i * 1024 + 3,
                b: i * 512 + 7,
                a: 65535 - i * 1000,
            })
            .collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Balanced);
        let encoded =
            encode_rgba16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 16);
        assert!(decoded.info.has_alpha);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| {
                [
                    p.r.to_ne_bytes(),
                    p.g.to_ne_bytes(),
                    p.b.to_ne_bytes(),
                    p.a.to_ne_bytes(),
                ]
                .concat()
            })
            .collect();
        assert_eq!(raw, expected);
    }

    #[test]
    fn roundtrip_gray16_pixel_perfect() {
        let pixels: Vec<Gray<u16>> = (0..32).map(|i| Gray(i * 2048 + 1)).collect();
        let img = Img::new(pixels.clone(), 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Balanced);
        let encoded =
            encode_gray16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 16);
        assert!(!decoded.info.has_alpha);
        let raw = decoded.pixels.copy_to_contiguous_bytes();
        let expected: Vec<u8> = pixels
            .iter()
            .flat_map(|p| p.value().to_ne_bytes())
            .collect();
        assert_eq!(raw, expected);
    }

    // ── 16-bit values that reduce to 8-bit (optimization path) ──────

    #[test]
    fn rgb16_reduces_to_8bit_when_samples_fit() {
        // Bit-replicated u16 values (`u16 = u8 * 0x0101`) — the strict
        // PNG-lossless 16→8 condition. e.g. value 0x0808 → BE [0x08, 0x08]
        // → reducible to 8-bit 0x08, decoder reconstructs 0x0808 via
        // bit-replication. Values that are mere `u8 << 8` (low byte 0)
        // are rejected by the default `downcast_16_to_8_replicated` flag.
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| {
                let v = i * 0x0101; // bit-replicated u8 → u16
                Rgb { r: v, g: 0, b: 0 }
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert_eq!(decoded.info.bit_depth, 8);
    }

    // ── RGBA fully opaque reduces to RGB ────────────────────────────

    #[test]
    fn rgba8_opaque_reduces_to_rgb() {
        let pixels: Vec<Rgba<u8>> = (0..32)
            .map(|i| Rgba {
                r: (i * 7) as u8,
                g: (i * 13) as u8,
                b: (i * 23) as u8,
                a: 255,
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        assert!(!decoded.info.has_alpha);
    }

    // ── Grayscale detection ─────────────────────────────────────────

    #[test]
    fn rgb8_grayscale_reduces_to_gray() {
        let pixels: Vec<Rgb<u8>> = (0..32)
            .map(|i| {
                let v = (i * 8) as u8;
                Rgb { r: v, g: v, b: v }
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
        // Should be decoded as grayscale
        assert!(!decoded.info.has_alpha);
    }

    // ── Small exact palette triggers indexed ─────────────────────────

    #[test]
    fn rgba8_few_colors_uses_indexed() {
        // 4 unique colors → should auto-index
        let pixels: Vec<Rgba<u8>> = (0..64)
            .map(|i| match i % 4 {
                0 => Rgba {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                1 => Rgba {
                    r: 0,
                    g: 255,
                    b: 0,
                    a: 255,
                },
                2 => Rgba {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255,
                },
                _ => Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            })
            .collect();
        let img = Img::new(pixels, 8, 8);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded =
            encode_rgba8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    // ── High effort on gray8 (low bpp brute-force path) ─────────────

    #[test]
    fn gray8_high_effort_roundtrip() {
        let pixels: Vec<Gray<u8>> = (0..64).map(|i| Gray((i * 4) as u8)).collect();
        let img = Img::new(pixels, 8, 8);
        let config = EncodeConfig::default().with_compression(Compression::High);
        let encoded =
            encode_gray8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    // ── 16-bit with high effort ─────────────────────────────────────

    #[test]
    fn rgb16_high_effort_roundtrip() {
        let pixels: Vec<Rgb<u16>> = (0..32)
            .map(|i| Rgb {
                r: i * 2048,
                g: i * 1024,
                b: i * 512,
            })
            .collect();
        let img = Img::new(pixels, 8, 4);
        let config = EncodeConfig::default().with_compression(Compression::High);
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.info.width, 8);
    }

    // ── Multi-frame APNG roundtrip ──────────────────────────────────

    #[test]
    fn apng_two_frame_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let sz = (w * h * 4) as usize;
        let frame0: Vec<u8> = (0..sz).map(|i| (i * 3) as u8).collect();
        let frame1: Vec<u8> = (0..sz).map(|i| (i * 7 + 1) as u8).collect();
        let frames = [
            ApngFrameInput::new(&frame0, 1, 30),
            ApngFrameInput::new(&frame1, 1, 30),
        ];
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::Fastest));
        let encoded =
            encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode_apng(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.info.width, w);
        assert_eq!(decoded.info.height, h);
        // Verify frame pixel data
        let f0 = decoded.frames[0].pixels.copy_to_contiguous_bytes();
        let f1 = decoded.frames[1].pixels.copy_to_contiguous_bytes();
        assert_eq!(f0, frame0);
        assert_eq!(f1, frame1);
    }

    #[test]
    fn apng_high_effort_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let sz = (w * h * 4) as usize;
        let frame0 = vec![128u8; sz];
        let frame1 = vec![64u8; sz];
        let frames = [
            ApngFrameInput::new(&frame0, 1, 10),
            ApngFrameInput::new(&frame1, 1, 10),
        ];
        let config = ApngEncodeConfig::default()
            .with_encode(EncodeConfig::default().with_compression(Compression::High));
        let encoded =
            encode_apng(&frames, w, h, &config, None, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode_apng(&encoded, &crate::PngDecodeConfig::strict(), &Unstoppable).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        let f0 = decoded.frames[0].pixels.copy_to_contiguous_bytes();
        let f1 = decoded.frames[1].pixels.copy_to_contiguous_bytes();
        assert_eq!(f0, frame0);
        assert_eq!(f1, frame1);
    }

    // ── Non-animated PNG through decode_apng ────────────────────────

    #[test]
    fn decode_apng_on_static_png() {
        let (pixels, w, h) = small_rgb_image();
        let img = Img::new(pixels, w, h);
        let config = EncodeConfig::default().with_compression(Compression::Fastest);
        let encoded = encode_rgb8(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode_apng(&encoded, &crate::PngDecodeConfig::none(), &Unstoppable).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert_eq!(decoded.info.width, w as u32);
        assert_eq!(decoded.num_plays, 0);
    }

    #[test]
    fn png3_hdr_cicp_clli_mdcv_16bit_roundtrip() {
        use zencodec::{Cicp, ContentLightLevel, MasteringDisplay};
        // 4x4 16-bit RGB with HDR-range samples.
        let (w, h) = (4usize, 4usize);
        let pixels: Vec<Rgb<u16>> = (0..(w * h) as u16)
            .map(|i| Rgb {
                r: i.wrapping_mul(4096),
                g: i.wrapping_mul(2048),
                b: 60000u16.wrapping_sub(i.wrapping_mul(3000)),
            })
            .collect();
        let img = Img::new(pixels, w, h);
        let clli = ContentLightLevel::new(1000, 400);
        let config = EncodeConfig::default()
            .with_cicp(Some(Cicp::BT2100_PQ))
            .with_content_light_level(Some(clli))
            .with_mastering_display(Some(MasteringDisplay::HDR10_REFERENCE));
        let encoded =
            encode_rgb16(img.as_ref(), None, &config, &Unstoppable, &Unstoppable).unwrap();
        let decoded =
            crate::decode(&encoded, &crate::PngDecodeConfig::default(), &Unstoppable).unwrap();
        // 16-bit HDR samples preserved (no downcast).
        assert_eq!(decoded.info.bit_depth, 16);
        // BT.2100 PQ signaling survives; PNG forces matrix-coefficients = 0 (RGB model).
        assert_eq!(decoded.info.cicp, Some(Cicp::new(9, 16, 0, true)));
        // HDR light-level + mastering-display chunks wired through the public encode API.
        assert_eq!(decoded.info.content_light_level, Some(clli));
        assert!(decoded.info.mastering_display.is_some());
    }
}
