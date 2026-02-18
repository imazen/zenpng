//! Low-level indexed PNG writer using zenflate for compression.
//!
//! Bypasses the `png` crate's streaming flate2 API to use zenflate's
//! buffer-based compression. Multi-strategy filter selection tries 8
//! strategies (5 single-filter + 3 adaptive) and keeps the smallest.

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use zencodec_types::ImageMetadata;
use zenflate::{CompressionLevel, Compressor, crc32};

use crate::error::PngError;

/// Encode palette-indexed pixel data into a complete PNG file.
///
/// Returns the raw PNG bytes. Tries multiple filter strategies and keeps
/// the one that compresses smallest.
pub(crate) fn write_indexed_png(
    indices: &[u8],
    width: u32,
    height: u32,
    palette_rgb: &[u8],
    palette_alpha: Option<&[u8]>,
    metadata: Option<&ImageMetadata<'_>>,
    compression_level: u8,
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

    // Compress with multi-strategy filter selection
    let compressed = compress_filtered(&packed_rows, row_bytes, h, compression_level)?;

    // Assemble PNG
    let trns_data = truncate_trns(palette_alpha);
    let est = 8
        + 25
        + (12 + n_colors * 3)
        + trns_data.as_ref().map_or(0, |t| 12 + t.len())
        + (12 + compressed.len())
        + 12
        + metadata_size_estimate(metadata);
    let mut out = Vec::with_capacity(est);

    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = bit_depth;
    ihdr[9] = 3; // indexed color
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Metadata chunks (before PLTE per PNG spec for iCCP)
    if let Some(meta) = metadata {
        if let Some(icc) = meta.icc_profile {
            write_iccp_chunk(&mut out, icc)?;
        }
        if let Some(exif) = meta.exif {
            write_exif_chunk(&mut out, exif);
        }
    }

    // PLTE
    write_chunk(&mut out, b"PLTE", &palette_rgb[..n_colors * 3]);

    // tRNS
    if let Some(trns) = &trns_data {
        write_chunk(&mut out, b"tRNS", trns);
    }

    // XMP as iTXt (after PLTE, before IDAT)
    if let Some(meta) = metadata {
        if let Some(xmp) = meta.xmp {
            let xmp_str = core::str::from_utf8(xmp).unwrap_or_default();
            if !xmp_str.is_empty() {
                write_itxt_chunk(&mut out, "XML:com.adobe.xmp", xmp_str);
            }
        }
    }

    // IDAT
    write_chunk(&mut out, b"IDAT", &compressed);

    // IEND
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

// ---- Compression with multi-strategy filter selection ----

fn compress_filtered(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    compression_level: u8,
) -> Result<Vec<u8>, PngError> {
    let level = CompressionLevel::new(compression_level.into());
    let mut compressor = Compressor::new(level);

    let filtered_size = (row_bytes + 1) * height;
    let mut best_compressed: Option<Vec<u8>> = None;

    // Reusable buffers
    let mut filtered = Vec::with_capacity(filtered_size);
    let compress_bound = Compressor::zlib_compress_bound(filtered_size);
    let mut compress_buf = vec![0u8; compress_bound];

    let strategies: &[Strategy] = &[
        Strategy::Single(0), // None
        Strategy::Single(1), // Sub
        Strategy::Single(2), // Up
        Strategy::Single(3), // Average
        Strategy::Single(4), // Paeth
        Strategy::Adaptive(AdaptiveHeuristic::MinSum),
        Strategy::Adaptive(AdaptiveHeuristic::Entropy),
        Strategy::Adaptive(AdaptiveHeuristic::Bigrams),
    ];

    for strategy in strategies {
        filtered.clear();
        filter_image(packed_rows, row_bytes, height, *strategy, &mut filtered);

        let compressed_len = compressor
            .zlib_compress(&filtered, &mut compress_buf)
            .map_err(|e| PngError::InvalidInput(alloc::format!("compression failed: {e}")))?;

        let dominated = best_compressed
            .as_ref()
            .is_some_and(|b| compressed_len >= b.len());
        if !dominated {
            best_compressed = Some(compress_buf[..compressed_len].to_vec());
        }
    }

    best_compressed.ok_or_else(|| PngError::InvalidInput("no filter strategies tried".to_string()))
}

// ---- Filter strategies ----

#[derive(Clone, Copy)]
enum Strategy {
    Single(u8),
    Adaptive(AdaptiveHeuristic),
}

#[derive(Clone, Copy)]
enum AdaptiveHeuristic {
    MinSum,
    Entropy,
    Bigrams,
}

fn filter_image(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    strategy: Strategy,
    out: &mut Vec<u8>,
) {
    let mut prev_row = vec![0u8; row_bytes];
    let mut candidates: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];

        match strategy {
            Strategy::Single(f) => {
                out.push(f);
                apply_filter(f, row, &prev_row, &mut candidates[0]);
                out.extend_from_slice(&candidates[0]);
            }
            Strategy::Adaptive(heuristic) => {
                for f in 0..5u8 {
                    apply_filter(f, row, &prev_row, &mut candidates[f as usize]);
                }
                let best_f = pick_best_filter(&candidates, heuristic);
                out.push(best_f);
                out.extend_from_slice(&candidates[best_f as usize]);
            }
        }

        prev_row.copy_from_slice(row);
    }
}

