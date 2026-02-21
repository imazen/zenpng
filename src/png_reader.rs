//! PNG chunk parsing, streaming row decoder, and post-processing.
//!
//! Mirrors `png_writer.rs` — everything `pub(crate)`.

use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zenflate::{Unstoppable, crc32};

use crate::error::PngError;

// ── PNG signature ───────────────────────────────────────────────────

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

// ── Chunk parser ────────────────────────────────────────────────────

/// Reference to a single PNG chunk (zero-copy borrow of the file data).
#[derive(Clone, Copy)]
pub(crate) struct ChunkRef<'a> {
    pub chunk_type: [u8; 4],
    pub data: &'a [u8],
}

/// Iterator over PNG chunks. Walks bytes after the 8-byte PNG signature.
pub(crate) struct ChunkIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ChunkIter<'a> {
    /// Create a new chunk iterator. `data` must be the full PNG file bytes
    /// (signature already validated by caller).
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 8, // skip PNG signature
        }
    }

    /// Current byte offset in the file.
    pub fn pos(&self) -> usize {
        self.pos
    }
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Result<ChunkRef<'a>, PngError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.data.len() {
            return None;
        }

        // Need at least 12 bytes: length(4) + type(4) + crc(4) (data can be 0)
        if self.pos + 12 > self.data.len() {
            return Some(Err(PngError::Decode("truncated chunk header".into())));
        }

        let length =
            u32::from_be_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap()) as usize;
        let chunk_type: [u8; 4] = self.data[self.pos + 4..self.pos + 8].try_into().unwrap();

        let data_start = self.pos + 8;
        let data_end = data_start + length;
        let crc_end = data_end + 4;

        if crc_end > self.data.len() {
            return Some(Err(PngError::Decode(alloc::format!(
                "truncated chunk {:?} at offset {}",
                core::str::from_utf8(&chunk_type).unwrap_or("????"),
                self.pos
            ))));
        }

        let chunk_data = &self.data[data_start..data_end];
        let stored_crc = u32::from_be_bytes(self.data[data_end..crc_end].try_into().unwrap());

        // CRC covers type + data
        let computed_crc = crc32(crc32(0, &chunk_type), chunk_data);
        if stored_crc != computed_crc {
            // PNG spec: bit 5 of the first byte indicates ancillary (lowercase).
            // Ancillary chunks with bad CRC should be skipped, not fatal.
            let is_ancillary = chunk_type[0] & 0x20 != 0;
            if is_ancillary {
                self.pos = crc_end;
                return self.next(); // skip this chunk, try next
            }
            return Some(Err(PngError::Decode(alloc::format!(
                "CRC mismatch in {:?} chunk at offset {}",
                core::str::from_utf8(&chunk_type).unwrap_or("????"),
                self.pos
            ))));
        }

        self.pos = crc_end;

        Some(Ok(ChunkRef {
            chunk_type,
            data: chunk_data,
        }))
    }
}

// ── IHDR ────────────────────────────────────────────────────────────

/// Parsed IHDR chunk.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) struct Ihdr {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlace: u8,
}

impl Ihdr {
    /// Parse IHDR from chunk data (must be exactly 13 bytes).
    pub fn parse(data: &[u8]) -> Result<Self, PngError> {
        if data.len() != 13 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR chunk is {} bytes, expected 13",
                data.len()
            )));
        }

        let width = u32::from_be_bytes(data[0..4].try_into().unwrap());
        let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
        let bit_depth = data[8];
        let color_type = data[9];
        let compression = data[10];
        let filter = data[11];
        let interlace = data[12];

        if width == 0 || height == 0 {
            return Err(PngError::Decode("IHDR: zero dimension".into()));
        }

        if compression != 0 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR: unknown compression method {}",
                compression
            )));
        }
        if filter != 0 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR: unknown filter method {}",
                filter
            )));
        }
        if interlace > 1 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR: unknown interlace method {}",
                interlace
            )));
        }

        let ihdr = Self {
            width,
            height,
            bit_depth,
            color_type,
            interlace,
        };
        ihdr.validate()?;
        Ok(ihdr)
    }

    /// Validate color_type / bit_depth combination per PNG spec.
    fn validate(&self) -> Result<(), PngError> {
        let valid = match self.color_type {
            0 => matches!(self.bit_depth, 1 | 2 | 4 | 8 | 16), // Grayscale
            2 => matches!(self.bit_depth, 8 | 16),             // RGB
            3 => matches!(self.bit_depth, 1 | 2 | 4 | 8),      // Indexed
            4 => matches!(self.bit_depth, 8 | 16),             // GrayAlpha
            6 => matches!(self.bit_depth, 8 | 16),             // RGBA
            _ => false,
        };
        if !valid {
            return Err(PngError::Decode(alloc::format!(
                "invalid color_type={} bit_depth={} combination",
                self.color_type,
                self.bit_depth
            )));
        }
        Ok(())
    }

    /// Number of channels for this color type.
    pub fn channels(&self) -> usize {
        match self.color_type {
            0 => 1, // Grayscale
            2 => 3, // RGB
            3 => 1, // Indexed (palette index)
            4 => 2, // GrayAlpha
            6 => 4, // RGBA
            _ => unreachable!("validated in parse"),
        }
    }

    /// Bytes per pixel for the filter unit (bpp), minimum 1.
    /// For sub-8-bit depths, this is 1.
    pub fn filter_bpp(&self) -> usize {
        let bits_per_pixel = self.channels() * self.bit_depth as usize;
        bits_per_pixel.div_ceil(8)
    }

    /// Raw row bytes (unfiltered row data, not including filter byte).
    /// For sub-8-bit depths, accounts for bit packing.
    pub fn raw_row_bytes(&self) -> usize {
        let bits_per_row = self.width as usize * self.channels() * self.bit_depth as usize;
        bits_per_row.div_ceil(8)
    }

    /// Stride = 1 (filter byte) + raw_row_bytes.
    pub fn stride(&self) -> usize {
        1 + self.raw_row_bytes()
    }

    /// Whether the image uses sub-8-bit depth (1, 2, or 4).
    pub fn is_sub_byte(&self) -> bool {
        self.bit_depth < 8
    }

    /// Whether this is a palette-indexed image.
    pub fn is_indexed(&self) -> bool {
        self.color_type == 3
    }

    /// Whether the source has an alpha channel (color type 4 or 6).
    pub fn has_alpha(&self) -> bool {
        self.color_type == 4 || self.color_type == 6
    }
}

// ── Ancillary metadata ──────────────────────────────────────────────

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
                self.trns = Some(chunk.data.to_vec());
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

// ── IdatSource — InputSource for IDAT chunks ────────────────────────

/// Streams raw IDAT chunk payload bytes to `StreamDecompressor`.
/// Walks chunks in the file without collecting IDAT data into a Vec.
pub(crate) struct IdatSource<'a> {
    /// Full PNG file bytes.
    data: &'a [u8],
    /// Byte offset of the next chunk header to check.
    chunk_pos: usize,
    /// Remaining bytes in the current IDAT chunk's data.
    current_data: &'a [u8],
    /// True when we've seen a non-IDAT chunk after IDAT.
    done: bool,
    /// Position of the first post-IDAT chunk (for metadata collection).
    pub post_idat_pos: usize,
}

impl<'a> IdatSource<'a> {
    /// Create a new IDAT source positioned at the first IDAT chunk.
    /// `first_idat_pos` is the byte offset of the first IDAT chunk header.
    pub fn new(data: &'a [u8], first_idat_pos: usize) -> Self {
        // Parse the first IDAT chunk inline
        let length =
            u32::from_be_bytes(data[first_idat_pos..first_idat_pos + 4].try_into().unwrap())
                as usize;
        let data_start = first_idat_pos + 8; // skip length + type
        let data_end = data_start + length;
        let next_pos = data_end + 4; // skip CRC

        Self {
            data,
            chunk_pos: next_pos,
            current_data: &data[data_start..data_end],
            done: false,
            post_idat_pos: 0,
        }
    }
}

