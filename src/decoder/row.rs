//! Streaming row decoder: IDAT source and row-by-row decompression + unfilter.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use zenflate::crc32;

use crate::chunk::ancillary::PngAncillary;
use crate::chunk::ihdr::Ihdr;
use crate::chunk::{ChunkIter, ChunkRef};
use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use super::postprocess::output_bytes_per_pixel;

// ── ChunkSource — InputSource for IDAT / fdAT chunks ────────────────

/// Which chunk run this source feeds: the image IDAT stream or one APNG
/// frame's fdAT run. Only the tag, the per-chunk payload offset and the
/// error literals differ, so `mode` is consulted solely on cold error paths;
/// the hot `fill_buf` loop reads the resolved `tag`/`payload_off` fields and
/// carries no mode branch beyond the `end_pos` compare IDAT always had.
#[derive(Clone, Copy)]
enum ChunkMode {
    Idat,
    Fdat,
}

impl ChunkMode {
    fn tag(self) -> [u8; 4] {
        match self {
            Self::Idat => *b"IDAT",
            Self::Fdat => *b"fdAT",
        }
    }

    /// Payload offset past the chunk header (fdAT leads with a 4-byte
    /// sequence number that is skipped to reach the deflate payload).
    fn payload_off(self) -> usize {
        match self {
            Self::Idat => 0,
            Self::Fdat => 4,
        }
    }

    fn err_truncated(self, what: &'static str) -> PngError {
        PngError::Truncated(match (self, what) {
            (Self::Idat, "header") => "truncated IDAT chunk header (length)".into(),
            (Self::Idat, "data") => "truncated IDAT chunk data".into(),
            (Self::Idat, "crc") => "truncated IDAT chunk CRC".into(),
            (Self::Idat, "chunk") => "truncated IDAT chunk".into(),
            (Self::Fdat, "header") => "truncated fdAT chunk header (length)".into(),
            (Self::Fdat, "data") => "truncated fdAT chunk data".into(),
            (Self::Fdat, "crc") => "truncated fdAT chunk CRC".into(),
            (Self::Fdat, "chunk") => "truncated fdAT chunk".into(),
            _ => unreachable!(),
        })
    }

    fn err_overflow(self) -> PngError {
        PngError::Decode(match self {
            Self::Idat => "IDAT chunk length overflow".into(),
            Self::Fdat => "fdAT chunk length overflow".into(),
        })
    }

    fn err_crc_mismatch(self) -> PngError {
        PngError::Decode(match self {
            Self::Idat => "CRC mismatch in IDAT chunk".into(),
            Self::Fdat => "CRC mismatch in fdAT chunk".into(),
        })
    }
}

/// Streams raw IDAT or fdAT chunk payload bytes to `StreamDecompressor`.
/// Walks chunks in the file without collecting chunk data into a Vec.
pub(crate) struct ChunkSource<'a> {
    /// Full PNG file bytes (borrowed or owned).
    data: Cow<'a, [u8]>,
    /// Byte offset of the next chunk header to check.
    chunk_pos: usize,
    /// Range (start, end) into `data` for remaining payload bytes in the
    /// current chunk (past the 4-byte fdAT sequence number for fdAT).
    current_range: (usize, usize),
    /// True when we've seen a non-`tag` chunk after the tag run.
    done: bool,
    /// Position of the first post-`tag`-run chunk (for metadata collection /
    /// APNG chunk scanning). 0 until set; end_pos-bounded runs leave it 0.
    pub post_chunk_pos: usize,
    /// Whether to skip CRC validation on tag chunks.
    skip_crc: bool,
    /// Stop before the chunk whose header starts at or after this offset
    /// (`usize::MAX` = the whole run). Used to feed one `iDOT` segment.
    end_pos: usize,
    /// Chunk type this source consumes, resolved at construction.
    tag: [u8; 4],
    /// Payload offset past the chunk header, resolved at construction.
    payload_off: usize,
    /// Which chunk kind this is — consulted only for error literals.
    mode: ChunkMode,
}

impl<'a> ChunkSource<'a> {
    /// Borrow the full PNG file data stored in this source.
    pub fn file_data(&self) -> &[u8] {
        &self.data
    }

    /// Create a new IDAT source positioned at the first IDAT chunk.
    /// `first_idat_pos` is the byte offset of the first IDAT chunk header.
    /// When `skip_crc` is true, IDAT CRC mismatches are tolerated.
    pub fn idat(
        data: Cow<'a, [u8]>,
        first_idat_pos: usize,
        skip_crc: bool,
    ) -> Result<Self, PngError> {
        Self::new(data, first_idat_pos, usize::MAX, skip_crc, ChunkMode::Idat)
    }

