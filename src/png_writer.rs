//! Low-level PNG writer using zenflate for compression.
//!
//! Bypasses the `png` crate's streaming flate2 API to use zenflate's
//! buffer-based compression. Multi-strategy filter selection tries 8
//! strategies (5 single-filter + 3 adaptive) and keeps the smallest.

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use zencodec_types::{Cicp, ContentLightLevel, ImageMetadata, MasteringDisplay};
use zenflate::{CompressionLevel, Compressor, crc32};

use crate::decode::PngChromaticities;
use crate::error::PngError;

/// All metadata to embed when writing a PNG file.
///
/// Aggregates both codec-generic metadata (`ImageMetadata`) and PNG-specific
/// color chunks (gAMA, sRGB, cHRM). Constructed by the encode functions.
pub(crate) struct PngWriteMetadata<'a> {
    /// ICC profile, EXIF, XMP from ImageMetadata.
    pub generic: Option<&'a ImageMetadata<'a>>,
    /// gAMA chunk value (scaled by 100000, e.g. 45455 = 1/2.2).
    pub source_gamma: Option<u32>,
    /// sRGB rendering intent (0-3).
    pub srgb_intent: Option<u8>,
    /// cHRM chromaticity values.
    pub chromaticities: Option<PngChromaticities>,
    /// cICP color description.
    pub cicp: Option<Cicp>,
    /// Content Light Level (HDR).
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering Display Color Volume (HDR).
    pub mastering_display: Option<MasteringDisplay>,
}

impl<'a> PngWriteMetadata<'a> {
    /// Build from ImageMetadata, inheriting cICP/cLLi/mDCV from it.
    pub fn from_metadata(meta: Option<&'a ImageMetadata<'a>>) -> Self {
        let (cicp, content_light_level, mastering_display) = meta
            .map(|m| (m.cicp, m.content_light_level, m.mastering_display))
            .unwrap_or((None, None, None));
        Self {
            generic: meta,
            source_gamma: None,
            srgb_intent: None,
            chromaticities: None,
            cicp,
            content_light_level,
            mastering_display,
        }
    }
}

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
    write_meta: &PngWriteMetadata<'_>,
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

    // Compress with multi-strategy filter selection (bpp=1 for indexed)
    let compressed = compress_filtered(&packed_rows, row_bytes, h, 1, compression_level)?;

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

