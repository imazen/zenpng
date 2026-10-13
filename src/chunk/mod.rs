//! PNG chunk parsing, iteration, and writing.

pub(crate) mod ancillary;
pub(crate) mod ihdr;
pub(crate) mod write;

use alloc::vec::Vec;

use zenflate::crc32;

use crate::error::PngError;

// ── PNG signature ───────────────────────────────────────────────────

pub(crate) const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

// ── Chunk parser ────────────────────────────────────────────────────

/// Reference to a single PNG chunk (zero-copy borrow of the file data).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChunkRef<'a> {
    pub chunk_type: [u8; 4],
    pub data: &'a [u8],
}

/// Iterator over PNG chunks. Walks bytes after the 8-byte PNG signature.
pub(crate) struct ChunkIter<'a> {
    data: &'a [u8],
    pos: usize,
    skip_critical_crc: bool,
    pub warnings: Vec<crate::decode::PngWarning>,
}

impl<'a> ChunkIter<'a> {
    /// Create a new chunk iterator that verifies every CRC. `data` must be the
    /// full PNG file bytes (signature already validated by caller). Decode
    /// paths use [`new_with_config`](Self::new_with_config) with their CRC
    /// policy.
    #[cfg(test)]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 8, // skip PNG signature
            skip_critical_crc: false,
            warnings: Vec::new(),
        }
    }

    /// Create a new chunk iterator with CRC leniency configuration.
    pub fn new_with_config(data: &'a [u8], skip_critical_crc: bool) -> Self {
        Self {
            data,
            pos: 8,
            skip_critical_crc,
            warnings: Vec::new(),
        }
    }

    /// Current byte offset in the file.
    pub fn pos(&self) -> usize {
        self.pos
    }
}

// ── Public ancillary-chunk API ──────────────────────────────────────

/// Position of an ancillary chunk relative to image data.
///
/// PNG-3 §14.2.1 requires a safe-to-copy unknown chunk copied to new output
/// to stay on the same side of `IDAT` it occupied in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AncillaryPosition {
    /// Before the first `IDAT` chunk.
    BeforeIdat,
    /// After the first `IDAT` chunk (also between consecutive `IDAT`s).
    AfterIdat,
}

/// One ancillary chunk borrowed from a PNG byte stream (zero-copy).
///
/// Yielded by [`ancillary_chunks`]. Every ancillary chunk is yielded —
/// including ones zenpng interprets (e.g. `tEXt`, `gAMA`) — so callers can
/// apply their own keep/drop policy.
#[derive(Clone, Copy, Debug)]
pub struct AncillaryChunk<'a> {
    /// Four-byte chunk name; the ancillary bit (lowercase first letter) is
    /// set by construction.
    pub tag: [u8; 4],
    /// Chunk payload, excluding the length, name and CRC fields.
    pub data: &'a [u8],
    /// Position relative to the first `IDAT`.
    pub position: AncillaryPosition,
}

impl AncillaryChunk<'_> {
    /// PNG safe-to-copy bit (lowercase fourth letter, PNG-3 §5.4): whether
    /// this chunk may be copied to output after changes to critical image
    /// data. An unknown unsafe-to-copy chunk must not be carried over a
    /// re-encode.
    #[must_use]
    pub fn is_safe_to_copy(&self) -> bool {
        self.tag[3] & 0x20 != 0
    }
}

/// An owned ancillary chunk for [`EncodeConfig::unknown_chunks`].
///
/// [`EncodeConfig::unknown_chunks`]: crate::EncodeConfig::unknown_chunks
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedAncillaryChunk {
    /// Four-byte chunk name: ancillary (lowercase first letter), ASCII
    /// letters only, uppercase third letter (the reserved bit stays clear).
    pub tag: [u8; 4],
    /// Chunk payload, excluding the length, name and CRC fields (the encoder
    /// computes a fresh CRC).
    pub data: Vec<u8>,
    /// Position relative to the first `IDAT`.
    pub position: AncillaryPosition,
}

impl OwnedAncillaryChunk {
    /// PNG safe-to-copy bit (lowercase fourth letter, PNG-3 §5.4).
    #[must_use]
    pub fn is_safe_to_copy(&self) -> bool {
        self.tag[3] & 0x20 != 0
    }
}

impl From<AncillaryChunk<'_>> for OwnedAncillaryChunk {
    fn from(c: AncillaryChunk<'_>) -> Self {
        Self {
            tag: c.tag,
            data: c.data.to_vec(),
            position: c.position,
        }
    }
}

/// Iterator over the ancillary chunks of a complete PNG byte stream.
///
/// Created by [`ancillary_chunks`]. Items are `Result` because chunk
/// structure errors surface where they occur. Iteration stops at `IEND`;
/// any bytes after it are not visited.
pub struct AncillaryChunks<'a> {
    inner: ChunkIter<'a>,
    after_idat: bool,
    finished: bool,
}

