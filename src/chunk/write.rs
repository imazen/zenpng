//! PNG chunk writing helpers.

use alloc::vec::Vec;

use zenflate::crc32;

/// Write a PNG chunk (length + type + data + CRC) to the output buffer.
///
/// # Panics
/// Debug-asserts that `data.len() <= u32::MAX` (PNG chunk length is u32).
pub(crate) fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    debug_assert!(
        data.len() <= u32::MAX as usize,
        "chunk data exceeds u32::MAX"
    );
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = crc32(crc32(0, chunk_type), data);
    out.extend_from_slice(&crc.to_be_bytes());
}