impl<'a> zenflate::InputSource for IdatSource<'a> {
    type Error = PngError;

    fn fill_buf(&mut self) -> Result<&[u8], PngError> {
        if !self.current_data.is_empty() {
            return Ok(self.current_data);
        }
        if self.done {
            return Ok(&[]);
        }

        // Advance to next chunk
        loop {
            if self.chunk_pos + 12 > self.data.len() {
                self.done = true;
                self.post_idat_pos = self.chunk_pos;
                return Ok(&[]);
            }

            let length = u32::from_be_bytes(
                self.data[self.chunk_pos..self.chunk_pos + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let chunk_type: [u8; 4] = self.data[self.chunk_pos + 4..self.chunk_pos + 8]
                .try_into()
                .unwrap();
            let data_start = self.chunk_pos + 8;
            let data_end = data_start + length;
            let crc_end = data_end + 4;

            if crc_end > self.data.len() {
                return Err(PngError::Decode("truncated IDAT chunk".into()));
            }

            if chunk_type != *b"IDAT" {
                // Not IDAT — we're done with the IDAT stream
                self.done = true;
                self.post_idat_pos = self.chunk_pos;
                return Ok(&[]);
            }

            // Validate CRC
            let stored_crc = u32::from_be_bytes(self.data[data_end..crc_end].try_into().unwrap());
            let computed_crc = crc32(crc32(0, &chunk_type), &self.data[data_start..data_end]);
            if stored_crc != computed_crc {
                return Err(PngError::Decode("CRC mismatch in IDAT chunk".into()));
            }

            self.current_data = &self.data[data_start..data_end];
            self.chunk_pos = crc_end;

            if !self.current_data.is_empty() {
                return Ok(self.current_data);
            }
            // Empty IDAT chunk — skip and try next
        }
    }

    fn consume(&mut self, n: usize) {
        self.current_data = &self.current_data[n..];
    }
}

// ── Unfilter ────────────────────────────────────────────────────────

/// Paeth predictor (identical to png_writer.rs).
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i16;
    let b = b as i16;
    let c = c as i16;
    let p = a + b - c;
    let pa = (p - a).unsigned_abs();
    let pb = (p - b).unsigned_abs();
    let pc = (p - c).unsigned_abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// Apply inverse filter to a row in-place given the previous (unfiltered) row.
fn unfilter_row(filter_type: u8, row: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), PngError> {
    let len = row.len();
    match filter_type {
        0 => {} // None
        1 => {
            // Sub: add left neighbor
            for i in bpp..len {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            // Up: add above
            for i in 0..len {
                row[i] = row[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            // Average: add floor((left + above) / 2)
            for i in 0..bpp.min(len) {
                row[i] = row[i].wrapping_add(prev[i] >> 1);
            }
            for i in bpp..len {
                let avg = ((row[i - bpp] as u16 + prev[i] as u16) >> 1) as u8;
                row[i] = row[i].wrapping_add(avg);
            }
        }
        4 => {
            // Paeth: add paeth_predictor(left, above, upper_left)
            for i in 0..bpp.min(len) {
                row[i] = row[i].wrapping_add(paeth_predictor(0, prev[i], 0));
            }
            for i in bpp..len {
                let pred = paeth_predictor(row[i - bpp], prev[i], prev[i - bpp]);
                row[i] = row[i].wrapping_add(pred);
            }
        }
        _ => {
            return Err(PngError::Decode(alloc::format!(
                "unknown filter type {}",
                filter_type
            )));
        }
    }
    Ok(())
}

// ── RowDecoder ──────────────────────────────────────────────────────

/// Streaming PNG row decoder. Reads IDAT chunks through `StreamDecompressor`,
/// unfilters each scanline, and yields raw (unfiltered) row data.
pub(crate) struct RowDecoder<'a> {
    decompressor: zenflate::StreamDecompressor<IdatSource<'a>>,
    ihdr: Ihdr,
    ancillary: PngAncillary,

    /// Full PNG file bytes (for post-IDAT metadata scanning).
    file_data: &'a [u8],
    /// Byte offset of the first IDAT chunk header.
    first_idat_pos: usize,

    prev_row: Vec<u8>,
    current_row: Vec<u8>,
    rows_yielded: u32,
    stride: usize,
    bpp: usize,
}

impl<'a> RowDecoder<'a> {
    /// Create a new RowDecoder from PNG file bytes.
    pub fn new(
        data: &'a [u8],
        limits: Option<&crate::decode::PngLimits>,
    ) -> Result<Self, PngError> {
        // Validate signature
        if data.len() < 8 || data[..8] != PNG_SIGNATURE {
            return Err(PngError::Decode("not a PNG file".into()));
        }

        let mut chunks = ChunkIter::new(data);

        // First chunk must be IHDR
        let ihdr_chunk = chunks
            .next()
            .ok_or_else(|| PngError::Decode("empty PNG (no chunks)".into()))??;
        if ihdr_chunk.chunk_type != *b"IHDR" {
            return Err(PngError::Decode("first chunk is not IHDR".into()));
        }
        let ihdr = Ihdr::parse(ihdr_chunk.data)?;

        // Collect pre-IDAT ancillary chunks
        let mut ancillary = PngAncillary::default();
        let mut first_idat_pos = None;

        for chunk_result in &mut chunks {
            let chunk = chunk_result?;
            if chunk.chunk_type == *b"IDAT" {
                // Record position of the IDAT chunk header
                // The iterator has advanced past this chunk, so back-calculate:
                // current pos = end of this chunk, header was at pos - 12 - data.len()
                first_idat_pos = Some(chunks.pos() - 12 - chunk.data.len());
                break;
            }
            ancillary.collect(&chunk)?;
        }

        let first_idat_pos =
            first_idat_pos.ok_or_else(|| PngError::Decode("no IDAT chunk found".into()))?;

        // Validate palette for indexed images
        if ihdr.is_indexed() && ancillary.palette.is_none() {
            return Err(PngError::Decode(
                "indexed color type requires PLTE chunk".into(),
            ));
        }

        // Apply limits
        if let Some(lim) = limits {
            let output_bpp = output_bytes_per_pixel(&ihdr, &ancillary) as u32;
            lim.validate(ihdr.width, ihdr.height, output_bpp)?;
        }

        let stride = ihdr.stride();
        let raw_row_bytes = ihdr.raw_row_bytes();
        let bpp = ihdr.filter_bpp();

        // Create IDAT source and decompressor
        let source = IdatSource::new(data, first_idat_pos);
        let decompressor = zenflate::StreamDecompressor::zlib(source, stride * 2);

        Ok(Self {
            decompressor,
            ihdr,
            ancillary,
            file_data: data,
            first_idat_pos,
            prev_row: vec![0u8; raw_row_bytes],
            current_row: vec![0u8; raw_row_bytes],
            rows_yielded: 0,
            stride,
            bpp,
        })
    }

    /// Get the IHDR info.
    pub fn ihdr(&self) -> &Ihdr {
        &self.ihdr
    }

    /// Get the ancillary metadata.
    pub fn ancillary(&self) -> &PngAncillary {
        &self.ancillary
    }

    /// Yield the next unfiltered raw row, or None if all rows have been read.
    pub fn next_raw_row(&mut self) -> Option<Result<&[u8], PngError>> {
        if self.rows_yielded >= self.ihdr.height {
            return None;
        }

        // Fill decompressor until we have a full stride
        loop {
            let available = self.decompressor.peek().len();
            if available >= self.stride {
                break;
            }
            if self.decompressor.is_done() {
                if available > 0 && available < self.stride {
                    return Some(Err(PngError::Decode(alloc::format!(
                        "truncated row data: got {} bytes, expected {} (row {})",
                        available,
                        self.stride,
                        self.rows_yielded
                    ))));
                }
                return None;
            }
            match self.decompressor.fill() {
                Ok(_) => {}
                Err(e) => {
                    return Some(Err(PngError::Decode(alloc::format!(
                        "decompression error: {e:?}"
                    ))));
                }
            }
        }

        let peeked = self.decompressor.peek();
        let filter_byte = peeked[0];
        let raw_row_bytes = self.stride - 1;

        // Copy filtered data to current_row
        self.current_row[..raw_row_bytes].copy_from_slice(&peeked[1..self.stride]);
        self.decompressor.advance(self.stride);

        // Apply inverse filter
        if let Err(e) = unfilter_row(
            filter_byte,
            &mut self.current_row[..raw_row_bytes],
            &self.prev_row,
            self.bpp,
        ) {
            return Some(Err(e));
        }

        // Swap current and prev
        core::mem::swap(&mut self.current_row, &mut self.prev_row);
        self.rows_yielded += 1;

        Some(Ok(&self.prev_row[..raw_row_bytes]))
    }

    /// After all rows consumed, parse post-IDAT chunks for late metadata.
    pub fn finish_metadata(&mut self) {
        // Scan forward from first_idat_pos to skip all IDAT chunks,
        // then collect metadata from post-IDAT chunks.
        let data = self.file_data;
        let mut pos = self.first_idat_pos;

        // Skip all IDAT chunks
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let crc_end = pos + 8 + length + 4;
            if crc_end > data.len() {
                return;
            }
            if chunk_type != *b"IDAT" {
                break;
            }
            pos = crc_end;
        }

        // Now collect post-IDAT chunks
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let data_start = pos + 8;
            let data_end = data_start + length;
            let crc_end = data_end + 4;

            if crc_end > data.len() {
                break;
            }

            if chunk_type == *b"IEND" {
                break;
            }

            let chunk_data = &data[data_start..data_end];
            self.ancillary.collect_late(&ChunkRef {
                chunk_type,
                data: chunk_data,
            });
            pos = crc_end;
        }
    }
}

// ── Post-processing ─────────────────────────────────────────────────

/// Compute output bytes per pixel after post-processing (for limits checks).
fn output_bytes_per_pixel(ihdr: &Ihdr, ancillary: &PngAncillary) -> usize {
    match ihdr.color_type {
        0 => {
            // Grayscale
            if ancillary.trns.is_some() {
                // Gray + tRNS → RGBA8 (for 8-bit) or GrayAlpha16 (4 bytes either way)
                4
            } else if ihdr.bit_depth == 16 {
                2
            } else {
                1
            }
        }
        2 => {
            // RGB
            if ancillary.trns.is_some() {
                if ihdr.bit_depth == 16 { 8 } else { 4 }
            } else if ihdr.bit_depth == 16 {
                6
            } else {
                3
            }
        }
        3 => {
            // Indexed → RGB8 or RGBA8
            if ancillary.trns.is_some() { 4 } else { 3 }
        }
        4 => {
            // GrayAlpha: GA8 → RGBA8 (4 bytes), GA16 → GrayAlpha16 (4 bytes)
            4
        }
        6 => {
            // RGBA
            if ihdr.bit_depth == 16 { 8 } else { 4 }
        }
        _ => 4,
    }
}

/// Scale sub-8-bit gray value to 8-bit.
fn scale_to_8bit(value: u8, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => {
            if value != 0 {
                255
            } else {
                0
            }
        }
        2 => value * 85, // 0→0, 1→85, 2→170, 3→255
        4 => value * 17, // 0→0, 1→17, ..., 15→255
        _ => value,
    }
}

/// Unpack sub-8-bit grayscale pixels from a packed row.
fn unpack_sub_byte_gray(raw: &[u8], width: usize, bit_depth: u8, out: &mut Vec<u8>) {
    let pixels_per_byte = 8 / bit_depth as usize;
    let mask = (1u8 << bit_depth) - 1;

    for x in 0..width {
        let byte_idx = x / pixels_per_byte;
        let bit_offset = (pixels_per_byte - 1 - x % pixels_per_byte) * bit_depth as usize;
        let value = (raw[byte_idx] >> bit_offset) & mask;
        out.push(scale_to_8bit(value, bit_depth));
    }
}

/// Unpack sub-8-bit indexed pixels from a packed row.
fn unpack_sub_byte_indexed(raw: &[u8], width: usize, bit_depth: u8, out: &mut Vec<u8>) {
    let pixels_per_byte = 8 / bit_depth as usize;
    let mask = (1u8 << bit_depth) - 1;

    for x in 0..width {
        let byte_idx = x / pixels_per_byte;
        let bit_offset = (pixels_per_byte - 1 - x % pixels_per_byte) * bit_depth as usize;
        let index = (raw[byte_idx] >> bit_offset) & mask;
        out.push(index);
    }
}

/// Post-process a raw unfiltered row into output pixels.
/// Returns the output pixel data for this row.
pub(crate) fn post_process_row(
    raw: &[u8],
    ihdr: &Ihdr,
    ancillary: &PngAncillary,
    out: &mut Vec<u8>,
) {
    out.clear();
    let width = ihdr.width as usize;

    match ihdr.color_type {
        0 => {
            // Grayscale
            if ihdr.is_sub_byte() {
                if let Some(ref trns) = ancillary.trns {
                    // tRNS value is in original bit depth range
                    let trns_val = if trns.len() >= 2 {
                        u16::from_be_bytes([trns[0], trns[1]])
                    } else {
                        0
                    };
                    // Unpack, compare raw values against tRNS, then scale
                    let pixels_per_byte = 8 / ihdr.bit_depth as usize;
                    let mask = (1u8 << ihdr.bit_depth) - 1;
                    for x in 0..width {
                        let byte_idx = x / pixels_per_byte;
                        let bit_offset =
                            (pixels_per_byte - 1 - x % pixels_per_byte) * ihdr.bit_depth as usize;
                        let raw_val = (raw[byte_idx] >> bit_offset) & mask;
                        let alpha = if raw_val as u16 == trns_val { 0u8 } else { 255 };
                        let g = scale_to_8bit(raw_val, ihdr.bit_depth);
                        out.extend_from_slice(&[g, g, g, alpha]);
                    }
                } else {
                    // Sub-8-bit without tRNS: unpack and scale to 8-bit
                    let mut gray_pixels = Vec::with_capacity(width);
                    unpack_sub_byte_gray(raw, width, ihdr.bit_depth, &mut gray_pixels);
                    out.extend_from_slice(&gray_pixels);
                }
            } else if ihdr.bit_depth == 16 {
                if let Some(ref trns) = ancillary.trns {
                    let trns_val = if trns.len() >= 2 {
                        u16::from_be_bytes([trns[0], trns[1]])
                    } else {
                        0
                    };
                    // Gray16 + tRNS → GrayAlpha16 (4 bytes per pixel, native endian)
                    for chunk in raw.chunks_exact(2) {
                        let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                        let alpha: u16 = if val == trns_val { 0 } else { 65535 };
                        out.extend_from_slice(&val.to_ne_bytes());
                        out.extend_from_slice(&alpha.to_ne_bytes());
                    }
                } else {
                    // Gray16 → native endian
                    for chunk in raw.chunks_exact(2) {
                        let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                        out.extend_from_slice(&val.to_ne_bytes());
                    }
                }
            } else {
                // Gray8
                if let Some(ref trns) = ancillary.trns {
                    let trns_val = if trns.len() >= 2 {
                        u16::from_be_bytes([trns[0], trns[1]])
                    } else {
                        0
                    };
                    // Gray8 + tRNS → RGBA8
                    for &g in raw.iter().take(width) {
                        let alpha = if g as u16 == trns_val { 0u8 } else { 255 };
                        out.extend_from_slice(&[g, g, g, alpha]);
                    }
                } else {
                    out.extend_from_slice(&raw[..width]);
                }
            }
        }
        2 => {
            // RGB
            if ihdr.bit_depth == 16 {
                if let Some(ref trns) = ancillary.trns {
                    // tRNS for RGB: 6 bytes (R16, G16, B16)
                    let (tr, tg, tb) = if trns.len() >= 6 {
                        (
                            u16::from_be_bytes([trns[0], trns[1]]),
                            u16::from_be_bytes([trns[2], trns[3]]),
                            u16::from_be_bytes([trns[4], trns[5]]),
                        )
                    } else {
                        (0, 0, 0)
                    };
                    // RGB16 + tRNS → RGBA16 native endian
                    for chunk in raw.chunks_exact(6) {
                        let r = u16::from_be_bytes([chunk[0], chunk[1]]);
                        let g = u16::from_be_bytes([chunk[2], chunk[3]]);
                        let b = u16::from_be_bytes([chunk[4], chunk[5]]);
                        let alpha: u16 = if r == tr && g == tg && b == tb {
                            0
                        } else {
                            65535
                        };
                        out.extend_from_slice(&r.to_ne_bytes());
                        out.extend_from_slice(&g.to_ne_bytes());
                        out.extend_from_slice(&b.to_ne_bytes());
                        out.extend_from_slice(&alpha.to_ne_bytes());
                    }
                } else {
                    // RGB16 → native endian
                    for chunk in raw.chunks_exact(2) {
                        let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                        out.extend_from_slice(&val.to_ne_bytes());
                    }
                }
            } else {
                // RGB8
                if let Some(ref trns) = ancillary.trns {
                    let (tr, tg, tb) = if trns.len() >= 6 {
                        (
                            u16::from_be_bytes([trns[0], trns[1]]) as u8,
                            u16::from_be_bytes([trns[2], trns[3]]) as u8,
                            u16::from_be_bytes([trns[4], trns[5]]) as u8,
                        )
                    } else {
                        (0, 0, 0)
                    };
                    // RGB8 + tRNS → RGBA8
                    for chunk in raw.chunks_exact(3).take(width) {
                        let alpha = if chunk[0] == tr && chunk[1] == tg && chunk[2] == tb {
                            0u8
                        } else {
                            255
                        };
                        out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], alpha]);
                    }
                } else {
                    let row_bytes = width * 3;
                    out.extend_from_slice(&raw[..row_bytes]);
                }
            }
        }
        3 => {
            // Indexed
            let palette = ancillary.palette.as_deref().unwrap_or(&[]);
            let trns = ancillary.trns.as_deref();
            let has_trns = trns.is_some();

            let indices: Vec<u8> = if ihdr.is_sub_byte() {
                let mut idx = Vec::with_capacity(width);
                unpack_sub_byte_indexed(raw, width, ihdr.bit_depth, &mut idx);
                idx
            } else {
                raw[..width].to_vec()
            };

            for &index in &indices {
                let i = index as usize;
                let (r, g, b) = if i * 3 + 2 < palette.len() {
                    (palette[i * 3], palette[i * 3 + 1], palette[i * 3 + 2])
                } else {
                    (0, 0, 0) // Out of range index
                };
                if has_trns {
                    let alpha = trns.and_then(|t| t.get(i)).copied().unwrap_or(255);
                    out.extend_from_slice(&[r, g, b, alpha]);
                } else {
                    out.extend_from_slice(&[r, g, b]);
                }
            }
        }
        4 => {
            // GrayAlpha
            if ihdr.bit_depth == 16 {
                // GrayAlpha16 → native endian
                for chunk in raw.chunks_exact(4) {
                    let v = u16::from_be_bytes([chunk[0], chunk[1]]);
                    let a = u16::from_be_bytes([chunk[2], chunk[3]]);
                    out.extend_from_slice(&v.to_ne_bytes());
                    out.extend_from_slice(&a.to_ne_bytes());
                }
            } else {
                // GrayAlpha8 → RGBA8 (matches decode.rs:182-192 behavior)
                for chunk in raw.chunks_exact(2).take(width) {
                    let g = chunk[0];
                    let a = chunk[1];
                    out.extend_from_slice(&[g, g, g, a]);
                }
            }
        }
        6 => {
            // RGBA
            if ihdr.bit_depth == 16 {
                // RGBA16 → native endian
                for chunk in raw.chunks_exact(2) {
                    let val = u16::from_be_bytes([chunk[0], chunk[1]]);
                    out.extend_from_slice(&val.to_ne_bytes());
                }
            } else {
                // RGBA8 — pass through
                let row_bytes = width * 4;
                out.extend_from_slice(&raw[..row_bytes]);
            }
        }
        _ => unreachable!("validated in IHDR parsing"),
    }
}

