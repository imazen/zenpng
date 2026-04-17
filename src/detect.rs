//! PNG source analysis and re-encoding recommendations.
//!
//! PNG is lossless, so there is no "quality level" to detect. Instead, this
//! module analyzes the PNG structure to determine:
//!
//! - **Color type and bit depth** — can we reduce it?
//! - **Interlacing** — Adam7 hurts compression ratio
//! - **Palette usage** — is indexed encoding possible/beneficial?
//! - **Compression efficiency** — is re-encoding likely to reduce size?
//! - **Encoder hints** — text chunks may identify the tool that created it
//!
//! # Example
//!
//! ```rust,ignore
//! use zenpng::detect::{probe, CompressionAssessment};
//!
//! let png_data = std::fs::read("image.png").unwrap();
//! let info = probe(&png_data).unwrap();
//!
//! println!("Color type: {:?}, bit depth: {}", info.color_type, info.bit_depth);
//!
//! if let Some(tool) = &info.creating_tool {
//!     println!("Created by: {}", tool);
//! }
//!
//! match info.compression_assessment {
//!     CompressionAssessment::Optimal => println!("Already well-compressed"),
//!     CompressionAssessment::Improvable { estimated_saving_pct } => {
//!         println!("Could save ~{:.0}% with better compression", estimated_saving_pct);
//!     }
//! }
//! ```

use alloc::string::String;
use alloc::vec::Vec;

/// Result of probing a PNG file.
#[derive(Debug, Clone)]
pub struct PngProbe {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// PNG color type.
    pub color_type: ColorType,
    /// Bits per channel (1, 2, 4, 8, or 16).
    pub bit_depth: u8,
    /// Whether the image has alpha (color type 4/6 or tRNS chunk).
    pub has_alpha: bool,
    /// Whether the image uses Adam7 interlacing.
    pub interlaced: bool,
    /// What kind of image sequence the file contains.
    pub sequence: zencodec::ImageSequence,
    /// Number of palette entries (0 if not indexed).
    pub palette_size: u16,
    /// Software/tool that created this PNG, if detectable.
    pub creating_tool: Option<String>,
    /// Total IDAT compressed data size in bytes.
    pub compressed_data_size: u64,
    /// Total raw (uncompressed) image data size in bytes.
    pub raw_data_size: u64,
    /// Compression ratio (compressed / raw). Lower = better compression.
    pub compression_ratio: f32,
    /// Assessment of whether re-encoding could improve compression.
    pub compression_assessment: CompressionAssessment,
    /// Recommendations for re-encoding.
    pub recommendations: Vec<Recommendation>,
}

/// PNG color type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    /// Grayscale (type 0).
    Grayscale,
    /// RGB truecolor (type 2).
    Rgb,
    /// Indexed / palette (type 3).
    Indexed,
    /// Grayscale + alpha (type 4).
    GrayscaleAlpha,
    /// RGBA truecolor + alpha (type 6).
    Rgba,
}

impl ColorType {
    fn from_png(ct: u8) -> Self {
        match ct {
            0 => Self::Grayscale,
            2 => Self::Rgb,
            3 => Self::Indexed,
            4 => Self::GrayscaleAlpha,
            6 => Self::Rgba,
            _ => Self::Rgb, // fallback
        }
    }

    /// Channels for this color type.
    fn channels(self) -> u8 {
        match self {
            Self::Grayscale => 1,
            Self::Rgb => 3,
            Self::Indexed => 1,
            Self::GrayscaleAlpha => 2,
            Self::Rgba => 4,
        }
    }
}

/// How well-compressed this PNG is relative to what zenpng can achieve.
#[derive(Debug, Clone)]
pub enum CompressionAssessment {
    /// Already well-compressed — re-encoding is unlikely to help much (<5%).
    Optimal,
    /// Re-encoding could reduce file size.
    Improvable {
        /// Estimated percentage reduction achievable (0-100).
        estimated_saving_pct: f32,
    },
}