/// Encode truecolor/grayscale pixel data into a complete PNG file.
///
/// `pixel_bytes` must be raw pixel data with correct byte order (big-endian
/// for 16-bit). Tries multiple filter strategies and keeps the smallest.
pub(crate) fn write_truecolor_png(
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
    color_type: u8,
    bit_depth: u8,
    write_meta: &PngWriteMetadata<'_>,
    compression_level: u8,
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

    // Compress with multi-strategy filter selection
    let compressed = compress_filtered(
        &pixel_bytes[..expected_len],
        row_bytes,
        h,
        bpp,
        compression_level,
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

// ---- Compression with multi-strategy filter selection ----

fn compress_filtered(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
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
    let mut verify_buf = vec![0u8; filtered_size];

    // Base strategies: 5 single-filter + 4 adaptive heuristics
    let base_strategies: &[Strategy] = &[
        Strategy::Single(0), // None
        Strategy::Single(1), // Sub
        Strategy::Single(2), // Up
        Strategy::Single(3), // Average
        Strategy::Single(4), // Paeth
        Strategy::Adaptive(AdaptiveHeuristic::MinSum),
        Strategy::Adaptive(AdaptiveHeuristic::Entropy),
        Strategy::Adaptive(AdaptiveHeuristic::Bigrams),
        Strategy::Adaptive(AdaptiveHeuristic::BigEnt),
    ];

    // At high compression levels, also try brute-force filter selection
    let brute_strategy;
    let strategies: Vec<Strategy> = if compression_level >= 9 {
        brute_strategy = Strategy::BruteForce { context_rows: 10 };
        let mut s: Vec<Strategy> = base_strategies.to_vec();
        s.push(brute_strategy);
        s
    } else {
        base_strategies.to_vec()
    };

    for strategy in &strategies {
        filtered.clear();
        filter_image(
            packed_rows,
            row_bytes,
            height,
            bpp,
            *strategy,
            &mut filtered,
        );

        let compressed_len = compressor
            .zlib_compress(&filtered, &mut compress_buf)
            .map_err(|e| PngError::InvalidInput(alloc::format!("compression failed: {e}")))?;

        // Verify decompression roundtrip (catches rare zenflate compression bugs)
        {
            let mut decompressor = zenflate::Decompressor::new();
            if decompressor
                .zlib_decompress(&compress_buf[..compressed_len], &mut verify_buf)
                .is_err()
            {
                // This strategy produced corrupt compressed output — skip it
                continue;
            }
        }

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

#[derive(Clone, Copy, Debug)]
enum Strategy {
    Single(u8),
    Adaptive(AdaptiveHeuristic),
    /// Per-row brute-force with trailing context: for each row, try all 5
    /// filters, compress (context + candidate) with fast DEFLATE, pick
    /// smallest. `context_rows` controls how many prior filtered rows to
    /// include as DEFLATE context (capped at DEFLATE's 32 KiB window).
    BruteForce { context_rows: usize },
}

#[derive(Clone, Copy, Debug)]
enum AdaptiveHeuristic {
    MinSum,
    Entropy,
    Bigrams,
    BigEnt,
}

fn filter_image(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    strategy: Strategy,
    out: &mut Vec<u8>,
) {
    match strategy {
        Strategy::BruteForce { context_rows } => {
            filter_image_brute(packed_rows, row_bytes, height, bpp, context_rows, out);
        }
        _ => {
            filter_image_heuristic(packed_rows, row_bytes, height, bpp, strategy, out);
        }
    }
}

fn filter_image_heuristic(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
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
                apply_filter(f, row, &prev_row, bpp, &mut candidates[0]);
                out.extend_from_slice(&candidates[0]);
            }
            Strategy::Adaptive(heuristic) => {
                for f in 0..5u8 {
                    apply_filter(f, row, &prev_row, bpp, &mut candidates[f as usize]);
                }
                let best_f = pick_best_filter(&candidates, heuristic);
                out.push(best_f);
                out.extend_from_slice(&candidates[best_f as usize]);
            }
            Strategy::BruteForce { .. } => unreachable!(),
        }

        prev_row.copy_from_slice(row);
    }
}

/// Per-row brute-force filter selection with trailing context.
///
/// For each row, tries all 5 PNG filters, compresses (context + candidate)
/// with fast DEFLATE, and picks the filter producing the smallest output.
/// The trailing context (previous `context_rows` filtered rows) lets the
/// evaluation compressor exploit cross-row patterns, matching how the final
/// full-stream compression will see the data.
fn filter_image_brute(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    context_rows: usize,
    out: &mut Vec<u8>,
) {
    let filtered_row_size = row_bytes + 1; // filter byte + row data

    // Cap context to DEFLATE's 32 KiB sliding window
    let max_context_bytes = 32 * 1024;
    let context_rows = context_rows
        .min(max_context_bytes / filtered_row_size)
        .max(1);
    let max_context = context_rows * filtered_row_size;

    // Eval compressor at L1 (hash-table greedy, very fast)
    let eval_level = CompressionLevel::new(1);
    let mut eval_compressor = Compressor::new(eval_level);

    let eval_max_input = max_context + filtered_row_size;
    let compress_bound = Compressor::zlib_compress_bound(eval_max_input);
    let mut compress_buf = vec![0u8; compress_bound];

    // Candidate buffers for each filter's filtered row data
    let mut candidate_data: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();

    let mut eval_buf = Vec::with_capacity(eval_max_input);
    let mut prev_row = vec![0u8; row_bytes];

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];

        // Get trailing context from already-committed filtered output
        let context_start = if out.len() > max_context {
            out.len() - max_context
        } else {
            0
        };
        let context = &out[context_start..];

        // Try all 5 filters, evaluate with context
        let mut best_f = 0u8;
        let mut best_size = usize::MAX;

        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidate_data[f as usize]);

            eval_buf.clear();
            eval_buf.extend_from_slice(context);
            eval_buf.push(f);
            eval_buf.extend_from_slice(&candidate_data[f as usize]);

            if let Ok(len) = eval_compressor.zlib_compress(&eval_buf, &mut compress_buf) {
                if len < best_size {
                    best_size = len;
                    best_f = f;
                }
            }
        }

        // Emit winning filter
        out.push(best_f);
        out.extend_from_slice(&candidate_data[best_f as usize]);

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
        AdaptiveHeuristic::BigEnt => {
            let mut best = 0u8;
            let mut best_score = f64::MAX;
            for f in 0..5u8 {
                let score = bigram_entropy_score(&candidates[f as usize]);
                if score < best_score {
                    best_score = score;
                    best = f;
                }
            }
            best
        }
    }
}