/// Determine the output PixelData variant info for `PngInfo` construction.
pub(crate) struct OutputFormat {
    pub channels: usize,
    pub bytes_per_channel: usize,
}

impl OutputFormat {
    pub fn from_ihdr(ihdr: &Ihdr, ancillary: &PngAncillary) -> Self {
        match ihdr.color_type {
            0 => {
                if ancillary.trns.is_some() {
                    // Gray + tRNS → RGBA
                    if ihdr.bit_depth == 16 {
                        Self {
                            channels: 2,
                            bytes_per_channel: 2,
                        }
                    } else {
                        Self {
                            channels: 4,
                            bytes_per_channel: 1,
                        }
                    }
                } else if ihdr.bit_depth == 16 {
                    Self {
                        channels: 1,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 1,
                        bytes_per_channel: 1,
                    }
                }
            }
            2 => {
                if ancillary.trns.is_some() {
                    if ihdr.bit_depth == 16 {
                        Self {
                            channels: 4,
                            bytes_per_channel: 2,
                        }
                    } else {
                        Self {
                            channels: 4,
                            bytes_per_channel: 1,
                        }
                    }
                } else if ihdr.bit_depth == 16 {
                    Self {
                        channels: 3,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 3,
                        bytes_per_channel: 1,
                    }
                }
            }
            3 => {
                if ancillary.trns.is_some() {
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                } else {
                    Self {
                        channels: 3,
                        bytes_per_channel: 1,
                    }
                }
            }
            4 => {
                if ihdr.bit_depth == 16 {
                    Self {
                        channels: 2,
                        bytes_per_channel: 2,
                    }
                } else {
                    // GA8 → RGBA8
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                }
            }
            6 => {
                if ihdr.bit_depth == 16 {
                    Self {
                        channels: 4,
                        bytes_per_channel: 2,
                    }
                } else {
                    Self {
                        channels: 4,
                        bytes_per_channel: 1,
                    }
                }
            }
            _ => unreachable!(),
        }
    }
}