/// Re-encoding recommendation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recommendation {
    /// Image uses Adam7 interlacing which hurts compression.
    /// Non-interlaced re-encoding will be smaller.
    RemoveInterlacing,
    /// RGBA image with no transparent pixels — could be RGB.
    DropUnusedAlpha,
    /// 16-bit image that could be 8-bit without precision loss.
    ReduceBitDepth,
    /// Truecolor image with few unique colors — indexed would be smaller.
    ConvertToIndexed {
        /// Estimated unique color count (0 = unknown).
        estimated_colors: u32,
    },
    /// Already optimally compressed — no action needed.
    AlreadyOptimal,
}

/// Errors that can occur during PNG probing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProbeError {
    /// Data is too short to be a PNG file.
    TooShort,
    /// Missing PNG signature.
    NotPng,
    /// PNG structure is truncated or malformed.
    Truncated,
}

impl core::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "data too short to be a PNG file"),
            Self::NotPng => write!(f, "not a PNG file (missing signature)"),
            Self::Truncated => write!(f, "truncated PNG file"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// PNG signature bytes.
const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Probe a PNG file from its raw bytes.
///
/// Parses the chunk structure to extract image properties, identify the
/// creating tool, measure compression ratio, and recommend re-encoding
/// strategies. No pixel decoding is performed.
pub fn probe(data: &[u8]) -> Result<PngProbe, ProbeError> {
    if data.len() < 8 {
        return Err(ProbeError::TooShort);
    }
    if data[..8] != PNG_SIG {
        return Err(ProbeError::NotPng);
    }

    // Parse IHDR
    if data.len() < 8 + 8 + 13 {
        return Err(ProbeError::Truncated);
    }
    let ihdr_len = u32::from_be_bytes(data[8..12].try_into().unwrap()) as usize;
    if &data[12..16] != b"IHDR" || ihdr_len != 13 || data.len() < 33 {
        return Err(ProbeError::Truncated);
    }
    let ihdr_data = &data[16..29];
    let width = u32::from_be_bytes(ihdr_data[0..4].try_into().unwrap());
    let height = u32::from_be_bytes(ihdr_data[4..8].try_into().unwrap());
    let bit_depth = ihdr_data[8];
    let color_type_raw = ihdr_data[9];
    let interlace = ihdr_data[12];

    let color_type = ColorType::from_png(color_type_raw);
    let interlaced = interlace == 1;

    // Scan chunks for metadata
    let mut creating_tool: Option<String> = None;
    let mut idat_total: u64 = 0;
    let mut has_trns = false;
    let mut palette_size: u16 = 0;
    let mut sequence = zencodec::ImageSequence::Single;

    let mut pos = 8; // skip PNG signature
    while pos + 12 <= data.len() {
        let chunk_len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let chunk_type = &data[pos + 4..pos + 8];
        let chunk_data_start = pos + 8;
        let chunk_data_end = (chunk_data_start + chunk_len).min(data.len());

        match chunk_type {
            b"IDAT" | b"fdAT" => {
                idat_total += chunk_len as u64;
            }
            b"PLTE" => {
                palette_size = (chunk_len / 3) as u16;
            }
            b"tRNS" => {
                has_trns = true;
            }
            b"acTL" => {
                let frame_count = if chunk_data_end - chunk_data_start >= 4 {
                    Some(u32::from_be_bytes(
                        data[chunk_data_start..chunk_data_start + 4]
                            .try_into()
                            .unwrap(),
                    ))
                } else {
                    None
                };
                sequence = zencodec::ImageSequence::Animation {
                    frame_count,
                    loop_count: None,
                    random_access: false,
                };
            }
            b"tEXt"
                // Parse tEXt: keyword\0value
                if chunk_data_end > chunk_data_start =>
            {
                let chunk_bytes = &data[chunk_data_start..chunk_data_end];
                if let Some(null_pos) = memchr::memchr(0, chunk_bytes) {
                    let keyword = core::str::from_utf8(&chunk_bytes[..null_pos]).ok();
                    let value_bytes = &chunk_bytes[null_pos + 1..];
                    let value = core::str::from_utf8(value_bytes).ok();

                    if let (Some(kw), Some(val)) = (keyword, value)
                        && (kw == "Software" || kw == "Creator" || kw == "Comment")
                        && creating_tool.is_none()
                    {
                        creating_tool = Some(String::from(val));
                    }
                }
            }
            b"iTXt"
                // Parse iTXt: keyword\0compression_flag\0method\0lang\0translated_kw\0text
                if chunk_data_end > chunk_data_start =>
            {
                let chunk_bytes = &data[chunk_data_start..chunk_data_end];
                if let Some(null_pos) = memchr::memchr(0, chunk_bytes) {
                    let keyword = core::str::from_utf8(&chunk_bytes[..null_pos]).ok();
                    if let Some(kw) = keyword
                        && (kw == "Software" || kw == "Creator")
                        && creating_tool.is_none()
                    {
                        // Skip compression_flag, method, lang_tag\0, translated_kw\0
                        let rest = &chunk_bytes[null_pos + 1..];
                        if rest.len() >= 2 {
                            let after_method = &rest[2..];
                            // Skip lang_tag\0
                            if let Some(p1) = memchr::memchr(0, after_method) {
                                let after_lang = &after_method[p1 + 1..];
                                // Skip translated_keyword\0
                                if let Some(p2) = memchr::memchr(0, after_lang) {
                                    let text = &after_lang[p2 + 1..];
                                    if let Ok(s) = core::str::from_utf8(text) {
                                        creating_tool = Some(String::from(s));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        // Advance: length + type(4) + data(chunk_len) + crc(4)
        pos = chunk_data_start + chunk_len + 4;
    }

    let has_alpha =
        color_type == ColorType::GrayscaleAlpha || color_type == ColorType::Rgba || has_trns;

    Ok(assemble_probe(ProbeInputs {
        width,
        height,
        color_type,
        bit_depth,
        has_alpha,
        has_trns,
        interlaced,
        sequence,
        palette_size,
        creating_tool,
        idat_total,
    }))
}

/// Fields pulled together from a chunk scan or from decoder state, used to
/// build a [`PngProbe`] via the shared heuristics.
struct ProbeInputs {
    width: u32,
    height: u32,
    color_type: ColorType,
    bit_depth: u8,
    has_alpha: bool,
    has_trns: bool,
    interlaced: bool,
    sequence: zencodec::ImageSequence,
    palette_size: u16,
    creating_tool: Option<String>,
    idat_total: u64,
}

fn assemble_probe(inp: ProbeInputs) -> PngProbe {
    // Calculate raw data size
    let channels = if inp.has_trns && inp.color_type == ColorType::Indexed {
        1 // indexed stays 1 channel even with tRNS
    } else {
        inp.color_type.channels()
    };
    let raw_data_size =
        inp.width as u64 * inp.height as u64 * channels as u64 * (inp.bit_depth.max(8) as u64 / 8);

    let compression_ratio = if raw_data_size > 0 {
        inp.idat_total as f32 / raw_data_size as f32
    } else {
        1.0
    };

    // Assess compression quality
    // Well-optimized PNGs typically achieve 0.3-0.7 ratio for photos,
    // 0.05-0.2 for screenshots/drawings. We consider anything below
    // ~0.15 as "likely already optimized with a good tool."
    let compression_assessment = if compression_ratio < 0.15 {
        CompressionAssessment::Optimal
    } else {
        // Rough estimate: zenpng at high effort can typically achieve
        // 10-30% better compression than average tools
        let estimated_saving = match compression_ratio {
            r if r > 0.8 => 25.0, // Poorly compressed — big gains likely
            r if r > 0.5 => 15.0, // Average compression
            r if r > 0.3 => 8.0,  // Decent compression
            _ => 3.0,             // Already pretty good
        };
        CompressionAssessment::Improvable {
            estimated_saving_pct: estimated_saving,
        }
    };

    // Build recommendations
    let mut recommendations = Vec::new();

    if inp.interlaced {
        recommendations.push(Recommendation::RemoveInterlacing);
    }

    if inp.color_type == ColorType::Rgba && !inp.has_trns {
        // Could potentially drop alpha if all pixels are opaque
        // (we can't verify without decoding, so this is a suggestion)
        recommendations.push(Recommendation::DropUnusedAlpha);
    }

    if inp.bit_depth == 16 {
        recommendations.push(Recommendation::ReduceBitDepth);
    }

    if recommendations.is_empty()
        && matches!(compression_assessment, CompressionAssessment::Optimal)
    {
        recommendations.push(Recommendation::AlreadyOptimal);
    }

    PngProbe {
        width: inp.width,
        height: inp.height,
        color_type: inp.color_type,
        bit_depth: inp.bit_depth,
        has_alpha: inp.has_alpha,
        interlaced: inp.interlaced,
        sequence: inp.sequence,
        palette_size: inp.palette_size,
        creating_tool: inp.creating_tool,
        compressed_data_size: inp.idat_total,
        raw_data_size,
        compression_ratio,
        compression_assessment,
        recommendations,
    }
}

impl PngProbe {
    /// Construct a `PngProbe` from decoder-produced [`crate::decode::PngInfo`]
    /// without a second chunk-level scan.
    ///
    /// `PngInfo` already carries every input the probe heuristics need
    /// (dimensions, color type, bit depth, interlace, alpha/sequence,
    /// palette size, compressed data size, creating-tool). This entry point
    /// is used inside the main decode path to avoid redoing the work that
    /// [`probe`] performs on its own. External callers that only have
    /// raw PNG bytes should keep using [`probe`].
    pub fn from_info(info: &crate::decode::PngInfo) -> Self {
        let color_type = ColorType::from_png(info.color_type);
        // tRNS presence can be inferred: `has_alpha` is set when the color
        // type has intrinsic alpha (4/6) or when a tRNS chunk was observed.
        // For color types without intrinsic alpha, `has_alpha` therefore
        // equals `has_trns`. For color types 4/6, tRNS is irrelevant to the
        // downstream heuristics (raw_data_size channels, DropUnusedAlpha).
        let intrinsic_alpha = info.color_type == 4 || info.color_type == 6;
        let has_trns = info.has_alpha && !intrinsic_alpha;
        assemble_probe(ProbeInputs {
            width: info.width,
            height: info.height,
            color_type,
            bit_depth: info.bit_depth,
            has_alpha: info.has_alpha,
            has_trns,
            interlaced: info.interlaced,
            sequence: info.sequence.clone(),
            palette_size: info.palette_size.unwrap_or(0),
            creating_tool: info.creating_tool.clone(),
            idat_total: info.compressed_data_size,
        })
    }

    /// Whether re-encoding is likely to produce a smaller file.
    pub fn is_improvable(&self) -> bool {
        matches!(
            self.compression_assessment,
            CompressionAssessment::Improvable { .. }
        )
    }

    /// Recommended zenpng compression effort for re-encoding.
    ///
    /// Returns a higher effort level when the source is already well-compressed
    /// (need to work harder to beat it), and lower effort when the source
    /// is poorly compressed (easy wins available at any effort).
    pub fn recommended_effort(&self) -> u32 {
        match self.compression_ratio {
            r if r > 0.7 => 7,  // Poorly compressed — even low effort wins
            r if r > 0.4 => 13, // Average — balanced effort
            r if r > 0.2 => 19, // Well compressed — need high effort
            _ => 27,            // Very well compressed — need crush level
        }
    }

    /// Bits per pixel (all channels combined).
    ///
    /// PNG24 = 24, PNG32 = 32, PNG48 = 48, PNG64 = 64,
    /// indexed = 8 (1/2/4/8 depending on bit depth),
    /// grayscale = 8 or 16.
    pub fn bits_per_pixel(&self) -> u16 {
        self.color_type.channels() as u16 * self.bit_depth as u16
    }
}

impl zencodec::SourceEncodingDetails for PngProbe {
    fn source_generic_quality(&self) -> Option<f32> {
        None
    }

    fn is_lossless(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_too_short() {
        assert_eq!(probe(&[]).unwrap_err(), ProbeError::TooShort);
        assert_eq!(probe(&[0; 7]).unwrap_err(), ProbeError::TooShort);
    }

    #[test]
    fn test_probe_not_png() {
        assert_eq!(probe(&[0; 32]).unwrap_err(), ProbeError::NotPng);
    }

    #[test]
    fn test_color_type_channels() {
        assert_eq!(ColorType::Grayscale.channels(), 1);
        assert_eq!(ColorType::Rgb.channels(), 3);
        assert_eq!(ColorType::Indexed.channels(), 1);
        assert_eq!(ColorType::GrayscaleAlpha.channels(), 2);
        assert_eq!(ColorType::Rgba.channels(), 4);
    }
}