    /// Like [`idat`](Self::idat), but stops before the IDAT chunk whose
    /// header is at `end_pos` — one segment of an `iDOT` table.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn idat_bounded(
        data: Cow<'a, [u8]>,
        first_idat_pos: usize,
        end_pos: usize,
        skip_crc: bool,
    ) -> Result<Self, PngError> {
        Self::new(data, first_idat_pos, end_pos, skip_crc, ChunkMode::Idat)
    }

    /// Create a new fdAT source positioned at the first fdAT chunk of an
    /// APNG frame. `first_fdat_pos` is the byte offset of that chunk's
    /// header. The 4-byte sequence number is skipped per chunk.
    pub fn fdat(data: &'a [u8], first_fdat_pos: usize, skip_crc: bool) -> Result<Self, PngError> {
        Self::new(
            Cow::Borrowed(data),
            first_fdat_pos,
            usize::MAX,
            skip_crc,
            ChunkMode::Fdat,
        )
    }

    /// Parse the first chunk inline. Bounds-checked and overflow-safe so the
    /// path stays sound and 32-bit-clean even if a caller ever passes an
    /// unvalidated offset. The first chunk's type and CRC are not checked:
    /// the IDAT path is fed by `ChunkIter` (which already validated it), and
    /// the fdAT path deliberately accepts whatever follows fcTL.
    fn new(
        data: Cow<'a, [u8]>,
        first_pos: usize,
        end_pos: usize,
        skip_crc: bool,
        mode: ChunkMode,
    ) -> Result<Self, PngError> {
        let length_bytes: [u8; 4] = data
            .get(first_pos..first_pos + 4)
            .ok_or_else(|| mode.err_truncated("header"))?
            .try_into()
            .unwrap(); // infallible: slice is exactly 4 bytes
        let length = u32::from_be_bytes(length_bytes) as usize;
        let data_start = first_pos + 8; // skip length + type
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| mode.err_overflow())?;
        let next_pos = data_end.checked_add(4).ok_or_else(|| mode.err_overflow())?; // skip CRC

        if data_end > data.len() {
            return Err(mode.err_truncated("data"));
        }
        if next_pos > data.len() {
            return Err(mode.err_truncated("crc"));
        }

        // For fdAT, skip the 4-byte sequence number to reach deflate data;
        // a payload shorter than that contributes nothing.
        let payload_start = data_start + mode.payload_off();
        Ok(Self {
            data,
            chunk_pos: next_pos,
            current_range: (payload_start.min(data_end), data_end),
            done: false,
            post_chunk_pos: 0,
            skip_crc,
            end_pos,
            tag: mode.tag(),
            payload_off: mode.payload_off(),
            mode,
        })
    }
}

impl zenflate::InputSource for ChunkSource<'_> {
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
            if self.chunk_pos >= self.end_pos {
                self.done = true;
                return Ok(&[]);
            }
            if self.chunk_pos + 12 > self.data.len() {
                self.done = true;
                self.post_chunk_pos = self.chunk_pos;
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
                return Err(self.mode.err_overflow());
            };
            let Some(crc_end) = data_end.checked_add(4) else {
                return Err(self.mode.err_overflow());
            };

            if crc_end > self.data.len() {
                return Err(self.mode.err_truncated("chunk"));
            }

            if chunk_type != self.tag {
                // Not a tag chunk — we're done with this tag run
                self.done = true;
                self.post_chunk_pos = self.chunk_pos;
                return Ok(&[]);
            }

            // Validate CRC (skip computation entirely when skip_crc is set)
            if !self.skip_crc {
                let stored_crc =
                    u32::from_be_bytes(self.data[data_end..crc_end].try_into().unwrap());
                let computed_crc = crc32(crc32(0, &chunk_type), &self.data[data_start..data_end]);
                if stored_crc != computed_crc {
                    return Err(self.mode.err_crc_mismatch());
                }
            }

            // Skip the per-chunk payload offset (fdAT's 4-byte sequence
            // number; 0 for IDAT) to reach deflate data.
            let payload_start = data_start + self.payload_off;
            self.current_range = (payload_start.min(data_end), data_end);
            self.chunk_pos = crc_end;

            let (start, end) = self.current_range;
            if start < end {
                return Ok(&self.data[start..end]);
            }
            // Empty payload — skip and try next
        }
    }

    fn consume(&mut self, n: usize) {
        self.current_range.0 += n;
    }
}

/// Fill `d` until one filtered row (`stride` bytes: filter byte + data) is
/// available. A stream that ends first is `Truncated` ("image data ends at
/// row `y`") for every decode path, so callers report it identically.
pub(crate) fn fill_row<S: zenflate::InputSource>(
    d: &mut zenflate::StreamDecompressor<S>,
    stride: usize,
    y: usize,
) -> crate::error::Result<()>
where
    S::Error: core::fmt::Debug,
{
    while d.peek().len() < stride {
        if d.is_done() {
            return Err(at!(PngError::Truncated(alloc::format!(
                "image data ends at row {y}"
            ))));
        }
        d.fill().map_err(|e| {
            at!(PngError::Decode(alloc::format!(
                "decompression error: {e:?}"
            )))
        })?;
    }
    Ok(())
}

