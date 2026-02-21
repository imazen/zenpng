//! Low-level PNG writer using zenflate for compression.
//!
//! Bypasses the `png` crate's streaming flate2 API to use zenflate's
//! buffer-based compression. Multi-strategy filter selection tries 8
//! strategies (5 single-filter + 3 adaptive) and keeps the smallest.

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zencodec_types::{Cicp, ContentLightLevel, ImageMetadata, MasteringDisplay};
use zenflate::{crc32, CompressionLevel, Compressor, Unstoppable};

use crate::decode::PngChromaticities;
use crate::error::PngError;

/// Time-aware budget for compression. Returns remaining time so the
/// compressor can calibrate iteration counts (e.g., zopfli).
pub(crate) trait Budget: Send + Sync {
    /// Remaining nanoseconds, or `None` if unlimited.
    fn remaining_ns(&self) -> Option<u64>;

    /// Whether the budget is exhausted.
    fn is_exhausted(&self) -> bool {
        self.remaining_ns().is_some_and(|ns| ns == 0)
    }
}

/// Unlimited budget — never exhausted.
pub(crate) struct Unlimited;

impl Budget for Unlimited {
    fn remaining_ns(&self) -> Option<u64> {
        None
    }
}

/// Combined stop signal: checks cancel first (hard `Cancelled`), then budget
/// exhaustion (graceful `TimedOut`). When budget expires, zenzop gracefully
/// stops at the next squeeze iteration boundary and returns best-so-far.
#[cfg(feature = "zopfli")]
struct BudgetStop<'a> {
    cancel: &'a dyn Stop,
    budget: &'a dyn Budget,
}

#[cfg(feature = "zopfli")]
impl Stop for BudgetStop<'_> {
    fn check(&self) -> Result<(), enough::StopReason> {
        self.cancel.check()?;
        if self.budget.is_exhausted() {
            return Err(enough::StopReason::TimedOut);
        }
        Ok(())
    }
}

