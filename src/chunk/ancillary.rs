//! Ancillary PNG chunk metadata collection.

use alloc::vec;
use alloc::vec::Vec;

use zenflate::Unstoppable;

use super::ChunkRef;
use crate::error::PngError;

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
    /// cHRM chromaticities (8 u32 values: wx, wy, rx, ry, gx, gy, bx, by).
    pub chrm: Option<[u32; 8]>,
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
                if chunk.data.len() % 3 != 0 || chunk.data.is_empty() {
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
                    let mut vals = [0u32; 8];
                    for (i, v) in vals.iter_mut().enumerate() {
                        *v = u32::from_be_bytes(chunk.data[i * 4..(i + 1) * 4].try_into().unwrap());
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
