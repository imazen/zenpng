//! Streaming row decoder: IDAT source and row-by-row decompression + unfilter.

use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;

use zenflate::crc32;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::chunk::{ChunkIter, ChunkRef};
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use super::postprocess::output_bytes_per_pixel;

// ── IdatSource — InputSource for IDAT chunks ────────────────────────

/// Streams raw IDAT chunk payload bytes to `StreamDecompressor`.
/// Walks chunks in the file without collecting IDAT data into a Vec.
pub(crate) struct IdatSource<'a> {
    /// Full PNG file bytes (borrowed or owned).
    data: Cow<'a, [u8]>,
    /// Byte offset of the next chunk header to check.
    chunk_pos: usize,
    /// Range (start, end) into `data` for remaining bytes in the current IDAT chunk.
    current_range: (usize, usize),
    /// True when we've seen a non-IDAT chunk after IDAT.
    done: bool,
    /// Position of the first post-IDAT chunk (for metadata collection).
    pub post_idat_pos: usize,
    /// Whether to skip CRC validation on IDAT chunks.
    skip_crc: bool,
}

impl<'a> IdatSource<'a> {
    /// Borrow the full PNG file data stored in this source.
    pub fn file_data(&self) -> &[u8] {
        &self.data
    }

    /// Create a new IDAT source positioned at the first IDAT chunk.
    /// `first_idat_pos` is the byte offset of the first IDAT chunk header.
    /// When `skip_crc` is true, IDAT CRC mismatches are tolerated.
    pub fn new(data: Cow<'a, [u8]>, first_idat_pos: usize, skip_crc: bool) -> Self {
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
            current_range: (data_start, data_end),
            done: false,
            post_idat_pos: 0,
            skip_crc,
        }
    }
}

impl zenflate::InputSource for IdatSource<'_> {
    type Error = PngError;

    fn fill_buf(&mut self) -> Result<&[u8], PngError> {
        let (start, end) = self.current_range;
        if start < end {
            return Ok(&self.data[start..end]);
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
            let Some(data_end) = data_start.checked_add(length) else {
                return Err(PngError::Decode("IDAT chunk length overflow".into()));
            };
            let Some(crc_end) = data_end.checked_add(4) else {
                return Err(PngError::Decode("IDAT chunk length overflow".into()));
            };

            if crc_end > self.data.len() {
                return Err(PngError::Decode("truncated IDAT chunk".into()));
            }

            if chunk_type != *b"IDAT" {
                // Not IDAT — we're done with the IDAT stream
                self.done = true;
                self.post_idat_pos = self.chunk_pos;
                return Ok(&[]);
            }

            // Validate CRC (skip computation entirely when skip_crc is set)
            if !self.skip_crc {
                let stored_crc =
                    u32::from_be_bytes(self.data[data_end..crc_end].try_into().unwrap());
                let computed_crc = crc32(crc32(0, &chunk_type), &self.data[data_start..data_end]);
                if stored_crc != computed_crc {
                    return Err(PngError::Decode("CRC mismatch in IDAT chunk".into()));
                }
            }

            self.current_range = (data_start, data_end);
            self.chunk_pos = crc_end;

            if data_start < data_end {
                return Ok(&self.data[data_start..data_end]);
            }
            // Empty IDAT chunk — skip and try next
        }
    }

    fn consume(&mut self, n: usize) {
        self.current_range.0 += n;
    }
}

// ── FdatSource — InputSource for fdAT chunks ────────────────────────

/// Streams raw fdAT chunk payload bytes to `StreamDecompressor`.
/// Like `IdatSource`, but for fdAT chunks. Each fdAT starts with a
/// 4-byte sequence number that is skipped to get the deflate payload.
pub(crate) struct FdatSource<'a> {
    /// Full PNG file bytes.
    data: &'a [u8],
    /// Byte offset of the next chunk header to check.
    chunk_pos: usize,
    /// Remaining bytes in the current fdAT chunk's deflate data.
    current_data: &'a [u8],
    /// True when we've seen a non-fdAT chunk after fdAT.
    done: bool,
    /// Position of the first post-fdAT chunk (for scanning).
    pub post_fdat_pos: usize,
    /// Whether to skip CRC validation on fdAT chunks.
    skip_crc: bool,
}

impl<'a> FdatSource<'a> {
    /// Create a new fdAT source positioned at the first fdAT chunk.
    /// `first_fdat_pos` is the byte offset of the first fdAT chunk header.
    pub fn new(data: &'a [u8], first_fdat_pos: usize, skip_crc: bool) -> Result<Self, PngError> {
        // Parse the first fdAT chunk inline
        let length_bytes: [u8; 4] = data
            .get(first_fdat_pos..first_fdat_pos + 4)
            .ok_or_else(|| PngError::Decode("truncated fdAT chunk header (length)".into()))?
            .try_into()
            .unwrap(); // infallible: slice is exactly 4 bytes
        let length = u32::from_be_bytes(length_bytes) as usize;
        let data_start = first_fdat_pos + 8; // skip length + type
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| PngError::Decode("fdAT chunk length overflow".into()))?;
        let next_pos = data_end
            .checked_add(4)
            .ok_or_else(|| PngError::Decode("fdAT chunk length overflow".into()))?;

