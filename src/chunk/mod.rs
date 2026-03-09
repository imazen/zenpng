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
    /// Create a new chunk iterator. `data` must be the full PNG file bytes
    /// (signature already validated by caller).
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

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Result<ChunkRef<'a>, PngError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
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
