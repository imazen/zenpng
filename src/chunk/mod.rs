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
#[derive(Clone, Copy)]
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

        // CRC covers type + data. Skip computation entirely when skip_critical_crc
        // is set (saves CRC-32 computation over all chunk data).
        if self.skip_critical_crc {
            let is_critical = chunk_type[0] & 0x20 == 0;
            if is_critical {
                self.warnings
                    .push(crate::decode::PngWarning::CriticalChunkCrcSkipped { chunk_type });
            }
        } else {
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
        }

        self.pos = crc_end;

        Some(Ok(ChunkRef {
            chunk_type,
            data: chunk_data,
        }))
    }
}