// ── Adam7 interlacing ───────────────────────────────────────────────

/// Adam7 pass parameters: (x_offset, y_offset, x_step, y_step).
const ADAM7_PASSES: [(usize, usize, usize, usize); 7] = [
    (0, 0, 8, 8), // pass 1
    (4, 0, 8, 8), // pass 2
    (0, 4, 4, 8), // pass 3
    (2, 0, 4, 4), // pass 4
    (0, 2, 2, 4), // pass 5
    (1, 0, 2, 2), // pass 6
    (0, 1, 1, 2), // pass 7
];

/// Compute dimensions of an Adam7 sub-image for a given pass.
fn adam7_pass_size(width: u32, height: u32, pass: usize) -> (u32, u32) {
    let (x_off, y_off, x_step, y_step) = ADAM7_PASSES[pass];
    let w = if width as usize > x_off {
        (width as usize - x_off).div_ceil(x_step)
    } else {
        0
    };
    let h = if height as usize > y_off {
        (height as usize - y_off).div_ceil(y_step)
    } else {
        0
    };
    (w as u32, h as u32)
}

/// Decode an interlaced PNG: decompress all 7 passes, unfilter, scatter to final image,
/// then return the assembled pixel rows.
pub(crate) fn decode_interlaced(
    data: &'_ [u8],
    limits: Option<&crate::decode::PngLimits>,
    cancel: &dyn Stop,
) -> Result<(Ihdr, PngAncillary, Vec<u8>, OutputFormat), PngError> {
    // Validate signature
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        return Err(PngError::Decode("not a PNG file".into()));
    }

    let mut chunks = ChunkIter::new(data);

    // Parse IHDR
    let ihdr_chunk = chunks
        .next()
        .ok_or_else(|| PngError::Decode("empty PNG".into()))??;
    if ihdr_chunk.chunk_type != *b"IHDR" {
        return Err(PngError::Decode("first chunk is not IHDR".into()));
    }
    let ihdr = Ihdr::parse(ihdr_chunk.data)?;

    // Collect pre-IDAT metadata
    let mut ancillary = PngAncillary::default();
    let mut first_idat_pos = None;
    for chunk_result in &mut chunks {
        let chunk = chunk_result?;
        if chunk.chunk_type == *b"IDAT" {
            first_idat_pos = Some(chunks.pos() - 12 - chunk.data.len());
            break;
        }
        ancillary.collect(&chunk)?;
    }
    let first_idat_pos =
        first_idat_pos.ok_or_else(|| PngError::Decode("no IDAT chunk found".into()))?;

    if ihdr.is_indexed() && ancillary.palette.is_none() {
        return Err(PngError::Decode(
            "indexed color type requires PLTE chunk".into(),
        ));
    }

    let fmt = OutputFormat::from_ihdr(&ihdr, &ancillary);

    if let Some(lim) = limits {
        let out_bpp = (fmt.channels * fmt.bytes_per_channel) as u32;
        lim.validate(ihdr.width, ihdr.height, out_bpp)?;
    }

    let bpp = ihdr.filter_bpp();
    let width = ihdr.width;
    let height = ihdr.height;

    // Allocate final output image
    let out_row_bytes = width as usize * fmt.channels * fmt.bytes_per_channel;
    let mut final_pixels = vec![0u8; out_row_bytes * height as usize];

    // Create IDAT source and decompressor
    let source = IdatSource::new(data, first_idat_pos);
    let mut decompressor = zenflate::StreamDecompressor::zlib(source, 32768);

    // Process each Adam7 pass
    for (pass, &(x_off, y_off, x_step, y_step)) in ADAM7_PASSES.iter().enumerate() {
        let (pw, ph) = adam7_pass_size(width, height, pass);
        if pw == 0 || ph == 0 {
            continue;
        }

        // Compute stride for this sub-image
        let bits_per_row = pw as usize * ihdr.channels() * ihdr.bit_depth as usize;
        let raw_row_bytes = bits_per_row.div_ceil(8);
        let pass_stride = 1 + raw_row_bytes;

        let mut prev_row = vec![0u8; raw_row_bytes];
        let mut current_row = vec![0u8; raw_row_bytes];
        let mut row_buf = Vec::new();

        for pass_y in 0..ph as usize {
            cancel.check()?;
            // Fill decompressor until we have a full stride
            loop {
                let available = decompressor.peek().len();
                if available >= pass_stride {
                    break;
                }
                if decompressor.is_done() {
                    return Err(PngError::Decode(alloc::format!(
                        "truncated interlaced data in pass {}",
                        pass + 1
                    )));
                }
                decompressor
                    .fill()
                    .map_err(|e| PngError::Decode(alloc::format!("decompression error: {e:?}")))?;
            }

            let peeked = decompressor.peek();
            let filter_byte = peeked[0];
            current_row[..raw_row_bytes].copy_from_slice(&peeked[1..pass_stride]);
            decompressor.advance(pass_stride);

            unfilter_row(
                filter_byte,
                &mut current_row[..raw_row_bytes],
                &prev_row,
                bpp,
            )?;

            // Post-process this sub-image row
            // Create a temporary Ihdr with the sub-image width for post-processing
            let sub_ihdr = Ihdr {
                width: pw,
                height: ph,
                ..ihdr
            };
            post_process_row(
                &current_row[..raw_row_bytes],
                &sub_ihdr,
                &ancillary,
                &mut row_buf,
            );

            // Scatter pixels to final positions
            let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
            let dest_y = y_off + pass_y * y_step;
            if dest_y < height as usize {
                for px in 0..pw as usize {
                    let dest_x = x_off + px * x_step;
                    if dest_x < width as usize {
                        let src_offset = px * pixel_bytes;
                        let dst_offset = dest_y * out_row_bytes + dest_x * pixel_bytes;
                        if src_offset + pixel_bytes <= row_buf.len()
                            && dst_offset + pixel_bytes <= final_pixels.len()
                        {
                            final_pixels[dst_offset..dst_offset + pixel_bytes]
                                .copy_from_slice(&row_buf[src_offset..src_offset + pixel_bytes]);
                        }
                    }
                }
            }

            core::mem::swap(&mut current_row, &mut prev_row);
        }
    }

    // Collect post-IDAT metadata: scan forward from first_idat_pos, skip IDATs
    {
        let mut pos = first_idat_pos;
        // Skip IDAT chunks
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let crc_end = pos + 8 + length + 4;
            if crc_end > data.len() {
                break;
            }
            if chunk_type != *b"IDAT" {
                break;
            }
            pos = crc_end;
        }
        // Collect late metadata
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let data_start = pos + 8;
            let data_end = data_start + length;
            let crc_end = data_end + 4;
            if crc_end > data.len() {
                break;
            }
            if chunk_type == *b"IEND" {
                break;
            }
            let chunk_data = &data[data_start..data_end];
            ancillary.collect_late(&ChunkRef {
                chunk_type,
                data: chunk_data,
            });
            pos = crc_end;
        }
    }

    Ok((ihdr, ancillary, final_pixels, fmt))
}

