//! PNG encode pipeline: filtering, compression, chunk assembly.

pub(crate) mod apng;
pub(crate) mod compress;
pub(crate) mod filter;
pub(crate) mod metadata;

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;

use crate::chunk::PNG_SIGNATURE;
use crate::chunk::write::write_chunk;
use crate::error::PngError;

pub(crate) use self::compress::compress_filtered;
pub(crate) use self::metadata::{PngWriteMetadata, metadata_size_estimate, write_all_metadata};

/// Compression options passed through the pipeline.
pub(crate) struct CompressOptions<'a> {
    /// Run screening and refinement phases in parallel.
    pub parallel: bool,
    /// Hard cancel — passed into zenflate/zenzop, aborts mid-compression.
    pub cancel: &'a dyn Stop,
    /// Soft deadline — checked between phases/strategies for graceful early return.
    pub deadline: &'a dyn Stop,
    /// Optional remaining-time query for zopfli iteration calibration.
    /// Returns remaining nanoseconds, or `None` if unknown/unlimited.
    #[allow(dead_code)] // read only with `zopfli` feature
    pub remaining_ns: Option<&'a dyn Fn() -> Option<u64>>,
}

/// Statistics for one compression phase.
#[derive(Clone, Debug)]
#[doc(hidden)]
#[allow(dead_code)]
pub struct PhaseStat {
    pub name: alloc::string::String,
    pub duration_ns: u64,
    pub best_size: usize,
    pub evaluations: u32,
}

/// Collected per-phase statistics from compression.
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct PhaseStats {
    pub phases: Vec<PhaseStat>,
    pub raw_size: usize,
}

/// Encode palette-indexed pixel data into a complete PNG file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_indexed_png(
    indices: &[u8],
    width: u32,
    height: u32,
    palette_rgb: &[u8],
    palette_alpha: Option<&[u8]>,
    write_meta: &PngWriteMetadata<'_>,
    effort: u32,
    opts: CompressOptions<'_>,
) -> Result<Vec<u8>, PngError> {
    let w = width as usize;
    let h = height as usize;
    let n_colors = palette_rgb.len() / 3;

    if n_colors == 0 || n_colors > 256 {
        return Err(PngError::InvalidInput(alloc::format!(
            "palette must have 1-256 entries, got {n_colors}"
        )));
    }
    if indices.len() < w * h {
        return Err(PngError::InvalidInput(
            "index buffer too small for dimensions".to_string(),
        ));
    }

    let bit_depth = select_bit_depth(n_colors);
    let packed_rows = pack_all_rows(indices, w, h, bit_depth);
    let row_bytes = packed_row_bytes(w, bit_depth);

    // Compress with multi-strategy filter selection (bpp=1 for indexed)
    let compressed = compress_filtered(&packed_rows, row_bytes, h, 1, effort, opts, None)?;

    // Assemble PNG
    let trns_data = truncate_trns(palette_alpha);
    let est = 8
        + 25
        + (12 + n_colors * 3)
        + trns_data.as_ref().map_or(0, |t| 12 + t.len())
        + (12 + compressed.len())
        + 12
        + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);

    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = bit_depth;
    ihdr[9] = 3; // indexed color
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Color metadata and generic metadata (before PLTE per PNG spec)
    write_all_metadata(&mut out, write_meta)?;

    // PLTE
    write_chunk(&mut out, b"PLTE", &palette_rgb[..n_colors * 3]);

    // tRNS
    if let Some(trns) = &trns_data {
        write_chunk(&mut out, b"tRNS", trns);
    }

    // IDAT
    write_chunk(&mut out, b"IDAT", &compressed);

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