/// Validate the PNG signature and iterate ancillary chunks (zero-copy).
///
/// `skip_critical_chunk_crc` selects the same CRC policy as decoding: pass
/// [`PngDecodeConfig::skip_critical_chunk_crc`] so iteration matches the
/// decode it accompanies. With verification on, ancillary chunks with a bad
/// CRC are skipped (never yielded) and a critical-chunk CRC error is
/// returned, exactly as the decoder treats them.
///
/// [`PngDecodeConfig::skip_critical_chunk_crc`]: crate::PngDecodeConfig::skip_critical_chunk_crc
pub fn ancillary_chunks(
    data: &[u8],
    skip_critical_chunk_crc: bool,
) -> Result<AncillaryChunks<'_>, PngError> {
    if !data.starts_with(&PNG_SIGNATURE) {
        return Err(PngError::NotPng("missing 8-byte PNG signature".into()));
    }
    Ok(AncillaryChunks {
        inner: ChunkIter::new_with_config(data, skip_critical_chunk_crc),
        after_idat: false,
        finished: false,
    })
}

impl<'a> Iterator for AncillaryChunks<'a> {
    type Item = Result<AncillaryChunk<'a>, PngError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            match self.inner.next() {
                None => {
                    self.finished = true;
                    return None;
                }
                Some(Err(e)) => {
                    self.finished = true;
                    return Some(Err(e));
                }
                Some(Ok(chunk)) => match &chunk.chunk_type {
                    b"IDAT" => self.after_idat = true,
                    b"IEND" => {
                        self.finished = true;
                        return None;
                    }
                    tag if tag[0] & 0x20 != 0 => {
                        return Some(Ok(AncillaryChunk {
                            tag: *tag,
                            data: chunk.data,
                            position: if self.after_idat {
                                AncillaryPosition::AfterIdat
                            } else {
                                AncillaryPosition::BeforeIdat
                            },
                        }));
                    }
                    _ => {}
                },
            }
        }
    }
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Result<ChunkRef<'a>, PngError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.pos >= self.data.len() {
                return None;
            }

            // Need at least 12 bytes: length(4) + type(4) + crc(4) (data can be 0)
            if self.pos + 12 > self.data.len() {
                return Some(Err(PngError::Truncated("truncated chunk header".into())));
            }

            let length =
                u32::from_be_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap()) as usize;
            let chunk_type: [u8; 4] = self.data[self.pos + 4..self.pos + 8].try_into().unwrap();

            let data_start = self.pos + 8;
            let Some(data_end) = data_start.checked_add(length) else {
                return Some(Err(PngError::Decode(alloc::format!(
                    "chunk length overflow at offset {}",
                    self.pos
                ))));
            };
            let Some(crc_end) = data_end.checked_add(4) else {
                return Some(Err(PngError::Decode(alloc::format!(
                    "chunk length overflow at offset {}",
                    self.pos
                ))));
            };

            if crc_end > self.data.len() {
                return Some(Err(PngError::Decode(alloc::format!(
                    "truncated chunk {:?} at offset {}",
                    core::str::from_utf8(&chunk_type).unwrap_or("????"),
                    self.pos
                ))));
            }

            let chunk_data = &self.data[data_start..data_end];
            let stored_crc = u32::from_be_bytes(self.data[data_end..crc_end].try_into().unwrap());

            // CRC covers type + data. Skip computation entirely when skip_critical_crc
            // is set (saves CRC-32 computation over all chunk data).
            if self.skip_critical_crc {
                // Skip CRC computation entirely — no warning emitted.
            } else {
                let computed_crc = crc32(crc32(0, &chunk_type), chunk_data);
                if stored_crc != computed_crc {
                    // PNG spec: bit 5 of the first byte indicates ancillary (lowercase).
                    // Ancillary chunks with bad CRC should be skipped, not fatal.
                    let is_ancillary = chunk_type[0] & 0x20 != 0;
                    if is_ancillary {
                        self.pos = crc_end;
                        continue; // skip this chunk, try next
                    }
                    return Some(Err(PngError::Decode(alloc::format!(
                        "CRC mismatch in {:?} chunk at offset {}",
                        core::str::from_utf8(&chunk_type).unwrap_or("????"),
                        self.pos
                    ))));
                }
            }

            self.pos = crc_end;

            return Some(Ok(ChunkRef {
                chunk_type,
                data: chunk_data,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal chunk: length(4) + type(4) + data + crc(4).
    fn make_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        let crc = crc32(crc32(0, chunk_type), data);
        buf.extend_from_slice(&crc.to_be_bytes());
        buf
    }

    fn make_png_with_chunks(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&PNG_SIGNATURE);
        for chunk in chunks {
            buf.extend_from_slice(chunk);
        }
        buf
    }

    #[test]
    fn iter_empty_data_returns_none() {
        // pos starts at 8, data is exactly 8 bytes (just the signature)
        let data = PNG_SIGNATURE.to_vec();
        let mut iter = ChunkIter::new(&data);
        assert!(iter.next().is_none());
    }

    #[test]
    fn iter_truncated_header() {
        // After signature, only 5 bytes — not enough for a chunk header (12)
        let mut data = PNG_SIGNATURE.to_vec();
        data.extend_from_slice(&[0; 5]);
        let mut iter = ChunkIter::new(&data);
        let result = iter.next();
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn iter_ancillary_bad_crc_is_skipped() {
        // Ancillary chunk type: lowercase first byte (bit 5 set)
        // "tEXt" is ancillary
        let mut text_chunk = make_chunk(b"tEXt", b"hello");
        // Corrupt the CRC (last 4 bytes)
        let len = text_chunk.len();
        text_chunk[len - 1] ^= 0xFF;

        // Follow with a valid IEND
        let iend_chunk = make_chunk(b"IEND", &[]);

        let data = make_png_with_chunks(&[text_chunk, iend_chunk]);
        let mut iter = ChunkIter::new(&data);
        // The bad-CRC tEXt should be skipped, IEND returned
        let chunk = iter.next().unwrap().unwrap();
        assert_eq!(&chunk.chunk_type, b"IEND");
    }

    #[test]
    fn iter_critical_bad_crc_is_error() {
        // Critical chunk type: uppercase first byte (bit 5 clear)
        // "IHDR" is critical
        let ihdr_data = vec![0u8; 13]; // dummy IHDR data
        let mut ihdr_chunk = make_chunk(b"IHDR", &ihdr_data);
        // Corrupt the CRC
        let len = ihdr_chunk.len();
        ihdr_chunk[len - 1] ^= 0xFF;

        let data = make_png_with_chunks(&[ihdr_chunk]);
        let mut iter = ChunkIter::new(&data);
        let result = iter.next();
        assert!(result.is_some());
        let err = result.unwrap().unwrap_err();
        assert!(err.to_string().contains("CRC mismatch"));
    }

    #[test]
    fn iter_valid_chunk_returns_data() {
        let iend_chunk = make_chunk(b"IEND", &[]);
        let data = make_png_with_chunks(&[iend_chunk]);
        let mut iter = ChunkIter::new(&data);
        let chunk = iter.next().unwrap().unwrap();
        assert_eq!(&chunk.chunk_type, b"IEND");
        assert_eq!(chunk.data.len(), 0);
        // After IEND, should return None
        assert!(iter.next().is_none());
    }

    #[test]
    fn iter_skip_crc_does_not_check() {
        let ihdr_data = vec![0u8; 13];
        let mut ihdr_chunk = make_chunk(b"IHDR", &ihdr_data);
        // Corrupt the CRC
        let len = ihdr_chunk.len();
        ihdr_chunk[len - 1] ^= 0xFF;

        let data = make_png_with_chunks(&[ihdr_chunk]);
        let mut iter = ChunkIter::new_with_config(&data, true);
        // With skip_crc, bad CRC should not cause error
        let chunk = iter.next().unwrap().unwrap();
        assert_eq!(&chunk.chunk_type, b"IHDR");
    }

    #[test]
    fn pos_advances_correctly() {
        let chunk1 = make_chunk(b"tEXt", b"hello");
        let chunk2 = make_chunk(b"IEND", &[]);
        let data = make_png_with_chunks(&[chunk1.clone(), chunk2]);
        let mut iter = ChunkIter::new(&data);
        assert_eq!(iter.pos(), 8); // after signature
        let _ = iter.next().unwrap().unwrap();
        assert_eq!(iter.pos(), 8 + chunk1.len());
    }

    // ── ancillary_chunks public API ──────────────────────────────────

    fn minimal_png(extra: &[Vec<u8>]) -> Vec<u8> {
        let mut chunks = vec![make_chunk(b"IHDR", &[0u8; 13])];
        chunks.extend_from_slice(extra);
        chunks.push(make_chunk(b"IEND", &[]));
        make_png_with_chunks(&chunks)
    }

    #[test]
    fn ancillary_iter_positions_and_skips_critical() {
        let pre = make_chunk(b"vpAg", b"before");
        let idat = make_chunk(b"IDAT", b"\x78\x9c");
        let post = make_chunk(b"prVa", b"after");
        let data = minimal_png(&[pre, idat, post]);
        let chunks: Vec<_> = ancillary_chunks(&data, false)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(&chunks[0].tag, b"vpAg");
        assert_eq!(chunks[0].data, b"before");
        assert_eq!(chunks[0].position, AncillaryPosition::BeforeIdat);
        assert_eq!(&chunks[1].tag, b"prVa");
        assert_eq!(chunks[1].data, b"after");
        assert_eq!(chunks[1].position, AncillaryPosition::AfterIdat);
    }

    #[test]
    fn ancillary_iter_between_idats_is_after() {
        let idat = make_chunk(b"IDAT", b"a");
        let mid = make_chunk(b"vpAg", b"m");
        let idat2 = make_chunk(b"IDAT", b"b");
        let data = minimal_png(&[idat, mid, idat2]);
        let chunks: Vec<_> = ancillary_chunks(&data, false)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].position, AncillaryPosition::AfterIdat);
    }

    #[test]
    fn ancillary_iter_safe_to_copy_bit() {
        let data = minimal_png(&[make_chunk(b"vpAg", b""), make_chunk(b"vpAG", b"")]);
        let chunks: Vec<_> = ancillary_chunks(&data, false)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(chunks[0].is_safe_to_copy());
        assert!(!chunks[1].is_safe_to_copy());
        let owned: OwnedAncillaryChunk = chunks[0].into();
        assert!(owned.is_safe_to_copy());
        assert_eq!(owned.data, chunks[0].data);
    }

    #[test]
    fn ancillary_iter_known_chunks_included() {
        // Handled chunks are yielded too: the caller owns the keep/drop policy.
        let data = minimal_png(&[make_chunk(b"tEXt", b"k\0v")]);
        let chunks: Vec<_> = ancillary_chunks(&data, false)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(&chunks[0].tag, b"tEXt");
    }

    #[test]
    fn ancillary_iter_rejects_non_png() {
        assert!(ancillary_chunks(b"not a png", false).is_err());
    }

    #[test]
    fn ancillary_iter_surfaces_truncation() {
        let mut data = minimal_png(&[]);
        data.truncate(data.len() - 2); // clip IEND's CRC
        let mut iter = ancillary_chunks(&data, false).unwrap();
        assert!(iter.next().unwrap().is_err());
    }

    #[test]
    fn ancillary_iter_error_is_terminal() {
        // A structural error must end iteration: ChunkIter does not advance
        // past the bad record, so a non-terminal iterator would repeat the
        // same error forever if the caller kept polling.
        let mut data = minimal_png(&[]);
        data.truncate(data.len() - 2); // clip IEND's CRC
        let mut iter = ancillary_chunks(&data, false).unwrap();
        assert!(iter.next().unwrap().is_err());
        assert!(iter.next().is_none());
        assert!(iter.next().is_none());
    }

    #[test]
    fn ancillary_iter_crc_policy_matches_decode() {
        let mut bad = make_chunk(b"vpAg", b"x");
        let n = bad.len();
        bad[n - 1] ^= 0xFF; // corrupt CRC
        let good = make_chunk(b"vpAi", b"y");
        let data = minimal_png(&[bad, good]);
        // Verifying: bad-CRC ancillary is skipped exactly like the decoder.
        let strict: Vec<_> = ancillary_chunks(&data, false)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(strict.len(), 1);
        assert_eq!(&strict[0].tag, b"vpAi");
        // Skipping: both are yielded unchecked.
        let lenient: Vec<_> = ancillary_chunks(&data, true)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(lenient.len(), 2);
    }

    #[test]
    fn ancillary_iter_stops_at_iend() {
        let after_iend = make_chunk(b"vpAg", b"late");
        let mut data = minimal_png(&[]);
        data.extend_from_slice(&after_iend);
        let mut iter = ancillary_chunks(&data, false).unwrap();
        assert!(iter.next().is_none());
    }

    #[test]
    fn many_bad_crc_ancillary_chunks_no_stack_overflow() {
        // Regression test: previously used recursion to skip bad-CRC ancillary
        // chunks, which could overflow the stack with enough consecutive bad chunks.
        let mut chunks = Vec::new();
        for _ in 0..10_000 {
            let mut bad_text = make_chunk(b"tEXt", b"x");
            let len = bad_text.len();
            bad_text[len - 1] ^= 0xFF; // corrupt CRC
            chunks.push(bad_text);
        }
        chunks.push(make_chunk(b"IEND", &[]));
        let data = make_png_with_chunks(&chunks);
        let mut iter = ChunkIter::new(&data);
        // Should skip all 10,000 bad ancillary chunks and find IEND
        let chunk = iter.next().unwrap().unwrap();
        assert_eq!(&chunk.chunk_type, b"IEND");
    }
}