// ── PngInfo construction ────────────────────────────────────────────

use crate::decode::{PngChromaticities, PngInfo};
use zencodec_types::{Cicp, ContentLightLevel, MasteringDisplay};

/// Build `PngInfo` from parsed IHDR and ancillary metadata.
pub(crate) fn build_png_info(ihdr: &Ihdr, ancillary: &PngAncillary) -> PngInfo {
    let has_alpha = ihdr.has_alpha() || ancillary.trns.is_some();
    let has_animation = ancillary.actl.is_some();
    let frame_count = ancillary.actl.map_or(1, |(n, _)| n);

    let source_gamma = ancillary.gamma;
    let srgb_intent = ancillary.srgb_intent;

    let chromaticities = ancillary.chrm.map(|c| PngChromaticities {
        white_x: c[0],
        white_y: c[1],
        red_x: c[2],
        red_y: c[3],
        green_x: c[4],
        green_y: c[5],
        blue_x: c[6],
        blue_y: c[7],
    });

    let cicp = ancillary
        .cicp
        .map(|c| Cicp::new(c[0], c[1], c[2], c[3] != 0));

    let content_light_level = ancillary.clli.map(|c| {
        ContentLightLevel::new(
            (c[0] / 10000).min(65535) as u16,
            (c[1] / 10000).min(65535) as u16,
        )
    });

    let mastering_display = ancillary.mdcv.as_ref().and_then(|m| {
        if m.len() < 24 {
            return None;
        }
        // mDCV: 6×u16 BE chromaticities (Rx, Ry, Gx, Gy, Bx, By)
        //     + 2×u16 BE white point (Wx, Wy)
        //     + u32 BE max_luminance + u32 BE min_luminance
        // = 24 bytes. u16 values in units of 0.00002 (same as MasteringDisplay).
        let read_u16 = |off: usize| u16::from_be_bytes(m[off..off + 2].try_into().unwrap());
        let read_u32 = |off: usize| u32::from_be_bytes(m[off..off + 4].try_into().unwrap());

        Some(MasteringDisplay::new(
            [
                [read_u16(0), read_u16(2)],  // Red
                [read_u16(4), read_u16(6)],  // Green
                [read_u16(8), read_u16(10)], // Blue
            ],
            [read_u16(12), read_u16(14)], // White point
            read_u32(16),                 // max_luminance
            read_u32(20),                 // min_luminance
        ))
    });

    PngInfo {
        width: ihdr.width,
        height: ihdr.height,
        has_alpha,
        has_animation,
        frame_count,
        bit_depth: ihdr.bit_depth,
        icc_profile: ancillary.icc_profile.clone(),
        exif: ancillary.exif.clone(),
        xmp: ancillary.xmp.clone(),
        source_gamma,
        srgb_intent,
        chromaticities,
        cicp,
        content_light_level,
        mastering_display,
    }
}

