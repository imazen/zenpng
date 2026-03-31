//! Ancillary PNG chunk metadata collection.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use zenflate::Unstoppable;

use super::ChunkRef;
use crate::decode::{PngBackground, PngTime, SignificantBits, TextChunk};
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

// ── fcTL frame control ──────────────────────────────────────────────

/// Parsed fcTL (frame control) chunk for APNG.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameControl {
    #[allow(dead_code)]
    pub sequence_number: u32,
    pub width: u32,
    pub height: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub delay_num: u16,
    pub delay_den: u16,
    pub dispose_op: u8,
    pub blend_op: u8,
}

impl FrameControl {
    /// Parse from 26-byte fcTL chunk data.
    /// Validates dimensions fit within the canvas defined by `canvas_width` × `canvas_height`.
    pub fn parse(data: &[u8], canvas_width: u32, canvas_height: u32) -> crate::error::Result<Self> {
        if data.len() != 26 {
            return Err(at!(PngError::Decode(alloc::format!(
                "fcTL chunk is {} bytes, expected 26",
                data.len()
            ))));
        }

        let sequence_number = u32::from_be_bytes(data[0..4].try_into().unwrap());
        let width = u32::from_be_bytes(data[4..8].try_into().unwrap());
        let height = u32::from_be_bytes(data[8..12].try_into().unwrap());
        let x_offset = u32::from_be_bytes(data[12..16].try_into().unwrap());
        let y_offset = u32::from_be_bytes(data[16..20].try_into().unwrap());
        let delay_num = u16::from_be_bytes(data[20..22].try_into().unwrap());
        let delay_den = u16::from_be_bytes(data[22..24].try_into().unwrap());
        let dispose_op = data[24];
        let blend_op = data[25];

        if width == 0 || height == 0 {
            return Err(at!(PngError::Decode("fcTL: zero frame dimension".into())));
        }
        if x_offset.checked_add(width).is_none_or(|v| v > canvas_width) {
            return Err(at!(PngError::Decode(alloc::format!(
                "fcTL: x_offset({x_offset}) + width({width}) exceeds canvas width({canvas_width})"
            ))));
        }
        if y_offset
            .checked_add(height)
            .is_none_or(|v| v > canvas_height)
        {
            return Err(at!(PngError::Decode(alloc::format!(
                "fcTL: y_offset({y_offset}) + height({height}) exceeds canvas height({canvas_height})"
            ))));
        }
        if dispose_op > 2 {
            return Err(at!(PngError::Decode(alloc::format!(
                "fcTL: invalid dispose_op {dispose_op}"
            ))));
        }
        if blend_op > 1 {
            return Err(at!(PngError::Decode(alloc::format!(
                "fcTL: invalid blend_op {blend_op}"
            ))));
        }

        Ok(Self {
            sequence_number,
            width,
            height,
            x_offset,
            y_offset,
            delay_num,
            delay_den,
            dispose_op,
            blend_op,
        })
    }

    /// Frame delay in milliseconds.
    /// Per APNG spec, if delay_den is 0 it is treated as 100.
    pub fn delay_ms(&self) -> u32 {
        let den = if self.delay_den == 0 {
            100
        } else {
            self.delay_den as u32
        };
        (self.delay_num as u32 * 1000 + den / 2) / den
    }
}

/// Collected ancillary chunk data.
#[derive(Clone, Debug, Default)]
pub(crate) struct PngAncillary {
    /// PLTE palette entries (R, G, B triples).
    pub palette: Option<Vec<u8>>,
    /// tRNS transparency data (raw bytes, interpretation depends on color type).
    pub trns: Option<Vec<u8>>,
    /// Decompressed ICC profile from iCCP chunk.
    pub icc_profile: Option<Vec<u8>>,
    /// gAMA value (scaled by 100000).
    pub gamma: Option<u32>,
    /// sRGB rendering intent (0-3).
    pub srgb_intent: Option<u8>,
    /// cHRM chromaticities (8 i32 values: wx, wy, rx, ry, gx, gy, bx, by).
    /// Signed to support wide-gamut spaces with imaginary primaries.
    pub chrm: Option<[i32; 8]>,
    /// cICP: colour primaries, transfer function, matrix coeffs, full range flag.
    pub cicp: Option<[u8; 4]>,
    /// cLLi: max content light level, max frame average light level (u32 each).
    pub clli: Option<[u32; 2]>,
    /// mDCV: mastering display color volume (raw 24 bytes).
    pub mdcv: Option<Vec<u8>>,
    /// eXIf: raw EXIF data.
    pub exif: Option<Vec<u8>>,
    /// XMP from iTXt chunk with keyword "XML:com.adobe.xmp".
    pub xmp: Option<Vec<u8>>,
    /// acTL animation control (num_frames, num_plays).
    pub actl: Option<(u32, u32)>,
    /// pHYs: pixels per unit X, pixels per unit Y, unit specifier.
    pub phys: Option<(u32, u32, u8)>,
    /// tEXt and zTXt text chunks (non-XMP).
    pub text_chunks: Vec<TextChunk>,
    /// bKGD background color.
    pub background: Option<PngBackground>,
    /// tIME last modification time.
    pub last_modified: Option<PngTime>,
    /// sBIT significant bits per channel.
    pub significant_bits: Option<SignificantBits>,
}

