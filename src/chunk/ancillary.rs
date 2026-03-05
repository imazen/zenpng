//! Ancillary PNG chunk metadata collection.

use alloc::vec;
use alloc::vec::Vec;

use zenflate::Unstoppable;

use super::ChunkRef;
use crate::error::PngError;

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
    pub fn parse(data: &[u8], canvas_width: u32, canvas_height: u32) -> Result<Self, PngError> {
        if data.len() != 26 {
            return Err(PngError::Decode(alloc::format!(
                "fcTL chunk is {} bytes, expected 26",
                data.len()
            )));
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
            return Err(PngError::Decode("fcTL: zero frame dimension".into()));
        }
        if x_offset.checked_add(width).is_none_or(|v| v > canvas_width) {
            return Err(PngError::Decode(alloc::format!(
                "fcTL: x_offset({x_offset}) + width({width}) exceeds canvas width({canvas_width})"
            )));
        }
        if y_offset
            .checked_add(height)
            .is_none_or(|v| v > canvas_height)
        {
            return Err(PngError::Decode(alloc::format!(
                "fcTL: y_offset({y_offset}) + height({height}) exceeds canvas height({canvas_height})"
            )));
        }
        if dispose_op > 2 {
            return Err(PngError::Decode(alloc::format!(
                "fcTL: invalid dispose_op {dispose_op}"
            )));
        }
        if blend_op > 1 {
            return Err(PngError::Decode(alloc::format!(
                "fcTL: invalid blend_op {blend_op}"
            )));
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
}

impl PngAncillary {
    /// Collect metadata from a single chunk. Returns true if this is an IDAT chunk
    /// (signals the caller to stop collecting pre-IDAT metadata).
    pub fn collect(&mut self, chunk: &ChunkRef<'_>) -> Result<bool, PngError> {
        match &chunk.chunk_type {
            b"IDAT" => return Ok(true),
            b"PLTE" => {
                if !chunk.data.len().is_multiple_of(3) || chunk.data.is_empty() {
                    return Err(PngError::Decode("invalid PLTE chunk length".into()));
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
                    self.actl = Some((num_frames, num_plays));
                }
            }
            _ => {} // ignore unknown chunks
        }
        Ok(false)
    }

    /// Parse iCCP chunk: null-terminated profile name, compression method, compressed data.
    fn parse_iccp(&mut self, data: &[u8]) -> Result<(), PngError> {
        // Find null terminator for profile name
        let null_pos = data
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| PngError::Decode("iCCP: missing profile name terminator".into()))?;

        // Byte after null is compression method (must be 0 = zlib)
        if null_pos + 2 > data.len() {
            return Err(PngError::Decode(
                "iCCP: truncated after profile name".into(),
            ));
        }
        let compression_method = data[null_pos + 1];
        if compression_method != 0 {
            return Err(PngError::Decode(alloc::format!(
                "iCCP: unknown compression method {}",
                compression_method
            )));
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
            .map_err(|e| PngError::Decode(alloc::format!("iCCP decompression failed: {e:?}")))?;
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
}