fn pick_best_filter(candidates: &[Vec<u8>], heuristic: AdaptiveHeuristic) -> u8 {
    match heuristic {
        AdaptiveHeuristic::MinSum => {
            let mut best = 0u8;
            let mut best_score = u64::MAX;
            for f in 0..5u8 {
                let score = sav_score(&candidates[f as usize]);
                if score < best_score {
                    best_score = score;
                    best = f;
                }
            }
            best
        }
        AdaptiveHeuristic::Entropy => {
            let mut best = 0u8;
            let mut best_score = f64::MAX;
            for f in 0..5u8 {
                let score = entropy_score(&candidates[f as usize]);
                if score < best_score {
                    best_score = score;
                    best = f;
                }
            }
            best
        }
        AdaptiveHeuristic::Bigrams => {
            let mut best = 0u8;
            let mut best_score = usize::MAX;
            for f in 0..5u8 {
                let score = bigrams_score(&candidates[f as usize]);
                if score < best_score {
                    best_score = score;
                    best = f;
                }
            }
            best
        }
    }
}

fn apply_filter(filter: u8, row: &[u8], prev_row: &[u8], out: &mut [u8]) {
    let len = row.len();
    match filter {
        0 => out[..len].copy_from_slice(row),
        1 => {
            // Sub
            out[0] = row[0];
            for i in 1..len {
                out[i] = row[i].wrapping_sub(row[i - 1]);
            }
        }
        2 => {
            // Up
            for i in 0..len {
                out[i] = row[i].wrapping_sub(prev_row[i]);
            }
        }
        3 => {
            // Average
            out[0] = row[0].wrapping_sub(prev_row[0] >> 1);
            for i in 1..len {
                let avg = ((row[i - 1] as u16 + prev_row[i] as u16) >> 1) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
            // Paeth
            out[0] = row[0].wrapping_sub(paeth_predictor(0, prev_row[0], 0));
            for i in 1..len {
                let pred = paeth_predictor(row[i - 1], prev_row[i], prev_row[i - 1]);
                out[i] = row[i].wrapping_sub(pred);
            }
        }
        _ => out[..len].copy_from_slice(row),
    }
}

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

// ---- Heuristic scoring ----

fn sav_score(data: &[u8]) -> u64 {
    data.iter()
        .map(|&b| if b > 128 { 256 - b as u64 } else { b as u64 })
        .sum()
}

fn entropy_score(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &b in data {
        counts[b as usize] += 1;
    }
    let len = data.len() as f64;
    let mut entropy = 0.0f64;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

fn bigrams_score(data: &[u8]) -> usize {
    if data.len() < 2 {
        return 0;
    }
    let mut seen = vec![0u64; 1024]; // 1024 * 64 = 65536 bits
    let mut count = 0usize;
    for pair in data.windows(2) {
        let key = (pair[0] as usize) << 8 | pair[1] as usize;
        let word = key >> 6;
        let bit = 1u64 << (key & 63);
        if seen[word] & bit == 0 {
            seen[word] |= bit;
            count += 1;
        }
    }
    count
}

// ---- Bit depth and packing ----

fn select_bit_depth(n_colors: usize) -> u8 {
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

fn packed_row_bytes(width: usize, bit_depth: u8) -> usize {
    match bit_depth {
        8 => width,
        4 => width.div_ceil(2),
        2 => width.div_ceil(4),
        1 => width.div_ceil(8),
        _ => width,
    }
}

fn pack_all_rows(indices: &[u8], width: usize, height: usize, bit_depth: u8) -> Vec<u8> {
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

fn truncate_trns(palette_alpha: Option<&[u8]>) -> Option<Vec<u8>> {
    let alpha = palette_alpha?;
    let last_non_opaque = alpha.iter().rposition(|&a| a != 255)?;
    Some(alpha[..=last_non_opaque].to_vec())
}

// ---- PNG chunk assembly ----

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = crc32(crc32(0, chunk_type), data);
    out.extend_from_slice(&crc.to_be_bytes());
}

// ---- Metadata chunk writers ----

fn write_iccp_chunk(out: &mut Vec<u8>, icc_profile: &[u8]) -> Result<(), PngError> {
    // iCCP: keyword "ICC Profile" + null + compression_method(0) + zlib-compressed profile
    let keyword = b"ICC Profile\0";
    let compression_method = [0u8]; // zlib

    // Compress the ICC profile with zenflate level 9
    let level = CompressionLevel::new(9);
    let mut compressor = Compressor::new(level);
    let bound = Compressor::zlib_compress_bound(icc_profile.len());
    let mut compressed = vec![0u8; bound];
    let compressed_len = compressor
        .zlib_compress(icc_profile, &mut compressed)
        .map_err(|e| PngError::InvalidInput(alloc::format!("ICC compression failed: {e}")))?;

    let mut chunk_data = Vec::with_capacity(keyword.len() + 1 + compressed_len);
    chunk_data.extend_from_slice(keyword);
    chunk_data.extend_from_slice(&compression_method);
    chunk_data.extend_from_slice(&compressed[..compressed_len]);

    write_chunk(out, b"iCCP", &chunk_data);
    Ok(())
}

fn write_exif_chunk(out: &mut Vec<u8>, exif: &[u8]) {
    write_chunk(out, b"eXIf", exif);
}

fn write_itxt_chunk(out: &mut Vec<u8>, keyword: &str, text: &str) {
    // iTXt: keyword + NUL + compression_flag(0) + compression_method(0)
    //       + language_tag("") + NUL + translated_keyword("") + NUL + text
    let mut chunk_data = Vec::with_capacity(keyword.len() + 5 + text.len());
    chunk_data.extend_from_slice(keyword.as_bytes());
    chunk_data.push(0); // null separator
    chunk_data.push(0); // compression flag: uncompressed
    chunk_data.push(0); // compression method
    chunk_data.push(0); // empty language tag + null
    chunk_data.push(0); // empty translated keyword + null
    chunk_data.extend_from_slice(text.as_bytes());

    write_chunk(out, b"iTXt", &chunk_data);
}

fn metadata_size_estimate(metadata: Option<&ImageMetadata<'_>>) -> usize {
    let Some(meta) = metadata else { return 0 };
    let mut size = 0;
    if let Some(icc) = meta.icc_profile {
        // Chunk overhead + keyword + compressed profile (estimate half)
        size += 12 + 13 + icc.len() / 2;
    }
    if let Some(exif) = meta.exif {
        size += 12 + exif.len();
    }
    if let Some(xmp) = meta.xmp {
        size += 12 + 25 + xmp.len();
    }
    size
}
