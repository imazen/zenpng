//! PNG filter strategies: single-filter, adaptive heuristic, and brute-force.

use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zenflate::{CompressionLevel, Compressor};

/// Heuristic strategies to screen in Phase 1.
pub(crate) const HEURISTIC_STRATEGIES: &[Strategy] = &[
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

/// Reduced strategy list for Fastest/Fast (compression_level <= 4).
///
/// Drops Single(Sub/Up/Average) — they rarely win screening. Keeps None
/// (wins on flat screenshots), Paeth (wins on some screenshots), and the
/// 3 best adaptive heuristics. 5 strategies instead of 9 = ~44% faster screen.
pub(crate) const FAST_STRATEGIES: &[Strategy] = &[
    Strategy::Single(0), // None
    Strategy::Single(4), // Paeth
    Strategy::Adaptive(AdaptiveHeuristic::MinSum),
    Strategy::Adaptive(AdaptiveHeuristic::Bigrams),
    Strategy::Adaptive(AdaptiveHeuristic::BigEnt),
];

#[derive(Clone, Copy, Debug)]
pub(crate) enum Strategy {
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
    /// Forking brute-force: maintains real DEFLATE compressor state across
    /// rows. For each row, clones the compressor, tries all 5 filters via
    /// incremental compression, picks the filter producing the smallest
    /// cumulative output. The winning fork becomes the state for the next row.
    ///
    /// Produces better results than context-based BruteForce because it uses
    /// actual DEFLATE state (hash tables, frequency counters) rather than a
    /// limited raw context window.
    BruteForceFork {
        eval_level: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum AdaptiveHeuristic {
    MinSum,
    Entropy,
    Bigrams,
    BigEnt,
}

pub(crate) fn filter_image(
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
        Strategy::BruteForceFork { eval_level } => {
            filter_image_brute_fork(packed_rows, row_bytes, height, bpp, eval_level, cancel, out);
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
            Strategy::BruteForce { .. }
            | Strategy::BruteForceBlock { .. }
            | Strategy::BruteForceFork { .. } => unreachable!(),
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

/// Forking brute-force filter selection using incremental DEFLATE state.
///
/// For each row, clones the compressor 5 times (one per filter), feeds each
/// clone the accumulated filtered stream via `deflate_compress_incremental`,
/// and picks the filter producing the smallest cumulative output. The winning
/// clone becomes the compressor for the next row.
///
/// This produces better filter choices than context-based brute-force because
/// it uses actual DEFLATE state (hash tables, frequency counters, match history)
/// rather than a limited raw context window.
#[allow(clippy::too_many_arguments)]
fn filter_image_brute_fork(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    eval_level: u32,
    cancel: &dyn Stop,
    out: &mut Vec<u8>,
) {
    let filtered_row_size = row_bytes + 1; // filter byte + row data

    let mut compressor = Compressor::new(CompressionLevel::new(eval_level));
    let mut candidate_data: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();
    let mut prev_row = vec![0u8; row_bytes];

    // Output buffer for incremental compression — sized for one row's worth
    let compress_bound = Compressor::deflate_compress_bound(filtered_row_size * height);
    let mut compress_buf = vec![0u8; compress_bound];

    // Track cumulative compressed size for the winning compressor
    let mut cumulative_output = 0usize;

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];
        let is_final = y == height - 1;

        // Try all 5 filters
        let mut best_f = 0u8;
        let mut best_size = usize::MAX;
        let mut best_compressor = None;

        for f in 0..5u8 {
            if cancel.check().is_err() {
                // On cancel, emit remaining rows with filter 0
                for rem_y in y..height {
                    out.push(0);
                    out.extend_from_slice(
                        &packed_rows[rem_y * row_bytes..(rem_y + 1) * row_bytes],
                    );
                }
                return;
            }

            apply_filter(f, row, &prev_row, bpp, &mut candidate_data[f as usize]);

            // Clone the compressor to try this filter
            let mut fork = compressor.clone();

            // Build the accumulated data: existing output + this candidate row
            // The incremental API expects the full accumulated buffer.
            // We already committed prior rows to `out`; now append the candidate.
            let new_start = out.len();
            out.push(f);
            out.extend_from_slice(&candidate_data[f as usize]);

            // Compress incrementally from where we left off
            let result = fork.deflate_compress_incremental(
                out,
                &mut compress_buf,
                is_final,
                zenflate::Unstoppable,
            );

            // Remove the candidate row (we haven't committed it yet)
            out.truncate(new_start);

            if let Ok(size) = result {
                let total = cumulative_output + size;
                if total < best_size {
                    best_size = total;
                    best_f = f;
                    best_compressor = Some((fork, size));
                }
            }
        }

        // Commit winning filter
        out.push(best_f);
        out.extend_from_slice(&candidate_data[best_f as usize]);

        if let Some((winner, size)) = best_compressor {
            compressor = winner;
            cumulative_output += size;
        }

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

pub(crate) fn apply_filter(filter: u8, row: &[u8], prev_row: &[u8], bpp: usize, out: &mut [u8]) {
    let len = row.len();
    match filter {
        0 => out[..len].copy_from_slice(row),
        1 => {
            let b = bpp.min(len);
            out[..b].copy_from_slice(&row[..b]);
            for i in bpp..len {
                out[i] = row[i].wrapping_sub(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..len {
                out[i] = row[i].wrapping_sub(prev_row[i]);
            }
        }
        3 => {
            for i in 0..bpp.min(len) {
                out[i] = row[i].wrapping_sub(prev_row[i] >> 1);
            }
            for i in bpp..len {
                let avg = ((row[i - bpp] as u16 + prev_row[i] as u16) >> 1) as u8;
                out[i] = row[i].wrapping_sub(avg);
            }
        }
        4 => {
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