// ── Probe helper ────────────────────────────────────────────────────

/// Probe PNG metadata without decoding pixels.
pub(crate) fn probe_png(data: &[u8]) -> Result<PngInfo, PngError> {
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        return Err(PngError::Decode("not a PNG file".into()));
    }

    let mut chunks = ChunkIter::new(data);

    let ihdr_chunk = chunks
        .next()
        .ok_or_else(|| PngError::Decode("empty PNG".into()))??;
    if ihdr_chunk.chunk_type != *b"IHDR" {
        return Err(PngError::Decode("first chunk is not IHDR".into()));
    }
    let ihdr = Ihdr::parse(ihdr_chunk.data)?;

    let mut ancillary = PngAncillary::default();
    for chunk_result in &mut chunks {
        let chunk = chunk_result?;
        match &chunk.chunk_type {
            b"IDAT" => break,
            b"IEND" => break,
            _ => {
                ancillary.collect(&chunk)?;
            }
        }
    }

    // Also scan post-IDAT chunks for late metadata
    for chunk_result in chunks {
        let chunk = chunk_result?;
        if chunk.chunk_type == *b"IEND" {
            break;
        }
        ancillary.collect_late(&chunk);
    }

    Ok(build_png_info(&ihdr, &ancillary))
}

// ── Full decode ─────────────────────────────────────────────────────

use imgref::ImgVec;
use rgb::{Gray, Rgb, Rgba};
use zencodec_types::{GrayAlpha, PixelData};

use crate::decode::PngDecodeOutput;

/// Decode PNG to pixels using our own decoder.
pub(crate) fn decode_png(
    data: &[u8],
    limits: Option<&crate::decode::PngLimits>,
    cancel: &dyn Stop,
) -> Result<PngDecodeOutput, PngError> {
    // Check for interlacing first
    if data.len() >= 29 && data[..8] == PNG_SIGNATURE {
        let interlace = data[28]; // IHDR interlace byte
        if interlace == 1 {
            return decode_interlaced_to_output(data, limits, cancel);
        }
    }

    let mut reader = RowDecoder::new(data, limits)?;
    let ihdr = *reader.ihdr();
    let fmt = OutputFormat::from_ihdr(&ihdr, reader.ancillary());

    let w = ihdr.width as usize;
    let h = ihdr.height as usize;
    let pixel_bytes = fmt.channels * fmt.bytes_per_channel;
    let out_row_bytes = w * pixel_bytes;

    let mut all_pixels = Vec::with_capacity(out_row_bytes * h);
    let mut row_buf = Vec::new();
    let mut raw_copy = vec![0u8; ihdr.raw_row_bytes()];

    while let Some(result) = reader.next_raw_row() {
        let raw = result?;
        cancel.check()?;
        raw_copy[..raw.len()].copy_from_slice(raw);
        post_process_row(
            &raw_copy[..raw.len()],
            &ihdr,
            reader.ancillary(),
            &mut row_buf,
        );
        all_pixels.extend_from_slice(&row_buf);
    }

    reader.finish_metadata();

    let info = build_png_info(&ihdr, reader.ancillary());
    let pixels = build_pixel_data(&ihdr, reader.ancillary(), all_pixels, w, h)?;

    Ok(PngDecodeOutput { pixels, info })
}

/// Decode interlaced PNG to PngDecodeOutput.
fn decode_interlaced_to_output(
    data: &[u8],
    limits: Option<&crate::decode::PngLimits>,
    cancel: &dyn Stop,
) -> Result<PngDecodeOutput, PngError> {
    let (ihdr, ancillary, pixels, _fmt) = decode_interlaced(data, limits, cancel)?;
    let w = ihdr.width as usize;
    let h = ihdr.height as usize;
    let info = build_png_info(&ihdr, &ancillary);
    let pixel_data = build_pixel_data(&ihdr, &ancillary, pixels, w, h)?;
    Ok(PngDecodeOutput {
        pixels: pixel_data,
        info,
    })
}