/// Encode palette-indexed pixel data into a PNG file using pre-compressed IDAT data.
///
/// Accepts a raw deflate stream and Adler-32 from an external compression pass
/// (e.g. zenquant's zoint optimizer). Wraps the deflate stream in a zlib
/// envelope (2-byte header + deflate + 4-byte Adler-32) and assembles the
/// full PNG file.
#[cfg(feature = "quantize")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_indexed_png_precompressed(
    width: u32,
    height: u32,
    palette_rgb: &[u8],
    palette_alpha: Option<&[u8]>,
    write_meta: &PngWriteMetadata<'_>,
    deflate_stream: &[u8],
    adler32: u32,
    bit_depth: u8,
) -> Result<Vec<u8>, PngError> {
    let n_colors = palette_rgb.len() / 3;

    if n_colors == 0 || n_colors > 256 {
        return Err(PngError::InvalidInput(alloc::format!(
            "palette must have 1-256 entries, got {n_colors}"
        )));
    }

    // Build zlib stream: header + deflate + adler32
    // CMF=0x78 (deflate, window=32KB), FLG=0x01 (no dict, check bits)
    let zlib_len = 2 + deflate_stream.len() + 4;
    let mut zlib_stream = Vec::with_capacity(zlib_len);
    zlib_stream.push(0x78);
    zlib_stream.push(0x01);
    zlib_stream.extend_from_slice(deflate_stream);
    zlib_stream.extend_from_slice(&adler32.to_be_bytes());

    // Assemble PNG
    let trns_data = truncate_trns(palette_alpha);
    let est = 8
        + 25
        + (12 + n_colors * 3)
        + trns_data.as_ref().map_or(0, |t| 12 + t.len())
        + (12 + zlib_stream.len())
        + 12
        + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);

    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = bit_depth;
    ihdr[9] = 3; // indexed color
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Color metadata and generic metadata (before PLTE per PNG spec)
    write_all_metadata(&mut out, write_meta)?;

    // PLTE
    write_chunk(&mut out, b"PLTE", &palette_rgb[..n_colors * 3]);

    // tRNS
    if let Some(trns) = &trns_data {
        write_chunk(&mut out, b"tRNS", trns);
    }

    // IDAT (pre-compressed zlib stream)
    write_chunk(&mut out, b"IDAT", &zlib_stream);

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

/// Encode truecolor/grayscale pixel data into a complete PNG file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_truecolor_png(
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
    color_type: u8,
    bit_depth: u8,
    write_meta: &PngWriteMetadata<'_>,
    effort: u32,
    opts: CompressOptions<'_>,
) -> Result<Vec<u8>, PngError> {
    let w = width as usize;
    let h = height as usize;

    let channels: usize = match color_type {
        0 => 1, // Grayscale
        2 => 3, // RGB
        4 => 2, // GrayscaleAlpha
        6 => 4, // RGBA
        _ => {
            return Err(PngError::InvalidInput(alloc::format!(
                "unsupported PNG color type: {color_type}"
            )));
        }
    };
    let bytes_per_channel = bit_depth as usize / 8;
    let bpp = channels * bytes_per_channel;
    let row_bytes = w * bpp;

    let expected_len = row_bytes * h;
    if pixel_bytes.len() < expected_len {
        return Err(PngError::InvalidInput(alloc::format!(
            "pixel buffer too small: need {expected_len}, got {}",
            pixel_bytes.len()
        )));
    }

    // Effort 0 fast path: write zlib stored blocks directly into the PNG output,
    // avoiding a separate compressed Vec allocation. For 42MB images this
    // eliminates one 42MB allocation + copy.
    if effort == 0 {
        let filtered_row = row_bytes + 1;
        let total_filtered = filtered_row * h;
        let num_blocks = if total_filtered == 0 {
            1
        } else {
            total_filtered.div_ceil(65535)
        };
        let idat_data_len = 2 + 5 * num_blocks + total_filtered + 4; // zlib wrapper

        let est = 8 + 25 + (12 + idat_data_len) + 12 + metadata_size_estimate(write_meta);
        let mut out = Vec::with_capacity(est);

        out.extend_from_slice(&PNG_SIGNATURE);

        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = bit_depth;
        ihdr[9] = color_type;
        write_chunk(&mut out, b"IHDR", &ihdr);
        write_all_metadata(&mut out, write_meta)?;

        // Write IDAT chunk directly: length + type + inline zlib data + CRC
        out.extend_from_slice(&(idat_data_len as u32).to_be_bytes());
        out.extend_from_slice(b"IDAT");
        let idat_start = out.len(); // CRC covers type + data

        compress::write_zlib_stored_inline(
            &mut out,
            &pixel_bytes[..expected_len],
            row_bytes,
            h,
        );

        // CRC-32 over "IDAT" + data
        let crc = zenflate::crc32(
            zenflate::crc32(0, b"IDAT"),
            &out[idat_start..],
        );
        out.extend_from_slice(&crc.to_be_bytes());

        write_chunk(&mut out, b"IEND", &[]);
        return Ok(out);
    }

    // Compress with multi-strategy filter selection
    let compressed = compress_filtered(
        &pixel_bytes[..expected_len],
        row_bytes,
        h,
        bpp,
        effort,
        opts,
        None,
    )?;

    // Assemble PNG
    let est = 8 + 25 + (12 + compressed.len()) + 12 + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);

    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = bit_depth;
    ihdr[9] = color_type;
    // ihdr[10] = 0 compression method
    // ihdr[11] = 0 filter method
    // ihdr[12] = 0 interlace method
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Color metadata and generic metadata (before IDAT)
    write_all_metadata(&mut out, write_meta)?;

    // IDAT
    write_chunk(&mut out, b"IDAT", &compressed);

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