/// Read a zlib stream to its end, discarding output (see
/// [`RowDecoder::finish_stream`]). `strict`: errors are returned rather than
/// ignored.
pub(crate) fn drain_stream<S: zenflate::InputSource>(
    d: &mut zenflate::StreamDecompressor<S>,
    strict: bool,
) -> crate::error::Result<()>
where
    S::Error: core::fmt::Debug,
{
    while !d.is_done() {
        match d.fill() {
            Ok(out) => {
                let n = out.len();
                d.advance(n);
                if n == 0 && !d.is_done() && d.fill().map(|o| o.is_empty()).unwrap_or(true) {
                    // No progress and not finished: the stream ends early.
                    if strict {
                        return Err(at!(PngError::Decode(
                            "zlib stream ends before its checksum".into()
                        )));
                    }
                    return Ok(());
                }
            }
            Err(e) if strict => {
                return Err(at!(PngError::Decode(alloc::format!(
                    "decompression error after the last row: {e:?}"
                ))));
            }
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

// ── Unfilter ────────────────────────────────────────────────────────

/// Apply inverse filter to a row in-place given the previous (unfiltered) row.
///
/// Dispatches to SIMD-accelerated implementations (AVX2/SSE2) for Up, Paeth,
/// Average, and Sub filters with bpp=4, falling back to scalar for other bpp.
pub(crate) fn unfilter_row(
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
    /// Boxed: zenflate's decoder keeps its Huffman tables inline (~10 KiB),
    /// and every move of `RowDecoder` would copy them (55K instructions of
    /// memcpy per 64 px decode, 2026-10-07 callgrind).
    decompressor: Box<zenflate::StreamDecompressor<ChunkSource<'a>>>,
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

    /// Validated `iDOT` segment table, if the file has a usable one.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    idot: Option<alloc::vec::Vec<super::idot::IdotSegment>>,

    /// Adler-32 is verified (strict decoding).
    verify_checksum: bool,
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
            return Err(at!(PngError::NotPng("missing PNG signature".into())));
        }

        let mut chunks = ChunkIter::new_with_config(&data, config.skip_critical_chunk_crc);

        // First chunk must be IHDR
        let ihdr_chunk = chunks
            .next()
            .ok_or_else(|| at!(PngError::Truncated("empty PNG (no chunks)".into())))??;
        if ihdr_chunk.chunk_type != *b"IHDR" {
            return Err(at!(PngError::Decode("first chunk is not IHDR".into())));
        }
        // Platform row-size check deferred until after `config.validate` below,
        // so an over-cap image reports `LimitExceeded` on every pointer width.
        let ihdr = Ihdr::parse_fields(ihdr_chunk.data)?;

        // Collect pre-IDAT ancillary chunks
        let mut ancillary = PngAncillary {
            keep_unknown: config.collect_unknown_chunks,
            ..Default::default()
        };
        let mut first_idat_pos = None;
        // (chunk position, data range) of the iDOT chunk; a second iDOT
        // makes the table ambiguous, so it is dropped (`Some(None)`).
        let mut idot_chunk: Option<Option<(usize, usize, usize)>> = None;

        while let Some(chunk_result) = chunks.next() {
            let chunk = chunk_result?;
            if chunk.chunk_type == *b"iDOT" {
                let pos = chunks.pos() - 12 - chunk.data.len();
                idot_chunk = Some(match idot_chunk {
                    None => Some((pos, pos + 8, pos + 8 + chunk.data.len())),
                    Some(_) => None,
                });
                continue;
            }
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
        ihdr.check_row_fits_platform()?;

        let stride = ihdr.stride()?;
        let raw_row_bytes = ihdr.raw_row_bytes()?;
        let bpp = ihdr.filter_bpp();
        // Inflate buffer: at least two rows (checked: `stride * 2` wrapped on
        // 32-bit for a max-width gray8 IHDR, stride == 2^31), and large enough
        // that zenflate's lookback compaction stays rare.
        let capacity = crate::alloc_util::decode_buffer_capacity(stride, ihdr.height)?;

        // The two one-row scratches are bounded by the row width → default
        // infallible, but honor an explicit `AllocPreference`. See
        // `crate::alloc_util`.
        let prev_row = crate::alloc_util::alloc_zeroed(config.alloc_pref, false, raw_row_bytes)?;
        let current_row = crate::alloc_util::alloc_zeroed(config.alloc_pref, false, raw_row_bytes)?;

        let idot = match idot_chunk {
            Some(Some((pos, ds, de))) => {
                super::idot::validate(&data, pos, &data[ds..de], &ihdr, first_idat_pos)
            }
            _ => None,
        };

        // Create IDAT source and decompressor — data is moved into ChunkSource
        let source = ChunkSource::idat(data, first_idat_pos, config.skip_critical_chunk_crc)?;
        let decompressor = Box::new(
            zenflate::StreamDecompressor::zlib(source, capacity)
                .with_skip_checksum(config.skip_decompression_checksum),
        );

        Ok(Self {
            decompressor,
            ihdr,
            ancillary,
            first_idat_pos,
            prev_row,
            current_row,
            rows_yielded: 0,
            stride,
            bpp,
            chunk_warnings,
            idot,
            verify_checksum: !config.skip_decompression_checksum,
        })
    }

    /// The validated `iDOT` segment table, if any.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn idot_segments(&self) -> Option<&[super::idot::IdotSegment]> {
        self.idot.as_deref()
    }

    /// Get the IHDR info.
    pub fn ihdr(&self) -> &Ihdr {
        &self.ihdr
    }

    /// Get the ancillary metadata.
    pub fn ancillary(&self) -> &PngAncillary {
        &self.ancillary
    }

    /// Mutable ancillary metadata: final info construction moves the
    /// opt-in unknown-chunk list out instead of cloning it.
    pub(crate) fn ancillary_mut(&mut self) -> &mut PngAncillary {
        &mut self.ancillary
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
    /// Copy the next filtered row (filter byte + data, [`stride`](Self::stride)
    /// bytes) into `dest` without unfiltering it, for a consumer that
    /// unfilters on another thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn next_filtered_row(
        &mut self,
        dest: &mut [u8],
    ) -> Option<crate::error::Result<()>> {
        if self.rows_yielded >= self.ihdr.height {
            return None;
        }
        if let Err(e) = self.fill_stride() {
            return Some(Err(e));
        }
        dest[..self.stride].copy_from_slice(&self.decompressor.peek()[..self.stride]);
        self.decompressor.advance(self.stride);
        self.rows_yielded += 1;
        Some(Ok(()))
    }

    /// Filtered row length: filter byte + raw row bytes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn stride(&self) -> usize {
        self.stride
    }

    /// Bytes per complete pixel, as the unfilter step uses it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn bpp(&self) -> usize {
        self.bpp
    }

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

    /// Fill the decompressor until the next row's stride is available.
    fn fill_stride(&mut self) -> crate::error::Result<()> {
        fill_row(
            &mut self.decompressor,
            self.stride,
            self.rows_yielded as usize,
        )
    }

    /// After the last row: read the rest of the zlib stream so its Adler-32
    /// footer is checked. Rows are complete before the stream is, so without
    /// this the footer was only checked when the last buffer fill happened to
    /// reach it. When the checksum is verified (strict), a mismatch, a missing
    /// footer or corrupt trailing data is an error; otherwise this only feeds
    /// the `DecompressionChecksumSkipped` warning and never fails.
    pub fn finish_stream(&mut self) -> crate::error::Result<()> {
        drain_stream(&mut self.decompressor, self.verify_checksum)
    }

    /// After all rows consumed, parse post-IDAT chunks for late metadata.
    pub fn finish_metadata(&mut self) {
        // Scan forward from first_idat_pos to skip all IDAT chunks,
        // then collect metadata from post-IDAT chunks.
        let data: &[u8] = self.decompressor.source_ref().file_data();
        let mut pos = self.first_idat_pos;
        let mut idat_bytes: u64 = 0;

        // Skip all IDAT chunks (and sum their payload sizes for source-encoding
        // analysis — zero extra cost since we're walking them anyway).
        while pos + 12 <= data.len() {
            let length = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
            let Some(crc_end) = (pos + 8).checked_add(length).and_then(|v| v.checked_add(4)) else {
                self.ancillary.idat_bytes = idat_bytes;
                return;
            };
            if crc_end > data.len() {
                self.ancillary.idat_bytes = idat_bytes;
                return;
            }
            if chunk_type != *b"IDAT" {
                break;
            }
            idat_bytes = idat_bytes.saturating_add(length as u64);
            pos = crc_end;
        }

        // Now collect post-IDAT chunks (and continue summing fdAT bytes
        // for APNG so `compressed_data_size` matches `detect::probe`).
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

            if chunk_type == *b"fdAT" {
                idat_bytes = idat_bytes.saturating_add(length as u64);
            }

            let chunk_data = &data[data_start..data_end];
            self.ancillary.collect_late(&ChunkRef {
                chunk_type,
                data: chunk_data,
            });
            pos = crc_end;
        }
        self.ancillary.idat_bytes = idat_bytes;
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