/// Build PixelData from the fully assembled pixel bytes.
fn build_pixel_data(
    ihdr: &Ihdr,
    ancillary: &PngAncillary,
    pixels: Vec<u8>,
    w: usize,
    h: usize,
) -> Result<PixelData, PngError> {
    match (ihdr.color_type, ihdr.bit_depth, ancillary.trns.is_some()) {
        // Grayscale
        (0, 16, false) => {
            let gray: &[Gray<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Gray16(ImgVec::new(gray.to_vec(), w, h)))
        }
        (0, 16, true) => {
            // Gray16 + tRNS → GrayAlpha16 (already processed to native u16 pairs)
            let ga: &[[u16; 2]] = bytemuck::cast_slice(&pixels);
            let ga_pixels: Vec<GrayAlpha<u16>> =
                ga.iter().map(|&[v, a]| GrayAlpha::new(v, a)).collect();
            Ok(PixelData::GrayAlpha16(ImgVec::new(ga_pixels, w, h)))
        }
        (0, _, false) if ihdr.bit_depth <= 8 => {
            let gray: Vec<Gray<u8>> = pixels.iter().map(|&g| Gray(g)).collect();
            Ok(PixelData::Gray8(ImgVec::new(gray, w, h)))
        }
        (0, _, true) if ihdr.bit_depth <= 8 => {
            // Gray + tRNS → RGBA8
            let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h)))
        }
        // RGB
        (2, 16, false) => {
            let rgb: &[Rgb<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgb16(ImgVec::new(rgb.to_vec(), w, h)))
        }
        (2, 16, true) => {
            let rgba: &[Rgba<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgba16(ImgVec::new(rgba.to_vec(), w, h)))
        }
        (2, 8, false) => {
            let rgb: &[Rgb<u8>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgb8(ImgVec::new(rgb.to_vec(), w, h)))
        }
        (2, 8, true) => {
            let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h)))
        }
        // Indexed
        (3, _, true) => {
            let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h)))
        }
        (3, _, false) => {
            let rgb: &[Rgb<u8>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgb8(ImgVec::new(rgb.to_vec(), w, h)))
        }
        // GrayAlpha
        (4, 16, _) => {
            let ga: &[[u16; 2]] = bytemuck::cast_slice(&pixels);
            let ga_pixels: Vec<GrayAlpha<u16>> =
                ga.iter().map(|&[v, a]| GrayAlpha::new(v, a)).collect();
            Ok(PixelData::GrayAlpha16(ImgVec::new(ga_pixels, w, h)))
        }
        (4, 8, _) => {
            // GA8 already expanded to RGBA8
            let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h)))
        }
        // RGBA
        (6, 16, _) => {
            let rgba: &[Rgba<u16>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgba16(ImgVec::new(rgba.to_vec(), w, h)))
        }
        (6, 8, _) => {
            let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&pixels);
            Ok(PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h)))
        }
        _ => Err(PngError::Decode(alloc::format!(
            "unsupported color_type={} bit_depth={}",
            ihdr.color_type,
            ihdr.bit_depth
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_parser_validates_signature() {
        let result = decode_png(b"not a png", None, &Unstoppable);
        assert!(result.is_err());
    }

    #[test]
    fn unfilter_none() {
        let mut row = vec![10, 20, 30];
        let prev = vec![0, 0, 0];
        unfilter_row(0, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![10, 20, 30]);
    }

    #[test]
    fn unfilter_sub() {
        // Sub: each byte adds the byte bpp positions to the left
        let mut row = vec![10, 5, 3];
        let prev = vec![0, 0, 0];
        unfilter_row(1, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![10, 15, 18]);
    }

    #[test]
    fn unfilter_up() {
        let mut row = vec![10, 20, 30];
        let prev = vec![5, 10, 15];
        unfilter_row(2, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![15, 30, 45]);
    }

    #[test]
    fn unfilter_average() {
        let mut row = vec![10, 5, 3];
        let prev = vec![0, 0, 0];
        unfilter_row(3, &mut row, &prev, 1).unwrap();
        // i=0: row[0] += prev[0] >> 1 = 10 + 0 = 10
        // i=1: row[1] += floor((row[0] + prev[1]) / 2) = 5 + 5 = 10
        // i=2: row[2] += floor((row[1] + prev[2]) / 2) = 3 + 5 = 8
        assert_eq!(row, vec![10, 10, 8]);
    }

    #[test]
    fn unfilter_paeth() {
        let mut row = vec![10, 5, 3];
        let prev = vec![0, 0, 0];
        unfilter_row(4, &mut row, &prev, 1).unwrap();
        // i=0: paeth(0, 0, 0) = 0, so 10 + 0 = 10
        // i=1: paeth(10, 0, 0) = 10, so 5 + 10 = 15
        // i=2: paeth(15, 0, 0) = 15, so 3 + 15 = 18
        assert_eq!(row, vec![10, 15, 18]);
    }

    #[test]
    fn ihdr_validates_color_type_bit_depth() {
        // Valid: Gray 8-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 8, 0, 0)).is_ok());
        // Valid: Indexed 4-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 4, 3, 0)).is_ok());
        // Invalid: RGB 4-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 4, 2, 0)).is_err());
        // Invalid: Indexed 16-bit
        assert!(Ihdr::parse(&make_ihdr(1, 1, 16, 3, 0)).is_err());
    }

    fn make_ihdr(w: u32, h: u32, bit_depth: u8, color_type: u8, interlace: u8) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&w.to_be_bytes());
        data.extend_from_slice(&h.to_be_bytes());
        data.push(bit_depth);
        data.push(color_type);
        data.push(0); // compression
        data.push(0); // filter
        data.push(interlace);
        data
    }

    #[test]
    fn scale_to_8bit_values() {
        assert_eq!(scale_to_8bit(0, 1), 0);
        assert_eq!(scale_to_8bit(1, 1), 255);
        assert_eq!(scale_to_8bit(0, 2), 0);
        assert_eq!(scale_to_8bit(1, 2), 85);
        assert_eq!(scale_to_8bit(3, 2), 255);
        assert_eq!(scale_to_8bit(0, 4), 0);
        assert_eq!(scale_to_8bit(15, 4), 255);
    }

    /// Compare our decoder's pixel output against the reference png crate
    /// for every PNGSuite file.
    #[test]
    fn pngsuite_comparison() {
        let suite_dir = "/home/lilith/work/codec-corpus/pngsuite";
        if !std::path::Path::new(suite_dir).exists() {
            eprintln!("Skipping PNGSuite comparison: directory not found");
            return;
        }

        let mut tested = 0;
        let mut skipped = 0;
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(suite_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }

            let filename = path.file_name().unwrap().to_str().unwrap().to_string();

            // Skip files that are intentionally corrupt (start with 'x')
            if filename.starts_with('x') {
                skipped += 1;
                continue;
            }

            let data = std::fs::read(&path).unwrap();

            // Decode with our decoder
            let our_result = decode_png(&data, None, &Unstoppable);

            // Decode with reference png crate
            let ref_result = decode_with_png_crate(&data);

            match (our_result, ref_result) {
                (Ok(ours), Ok(reference)) => {
                    // Compare pixel data
                    let our_bytes = pixel_data_to_bytes(&ours.pixels);
                    let ref_bytes = pixel_data_to_bytes(&reference.pixels);

                    if our_bytes != ref_bytes {
                        // Check if it's a format difference we can explain
                        let our_desc = format_pixel_data(&ours.pixels);
                        let ref_desc = format_pixel_data(&reference.pixels);
                        failures.push(alloc::format!(
                            "{}: pixel mismatch (ours={}, ref={}, our_len={}, ref_len={})",
                            filename,
                            our_desc,
                            ref_desc,
                            our_bytes.len(),
                            ref_bytes.len()
                        ));
                    } else {
                        tested += 1;
                    }
                }
                (Err(e), Ok(_)) => {
                    failures.push(alloc::format!(
                        "{}: we failed but ref succeeded: {}",
                        filename,
                        e
                    ));
                }
                (Ok(_), Err(_)) => {
                    // We succeeded where ref failed — that's fine
                    tested += 1;
                }
                (Err(_), Err(_)) => {
                    // Both failed — that's fine
                    skipped += 1;
                }
            }
        }

        eprintln!(
            "PNGSuite: {} matched, {} skipped, {} failures",
            tested,
            skipped,
            failures.len()
        );
        if !failures.is_empty() {
            for f in &failures {
                eprintln!("  FAIL: {}", f);
            }
            panic!(
                "{} PNGSuite comparison failures (see stderr)",
                failures.len()
            );
        }
    }

    /// Decode using the reference png crate.
    fn decode_with_png_crate(data: &[u8]) -> Result<PngDecodeOutput, String> {
        use std::io::Cursor;

        let cursor = Cursor::new(data);
        let mut decoder = png::Decoder::new(cursor);
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let w = reader.info().width as usize;
        let h = reader.info().height as usize;
        let src_bit_depth = reader.info().bit_depth as u8;

        let (ct, bd) = reader.output_color_type();
        let buffer_size = reader.output_buffer_size().ok_or("no buffer size")?;
        let mut raw_pixels = vec![0u8; buffer_size];
        let output_info = reader
            .next_frame(&mut raw_pixels)
            .map_err(|e| e.to_string())?;
        raw_pixels.truncate(output_info.buffer_size());

        // Convert to native endian for 16-bit
        let pixels = match (ct, bd) {
            (png::ColorType::Rgba, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let rgba: &[Rgba<u16>] = bytemuck::cast_slice(&native);
                PixelData::Rgba16(ImgVec::new(rgba.to_vec(), w, h))
            }
            (png::ColorType::Rgba, _) => {
                let rgba: &[Rgba<u8>] = bytemuck::cast_slice(&raw_pixels);
                PixelData::Rgba8(ImgVec::new(rgba.to_vec(), w, h))
            }
            (png::ColorType::Rgb, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let rgb: &[Rgb<u16>] = bytemuck::cast_slice(&native);
                PixelData::Rgb16(ImgVec::new(rgb.to_vec(), w, h))
            }
            (png::ColorType::Rgb, _) => {
                let rgb: &[Rgb<u8>] = bytemuck::cast_slice(&raw_pixels);
                PixelData::Rgb8(ImgVec::new(rgb.to_vec(), w, h))
            }
            (png::ColorType::GrayscaleAlpha, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let ga: &[[u16; 2]] = bytemuck::cast_slice(&native);
                let pixels: Vec<GrayAlpha<u16>> =
                    ga.iter().map(|&[v, a]| GrayAlpha::new(v, a)).collect();
                PixelData::GrayAlpha16(ImgVec::new(pixels, w, h))
            }
            (png::ColorType::GrayscaleAlpha, _) => {
                // GA8 → RGBA8 (matches our decoder behavior)
                let rgba: Vec<Rgba<u8>> = raw_pixels
                    .chunks_exact(2)
                    .map(|ga| Rgba {
                        r: ga[0],
                        g: ga[0],
                        b: ga[0],
                        a: ga[1],
                    })
                    .collect();
                PixelData::Rgba8(ImgVec::new(rgba, w, h))
            }
            (png::ColorType::Grayscale, png::BitDepth::Sixteen) => {
                let native = be_to_native_16_ref(&raw_pixels);
                let gray: &[Gray<u16>] = bytemuck::cast_slice(&native);
                PixelData::Gray16(ImgVec::new(gray.to_vec(), w, h))
            }
            (png::ColorType::Grayscale, _) => {
                let gray: Vec<Gray<u8>> = raw_pixels.iter().map(|&g| Gray(g)).collect();
                PixelData::Gray8(ImgVec::new(gray, w, h))
            }
            (png::ColorType::Indexed, _) => {
                return Err("indexed not expanded".into());
            }
        };

        let info = PngInfo {
            width: w as u32,
            height: h as u32,
            has_alpha: false,
            has_animation: false,
            frame_count: 1,
            bit_depth: src_bit_depth,
            icc_profile: None,
            exif: None,
            xmp: None,
            source_gamma: None,
            srgb_intent: None,
            chromaticities: None,
            cicp: None,
            content_light_level: None,
            mastering_display: None,
        };

        Ok(PngDecodeOutput { pixels, info })
    }

    fn be_to_native_16_ref(bytes: &[u8]) -> Vec<u8> {
        if cfg!(target_endian = "big") {
            return bytes.to_vec();
        }
        let mut out = bytes.to_vec();
        for chunk in out.chunks_exact_mut(2) {
            chunk.swap(0, 1);
        }
        out
    }

    fn pixel_data_to_bytes(pixels: &PixelData) -> Vec<u8> {
        use rgb::ComponentBytes;
        match pixels {
            PixelData::Rgb8(img) => img.buf().as_bytes().to_vec(),
            PixelData::Rgba8(img) => img.buf().as_bytes().to_vec(),
            PixelData::Gray8(img) => img.buf().as_bytes().to_vec(),
            PixelData::Rgb16(img) => bytemuck::cast_slice::<Rgb<u16>, u8>(img.buf()).to_vec(),
            PixelData::Rgba16(img) => bytemuck::cast_slice::<Rgba<u16>, u8>(img.buf()).to_vec(),
            PixelData::Gray16(img) => bytemuck::cast_slice::<Gray<u16>, u8>(img.buf()).to_vec(),
            PixelData::GrayAlpha16(img) => {
                let mut bytes = Vec::with_capacity(img.buf().len() * 4);
                for ga in img.buf() {
                    bytes.extend_from_slice(&ga.v.to_ne_bytes());
                    bytes.extend_from_slice(&ga.a.to_ne_bytes());
                }
                bytes
            }
            _ => Vec::new(),
        }
    }

    fn format_pixel_data(pixels: &PixelData) -> &'static str {
        match pixels {
            PixelData::Rgb8(_) => "Rgb8",
            PixelData::Rgba8(_) => "Rgba8",
            PixelData::Gray8(_) => "Gray8",
            PixelData::Rgb16(_) => "Rgb16",
            PixelData::Rgba16(_) => "Rgba16",
            PixelData::Gray16(_) => "Gray16",
            PixelData::GrayAlpha16(_) => "GrayAlpha16",
            _ => "Other",
        }
    }

    /// Walk a directory tree collecting all .png files.
    fn collect_pngs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut pngs = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for entry in entries {
                let Ok(entry) = entry else { continue };
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("png") {
                    pngs.push(path);
                }
            }
        }
        pngs.sort();
        pngs
    }

    /// Mass-test our decoder against the png crate on all PNG corpuses.
    #[test]
    fn corpus_comparison_vs_png_crate() {
        let corpus_dirs: Vec<(&str, &str)> = vec![
            ("codec-corpus", "/home/lilith/work/codec-corpus"),
            ("discover", "/mnt/v/discover/images"),
            ("kodak", "/mnt/v/discover/kodak/images"),
            ("image-png", "/home/lilith/work/jpeg-encoder/external/image-png"),
        ];

        let mut total_tested = 0u32;
        let mut total_skipped = 0u32;
        let mut total_both_err = 0u32;
        let mut failures: Vec<String> = Vec::new();

        for (corpus_name, dir) in &corpus_dirs {
            let dir_path = std::path::Path::new(dir);
            if !dir_path.exists() {
                eprintln!("Corpus '{}' not found at {}, skipping", corpus_name, dir);
                continue;
            }

            let pngs = collect_pngs(dir_path);
            eprintln!("Corpus '{}': {} PNG files found", corpus_name, pngs.len());

            let mut corpus_tested = 0u32;
            let mut corpus_skipped = 0u32;
            let mut corpus_both_err = 0u32;

            for path in &pngs {
                let filename = path.strip_prefix(dir_path).unwrap_or(path);
                let filename_str = filename.display().to_string();

                // Skip intentionally corrupt PNGSuite files
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem.starts_with('x') && filename_str.contains("pngsuite") {
                        corpus_skipped += 1;
                        continue;
                    }
                }

                let data = match std::fs::read(path) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("  read error {}: {}", filename_str, e);
                        corpus_skipped += 1;
                        continue;
                    }
                };

                let our_result = decode_png(&data, None, &Unstoppable);
                let ref_result = decode_with_png_crate(&data);

                match (our_result, ref_result) {
                    (Ok(ours), Ok(reference)) => {
                        let our_bytes = pixel_data_to_bytes(&ours.pixels);
                        let ref_bytes = pixel_data_to_bytes(&reference.pixels);

                        if our_bytes != ref_bytes {
                            let our_desc = format_pixel_data(&ours.pixels);
                            let ref_desc = format_pixel_data(&reference.pixels);
                            failures.push(alloc::format!(
                                "[{}] {}: pixel mismatch (ours={}, ref={}, our_len={}, ref_len={})",
                                corpus_name, filename_str, our_desc, ref_desc,
                                our_bytes.len(), ref_bytes.len()
                            ));
                        } else {
                            corpus_tested += 1;
                        }
                    }
                    (Err(e), Ok(_)) => {
                        failures.push(alloc::format!(
                            "[{}] {}: we failed but ref succeeded: {}",
                            corpus_name, filename_str, e
                        ));
                    }
                    (Ok(_), Err(_)) => {
                        // We decode something the ref can't — that's fine
                        corpus_tested += 1;
                    }
                    (Err(_), Err(_)) => {
                        corpus_both_err += 1;
                    }
                }
            }

            eprintln!(
                "  {} matched, {} skipped, {} both-err, {} failures so far",
                corpus_tested, corpus_skipped, corpus_both_err, failures.len()
            );
            total_tested += corpus_tested;
            total_skipped += corpus_skipped;
            total_both_err += corpus_both_err;
        }

        eprintln!(
            "\n=== TOTAL: {} matched, {} skipped, {} both-err, {} failures ===",
            total_tested, total_skipped, total_both_err, failures.len()
        );

        if !failures.is_empty() {
            eprintln!("\nFailures:");
            for f in &failures {
                eprintln!("  FAIL: {}", f);
            }
            panic!(
                "{} corpus comparison failures (see stderr)",
                failures.len()
            );
        }
    }
}