impl PngAncillary {
    /// Collect metadata from a single chunk. Returns true if this is an IDAT chunk
    /// (signals the caller to stop collecting pre-IDAT metadata).
    pub fn collect(&mut self, chunk: &ChunkRef<'_>) -> crate::error::Result<bool> {
        match &chunk.chunk_type {
            b"IDAT" => return Ok(true),
            b"PLTE" => {
                if !chunk.data.len().is_multiple_of(3) || chunk.data.is_empty() {
                    return Err(at!(PngError::Decode("invalid PLTE chunk length".into())));
                }
                self.palette = Some(chunk.data.to_vec());
            }
            b"tRNS" => {
                if !chunk.data.is_empty() {
                    // For indexed color, tRNS must not exceed palette entries.
                    // If oversized, discard data but preserve the chunk's presence
                    // so we still output RGBA format (with all alpha=255).
                    if let Some(ref palette) = self.palette {
                        let max_entries = palette.len() / 3;
                        if chunk.data.len() > max_entries {
                            self.trns = Some(Vec::new());
                        } else {
                            self.trns = Some(chunk.data.to_vec());
                        }
                    } else {
                        self.trns = Some(chunk.data.to_vec());
                    }
                }
            }
            b"iCCP" => {
                // iCCP is ancillary — ignore parse failures (e.g., broken profiles)
                let _ = self.parse_iccp(chunk.data);
            }
            b"gAMA" => {
                if chunk.data.len() == 4 {
                    self.gamma = Some(u32::from_be_bytes(chunk.data[..4].try_into().unwrap()));
                }
            }
            b"sRGB" => {
                if !chunk.data.is_empty() {
                    self.srgb_intent = Some(chunk.data[0]);
                }
            }
            b"cHRM" => {
                if chunk.data.len() == 32 {
                    let mut vals = [0i32; 8];
                    for (i, v) in vals.iter_mut().enumerate() {
                        *v = i32::from_be_bytes(chunk.data[i * 4..(i + 1) * 4].try_into().unwrap());
                    }
                    self.chrm = Some(vals);
                }
            }
            b"cICP" => {
                if chunk.data.len() == 4 {
                    self.cicp = Some(chunk.data[..4].try_into().unwrap());
                }
            }
            b"cLLI" => {
                if chunk.data.len() == 8 {
                    let max_cll = u32::from_be_bytes(chunk.data[0..4].try_into().unwrap());
                    let max_fall = u32::from_be_bytes(chunk.data[4..8].try_into().unwrap());
                    self.clli = Some([max_cll, max_fall]);
                }
            }
            b"mDCV" => {
                if chunk.data.len() == 24 {
                    self.mdcv = Some(chunk.data.to_vec());
                }
            }
            b"eXIf" => {
                self.exif = Some(chunk.data.to_vec());
            }
            b"iTXt" => {
                self.try_parse_xmp(chunk.data);
            }
            b"acTL" => {
                if chunk.data.len() == 8 {
                    let num_frames = u32::from_be_bytes(chunk.data[0..4].try_into().unwrap());
                    let num_plays = u32::from_be_bytes(chunk.data[4..8].try_into().unwrap());
                    if num_frames == 0 {
                        return Err(at!(PngError::Decode("acTL: num_frames must be > 0".into())));
                    }
                    if num_frames > 65536 {
                        return Err(at!(PngError::LimitExceeded(alloc::format!(
                            "acTL: num_frames {} exceeds limit of 65536",
                            num_frames
                        ))));
                    }
                    self.actl = Some((num_frames, num_plays));
                }
            }
            b"pHYs" => {
                if chunk.data.len() == 9 {
                    let ppux = u32::from_be_bytes(chunk.data[0..4].try_into().unwrap());
                    let ppuy = u32::from_be_bytes(chunk.data[4..8].try_into().unwrap());
                    let unit = chunk.data[8];
                    self.phys = Some((ppux, ppuy, unit));
                }
            }
            b"tEXt" => {
                self.parse_text(chunk.data, false);
            }
            b"zTXt" => {
                self.parse_ztxt(chunk.data);
            }
            b"bKGD" => {
                self.parse_bkgd(chunk.data);
            }
            b"tIME" => {
                if chunk.data.len() == 7 {
                    let year = u16::from_be_bytes(chunk.data[0..2].try_into().unwrap());
                    self.last_modified = Some(PngTime {
                        year,
                        month: chunk.data[2],
                        day: chunk.data[3],
                        hour: chunk.data[4],
                        minute: chunk.data[5],
                        second: chunk.data[6],
                    });
                }
            }
            b"sBIT" => {
                self.parse_sbit(chunk.data);
            }
            _ => {} // ignore unknown chunks
        }
        Ok(false)
    }