        if data_end > data.len() {
            return Err(PngError::Decode("truncated fdAT chunk data".into()));
        }
        if next_pos > data.len() {
            return Err(PngError::Decode("truncated fdAT chunk CRC".into()));
        }

        // Skip the 4-byte sequence number to get to deflate data
        let deflate_start = data_start + 4;
        let deflate_data = if deflate_start < data_end {
            &data[deflate_start..data_end]
        } else {
            &data[data_end..data_end] // empty
        };

        Ok(Self {
            data,
            chunk_pos: next_pos,
            current_data: deflate_data,
            done: false,
            post_fdat_pos: 0,
            skip_crc,
        })
    }
}

impl<'a> zenflate::InputSource for FdatSource<'a> {
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
                self.post_fdat_pos = self.chunk_pos;
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
            let Some(data_end) = data_start.checked_add(length) else {
                return Err(PngError::Decode("fdAT chunk length overflow".into()));
            };
            let Some(crc_end) = data_end.checked_add(4) else {
                return Err(PngError::Decode("fdAT chunk length overflow".into()));
            };

            if crc_end > self.data.len() {
                return Err(PngError::Decode("truncated fdAT chunk".into()));
            }

            if chunk_type != *b"fdAT" {
                // Not fdAT — we're done with this frame's data
                self.done = true;
                self.post_fdat_pos = self.chunk_pos;
                return Ok(&[]);
            }

            // Validate CRC (skip computation entirely when skip_crc is set)
            if !self.skip_crc {
                let stored_crc =
                    u32::from_be_bytes(self.data[data_end..crc_end].try_into().unwrap());
                let computed_crc = crc32(crc32(0, &chunk_type), &self.data[data_start..data_end]);
                if stored_crc != computed_crc {
                    return Err(PngError::Decode("CRC mismatch in fdAT chunk".into()));
                }
            }

            // Skip 4-byte sequence number to get deflate data
            let deflate_start = data_start + 4;
            if deflate_start < data_end {
                self.current_data = &self.data[deflate_start..data_end];
            } else {
                self.current_data = &[];
            }
            self.chunk_pos = crc_end;

            if !self.current_data.is_empty() {
                return Ok(self.current_data);
            }
            // Empty fdAT chunk — skip and try next
        }
    }

    fn consume(&mut self, n: usize) {
        self.current_data = &self.current_data[n..];
    }
}

// ── Unfilter ────────────────────────────────────────────────────────

/// Apply inverse filter to a row in-place given the previous (unfiltered) row.
///
/// Dispatches to SIMD-accelerated implementations (AVX2/SSE2) for Up, Paeth,
/// Average, and Sub filters with bpp=4, falling back to scalar for other bpp.
pub(super) fn unfilter_row(
    filter_type: u8,
    row: &mut [u8],
    prev: &[u8],
    bpp: usize,
) -> crate::error::Result<()> {
    crate::simd::unfilter_row(filter_type, row, prev, bpp)
}

// ── RowDecoder ──────────────────────────────────────────────────────

/// Streaming PNG row decoder. Reads IDAT chunks through `StreamDecompressor`,
/// unfilters each scanline, and yields raw (unfiltered) row data.
pub(crate) struct RowDecoder<'a> {
    decompressor: zenflate::StreamDecompressor<IdatSource<'a>>,
    ihdr: Ihdr,
    ancillary: PngAncillary,

    /// Byte offset of the first IDAT chunk header.
    first_idat_pos: usize,

    prev_row: Vec<u8>,
    current_row: Vec<u8>,
    rows_yielded: u32,
    stride: usize,
    bpp: usize,

    /// Warnings collected from chunk CRC validation.
    chunk_warnings: Vec<crate::decode::PngWarning>,
}