/// Like `write_truecolor_png` but also collects per-phase compression statistics.
#[cfg(feature = "_dev")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_truecolor_png_with_stats(
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
    color_type: u8,
    bit_depth: u8,
    write_meta: &PngWriteMetadata<'_>,
    effort: u32,
    opts: CompressOptions<'_>,
    stats: &mut PhaseStats,
) -> Result<Vec<u8>, PngError> {
    let w = width as usize;
    let h = height as usize;

    let channels: usize = match color_type {
        0 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => {
            return Err(PngError::InvalidInput(alloc::format!(
                "unsupported PNG color type: {color_type}"
            )));
        }
    };
    let bytes_per_channel = bit_depth as usize / 8;
    let bpp = channels * bytes_per_channel;
    let row_bytes = w * bpp;

    let expected_len = row_bytes * h;
    if pixel_bytes.len() < expected_len {
        return Err(PngError::InvalidInput(alloc::format!(
            "pixel buffer too small: need {expected_len}, got {}",
            pixel_bytes.len()
        )));
    }

    let compressed = compress_filtered(
        &pixel_bytes[..expected_len],
        row_bytes,
        h,
        bpp,
        effort,
        opts,
        Some(stats),
    )?;

    let est = 8 + 25 + (12 + compressed.len()) + 12 + metadata_size_estimate(write_meta);
    let mut out = Vec::with_capacity(est);

    out.extend_from_slice(&PNG_SIGNATURE);

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = bit_depth;
    ihdr[9] = color_type;
    write_chunk(&mut out, b"IHDR", &ihdr);

    write_all_metadata(&mut out, write_meta)?;
    write_chunk(&mut out, b"IDAT", &compressed);
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

// ---- Bit depth and packing (indexed only) ----

pub(crate) fn select_bit_depth(n_colors: usize) -> u8 {
    if n_colors <= 2 {
        1
    } else if n_colors <= 4 {
        2
    } else if n_colors <= 16 {
        4
    } else {
        8
    }
}

pub(crate) fn packed_row_bytes(width: usize, bit_depth: u8) -> usize {
    match bit_depth {
        8 => width,
        4 => width.div_ceil(2),
        2 => width.div_ceil(4),
        1 => width.div_ceil(8),
        _ => width,
    }
}

pub(crate) fn pack_all_rows(indices: &[u8], width: usize, height: usize, bit_depth: u8) -> Vec<u8> {
    if bit_depth == 8 {
        return indices[..width * height].to_vec();
    }

    let row_bytes = packed_row_bytes(width, bit_depth);
    let mut packed = vec![0u8; row_bytes * height];
    let ppb = 8 / bit_depth as usize;
    let mask = (1u8 << bit_depth) - 1;

    for y in 0..height {
        let src_row = &indices[y * width..y * width + width];
        let dst_row = &mut packed[y * row_bytes..y * row_bytes + row_bytes];
        for (x, &idx) in src_row.iter().enumerate() {
            let byte_pos = x / ppb;
            let bit_offset = (ppb - 1 - x % ppb) * bit_depth as usize;
            dst_row[byte_pos] |= (idx & mask) << bit_offset;
        }
    }
    packed
}

pub(crate) fn truncate_trns(palette_alpha: Option<&[u8]>) -> Option<Vec<u8>> {
    let alpha = palette_alpha?;
    let last_non_opaque = alpha.iter().rposition(|&a| a != 255)?;
    Some(alpha[..=last_non_opaque].to_vec())
}