fn apply_filter(filter: u8, row: &[u8], prev_row: &[u8], bpp: usize, out: &mut [u8]) {
    let len = row.len();
    match filter {
        0 => out[..len].copy_from_slice(row),
        1 => {
            // Sub: first bpp bytes raw, rest subtract left neighbor
            let b = bpp.min(len);
            out[..b].copy_from_slice(&row[..b]);
            for i in bpp..len {
                out[i] = row[i].wrapping_sub(row[i - bpp]);
            }
        }
        2 => {
            // Up
            for i in 0..len {
                out[i] = row[i].wrapping_sub(prev_row[i]);
            }
        }
        3 => {
            // Average: first bpp bytes use only above, rest use left+above
            for i in 0..bpp.min(len) {
                out[i] = row[i].wrapping_sub(prev_row[i] >> 1);
            }
            for i in bpp..len {
                let avg = ((row[i - bpp] as u16 + prev_row[i] as u16) >> 1) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
            // Paeth: first bpp bytes use paeth(0, above, 0), rest use full paeth
            for i in 0..bpp.min(len) {
                out[i] = row[i].wrapping_sub(paeth_predictor(0, prev_row[i], 0));
            }
            for i in bpp..len {
                let pred = paeth_predictor(row[i - bpp], prev_row[i], prev_row[i - bpp]);
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

/// Shannon entropy of byte-pair bigrams.
///
/// Unlike `bigrams_score` which counts unique bigrams, this computes the
/// actual entropy of the bigram distribution. Better at distinguishing
/// between filtered rows that have similar unique-bigram counts but
/// different frequency distributions.
fn bigram_entropy_score(data: &[u8]) -> f64 {
    if data.len() < 2 {
        return 0.0;
    }
    let mut counts = vec![0u32; 65536];
    let n = data.len() - 1;
    for pair in data.windows(2) {
        let key = (pair[0] as usize) << 8 | pair[1] as usize;
        counts[key] += 1;
    }
    let len = n as f64;
    let mut entropy = 0.0f64;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

// ---- Bit depth and packing (indexed only) ----

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

// ---- Metadata writing ----

/// Write all metadata chunks in correct PNG order.
///
/// Chunk order: sRGB → gAMA → cHRM → iCCP → cICP → mDCV → cLLi → eXIf → iTXt(XMP)
///
/// Per PNG spec: sRGB/gAMA/cHRM must come before PLTE and IDAT.
/// iCCP must come before PLTE. cICP/mDCV/cLLi must come before IDAT.
fn write_all_metadata(out: &mut Vec<u8>, meta: &PngWriteMetadata<'_>) -> Result<(), PngError> {
    // sRGB rendering intent
    if let Some(intent) = meta.srgb_intent {
        write_srgb_chunk(out, intent);
    }

    // gAMA (source gamma)
    if let Some(gamma) = meta.source_gamma {
        write_gama_chunk(out, gamma);
    }

    // cHRM (chromaticities)
    if let Some(chrm) = &meta.chromaticities {
        write_chrm_chunk(out, chrm);
    }

    // iCCP (ICC profile) — mutually exclusive with sRGB per spec,
    // but we write both if provided (decoders handle this fine)
    if let Some(generic) = meta.generic {
        if let Some(icc) = generic.icc_profile {
            write_iccp_chunk(out, icc)?;
        }
    }

    // cICP (coding-independent code points)
    if let Some(cicp) = &meta.cicp {
        write_cicp_chunk(out, cicp);
    }

    // mDCV (mastering display color volume)
    if let Some(mdcv) = &meta.mastering_display {
        write_mdcv_chunk(out, mdcv);
    }

    // cLLi (content light level info)
    if let Some(clli) = &meta.content_light_level {
        write_clli_chunk(out, clli);
    }

    // eXIf
    if let Some(generic) = meta.generic {
        if let Some(exif) = generic.exif {
            write_exif_chunk(out, exif);
        }
    }

    // iTXt for XMP
    if let Some(generic) = meta.generic {
        if let Some(xmp) = generic.xmp {
            let xmp_str = core::str::from_utf8(xmp).unwrap_or_default();
            if !xmp_str.is_empty() {
                write_itxt_chunk(out, "XML:com.adobe.xmp", xmp_str);
            }
        }
    }

    Ok(())
}

// ---- Individual chunk writers ----

fn write_srgb_chunk(out: &mut Vec<u8>, intent: u8) {
    write_chunk(out, b"sRGB", &[intent]);
}

fn write_gama_chunk(out: &mut Vec<u8>, gamma: u32) {
    write_chunk(out, b"gAMA", &gamma.to_be_bytes());
}

fn write_chrm_chunk(out: &mut Vec<u8>, chrm: &PngChromaticities) {
    // cHRM: 8 u32 values in order: white_x, white_y, red_x, red_y, green_x, green_y, blue_x, blue_y
    let mut data = [0u8; 32];
    data[0..4].copy_from_slice(&chrm.white_x.to_be_bytes());
    data[4..8].copy_from_slice(&chrm.white_y.to_be_bytes());
    data[8..12].copy_from_slice(&chrm.red_x.to_be_bytes());
    data[12..16].copy_from_slice(&chrm.red_y.to_be_bytes());
    data[16..20].copy_from_slice(&chrm.green_x.to_be_bytes());
    data[20..24].copy_from_slice(&chrm.green_y.to_be_bytes());
    data[24..28].copy_from_slice(&chrm.blue_x.to_be_bytes());
    data[28..32].copy_from_slice(&chrm.blue_y.to_be_bytes());
    write_chunk(out, b"cHRM", &data);
}

fn write_cicp_chunk(out: &mut Vec<u8>, cicp: &Cicp) {
    // cICP: 4 bytes — color_primaries, transfer_function, matrix_coefficients, full_range
    let data = [
        cicp.color_primaries,
        cicp.transfer_characteristics,
        cicp.matrix_coefficients,
        if cicp.full_range { 1 } else { 0 },
    ];
    write_chunk(out, b"cICP", &data);
}

fn write_mdcv_chunk(out: &mut Vec<u8>, mdcv: &MasteringDisplay) {
    // mDCV: 6×u16 chromaticities (R, G, B primaries as xy pairs) + 2×u16 white point
    //       + u32 max_luminance + u32 min_luminance = 24 bytes
    // PNG mDCV uses u16 in units of 0.00002 (same as zencodec MasteringDisplay)
    let mut data = [0u8; 24];
    // Chromaticities: Rx, Ry, Gx, Gy, Bx, By (6 u16 values)
    for (i, &[x, y]) in mdcv.primaries.iter().enumerate() {
        data[i * 4..i * 4 + 2].copy_from_slice(&x.to_be_bytes());
        data[i * 4 + 2..i * 4 + 4].copy_from_slice(&y.to_be_bytes());
    }
    // White point: Wx, Wy
    data[12..14].copy_from_slice(&mdcv.white_point[0].to_be_bytes());
    data[14..16].copy_from_slice(&mdcv.white_point[1].to_be_bytes());
    // Luminances (u32, 0.0001 cd/m²)
    data[16..20].copy_from_slice(&mdcv.max_luminance.to_be_bytes());
    data[20..24].copy_from_slice(&mdcv.min_luminance.to_be_bytes());
    write_chunk(out, b"mDCV", &data);
}

fn write_clli_chunk(out: &mut Vec<u8>, clli: &ContentLightLevel) {
    // cLLi: u32 max_content_light_level + u32 max_frame_average_light_level
    // PNG cLLi uses 0.0001 cd/m² units; zencodec ContentLightLevel uses cd/m² (u16)
    let max_cll = clli.max_content_light_level as u32 * 10000;
    let max_fall = clli.max_frame_average_light_level as u32 * 10000;
    let mut data = [0u8; 8];
    data[0..4].copy_from_slice(&max_cll.to_be_bytes());
    data[4..8].copy_from_slice(&max_fall.to_be_bytes());
    write_chunk(out, b"cLLI", &data);
}

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

fn metadata_size_estimate(meta: &PngWriteMetadata<'_>) -> usize {
    let mut size = 0;
    if let Some(generic) = meta.generic {
        if let Some(icc) = generic.icc_profile {
            size += 12 + 13 + icc.len() / 2;
        }
        if let Some(exif) = generic.exif {
            size += 12 + exif.len();
        }
        if let Some(xmp) = generic.xmp {
            size += 12 + 25 + xmp.len();
        }
    }
    // Color chunks: sRGB(1) + gAMA(4) + cHRM(32) + cICP(4) + mDCV(24) + cLLi(8)
    // Each chunk has 12 bytes overhead (len + type + crc)
    if meta.srgb_intent.is_some() {
        size += 13;
    }
    if meta.source_gamma.is_some() {
        size += 16;
    }
    if meta.chromaticities.is_some() {
        size += 44;
    }
    if meta.cicp.is_some() {
        size += 16;
    }
    if meta.mastering_display.is_some() {
        size += 36;
    }
    if meta.content_light_level.is_some() {
        size += 20;
    }
    size
}