impl<'a> RowDecoder<'a> {
    /// Create a new RowDecoder from PNG file bytes.
    ///
    /// Accepts `Cow<'a, [u8]>` so that callers can donate owned data
    /// (making the decoder `'static`) or pass borrowed data for zero-copy.
    pub fn new(
        data: Cow<'a, [u8]>,
        config: &crate::decode::PngDecodeConfig,
    ) -> crate::error::Result<Self> {
        // Validate signature
        if data.len() < 8 || data[..8] != crate::chunk::PNG_SIGNATURE {
            return Err(at!(PngError::Decode("not a PNG file".into())));
        }

        let mut chunks = ChunkIter::new_with_config(&data, config.skip_critical_chunk_crc);

        // First chunk must be IHDR
        let ihdr_chunk = chunks
            .next()
            .ok_or_else(|| at!(PngError::Decode("empty PNG (no chunks)".into())))??;
        if ihdr_chunk.chunk_type != *b"IHDR" {
            return Err(at!(PngError::Decode("first chunk is not IHDR".into())));
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

        // Collect warnings from chunk CRC validation
        let chunk_warnings = chunks.warnings;

        let first_idat_pos =
            first_idat_pos.ok_or_else(|| at!(PngError::Decode("no IDAT chunk found".into())))?;

        // Validate palette for indexed images
        if ihdr.is_indexed() && ancillary.palette.is_none() {
            return Err(at!(PngError::Decode(
                "indexed color type requires PLTE chunk".into(),
            )));
        }

        // Apply limits
        let output_bpp = output_bytes_per_pixel(&ihdr, &ancillary) as u32;
        config.validate(ihdr.width, ihdr.height, output_bpp)?;

        let stride = ihdr.stride()?;
        let raw_row_bytes = ihdr.raw_row_bytes()?;
        let bpp = ihdr.filter_bpp();

        // Create IDAT source and decompressor — data is moved into IdatSource
        let source = IdatSource::new(data, first_idat_pos, config.skip_critical_chunk_crc);
        let decompressor = zenflate::StreamDecompressor::zlib(source, stride * 2)
            .with_skip_checksum(config.skip_decompression_checksum);

        Ok(Self {
            decompressor,
            ihdr,
            ancillary,
            first_idat_pos,
            prev_row: vec![0u8; raw_row_bytes],
            current_row: vec![0u8; raw_row_bytes],
            rows_yielded: 0,
            stride,
            bpp,
            chunk_warnings,
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

    /// Byte offset of the first IDAT chunk header.
    pub fn first_idat_pos(&self) -> usize {
        self.first_idat_pos
    }

    /// Yield the next unfiltered raw row, or None if all rows have been read.
    pub fn next_raw_row(&mut self) -> Option<crate::error::Result<&[u8]>> {
        if self.rows_yielded >= self.ihdr.height {
            return None;
        }

        if let Err(e) = self.fill_stride() {
            return Some(Err(e));
        }
        if self.decompressor.peek().len() < self.stride {
            return None;
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

    /// Unfilter the next row directly into `dest`, using caller-provided `prev`.
    ///
    /// The caller owns both buffers — typically `prev` is the previous row in
    /// the output buffer (via `split_at_mut`), eliminating the prev_row copy.
    /// For row 0, pass a zeroed slice.
    pub fn next_raw_row_direct(
        &mut self,
        dest: &mut [u8],
        prev: &[u8],
    ) -> Option<crate::error::Result<()>> {
        if self.rows_yielded >= self.ihdr.height {
            return None;
        }

        if let Err(e) = self.fill_stride() {
            return Some(Err(e));
        }
        if self.decompressor.peek().len() < self.stride {
            return None;
        }

        let peeked = self.decompressor.peek();
        let filter_byte = peeked[0];
        let raw_row_bytes = self.stride - 1;

        // Single copy: decompressor → dest
        dest[..raw_row_bytes].copy_from_slice(&peeked[1..self.stride]);
        self.decompressor.advance(self.stride);

        // Unfilter in-place using caller's prev slice
        if let Err(e) = unfilter_row(filter_byte, &mut dest[..raw_row_bytes], prev, self.bpp) {
            return Some(Err(e));
        }

        self.rows_yielded += 1;
        Some(Ok(()))
    }

    /// Fill the decompressor until at least one full stride is available.
    fn fill_stride(&mut self) -> crate::error::Result<()> {
        loop {
            let available = self.decompressor.peek().len();
            if available >= self.stride {
                return Ok(());
            }
            if self.decompressor.is_done() {
                if available > 0 && available < self.stride {
                    return Err(at!(PngError::Decode(alloc::format!(
                        "truncated row data: got {} bytes, expected {} (row {})",
                        available,
                        self.stride,
                        self.rows_yielded
                    ))));
                }
                return Ok(());
            }
            match self.decompressor.fill() {
                Ok(_) => {}
                Err(e) => {
                    return Err(at!(PngError::Decode(alloc::format!(
                        "decompression error: {e:?}"
                    ))));
                }
            }
        }
    }

    /// After all rows consumed, parse post-IDAT chunks for late metadata.
    pub fn finish_metadata(&mut self) {
        // Scan forward from first_idat_pos to skip all IDAT chunks,
        // then collect metadata from post-IDAT chunks.
        let data: &[u8] = self.decompressor.source_ref().file_data();
        let mut pos = self.first_idat_pos;

        // Skip all IDAT chunks
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let Some(crc_end) = (pos + 8).checked_add(length).and_then(|v| v.checked_add(4)) else {
                return;
            };
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
            let Some(data_end) = data_start.checked_add(length) else {
                break;
            };
            let Some(crc_end) = data_end.checked_add(4) else {
                break;
            };

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

    /// Collect decode-related warnings (chunk CRC skips, decompression checksum).
    pub fn collect_decode_warnings(&self) -> Vec<crate::decode::PngWarning> {
        let mut warnings = self.chunk_warnings.clone();
        if self.decompressor.checksum_matched() == Some(false) {
            warnings.push(crate::decode::PngWarning::DecompressionChecksumSkipped);
        }
        warnings
    }
}