/// Compression options passed through the pipeline.
pub(crate) struct CompressOptions<'a> {
    /// Whether to use zopfli for final compression (Crush level).
    #[allow(dead_code)] // read only with `zopfli` feature
    pub use_zopfli: bool,
    /// Budget mode: progressively escalate through compression tiers
    /// instead of jumping straight to the target level. The budget
    /// controls how far we get.
    pub is_budget: bool,
    /// Soft time budget — checked between strategies, returns best-so-far when exhausted.
    pub budget: &'a dyn Budget,
    /// Hard cancel — passed into zenflate, aborts mid-compression.
    pub cancel: &'a dyn Stop,
}

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
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_indexed_png(
    indices: &[u8],
    width: u32,
    height: u32,
    palette_rgb: &[u8],
    palette_alpha: Option<&[u8]>,
    write_meta: &PngWriteMetadata<'_>,
    compression_level: u8,
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
    let compressed = compress_filtered(&packed_rows, row_bytes, h, 1, compression_level, opts)?;

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
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_truecolor_png(
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
    color_type: u8,
    bit_depth: u8,
    write_meta: &PngWriteMetadata<'_>,
    compression_level: u8,
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

    // Compress with multi-strategy filter selection
    let compressed = compress_filtered(
        &pixel_bytes[..expected_len],
        row_bytes,
        h,
        bpp,
        compression_level,
        opts,
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

/// Heuristic strategies to screen in Phase 1.
const HEURISTIC_STRATEGIES: &[Strategy] = &[
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

/// Try compressing `filtered` data with all `compressors`, updating `best_compressed`
/// if a smaller result is found. Returns the best compressed size for this particular
/// filtered stream (used for ranking candidates).
fn try_compress(
    filtered: &[u8],
    compressors: &mut [Compressor],
    compress_buf: &mut [u8],
    verify_buf: &mut [u8],
    best_compressed: &mut Option<Vec<u8>>,
    cancel: &dyn Stop,
) -> Result<usize, PngError> {
    let mut best_for_stream = usize::MAX;
    for compressor in compressors.iter_mut() {
        let compressed_len = match compressor.zlib_compress(filtered, compress_buf, cancel) {
            Ok(len) => len,
            Err(zenflate::CompressionError::Stopped(reason)) => return Err(reason.into()),
            Err(e) => {
                return Err(PngError::InvalidInput(alloc::format!(
                    "compression failed: {e}"
                )));
            }
        };

        // Verify decompression roundtrip
        {
            let mut decompressor = zenflate::Decompressor::new();
            if decompressor
                .zlib_decompress(&compress_buf[..compressed_len], verify_buf, Unstoppable)
                .is_err()
            {
                continue;
            }
        }

        best_for_stream = best_for_stream.min(compressed_len);

        let dominated = best_compressed
            .as_ref()
            .is_some_and(|b| compressed_len >= b.len());
        if !dominated {
            *best_compressed = Some(compress_buf[..compressed_len].to_vec());
        }
    }
    Ok(best_for_stream)
}

/// Progressive adaptive compression engine.
///
/// Instead of a flat loop over all strategies × all levels, works in phases:
///
/// **Phase 1 (Screen):** Try all heuristic strategies with a cheap L1 compressor
/// to rank them. Cost: ~1ms per strategy. This gets us a valid result immediately.
///
/// **Phase 2 (Refine):** Compress the top 3 filtered streams at the target
/// compression level(s). For L10+, tries L10/L11/L12. This is where 90%+ of
/// final quality comes from.
///
/// **Phase 3 (Brute-force):** Per-row brute-force filter selection with DEFLATE
/// context evaluation. Only for level >= 6. Expensive (~3-4s per config on
/// 1024×1024) but can beat heuristics on complex images.
///
/// **Phase 4 (Zopfli):** Adaptive zopfli compression on the best candidates.
/// Only for Crush/Budget with the `zopfli` feature enabled.
///
/// Each phase checks the deadline before starting. `Budget(ms)` gets the best
/// result achievable within the time limit.
fn compress_filtered(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    compression_level: u8,
    opts: CompressOptions<'_>,
) -> Result<Vec<u8>, PngError> {
    let filtered_size = (row_bytes + 1) * height;
    let mut best_compressed: Option<Vec<u8>> = None;

    // Reusable buffers
    let mut filtered = Vec::with_capacity(filtered_size);
    let compress_bound = Compressor::zlib_compress_bound(filtered_size);
    let mut compress_buf = vec![0u8; compress_bound];
    let mut verify_buf = vec![0u8; filtered_size];

    // For L0-1, screening IS the final compression — no Phase 2 needed.
    // For L2+, screen at L1 then refine the best candidates at the target level.
    let screen_level = if compression_level <= 1 {
        compression_level as u32
    } else {
        1
    };

    // Refinement tiers: which compression levels to try in Phase 2.
    //
    // Budget mode escalates through tiers progressively — L4, L6, L9,
    // then L10/11/12 — checking the deadline between each. This way a
    // tight budget gets a quick L4 result, while a generous budget reaches
    // near-optimal.
    //
    // Non-budget modes jump straight to the target level(s).
    let refine_tiers: &[u32] = if opts.is_budget {
        &[4, 6, 9, 10, 11, 12]
    } else if compression_level >= 10 {
        &[10, 11, 12]
    } else {
        // Return a static slice for the target level. For levels 2-9
        // this is a single entry matching the requested compression.
        match compression_level {
            2 => &[2],
            3 => &[3],
            4 => &[4],
            5 => &[5],
            6 => &[6],
            7 => &[7],
            8 => &[8],
            9 => &[9],
            _ => &[1], // L0-1 won't reach here (needs_refine is false)
        }
    };

    let needs_refine = compression_level >= 2 || opts.is_budget;

    // Budget mode enables all phases regardless of the nominal compression_level,
    // since the deadline controls how far we actually get.
    let can_brute_force = compression_level >= 6 || opts.is_budget;

    // Brute-force configs: (context_rows, eval_level)
    let brute_configs: &[(usize, u32)] = if opts.is_budget {
        &[(10, 1), (10, 4)]
    } else {
        match compression_level {
            10.. => &[(10, 1), (10, 4)],
            9 => &[(8, 1)],
            6..=8 => &[(3, 1)],
            _ => &[],
        }
    };

    // Block-wise brute-force configs: (context_rows, eval_level)
    // Only at L9+ where the extra cost is justified by quality gains.
    let block_brute_configs: &[(usize, u32)] = if opts.is_budget {
        &[(10, 1)]
    } else {
        match compression_level {
            10.. => &[(10, 1)],
            9 => &[(8, 1)],
            _ => &[],
        }
    };

    // ---- Phase 1: Screen all heuristic strategies ----
    // Use a cheap compressor to rank strategies by compressed size.
    let mut screen_compressor = Compressor::new(CompressionLevel::new(screen_level));
    // (screen_size, filtered_data) — sorted later to pick top candidates
    let mut screen_results: Vec<(usize, Vec<u8>)> = Vec::with_capacity(HEURISTIC_STRATEGIES.len());

    for (i, strategy) in HEURISTIC_STRATEGIES.iter().enumerate() {
        // Always try at least one strategy (even with zero budget),
        // but check budget before subsequent strategies.
        if i > 0 && opts.budget.is_exhausted() {
            break;
        }

        filtered.clear();
        filter_image(
            packed_rows,
            row_bytes,
            height,
            bpp,
            *strategy,
            opts.cancel,
            &mut filtered,
        );

        let compressed_len =
            match screen_compressor.zlib_compress(&filtered, &mut compress_buf, opts.cancel) {
                Ok(len) => len,
                Err(zenflate::CompressionError::Stopped(reason)) => return Err(reason.into()),
                Err(e) => {
                    return Err(PngError::InvalidInput(alloc::format!(
                        "compression failed: {e}"
                    )));
                }
            };

        // Verify decompression roundtrip
        let valid = {
            let mut decompressor = zenflate::Decompressor::new();
            decompressor
                .zlib_decompress(
                    &compress_buf[..compressed_len],
                    &mut verify_buf,
                    Unstoppable,
                )
                .is_ok()
        };

        if valid {
            // If screen level IS the target level, this is already a final result
            let dominated = best_compressed
                .as_ref()
                .is_some_and(|b| compressed_len >= b.len());
            if !dominated {
                best_compressed = Some(compress_buf[..compressed_len].to_vec());
            }
            screen_results.push((compressed_len, filtered.clone()));
        }
    }

    // Sort by screen size — best first
    screen_results.sort_by_key(|(size, _)| *size);

    // Early return: L0-1 don't need refinement, or deadline hit
    if !needs_refine || opts.budget.is_exhausted() {
        return best_compressed
            .ok_or_else(|| PngError::InvalidInput("no filter strategies tried".to_string()));
    }

    // ---- Phase 2: Refine top 3 at target level(s) ----
    //
    // Iterate tier-by-tier so we can deadline-check between tiers.
    // In Budget mode this means we get L4 results quickly, then L6, L9, etc.
    // In non-Budget mode there are typically 1-3 levels so the overhead is minimal.
    let top_n = screen_results.len().min(3);

    // Track the best zenflate size per candidate for zopfli ranking later
    #[cfg(feature = "zopfli")]
    let mut zopfli_candidates: Vec<(usize, Vec<u8>)> = Vec::new();

    for &tier_level in refine_tiers {
        if opts.budget.is_exhausted() {
            break;
        }

        let mut tier_compressor = Compressor::new(CompressionLevel::new(tier_level));

        for (_, filtered_data) in &screen_results[..top_n] {
            let _best_size = try_compress(
                filtered_data,
                core::slice::from_mut(&mut tier_compressor),
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;

            #[cfg(feature = "zopfli")]
            if opts.use_zopfli && _best_size < usize::MAX {
                // Only add to zopfli candidates at the highest tier we reach
                // (avoid duplicates — we'll deduplicate by taking top 3 by size later)
                zopfli_candidates.push((_best_size, filtered_data.clone()));
            }
        }
    }

    // ---- Phase 3: Brute-force ----
    // Brute-force filtering is expensive (~3-4s per config), so compress at
    // the highest tiers only. We already have lower-tier results from Phase 2.
    if can_brute_force && !opts.budget.is_exhausted() {
        let brute_levels: &[u32] = if compression_level >= 10 || opts.is_budget {
            &[10, 11, 12]
        } else {
            &[compression_level as u32]
        };
        let mut brute_compressors: Vec<Compressor> = brute_levels
            .iter()
            .map(|&l| Compressor::new(CompressionLevel::new(l)))
            .collect();

        for &(context_rows, eval_level) in brute_configs {
            if opts.budget.is_exhausted() {
                break;
            }

            filtered.clear();
            filter_image(
                packed_rows,
                row_bytes,
                height,
                bpp,
                Strategy::BruteForce {
                    context_rows,
                    eval_level,
                },
                opts.cancel,
                &mut filtered,
            );

            let _best_size = try_compress(
                &filtered,
                &mut brute_compressors,
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;

            #[cfg(feature = "zopfli")]
            if opts.use_zopfli && _best_size < usize::MAX {
                zopfli_candidates.push((_best_size, filtered.clone()));
            }
        }

        // Block-wise brute-force: runs after per-row so it has that result
        // as a baseline to beat.
        for &(context_rows, eval_level) in block_brute_configs {
            if opts.budget.is_exhausted() {
                break;
            }

            filtered.clear();
            filter_image(
                packed_rows,
                row_bytes,
                height,
                bpp,
                Strategy::BruteForceBlock {
                    context_rows,
                    eval_level,
                },
                opts.cancel,
                &mut filtered,
            );

            let _best_size = try_compress(
                &filtered,
                &mut brute_compressors,
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;

            #[cfg(feature = "zopfli")]
            if opts.use_zopfli && _best_size < usize::MAX {
                zopfli_candidates.push((_best_size, filtered.clone()));
            }
        }
    }

    // ---- Phase 4: Zopfli ----
    #[cfg(feature = "zopfli")]
    if opts.use_zopfli && !zopfli_candidates.is_empty() && !opts.budget.is_exhausted() {
        // Sort by zenflate size, take top 3
        zopfli_candidates.sort_by_key(|(size, _)| *size);
        zopfli_candidates.truncate(3);

        let best = zopfli_adaptive(
            &zopfli_candidates,
            opts.budget,
            opts.cancel,
            &mut best_compressed,
        )?;
        if let Some(b) = best {
            best_compressed = Some(b);
        }
    }

    best_compressed.ok_or_else(|| PngError::InvalidInput("no filter strategies tried".to_string()))
}

/// Adaptive zopfli compression with time budgeting.
///
/// Strategy:
/// 1. Calibrate: compress top candidate with 5 iterations, measure wall time.
/// 2. From calibration, estimate iterations that fit in remaining budget.
/// 3. If we can afford more iterations, run them in parallel on top candidates.
/// 4. Always keep the best result found at any stage.
#[cfg(feature = "zopfli")]
fn zopfli_adaptive(
    candidates: &[(usize, Vec<u8>)],
    budget: &dyn Budget,
    cancel: &dyn Stop,
    current_best: &mut Option<Vec<u8>>,
) -> Result<Option<Vec<u8>>, PngError> {
    use std::time::Instant;

    // Combined stop: cancel (hard Cancelled) + budget (graceful TimedOut).
    // When budget expires, zenzop gracefully returns best-so-far instead of erroring.
    let stop = BudgetStop { cancel, budget };

    let mut best: Option<Vec<u8>> = None;
    let mut update_best = |compressed: Vec<u8>| {
        let dominated = best.as_ref().is_some_and(|b| compressed.len() >= b.len())
            || current_best
                .as_ref()
                .is_some_and(|b| compressed.len() >= b.len());
        if !dominated {
            best = Some(compressed);
        }
    };

    // Phase 1: Calibration — compress best candidate with 5 iterations.
    let calibration_iters = 5u64;
    let cal_start = Instant::now();
    let cal_result = compress_with_zopfli_n(&candidates[0].1, calibration_iters, &stop)?;
    let cal_elapsed = cal_start.elapsed();
    update_best(cal_result);

    // Estimate time per iteration from calibration.
    let ms_per_iter = cal_elapsed.as_secs_f64() * 1000.0 / calibration_iters as f64;

    // Phase 2: Determine max affordable iterations.
    // Calibration gives us a target, but the BudgetStop provides a hard backstop —
    // if calibration overestimates, zenzop will gracefully stop when budget expires.
    let max_iters = if let Some(remaining_ns) = budget.remaining_ns() {
        let remaining_ms = remaining_ns as f64 / 1_000_000.0;
        if remaining_ms < ms_per_iter * 2.0 {
            // Not enough time for even a meaningful run — skip
            return Ok(best);
        }
        // Divide remaining time across candidates running in parallel.
        // With N threads, wall time = time for one candidate.
        let n_candidates = candidates.len().min(3) as f64;
        let parallel_factor = n_candidates.min(num_cpus() as f64);
        let ms_per_candidate = remaining_ms * parallel_factor / n_candidates;
        let iters = (ms_per_candidate / ms_per_iter).floor() as u64;
        iters.clamp(5, 100)
    } else {
        // No time limit — use fixed 50 iterations
        50u64
    };

    if max_iters <= calibration_iters {
        return Ok(best);
    }

    // Phase 3: Run top candidates in parallel with calculated iterations.
    // All threads share the combined stop — budget expiry gracefully stops
    // all threads, cancellation hard-aborts them.
    let zopfli_results: Vec<Result<Vec<u8>, PngError>> = std::thread::scope(|s| {
        let handles: Vec<_> = candidates
            .iter()
            .map(|(_size, data)| s.spawn(|| compress_with_zopfli_n(data, max_iters, &stop)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    for result in zopfli_results {
        update_best(result?);
    }

    Ok(best)
}

#[cfg(feature = "zopfli")]
fn compress_with_zopfli_n(
    data: &[u8],
    iterations: u64,
    stop: &dyn Stop,
) -> Result<Vec<u8>, PngError> {
    use std::io::Write;
    let options = zenzop::Options {
        iteration_count: core::num::NonZeroU64::new(iterations.max(1)).unwrap(),
        ..Default::default()
    };
    let mut encoder = zenzop::ZlibEncoder::with_stop(options, Vec::new(), stop)
        .map_err(|e| zenzop_err(e, stop))?;
    encoder.write_all(data).map_err(|e| zenzop_err(e, stop))?;
    let result = encoder.finish().map_err(|e| zenzop_err(e, stop))?;
    Ok(result.into_inner())
}

/// Convert a zenzop I/O error to a `PngError`.
///
/// Zenzop only returns errors for `Cancelled` stops — budget exhaustion (`TimedOut`)
/// is handled gracefully and returns `Ok` with suboptimal-but-valid output.
#[cfg(feature = "zopfli")]
fn zenzop_err(e: std::io::Error, stop: &dyn Stop) -> PngError {
    if let Err(reason) = stop.check() {
        return PngError::Stopped(reason);
    }
    PngError::InvalidInput(alloc::format!("zopfli compression failed: {e}"))
}

/// Best-effort CPU count for parallel zopfli.
#[cfg(feature = "zopfli")]
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

// ---- Filter strategies ----

#[derive(Clone, Copy, Debug)]
enum Strategy {
    Single(u8),
    Adaptive(AdaptiveHeuristic),
    /// Per-row brute-force with trailing context: for each row, try all 5
    /// filters, compress (context + candidate) with DEFLATE at `eval_level`,
    /// pick smallest. `context_rows` controls how many prior filtered rows to
    /// include as DEFLATE context (capped at DEFLATE's 32 KiB window).
    BruteForce {
        context_rows: usize,
        eval_level: u32,
    },
    /// Block-wise brute-force: evaluates all 5^B filter combinations for
    /// groups of B rows simultaneously. Finds better local optima than
    /// per-row greedy by considering cross-row DEFLATE interactions.
    BruteForceBlock {
        context_rows: usize,
        eval_level: u32,
    },
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
    cancel: &dyn Stop,
    out: &mut Vec<u8>,
) {
    match strategy {
        Strategy::BruteForce {
            context_rows,
            eval_level,
        } => {
            filter_image_brute(
                packed_rows,
                row_bytes,
                height,
                bpp,
                context_rows,
                eval_level,
                cancel,
                out,
            );
        }
        Strategy::BruteForceBlock {
            context_rows,
            eval_level,
        } => {
            filter_image_blockwise(
                packed_rows,
                row_bytes,
                height,
                bpp,
                context_rows,
                eval_level,
                cancel,
                out,
            );
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
            Strategy::BruteForce { .. } | Strategy::BruteForceBlock { .. } => unreachable!(),
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
#[allow(clippy::too_many_arguments)]
fn filter_image_brute(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    context_rows: usize,
    eval_level: u32,
    cancel: &dyn Stop,
    out: &mut Vec<u8>,
) {
    let filtered_row_size = row_bytes + 1; // filter byte + row data

    // Cap context to DEFLATE's 32 KiB sliding window
    let max_context_bytes = 32 * 1024;
    let context_rows = context_rows
        .min(max_context_bytes / filtered_row_size)
        .max(1);
    let max_context = context_rows * filtered_row_size;

    let eval_level = CompressionLevel::new(eval_level);
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

            if let Ok(len) = eval_compressor.zlib_compress(&eval_buf, &mut compress_buf, cancel) {
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

/// Pre-compute all 5 filter outputs for every row.
///
/// Layout: flat buffer with `[row0_f0, row0_f1, ..., row0_f4, row1_f0, ...]`.
/// Each entry is `row_bytes` long. Total size: `5 * height * row_bytes`.
fn precompute_all_filters(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
) -> Vec<u8> {
    let mut buf = vec![0u8; 5 * height * row_bytes];
    let mut prev_row = vec![0u8; row_bytes];

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];
        for f in 0..5u8 {
            let offset = (y * 5 + f as usize) * row_bytes;
            apply_filter(f, row, &prev_row, bpp, &mut buf[offset..offset + row_bytes]);
        }
        prev_row.copy_from_slice(row);
    }
    buf
}

/// Index into a precomputed filter buffer.
#[inline]
fn precomputed_row(buf: &[u8], row_bytes: usize, row: usize, filter: usize) -> &[u8] {
    let offset = (row * 5 + filter) * row_bytes;
    &buf[offset..offset + row_bytes]
}

/// Pick block size B in [2, 5] based on image size and evaluation budget.
///
/// Filters ~20 rows with MinSum heuristic, then picks B so that the total
/// number of evaluations `5^B * ceil(height/B)` stays under a budget.
fn learn_block_size(height: usize) -> usize {
    // Budget: 200K evals for large images, 50K for small (<=64 rows)
    let budget = if height <= 64 { 50_000 } else { 200_000 };

    // Try B from 5 down to 2, pick the largest that fits budget
    for b in (2..=5).rev() {
        let blocks = height.div_ceil(b);
        let pow5 = 5usize.pow(b as u32);
        let evals = pow5 * blocks;
        if evals <= budget {
            return b;
        }
    }
    2 // fallback
}

/// Block-wise brute-force filter selection.
///
/// Evaluates all 5^B filter combinations for groups of B rows simultaneously,
/// finding better local optima than per-row greedy selection.
#[allow(clippy::too_many_arguments)]
fn filter_image_blockwise(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    context_rows: usize,
    eval_level: u32,
    cancel: &dyn Stop,
    out: &mut Vec<u8>,
) {
    if height == 0 {
        return;
    }

    let block_size = learn_block_size(height);
    let filtered_row_size = row_bytes + 1; // filter byte + row data

    // Cap context to DEFLATE's 32 KiB sliding window
    let max_context_bytes = 32 * 1024;
    let context_rows = context_rows
        .min(max_context_bytes / filtered_row_size)
        .max(1);
    let max_context = context_rows * filtered_row_size;

    let eval_level = CompressionLevel::new(eval_level);
    let mut eval_compressor = Compressor::new(eval_level);

    let eval_max_input = max_context + block_size * filtered_row_size;
    let compress_bound = Compressor::zlib_compress_bound(eval_max_input);
    let mut compress_buf = vec![0u8; compress_bound];

    // Pre-compute all filter variants if they fit in 64 MB
    let total_precompute = 5 * height * row_bytes;
    let precomputed = if total_precompute <= 64 * 1024 * 1024 {
        Some(precompute_all_filters(packed_rows, row_bytes, height, bpp))
    } else {
        None
    };

    // Scratch space for per-block filter computation when not precomputed
    let mut block_filters: Vec<u8> = if precomputed.is_none() {
        vec![0u8; 5 * block_size * row_bytes]
    } else {
        Vec::new()
    };

    let mut eval_buf = Vec::with_capacity(eval_max_input);

    let mut block_start = 0;
    while block_start < height {
        // Check cancel between blocks
        if cancel.check().is_err() {
            // Fill remaining rows with filter 0 (None)
            for y in block_start..height {
                out.push(0);
                if let Some(ref pc) = precomputed {
                    out.extend_from_slice(precomputed_row(pc, row_bytes, y, 0));
                } else {
                    out.extend_from_slice(&packed_rows[y * row_bytes..(y + 1) * row_bytes]);
                }
            }
            return;
        }

        let actual_block = block_size.min(height - block_start);
        let combos = 5usize.pow(actual_block as u32);

        // Compute filter variants for this block if not precomputed
        if precomputed.is_none() {
            let mut prev_row = vec![0u8; row_bytes];
            if block_start > 0 {
                prev_row.copy_from_slice(
                    &packed_rows[(block_start - 1) * row_bytes..block_start * row_bytes],
                );
            }
            for i in 0..actual_block {
                let y = block_start + i;
                let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];
                for f in 0..5u8 {
                    let offset = (i * 5 + f as usize) * row_bytes;
                    apply_filter(
                        f,
                        row,
                        &prev_row,
                        bpp,
                        &mut block_filters[offset..offset + row_bytes],
                    );
                }
                prev_row.copy_from_slice(row);
            }
        }

        // Get trailing context from already-committed filtered output
        let context_start = if out.len() > max_context {
            out.len() - max_context
        } else {
            0
        };
        let context = &out[context_start..];

        let mut best_combo = 0usize;
        let mut best_size = usize::MAX;

        for combo in 0..combos {
            // Decode combo into per-row filter choices
            eval_buf.clear();
            eval_buf.extend_from_slice(context);

            let mut c = combo;
            for i in 0..actual_block {
                let f = (c % 5) as u8;
                c /= 5;

                eval_buf.push(f);
                if let Some(ref pc) = precomputed {
                    eval_buf.extend_from_slice(precomputed_row(
                        pc,
                        row_bytes,
                        block_start + i,
                        f as usize,
                    ));
                } else {
                    let offset = (i * 5 + f as usize) * row_bytes;
                    eval_buf.extend_from_slice(&block_filters[offset..offset + row_bytes]);
                }
            }

            if let Ok(len) = eval_compressor.zlib_compress(&eval_buf, &mut compress_buf, cancel) {
                if len < best_size {
                    best_size = len;
                    best_combo = combo;
                }
            }
        }

        // Commit winning combination
        let mut c = best_combo;
        for i in 0..actual_block {
            let f = (c % 5) as u8;
            c /= 5;

            out.push(f);
            if let Some(ref pc) = precomputed {
                out.extend_from_slice(precomputed_row(pc, row_bytes, block_start + i, f as usize));
            } else {
                let offset = (i * 5 + f as usize) * row_bytes;
                out.extend_from_slice(&block_filters[offset..offset + row_bytes]);
            }
        }

        block_start += actual_block;
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
        .zlib_compress(icc_profile, &mut compressed, Unstoppable)
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

#[cfg(all(test, feature = "zopfli"))]
mod zopfli_tests {
    use super::*;
    use core::sync::atomic::{AtomicI64, Ordering};

    /// Budget that expires after a fixed number of `remaining_ns()` calls.
    /// Deterministic — no wall-clock dependency.
    struct CallCountBudget(AtomicI64);

    impl CallCountBudget {
        fn new(calls: i64) -> Self {
            Self(AtomicI64::new(calls))
        }
    }

    impl Budget for CallCountBudget {
        fn remaining_ns(&self) -> Option<u64> {
            let prev = self.0.fetch_sub(1, Ordering::Relaxed);
            if prev <= 0 {
                Some(0)
            } else {
                // Report 1 second remaining so zopfli_adaptive's Phase 2
                // calculates a reasonable iteration count.
                Some(1_000_000_000)
            }
        }
    }

    /// Stop that fires `Cancelled` after a fixed number of `check()` calls.
    struct CallCountCancel(AtomicI64);

    impl CallCountCancel {
        fn new(calls: i64) -> Self {
            Self(AtomicI64::new(calls))
        }
    }

    impl Stop for CallCountCancel {
        fn check(&self) -> Result<(), enough::StopReason> {
            let prev = self.0.fetch_sub(1, Ordering::Relaxed);
            if prev <= 0 {
                Err(enough::StopReason::Cancelled)
            } else {
                Ok(())
            }
        }
    }

    /// Generate compressible test data (repeating pattern).
    fn test_data() -> Vec<u8> {
        let pattern: Vec<u8> = (0..=255).collect();
        pattern.repeat(8) // 2048 bytes, compresses well
    }

    fn verify_zlib(compressed: &[u8], expected: &[u8]) {
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(compressed)
            .expect("decompression failed — zlib stream is invalid");
        assert_eq!(decompressed, expected);
    }

    // ---- compress_with_zopfli_n tests ----

    #[test]
    fn zopfli_unlimited_returns_valid_output() {
        let data = test_data();
        let stop = enough::Unstoppable;
        let result = compress_with_zopfli_n(&data, 5, &stop).unwrap();
        verify_zlib(&result, &data);
    }

    #[test]
    fn zopfli_budget_expiry_returns_valid_output() {
        // BudgetStop with a budget that expires after 2 squeeze iterations.
        // Zenzop should gracefully stop and return best-so-far.
        let data = test_data();
        let cancel = enough::Unstoppable;
        let budget = CallCountBudget::new(2);
        let stop = BudgetStop {
            cancel: &cancel,
            budget: &budget,
        };
        let result = compress_with_zopfli_n(&data, 50, &stop).unwrap();
        verify_zlib(&result, &data);
    }

    #[test]
    fn zopfli_cancel_returns_stopped() {
        // Cancel after a few check() calls — zenzop should error.
        let data = test_data();
        let cancel = CallCountCancel::new(2);
        let result = compress_with_zopfli_n(&data, 50, &cancel);
        assert!(
            matches!(
                result,
                Err(PngError::Stopped(enough::StopReason::Cancelled))
            ),
            "expected Stopped(Cancelled), got {result:?}",
        );
    }

    #[test]
    fn budget_stop_cancel_takes_priority() {
        // Both cancel and budget would fire — cancel should win.
        let cancel = CallCountCancel::new(0); // fires immediately
        let budget = CallCountBudget::new(0); // also exhausted
        let stop = BudgetStop {
            cancel: &cancel,
            budget: &budget,
        };
        let result = stop.check();
        assert!(matches!(result, Err(enough::StopReason::Cancelled)));
    }

    #[test]
    fn budget_stop_budget_fires_timed_out() {
        // Cancel is fine but budget is exhausted — should get TimedOut.
        let cancel = enough::Unstoppable;
        let budget = CallCountBudget::new(0); // exhausted
        let stop = BudgetStop {
            cancel: &cancel,
            budget: &budget,
        };
        let result = stop.check();
        assert!(matches!(result, Err(enough::StopReason::TimedOut)));
    }

    #[test]
    fn budget_stop_neither_fires() {
        // Both cancel and budget are fine — should get Ok.
        let cancel = enough::Unstoppable;
        let budget = CallCountBudget::new(100);
        let stop = BudgetStop {
            cancel: &cancel,
            budget: &budget,
        };
        assert!(stop.check().is_ok());
    }

    // ---- zopfli_adaptive tests ----

    #[test]
    fn zopfli_adaptive_unlimited_returns_valid() {
        let data = test_data();
        let compressed_size = {
            let c = compress_with_zopfli_n(&data, 5, &enough::Unstoppable).unwrap();
            c.len()
        };
        let candidates = vec![(compressed_size, data.clone())];
        let cancel = enough::Unstoppable;
        let mut current_best = None;

        let result = zopfli_adaptive(&candidates, &Unlimited, &cancel, &mut current_best).unwrap();

        assert!(result.is_some(), "should find a result");
        verify_zlib(result.as_ref().unwrap(), &data);
    }

    #[test]
    fn zopfli_adaptive_budget_expiry_returns_valid() {
        // Give enough calls for calibration (5 iterations ≈ 5 checks)
        // plus Phase 2 budget query, then expire during Phase 3.
        let data = test_data();
        let compressed_size = {
            let c = compress_with_zopfli_n(&data, 5, &enough::Unstoppable).unwrap();
            c.len()
        };
        let candidates = vec![(compressed_size, data.clone())];
        // ~7 calls for calibration + Phase 2 query, then a few more for Phase 3
        // before expiry. The exact count doesn't matter much — what matters is
        // that the result is valid even when budget expires mid-compression.
        let budget = CallCountBudget::new(10);
        let cancel = enough::Unstoppable;
        let mut current_best = None;

        let result = zopfli_adaptive(&candidates, &budget, &cancel, &mut current_best).unwrap();

        // Should have at least the calibration result.
        assert!(result.is_some(), "should have calibration result");
        verify_zlib(result.as_ref().unwrap(), &data);
    }

    #[test]
    fn zopfli_adaptive_cancel_returns_stopped() {
        let data = test_data();
        let compressed_size = {
            let c = compress_with_zopfli_n(&data, 5, &enough::Unstoppable).unwrap();
            c.len()
        };
        let candidates = vec![(compressed_size, data.clone())];
        // Cancel fires after 2 checks — should abort during calibration.
        let cancel = CallCountCancel::new(2);
        let mut current_best = None;

        let result = zopfli_adaptive(&candidates, &Unlimited, &cancel, &mut current_best);
        assert!(
            matches!(
                result,
                Err(PngError::Stopped(enough::StopReason::Cancelled))
            ),
            "expected Stopped(Cancelled), got {result:?}",
        );
    }

    // ---- non-regression tests ----

    /// Compress data with zenflate at the given level, return the zlib stream.
    fn zenflate_baseline(data: &[u8], level: u32) -> Vec<u8> {
        let mut compressor = Compressor::new(CompressionLevel::new(level));
        let bound = Compressor::zlib_compress_bound(data.len());
        let mut buf = vec![0u8; bound];
        let len = compressor
            .zlib_compress(data, &mut buf, enough::Unstoppable)
            .unwrap();
        buf[..len].to_vec()
    }

    /// Property: zopfli_adaptive must NEVER return a result larger than
    /// the zenflate baseline passed as current_best. Sweep all budget
    /// values from immediate expiry through generous.
    #[test]
    fn zopfli_adaptive_never_regresses_vs_zenflate() {
        let data = test_data();
        let zenflate_result = zenflate_baseline(&data, 12);
        let zenflate_size = zenflate_result.len();
        let candidates = vec![(zenflate_size, data.clone())];

        // Sweep: budget expires at various points — from before calibration
        // starts to well after the parallel phase finishes.
        for budget_calls in 0..=50 {
            let budget = CallCountBudget::new(budget_calls);
            let cancel = enough::Unstoppable;
            let mut current_best = Some(zenflate_result.clone());

            let result = zopfli_adaptive(&candidates, &budget, &cancel, &mut current_best).unwrap();

            if let Some(ref better) = result {
                assert!(
                    better.len() < zenflate_size,
                    "budget_calls={budget_calls}: zopfli ({}) must be strictly smaller \
                     than zenflate ({zenflate_size})",
                    better.len(),
                );
                verify_zlib(better, &data);
            }
            // None means zenflate won — no regression possible.
        }
    }

    /// Same property but with multiple candidates, testing the parallel phase.
    #[test]
    fn zopfli_adaptive_multi_candidate_never_regresses() {
        // Three different filter outputs (simulated with different data patterns).
        let patterns: Vec<Vec<u8>> = vec![
            (0..=255u8).collect::<Vec<_>>().repeat(8),
            (0..=255u8).rev().collect::<Vec<_>>().repeat(8),
            (0..=255u8)
                .flat_map(|b| [b, b])
                .collect::<Vec<_>>()
                .repeat(4),
        ];

        let candidates: Vec<(usize, Vec<u8>)> = patterns
            .iter()
            .map(|p| {
                let baseline = zenflate_baseline(p, 12);
                (baseline.len(), p.clone())
            })
            .collect();

        // Use the smallest zenflate result as current_best (realistic scenario).
        let best_pattern_idx = candidates
            .iter()
            .enumerate()
            .min_by_key(|(_, (size, _))| *size)
            .unwrap()
            .0;
        let zenflate_best = zenflate_baseline(&patterns[best_pattern_idx], 12);
        let zenflate_best_size = zenflate_best.len();

        for budget_calls in [0, 1, 3, 5, 8, 12, 20, 50] {
            let budget = CallCountBudget::new(budget_calls);
            let cancel = enough::Unstoppable;
            let mut current_best = Some(zenflate_best.clone());

            let result = zopfli_adaptive(&candidates, &budget, &cancel, &mut current_best).unwrap();

            if let Some(ref better) = result {
                assert!(
                    better.len() < zenflate_best_size,
                    "budget_calls={budget_calls}: zopfli ({}) must be strictly smaller \
                     than zenflate ({zenflate_best_size})",
                    better.len(),
                );
                // Verify it decompresses to one of the candidate patterns.
                let decompressed =
                    miniz_oxide::inflate::decompress_to_vec_zlib(better).expect("invalid zlib");
                assert!(
                    patterns.iter().any(|p| *p == decompressed),
                    "budget_calls={budget_calls}: decompressed data doesn't match any candidate",
                );
            }
        }
    }
}