    /// Parse iCCP chunk: null-terminated profile name, compression method, compressed data.
    fn parse_iccp(&mut self, data: &[u8]) -> crate::error::Result<()> {
        // Find null terminator for profile name
        let null_pos = data
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| at!(PngError::Decode("iCCP: missing profile name terminator".into())))?;

        // Byte after null is compression method (must be 0 = zlib)
        if null_pos + 2 > data.len() {
            return Err(at!(PngError::Decode(
                "iCCP: truncated after profile name".into(),
            )));
        }
        let compression_method = data[null_pos + 1];
        if compression_method != 0 {
            return Err(at!(PngError::Decode(alloc::format!(
                "iCCP: unknown compression method {}",
                compression_method
            ))));
        }

        let compressed = &data[null_pos + 2..];
        if compressed.is_empty() {
            return Ok(()); // No profile data
        }

        // Decompress using zenflate batch decompressor
        // ICC profiles are typically 1-4 KB, allocate generous output buffer
        let max_output = 1024 * 1024; // 1 MB limit for ICC profiles
        let mut output = vec![0u8; max_output];
        let mut decompressor = zenflate::Decompressor::new();
        let outcome = decompressor
            .zlib_decompress(compressed, &mut output, Unstoppable)
            .map_err(|e| at!(PngError::Decode(alloc::format!("iCCP decompression failed: {e:?}"))))?;
        output.truncate(outcome.output_written);
        self.icc_profile = Some(output);
        Ok(())
    }

    /// Try to extract XMP from an iTXt chunk.
    fn try_parse_xmp(&mut self, data: &[u8]) {
        // iTXt: keyword(null) compression_flag(1) compression_method(1)
        //       language_tag(null) translated_keyword(null) text
        let keyword = b"XML:com.adobe.xmp";
        if data.len() <= keyword.len() + 1 {
            return;
        }
        if &data[..keyword.len()] != keyword.as_slice() || data[keyword.len()] != 0 {
            return;
        }

        let rest = &data[keyword.len() + 1..];
        if rest.len() < 2 {
            return;
        }

        let compression_flag = rest[0];
        let _compression_method = rest[1];
        let rest = &rest[2..];

        // Skip language tag (null-terminated)
        let lang_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        if lang_end >= rest.len() {
            return;
        }
        let rest = &rest[lang_end + 1..];

        // Skip translated keyword (null-terminated)
        let trans_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        if trans_end >= rest.len() {
            return;
        }
        let text_data = &rest[trans_end + 1..];

        if compression_flag == 0 {
            // Uncompressed
            if !text_data.is_empty() {
                self.xmp = Some(text_data.to_vec());
            }
        } else if compression_flag == 1 {
            // zlib compressed
            let max_output = 4 * 1024 * 1024; // 4 MB limit for XMP
            let mut output = vec![0u8; max_output];
            let mut decompressor = zenflate::Decompressor::new();
            if let Ok(outcome) = decompressor.zlib_decompress(text_data, &mut output, Unstoppable) {
                output.truncate(outcome.output_written);
                if !output.is_empty() {
                    self.xmp = Some(output);
                }
            }
        }
    }

    /// Parse tEXt chunk: keyword\0text (Latin-1).
    fn parse_text(&mut self, data: &[u8], compressed: bool) {
        if let Some(null_pos) = data.iter().position(|&b| b == 0) {
            let keyword = &data[..null_pos];
            let text = &data[null_pos + 1..];
            if !keyword.is_empty() && keyword.len() <= 79 {
                // Latin-1 → UTF-8 (lossy for non-ASCII but preserves valid text)
                let kw: String = keyword.iter().map(|&b| b as char).collect();
                let val: String = text.iter().map(|&b| b as char).collect();
                self.text_chunks.push(TextChunk {
                    keyword: kw,
                    text: val,
                    compressed,
                });
            }
        }
    }

    /// Parse zTXt chunk: keyword\0compression_method + compressed_text.
    fn parse_ztxt(&mut self, data: &[u8]) {
        let Some(null_pos) = data.iter().position(|&b| b == 0) else {
            return;
        };
        let keyword = &data[..null_pos];
        if keyword.is_empty() || keyword.len() > 79 {
            return;
        }
        // Byte after null is compression method (must be 0 = zlib)
        if null_pos + 1 >= data.len() {
            return;
        }
        let compression_method = data[null_pos + 1];
        if compression_method != 0 {
            return;
        }
        let compressed = &data[null_pos + 2..];
        if compressed.is_empty() {
            return;
        }

        // Decompress
        let max_output = 1024 * 1024; // 1 MB limit for text
        let mut output = vec![0u8; max_output];
        let mut decompressor = zenflate::Decompressor::new();
        if let Ok(outcome) = decompressor.zlib_decompress(compressed, &mut output, Unstoppable) {
            output.truncate(outcome.output_written);
            let kw: String = keyword.iter().map(|&b| b as char).collect();
            let val: String = output.iter().map(|&b| b as char).collect();
            self.text_chunks.push(TextChunk {
                keyword: kw,
                text: val,
                compressed: true,
            });
        }
    }

    /// Parse bKGD chunk. Interpretation depends on color type (from PLTE presence).
    fn parse_bkgd(&mut self, data: &[u8]) {
        if self.palette.is_some() {
            // Indexed: 1 byte palette index
            if data.len() == 1 {
                self.background = Some(PngBackground::Indexed(data[0]));
            }
        } else if data.len() == 2 {
            // Grayscale: 2 bytes (u16)
            let val = u16::from_be_bytes(data[0..2].try_into().unwrap());
            self.background = Some(PngBackground::Gray(val));
        } else if data.len() == 6 {
            // RGB: 6 bytes (3 × u16)
            let r = u16::from_be_bytes(data[0..2].try_into().unwrap());
            let g = u16::from_be_bytes(data[2..4].try_into().unwrap());
            let b = u16::from_be_bytes(data[4..6].try_into().unwrap());
            self.background = Some(PngBackground::Rgb(r, g, b));
        }
    }

    /// Parse sBIT chunk. Length depends on color type.
    fn parse_sbit(&mut self, data: &[u8]) {
        // sBIT length determines color type:
        // 1 byte = grayscale, 2 bytes = grayscale+alpha, 3 bytes = RGB or indexed, 4 bytes = RGBA
        // We determine which based on whether palette is present and data length.
        match data.len() {
            1 => {
                self.significant_bits = Some(SignificantBits::Gray(data[0]));
            }
            2 => {
                self.significant_bits = Some(SignificantBits::GrayAlpha(data[0], data[1]));
            }
            3 => {
                // RGB (or indexed, which also uses 3 bytes for sBIT)
                self.significant_bits = Some(SignificantBits::Rgb(data[0], data[1], data[2]));
            }
            4 => {
                self.significant_bits =
                    Some(SignificantBits::Rgba(data[0], data[1], data[2], data[3]));
            }
            _ => {} // ignore invalid lengths
        }
    }

    /// Collect late metadata from post-IDAT chunks (eXIf, iTXt that some
    /// encoders place after IDAT).
    pub fn collect_late(&mut self, chunk: &ChunkRef<'_>) {
        match &chunk.chunk_type {
            b"eXIf" => {
                if self.exif.is_none() {
                    self.exif = Some(chunk.data.to_vec());
                }
            }
            b"iTXt" => {
                if self.xmp.is_none() {
                    self.try_parse_xmp(chunk.data);
                }
            }
            b"tEXt" => {
                self.parse_text(chunk.data, false);
            }
            b"zTXt" => {
                self.parse_ztxt(chunk.data);
            }
            b"tIME" => {
                if self.last_modified.is_none() && chunk.data.len() == 7 {
                    let year = u16::from_be_bytes(chunk.data[0..2].try_into().unwrap());
                    self.last_modified = Some(PngTime {
                        year,
                        month: chunk.data[2],
                        day: chunk.data[3],
                        hour: chunk.data[4],
                        minute: chunk.data[5],
                        second: chunk.data[6],
                    });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk<'a>(chunk_type: &[u8; 4], data: &'a [u8]) -> ChunkRef<'a> {
        ChunkRef {
            chunk_type: *chunk_type,
            data,
        }
    }

    // ---- PngAncillary::collect ----

    #[test]
    fn collect_idat_returns_true() {
        let mut anc = PngAncillary::default();
        let chunk = make_chunk(b"IDAT", &[]);
        assert!(anc.collect(&chunk).unwrap());
    }

    #[test]
    fn collect_plte() {
        let mut anc = PngAncillary::default();
        let data = [255, 0, 0, 0, 255, 0, 0, 0, 255]; // 3 entries
        let chunk = make_chunk(b"PLTE", &data);
        assert!(!anc.collect(&chunk).unwrap());
        assert_eq!(anc.palette.as_ref().unwrap().len(), 9);
    }

    #[test]
    fn collect_plte_invalid_length() {
        let mut anc = PngAncillary::default();
        let data = [255, 0]; // not a multiple of 3
        let chunk = make_chunk(b"PLTE", &data);
        assert!(anc.collect(&chunk).is_err());
    }

    #[test]
    fn collect_plte_empty() {
        let mut anc = PngAncillary::default();
        let chunk = make_chunk(b"PLTE", &[]);
        assert!(anc.collect(&chunk).is_err());
    }

    #[test]
    fn collect_trns_basic() {
        let mut anc = PngAncillary::default();
        let data = [128, 64];
        let chunk = make_chunk(b"tRNS", &data);
        assert!(!anc.collect(&chunk).unwrap());
        assert_eq!(anc.trns.as_ref().unwrap(), &data);
    }

    #[test]
    fn collect_trns_empty_ignored() {
        let mut anc = PngAncillary::default();
        let chunk = make_chunk(b"tRNS", &[]);
        assert!(!anc.collect(&chunk).unwrap());
        assert!(anc.trns.is_none());
    }

    #[test]
    fn collect_trns_oversized_after_plte() {
        let mut anc = PngAncillary::default();
        // 2-entry palette
        anc.collect(&make_chunk(b"PLTE", &[0, 0, 0, 255, 255, 255]))
            .unwrap();
        // tRNS with 3 entries (more than palette)
        anc.collect(&make_chunk(b"tRNS", &[128, 64, 32])).unwrap();
        // Should store empty vec (presence preserved, data discarded)
        assert_eq!(anc.trns.as_ref().unwrap().len(), 0);
    }

    #[test]
    fn collect_gama() {
        let mut anc = PngAncillary::default();
        let data = 45455u32.to_be_bytes(); // sRGB gamma
        anc.collect(&make_chunk(b"gAMA", &data)).unwrap();
        assert_eq!(anc.gamma, Some(45455));
    }

    #[test]
    fn collect_gama_wrong_size() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"gAMA", &[0, 0])).unwrap();
        assert!(anc.gamma.is_none());
    }

    #[test]
    fn collect_srgb() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"sRGB", &[1])).unwrap();
        assert_eq!(anc.srgb_intent, Some(1));
    }

    #[test]
    fn collect_srgb_empty() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"sRGB", &[])).unwrap();
        assert!(anc.srgb_intent.is_none());
    }

    #[test]
    fn collect_chrm() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 32];
        for i in 0..8 {
            let val = (i as i32 + 1) * 10000;
            data[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
        }
        anc.collect(&make_chunk(b"cHRM", &data)).unwrap();
        let chrm = anc.chrm.unwrap();
        assert_eq!(chrm[0], 10000);
        assert_eq!(chrm[7], 80000);
    }

    #[test]
    fn collect_chrm_wrong_size() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"cHRM", &[0; 16])).unwrap();
        assert!(anc.chrm.is_none());
    }

    #[test]
    fn collect_cicp() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"cICP", &[1, 13, 0, 1])).unwrap();
        assert_eq!(anc.cicp, Some([1, 13, 0, 1]));
    }

    #[test]
    fn collect_clli() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 8];
        data[0..4].copy_from_slice(&1000u32.to_be_bytes());
        data[4..8].copy_from_slice(&500u32.to_be_bytes());
        anc.collect(&make_chunk(b"cLLI", &data)).unwrap();
        assert_eq!(anc.clli, Some([1000, 500]));
    }

    #[test]
    fn collect_mdcv() {
        let mut anc = PngAncillary::default();
        let data = [0u8; 24];
        anc.collect(&make_chunk(b"mDCV", &data)).unwrap();
        assert!(anc.mdcv.is_some());
    }

    #[test]
    fn collect_exif() {
        let mut anc = PngAncillary::default();
        let data = b"Exif\x00\x00MM";
        anc.collect(&make_chunk(b"eXIf", data)).unwrap();
        assert_eq!(anc.exif.as_ref().unwrap(), data);
    }

    #[test]
    fn collect_actl() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 8];
        data[0..4].copy_from_slice(&10u32.to_be_bytes());
        data[4..8].copy_from_slice(&0u32.to_be_bytes());
        anc.collect(&make_chunk(b"acTL", &data)).unwrap();
        assert_eq!(anc.actl, Some((10, 0)));
    }

    #[test]
    fn collect_unknown_chunk_ignored() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"zZzZ", &[1, 2, 3])).unwrap();
        // No fields changed
        assert!(anc.palette.is_none());
    }

    // ---- collect_late ----

    #[test]
    fn collect_late_exif() {
        let mut anc = PngAncillary::default();
        let data = b"late exif";
        anc.collect_late(&make_chunk(b"eXIf", data));
        assert_eq!(anc.exif.as_ref().unwrap(), data);
    }

    #[test]
    fn collect_late_exif_no_overwrite() {
        let mut anc = PngAncillary {
            exif: Some(b"first".to_vec()),
            ..Default::default()
        };
        anc.collect_late(&make_chunk(b"eXIf", b"second"));
        assert_eq!(anc.exif.as_ref().unwrap(), b"first");
    }

    #[test]
    fn collect_late_unknown_ignored() {
        let mut anc = PngAncillary::default();
        anc.collect_late(&make_chunk(b"zZzZ", &[1, 2, 3]));
    }

    // ---- XMP parsing ----

    #[test]
    fn xmp_uncompressed() {
        let mut anc = PngAncillary::default();
        // Build iTXt chunk data for XMP
        let mut data = Vec::new();
        data.extend_from_slice(b"XML:com.adobe.xmp");
        data.push(0); // null terminator
        data.push(0); // compression_flag = 0 (uncompressed)
        data.push(0); // compression_method
        data.push(0); // language tag (empty, null-terminated)
        data.push(0); // translated keyword (empty, null-terminated)
        data.extend_from_slice(b"<x:xmpmeta/>");
        anc.collect(&make_chunk(b"iTXt", &data)).unwrap();
        assert_eq!(anc.xmp.as_ref().unwrap(), b"<x:xmpmeta/>");
    }

    #[test]
    fn xmp_wrong_keyword_ignored() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(b"Description");
        data.push(0);
        data.push(0);
        data.push(0);
        data.push(0);
        data.push(0);
        data.extend_from_slice(b"some text");
        anc.collect(&make_chunk(b"iTXt", &data)).unwrap();
        assert!(anc.xmp.is_none());
    }

    #[test]
    fn xmp_too_short_ignored() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"iTXt", b"XML:com.adobe.xmp"))
            .unwrap();
        assert!(anc.xmp.is_none());
    }

    // ---- iCCP parsing ----

    #[test]
    fn iccp_valid_profile() {
        let mut anc = PngAncillary::default();
        // Build iCCP: "sRGB\0" + compression_method(0) + zlib-compressed data
        let mut data = Vec::new();
        data.extend_from_slice(b"sRGB");
        data.push(0); // null terminator
        data.push(0); // compression method = zlib

        // Compress some dummy ICC data
        let icc_data = b"dummy icc profile data for testing";
        let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(1));
        let bound = zenflate::Compressor::zlib_compress_bound(icc_data.len());
        let mut compressed = vec![0u8; bound];
        let len = compressor
            .zlib_compress(icc_data, &mut compressed, Unstoppable)
            .unwrap();
        data.extend_from_slice(&compressed[..len]);

        anc.collect(&make_chunk(b"iCCP", &data)).unwrap();
        assert_eq!(anc.icc_profile.as_ref().unwrap(), icc_data);
    }

    #[test]
    fn iccp_no_null_terminator() {
        let mut anc = PngAncillary::default();
        // iCCP without null terminator — should silently fail (ancillary)
        anc.collect(&make_chunk(b"iCCP", b"sRGB")).unwrap();
        assert!(anc.icc_profile.is_none());
    }

    #[test]
    fn iccp_bad_compression() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(b"sRGB");
        data.push(0);
        data.push(1); // bad compression method
        data.push(0);
        anc.collect(&make_chunk(b"iCCP", &data)).unwrap();
        assert!(anc.icc_profile.is_none());
    }

    #[test]
    fn iccp_truncated_after_name() {
        let mut anc = PngAncillary::default();
        let data = [b's', b'R', b'G', b'B', 0]; // null terminator but no compression byte
        anc.collect(&make_chunk(b"iCCP", &data)).unwrap();
        assert!(anc.icc_profile.is_none());
    }

    // ---- FrameControl ----

    #[test]
    fn fctl_delay_den_zero_treated_as_100() {
        let mut data = [0u8; 26];
        // width=1, height=1
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&1u32.to_be_bytes());
        // delay_num=5, delay_den=0
        data[20..22].copy_from_slice(&5u16.to_be_bytes());
        data[22..24].copy_from_slice(&0u16.to_be_bytes());

        let fctl = FrameControl::parse(&data, 100, 100).unwrap();
        assert_eq!(fctl.delay_ms(), 50); // 5/100 * 1000 = 50ms
    }

    // ---- pHYs parsing ----

    #[test]
    fn collect_phys_meter() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 9];
        data[0..4].copy_from_slice(&3780u32.to_be_bytes()); // ~96 DPI
        data[4..8].copy_from_slice(&3780u32.to_be_bytes());
        data[8] = 1; // meter
        anc.collect(&make_chunk(b"pHYs", &data)).unwrap();
        assert_eq!(anc.phys, Some((3780, 3780, 1)));
    }

    #[test]
    fn collect_phys_unknown_unit() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 9];
        data[0..4].copy_from_slice(&1u32.to_be_bytes());
        data[4..8].copy_from_slice(&2u32.to_be_bytes());
        data[8] = 0; // unknown
        anc.collect(&make_chunk(b"pHYs", &data)).unwrap();
        assert_eq!(anc.phys, Some((1, 2, 0)));
    }

    #[test]
    fn collect_phys_wrong_size_ignored() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"pHYs", &[0; 5])).unwrap();
        assert!(anc.phys.is_none());
    }

    // ---- tEXt parsing ----

    #[test]
    fn collect_text_basic() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(b"Comment");
        data.push(0);
        data.extend_from_slice(b"hello world");
        anc.collect(&make_chunk(b"tEXt", &data)).unwrap();
        assert_eq!(anc.text_chunks.len(), 1);
        assert_eq!(anc.text_chunks[0].keyword, "Comment");
        assert_eq!(anc.text_chunks[0].text, "hello world");
        assert!(!anc.text_chunks[0].compressed);
    }

    #[test]
    fn collect_text_empty_value() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(b"Title");
        data.push(0);
        // empty value
        anc.collect(&make_chunk(b"tEXt", &data)).unwrap();
        assert_eq!(anc.text_chunks.len(), 1);
        assert_eq!(anc.text_chunks[0].keyword, "Title");
        assert_eq!(anc.text_chunks[0].text, "");
    }

    #[test]
    fn collect_text_no_null_ignored() {
        let mut anc = PngAncillary::default();
        // No null separator — should be ignored
        anc.collect(&make_chunk(b"tEXt", b"nodivider")).unwrap();
        assert!(anc.text_chunks.is_empty());
    }

    #[test]
    fn collect_text_keyword_too_long_ignored() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(&[b'A'; 80]); // 80 bytes > 79 limit
        data.push(0);
        data.extend_from_slice(b"value");
        anc.collect(&make_chunk(b"tEXt", &data)).unwrap();
        assert!(anc.text_chunks.is_empty());
    }

    #[test]
    fn collect_text_multiple_chunks() {
        let mut anc = PngAncillary::default();
        let mut d1 = Vec::new();
        d1.extend_from_slice(b"Author");
        d1.push(0);
        d1.extend_from_slice(b"Alice");
        let mut d2 = Vec::new();
        d2.extend_from_slice(b"Comment");
        d2.push(0);
        d2.extend_from_slice(b"test image");
        anc.collect(&make_chunk(b"tEXt", &d1)).unwrap();
        anc.collect(&make_chunk(b"tEXt", &d2)).unwrap();
        assert_eq!(anc.text_chunks.len(), 2);
        assert_eq!(anc.text_chunks[0].keyword, "Author");
        assert_eq!(anc.text_chunks[1].keyword, "Comment");
    }

    // ---- zTXt parsing ----

    #[test]
    fn collect_ztxt_basic() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(b"Comment");
        data.push(0);
        data.push(0); // compression method = zlib

        let text = b"compressed text data";
        let mut compressor = zenflate::Compressor::new(zenflate::CompressionLevel::new(1));
        let bound = zenflate::Compressor::zlib_compress_bound(text.len());
        let mut compressed = vec![0u8; bound];
        let len = compressor
            .zlib_compress(text, &mut compressed, Unstoppable)
            .unwrap();
        data.extend_from_slice(&compressed[..len]);

        anc.collect(&make_chunk(b"zTXt", &data)).unwrap();
        assert_eq!(anc.text_chunks.len(), 1);
        assert_eq!(anc.text_chunks[0].keyword, "Comment");
        assert_eq!(anc.text_chunks[0].text, "compressed text data");
        assert!(anc.text_chunks[0].compressed);
    }

    #[test]
    fn collect_ztxt_bad_compression_method_ignored() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(b"Comment");
        data.push(0);
        data.push(1); // bad compression method
        data.extend_from_slice(&[0; 10]);
        anc.collect(&make_chunk(b"zTXt", &data)).unwrap();
        assert!(anc.text_chunks.is_empty());
    }

    // ---- bKGD parsing ----

    #[test]
    fn collect_bkgd_indexed() {
        let mut anc = PngAncillary::default();
        // Set palette so bKGD interprets as indexed
        anc.collect(&make_chunk(b"PLTE", &[0, 0, 0, 255, 255, 255]))
            .unwrap();
        anc.collect(&make_chunk(b"bKGD", &[1])).unwrap();
        assert_eq!(anc.background, Some(PngBackground::Indexed(1)));
    }

    #[test]
    fn collect_bkgd_gray() {
        let mut anc = PngAncillary::default();
        let data = 128u16.to_be_bytes();
        anc.collect(&make_chunk(b"bKGD", &data)).unwrap();
        assert_eq!(anc.background, Some(PngBackground::Gray(128)));
    }

    #[test]
    fn collect_bkgd_rgb() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 6];
        data[0..2].copy_from_slice(&100u16.to_be_bytes());
        data[2..4].copy_from_slice(&200u16.to_be_bytes());
        data[4..6].copy_from_slice(&300u16.to_be_bytes());
        anc.collect(&make_chunk(b"bKGD", &data)).unwrap();
        assert_eq!(anc.background, Some(PngBackground::Rgb(100, 200, 300)));
    }

    #[test]
    fn collect_bkgd_wrong_size_ignored() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"bKGD", &[0; 4])).unwrap();
        assert!(anc.background.is_none());
    }

    // ---- tIME parsing ----

    #[test]
    fn collect_time_valid() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 7];
        data[0..2].copy_from_slice(&2026u16.to_be_bytes());
        data[2] = 3; // month
        data[3] = 18; // day
        data[4] = 14; // hour
        data[5] = 30; // minute
        data[6] = 0; // second
        anc.collect(&make_chunk(b"tIME", &data)).unwrap();
        let t = anc.last_modified.unwrap();
        assert_eq!(t.year, 2026);
        assert_eq!(t.month, 3);
        assert_eq!(t.day, 18);
        assert_eq!(t.hour, 14);
        assert_eq!(t.minute, 30);
        assert_eq!(t.second, 0);
    }

    #[test]
    fn collect_time_wrong_size_ignored() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"tIME", &[0; 5])).unwrap();
        assert!(anc.last_modified.is_none());
    }

    #[test]
    fn collect_late_time() {
        let mut anc = PngAncillary::default();
        let mut data = [0u8; 7];
        data[0..2].copy_from_slice(&2025u16.to_be_bytes());
        data[2] = 12;
        data[3] = 25;
        data[4] = 0;
        data[5] = 0;
        data[6] = 0;
        anc.collect_late(&make_chunk(b"tIME", &data));
        let t = anc.last_modified.unwrap();
        assert_eq!(t.year, 2025);
        assert_eq!(t.month, 12);
        assert_eq!(t.day, 25);
    }

    // ---- sBIT parsing ----

    #[test]
    fn collect_sbit_gray() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"sBIT", &[5])).unwrap();
        assert_eq!(anc.significant_bits, Some(SignificantBits::Gray(5)));
    }

    #[test]
    fn collect_sbit_gray_alpha() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"sBIT", &[5, 8])).unwrap();
        assert_eq!(anc.significant_bits, Some(SignificantBits::GrayAlpha(5, 8)));
    }

    #[test]
    fn collect_sbit_rgb() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"sBIT", &[5, 6, 5])).unwrap();
        assert_eq!(anc.significant_bits, Some(SignificantBits::Rgb(5, 6, 5)));
    }

    #[test]
    fn collect_sbit_rgba() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"sBIT", &[5, 6, 5, 8])).unwrap();
        assert_eq!(
            anc.significant_bits,
            Some(SignificantBits::Rgba(5, 6, 5, 8))
        );
    }

    #[test]
    fn collect_sbit_wrong_size_ignored() {
        let mut anc = PngAncillary::default();
        anc.collect(&make_chunk(b"sBIT", &[5, 6, 5, 8, 1])).unwrap();
        assert!(anc.significant_bits.is_none());
    }

    // ---- collect_late text chunks ----

    #[test]
    fn collect_late_text() {
        let mut anc = PngAncillary::default();
        let mut data = Vec::new();
        data.extend_from_slice(b"Comment");
        data.push(0);
        data.extend_from_slice(b"post-IDAT text");
        anc.collect_late(&make_chunk(b"tEXt", &data));
        assert_eq!(anc.text_chunks.len(), 1);
        assert_eq!(anc.text_chunks[0].text, "post-IDAT text");
    }
}
