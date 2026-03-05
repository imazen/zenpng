//! PNG filter strategies: single-filter, adaptive heuristic, and brute-force.

use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zenflate::{CompressionLevel, Compressor, CompressorSnapshot, Unstoppable};

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

/// Reduced strategy list for Fast (effort 5-7).
///
/// Drops Single(Sub/Up/Average) — they rarely win screening. Keeps None
/// (wins on flat screenshots), Paeth (wins on some screenshots), and the
/// 3 cheapest adaptive heuristics. BigEnt is excluded — it's 30-170x slower
/// than MinSum due to 256KB memset + 65536-entry iteration per row, making
/// it inappropriate for "fast" tier.
pub(crate) const FAST_STRATEGIES: &[Strategy] = &[
    Strategy::Single(0), // None
    Strategy::Single(4), // Paeth
    Strategy::Adaptive(AdaptiveHeuristic::MinSum),
    Strategy::Adaptive(AdaptiveHeuristic::Bigrams),
    Strategy::Adaptive(AdaptiveHeuristic::Entropy),
];

/// Minimal strategy list for low effort (effort 3-4).
///
/// Just 3 strategies: None (best for flat content), Paeth (best single
/// filter overall), and Bigrams (best cheap adaptive). Enough for a
/// quick ranking without the cost of 5+ evaluations.
pub(crate) const MINIMAL_STRATEGIES: &[Strategy] = &[
    Strategy::Single(0), // None
    Strategy::Single(4), // Paeth
    Strategy::Adaptive(AdaptiveHeuristic::Bigrams),
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
    /// Adaptive narrowing fork: like BruteForceFork but uses cheap cost
    /// estimation to rank all 5 filters, then full incremental compression
    /// only on the top `narrow_to` candidates. Tracks filter win history to
    /// skip estimation for filters that haven't won recently.
    ///
    /// Produces similar quality to BruteForceFork at ~40-60% of the cost
    /// when narrowing from 5 to 2 candidates.
    AdaptiveFork {
        eval_level: u32,
        /// Number of top candidates to fully compress (typically 2).
        narrow_to: usize,
    },
    /// Beam search over incremental DEFLATE state. Maintains `beam_width`
    /// best partial filter sequences instead of greedily committing to one.
    /// At each row, expands each beam entry by all 5 filters (K×5 candidates),
    /// keeps the top K by cumulative compressed size. Finds better filter
    /// sequences than greedy BruteForceFork at ~K× the cost.
    BruteForceBeam {
        eval_level: u32,
        beam_width: usize,
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
        Strategy::AdaptiveFork {
            eval_level,
            narrow_to,
        } => {
            filter_image_adaptive_fork(
                packed_rows,
                row_bytes,
                height,
                bpp,
                eval_level,
                narrow_to,
                cancel,
                out,
            );
        }
        Strategy::BruteForceBeam {
            eval_level,
            beam_width,
        } => {
            filter_image_brute_beam(
                packed_rows,
                row_bytes,
                height,
                bpp,
                eval_level,
                beam_width,
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

    // Pre-allocate scratch buffers for heuristics that need them.
    // Previously these were allocated per-row inside pick_best_filter,
    // causing massive allocation churn (e.g. BigEnt: 256KB × height).
    let mut scratch = match strategy {
        Strategy::Adaptive(h) => Some(HeuristicScratch::new(h)),
        _ => None,
    };

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
                let best_f = pick_best_filter(&candidates, heuristic, scratch.as_mut().unwrap());
                out.push(best_f);
                out.extend_from_slice(&candidates[best_f as usize]);
            }
            Strategy::BruteForce { .. }
            | Strategy::BruteForceBlock { .. }
            | Strategy::BruteForceFork { .. }
            | Strategy::AdaptiveFork { .. }
            | Strategy::BruteForceBeam { .. } => unreachable!(),
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

            if let Ok(len) = eval_compressor.zlib_compress(&eval_buf, &mut compress_buf, cancel)
                && len < best_size
            {
                best_size = len;
                best_f = f;
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
/// For each row, snapshots the compressor state, tries all 5 filters via
/// `deflate_compress_incremental`, and restores the winning state. Uses
/// [`CompressorSnapshot`] for cheaper state save/restore than full cloning.
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
        let mut best_snap = None;

        // Snapshot before trying filters — cheaper than full clone
        let snap = compressor.snapshot();

        for f in 0..5u8 {
            if cancel.check().is_err() {
                // On cancel, emit remaining rows with filter 0
                for rem_y in y..height {
                    out.push(0);
                    out.extend_from_slice(&packed_rows[rem_y * row_bytes..(rem_y + 1) * row_bytes]);
                }
                return;
            }

            apply_filter(f, row, &prev_row, bpp, &mut candidate_data[f as usize]);

            // Restore to pre-row state and try this filter
            compressor.restore(snap.clone());

            let new_start = out.len();
            out.push(f);
            out.extend_from_slice(&candidate_data[f as usize]);

            let result = compressor.deflate_compress_incremental(
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
                    best_snap = Some((compressor.snapshot(), size));
                }
            }
        }

        // Commit winning filter
        out.push(best_f);
        out.extend_from_slice(&candidate_data[best_f as usize]);

        if let Some((winner_snap, size)) = best_snap {
            compressor.restore(winner_snap);
            cumulative_output += size;
        }

        prev_row.copy_from_slice(row);
    }
}

/// Adaptive narrowing fork: uses cheap cost estimation to rank filters, then
/// full incremental compression only on the top candidates.
///
/// Per row:
/// 1. Apply all 5 filters (always needed for the filter bytes).
/// 2. For each filter in the active set, estimate bit cost via
///    `deflate_estimate_cost_incremental()` (runs matchfinder, accumulates
///    costs from current Huffman code lengths, but no encoding/bitstream).
/// 3. Sort by estimated cost, take top `narrow_to`.
/// 4. Full `deflate_compress_incremental()` on the top candidates.
/// 5. Commit the winner.
///
/// Adaptive tracking: maintains a score for each filter (0-4). Each time a
/// filter wins, its score goes to `height/4` (stays active for many rows).
/// Scores decay by 1 each row. Filters with score 0 are skipped during
/// estimation (but re-activated periodically every 16 rows to avoid permanent
/// exclusion). This typically reduces from 5 estimations to 2-3 per row
/// after a warmup period.
#[allow(clippy::too_many_arguments)]
fn filter_image_adaptive_fork(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    eval_level: u32,
    narrow_to: usize,
    cancel: &dyn Stop,
    out: &mut Vec<u8>,
) {
    let filtered_row_size = row_bytes + 1;
    let narrow_to = narrow_to.clamp(1, 5);

    let mut compressor = Compressor::new(CompressionLevel::new(eval_level));
    let mut candidate_data: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();
    let mut prev_row = vec![0u8; row_bytes];

    let compress_bound = Compressor::deflate_compress_bound(filtered_row_size * height);
    let mut compress_buf = vec![0u8; compress_bound];

    let mut cumulative_output = 0usize;

    // Adaptive filter tracking: score per filter, decays over time.
    // Higher score = more likely to be estimated. Winning resets score high.
    let win_score = (height / 4).max(8) as u32;
    let mut filter_score = [win_score; 5]; // start all active

    // Scratch for sorting: (estimated_cost, filter_index)
    let mut estimates: Vec<(u64, u8)> = Vec::with_capacity(5);

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];
        let is_final = y == height - 1;

        if cancel.check().is_err() {
            for rem_y in y..height {
                out.push(0);
                out.extend_from_slice(&packed_rows[rem_y * row_bytes..(rem_y + 1) * row_bytes]);
            }
            return;
        }

        // Apply all 5 filters (cheap, needed for data regardless)
        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidate_data[f as usize]);
        }

        // Snapshot before estimation
        let snap = compressor.snapshot();

        // Phase 1: Estimate costs for active filters
        estimates.clear();
        let periodic_reactivation = y % 16 == 0;

        for f in 0..5u8 {
            // Skip inactive filters unless periodic reactivation
            if filter_score[f as usize] == 0 && !periodic_reactivation {
                continue;
            }

            compressor.restore(snap.clone());

            let new_start = out.len();
            out.push(f);
            out.extend_from_slice(&candidate_data[f as usize]);

            let cost = compressor
                .deflate_estimate_cost_incremental(out, Unstoppable)
                .unwrap_or(u64::MAX);

            out.truncate(new_start);
            estimates.push((cost, f));
        }

        // Sort by estimated cost (ascending), take top narrow_to
        estimates.sort_unstable_by_key(|&(cost, _)| cost);
        estimates.truncate(narrow_to);

        // Phase 2: Full incremental compression on narrowed candidates
        let mut best_f = estimates[0].1;
        let mut best_size = usize::MAX;
        let mut best_snap = None;

        for &(_, f) in &estimates {
            compressor.restore(snap.clone());

            let new_start = out.len();
            out.push(f);
            out.extend_from_slice(&candidate_data[f as usize]);

            let result = compressor.deflate_compress_incremental(
                out,
                &mut compress_buf,
                is_final,
                Unstoppable,
            );

            out.truncate(new_start);

            if let Ok(size) = result {
                let total = cumulative_output + size;
                if total < best_size {
                    best_size = total;
                    best_f = f;
                    best_snap = Some((compressor.snapshot(), size));
                }
            }
        }

        // Commit winning filter
        out.push(best_f);
        out.extend_from_slice(&candidate_data[best_f as usize]);

        if let Some((winner_snap, size)) = best_snap {
            compressor.restore(winner_snap);
            cumulative_output += size;
        }

        // Update adaptive scores: winner gets boosted, all decay by 1
        for score in filter_score.iter_mut() {
            *score = score.saturating_sub(1);
        }
        filter_score[best_f as usize] = win_score;

        prev_row.copy_from_slice(row);
    }
}

/// Beam search filter selection using incremental DEFLATE state.
///
/// Maintains `beam_width` best partial filter sequences instead of greedily
/// committing to one. At each row, expands each beam entry by all 5 filters
/// (K×5 candidates), evaluates via incremental DEFLATE compression, and keeps
/// the top K by cumulative compressed size. The winning entry's filtered data
/// becomes the output.
///
/// Uses [`CompressorSnapshot`] for cheaper state save/restore than full cloning.
///
/// This finds better filter sequences than greedy BruteForceFork at ~K× the
/// cost, because suboptimal choices at row Y can be recovered when they enable
/// better compression at subsequent rows.
#[allow(clippy::too_many_arguments)]
fn filter_image_brute_beam(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    eval_level: u32,
    beam_width: usize,
    cancel: &dyn Stop,
    out: &mut Vec<u8>,
) {
    if height == 0 {
        return;
    }

    let filtered_row_size = row_bytes + 1;
    let compress_bound = Compressor::deflate_compress_bound(filtered_row_size * height);
    let mut compress_buf = vec![0u8; compress_bound];

    struct BeamEntry {
        compressor: Compressor,
        cumulative_size: usize,
        filtered: Vec<u8>,
    }

    let init_compressor = Compressor::new(CompressionLevel::new(eval_level));
    let mut beam = vec![BeamEntry {
        compressor: init_compressor,
        cumulative_size: 0,
        filtered: Vec::with_capacity(filtered_row_size * height),
    }];

    let mut candidate_data: Vec<Vec<u8>> = (0..5).map(|_| vec![0u8; row_bytes]).collect();
    let mut prev_row = vec![0u8; row_bytes];

    // (cumulative_size, beam_idx, filter, snapshot)
    let mut candidates: Vec<(usize, usize, u8, CompressorSnapshot)> =
        Vec::with_capacity(beam_width * 5);

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];
        let is_final = y == height - 1;

        // Compute all 5 filter variants for this row
        for f in 0..5u8 {
            apply_filter(f, row, &prev_row, bpp, &mut candidate_data[f as usize]);
        }

        // Evaluate all beam × filter combinations
        candidates.clear();

        for (b, entry) in beam.iter_mut().enumerate() {
            // Snapshot before trying filters — cheaper than 5 full clones
            let snap = entry.compressor.snapshot();

            for f in 0..5u8 {
                if cancel.check().is_err() {
                    // On cancel, emit best beam entry so far + remaining rows unfiltered
                    let best = beam.into_iter().min_by_key(|e| e.cumulative_size).unwrap();
                    out.extend_from_slice(&best.filtered);
                    for rem_y in y..height {
                        out.push(0);
                        out.extend_from_slice(
                            &packed_rows[rem_y * row_bytes..(rem_y + 1) * row_bytes],
                        );
                    }
                    return;
                }

                // Restore to pre-row state and try this filter
                entry.compressor.restore(snap.clone());

                let start = entry.filtered.len();
                entry.filtered.push(f);
                entry
                    .filtered
                    .extend_from_slice(&candidate_data[f as usize]);

                if let Ok(size) = entry.compressor.deflate_compress_incremental(
                    &entry.filtered,
                    &mut compress_buf,
                    is_final,
                    Unstoppable,
                ) {
                    candidates.push((
                        entry.cumulative_size + size,
                        b,
                        f,
                        entry.compressor.snapshot(),
                    ));
                }

                // Truncate back
                entry.filtered.truncate(start);
            }
        }

        // Sort by cumulative size, keep top beam_width
        candidates.sort_by_key(|(size, ..)| *size);
        candidates.truncate(beam_width);

        // If all candidates failed (compress_buf too small for incremental output),
        // fall back: keep existing beam entries, append filter 0 (None) to each.
        if candidates.is_empty() {
            for entry in &mut beam {
                entry.filtered.push(0);
                entry.filtered.extend_from_slice(&candidate_data[0]);
            }
            prev_row.copy_from_slice(row);
            continue;
        }

        // Build new beam: move parent compressors when possible, clone otherwise,
        // then restore the winning snapshot into each.
        let mut parent_usage = vec![0usize; beam.len()];
        for &(_, b, _, _) in &candidates {
            parent_usage[b] += 1;
        }

        let mut new_beam: Vec<BeamEntry> = Vec::with_capacity(beam_width);
        for (size, b, f, snap) in candidates.drain(..) {
            parent_usage[b] -= 1;
            let (mut filtered, mut compressor) = if parent_usage[b] == 0 {
                // Last use of this parent — move instead of clone
                let entry = &mut beam[b];
                (
                    core::mem::take(&mut entry.filtered),
                    core::mem::replace(
                        &mut entry.compressor,
                        Compressor::new(CompressionLevel::none()),
                    ),
                )
            } else {
                (beam[b].filtered.clone(), beam[b].compressor.clone())
            };
            compressor.restore(snap);
            filtered.push(f);
            filtered.extend_from_slice(&candidate_data[f as usize]);
            new_beam.push(BeamEntry {
                compressor,
                cumulative_size: size,
                filtered,
            });
        }

        beam = new_beam;
        prev_row.copy_from_slice(row);
    }

    // Output best beam entry's filtered data
    let best = beam.into_iter().min_by_key(|e| e.cumulative_size).unwrap();
    *out = best.filtered;
}

/// Pre-compute all 5 filter outputs for every row.
///
/// Layout: flat buffer with `[row0_f0, row0_f1, ..., row0_f4, row1_f0, ...]`.
/// Each entry is `row_bytes` long. Total size: `5 * height * row_bytes`.
pub(crate) fn precompute_all_filters(
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

/// Build filtered output from precomputed filter data.
///
/// Instead of applying filters per-strategy (which duplicates work when
/// multiple adaptive strategies each apply the same 5 filters), this reads
/// from a shared precomputed buffer. Saves 5× filter application per
/// additional adaptive strategy.
///
/// For Single strategies, copies the corresponding filter's precomputed row.
/// For Adaptive strategies, scores all 5 candidates per row and picks the best.
pub(crate) fn filter_image_from_precomputed(
    precomputed: &[u8],
    row_bytes: usize,
    height: usize,
    strategy: Strategy,
    scratch: &mut HeuristicScratch,
    out: &mut Vec<u8>,
) {
    match strategy {
        Strategy::Single(f) => {
            for y in 0..height {
                out.push(f);
                out.extend_from_slice(precomputed_row(precomputed, row_bytes, y, f as usize));
            }
        }
        Strategy::Adaptive(heuristic) => {
            for y in 0..height {
                let base = y * 5 * row_bytes;
                let best_f = score_candidates(
                    |f| {
                        let start = base + f as usize * row_bytes;
                        &precomputed[start..start + row_bytes]
                    },
                    &heuristic,
                    scratch,
                );
                out.push(best_f);
                out.extend_from_slice(precomputed_row(precomputed, row_bytes, y, best_f as usize));
            }
        }
        Strategy::BruteForce { .. }
        | Strategy::BruteForceBlock { .. }
        | Strategy::BruteForceFork { .. }
        | Strategy::BruteForceBeam { .. }
        | Strategy::AdaptiveFork { .. } => {
            unreachable!("brute force strategies not supported with precomputed filters");
        }
    }
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

            if let Ok(len) = eval_compressor.zlib_compress(&eval_buf, &mut compress_buf, cancel)
                && len < best_size
            {
                best_size = len;
                best_combo = combo;
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

/// Reusable scratch buffers for heuristic scoring.
///
/// Hoisted outside the per-row loop to avoid per-row allocation churn.
/// Uses sparse tracking: instead of `fill(0)` on large buffers between
/// calls, we track which entries were touched and reset only those.
///
/// - Bigrams: `touched_words` tracks which u64 words in `bigram_seen` were
///   modified. Reset only those words after scoring (avoids 8KB memset).
/// - BigEnt: `nonzero_keys` tracks which entries in `bigram_counts` are
///   nonzero. Entropy is computed over only those entries, and they're
///   reset during computation (avoids 256KB memset + 65536-entry iteration).
pub(crate) struct HeuristicScratch {
    /// Bigrams: 65536-bit bitset (8KB). Used by Bigrams heuristic.
    bigram_seen: Vec<u64>,
    /// BigEnt: 65536-entry frequency table (256KB). Used by BigEnt heuristic.
    bigram_counts: Vec<u32>,
    /// Sparse tracking for BigEnt: indices of nonzero entries in bigram_counts.
    nonzero_keys: Vec<u16>,
    /// Sparse tracking for Bigrams: indices of modified u64 words in bigram_seen.
    touched_words: Vec<u16>,
}

impl HeuristicScratch {
    fn new(heuristic: AdaptiveHeuristic) -> Self {
        Self {
            bigram_seen: if matches!(heuristic, AdaptiveHeuristic::Bigrams) {
                vec![0u64; 1024]
            } else {
                Vec::new()
            },
            bigram_counts: if matches!(heuristic, AdaptiveHeuristic::BigEnt) {
                vec![0u32; 65536]
            } else {
                Vec::new()
            },
            nonzero_keys: if matches!(heuristic, AdaptiveHeuristic::BigEnt) {
                Vec::with_capacity(8192)
            } else {
                Vec::new()
            },
            touched_words: if matches!(heuristic, AdaptiveHeuristic::Bigrams) {
                Vec::with_capacity(1024)
            } else {
                Vec::new()
            },
        }
    }

    /// Create a scratch that works for all heuristic types.
    ///
    /// Used when screening multiple strategies with shared precomputed
    /// filter data — one scratch serves all adaptive heuristics.
    pub(crate) fn new_universal() -> Self {
        Self {
            bigram_seen: vec![0u64; 1024],
            bigram_counts: vec![0u32; 65536],
            nonzero_keys: Vec::with_capacity(8192),
            touched_words: Vec::with_capacity(1024),
        }
    }
}

fn pick_best_filter(
    candidates: &[Vec<u8>],
    heuristic: AdaptiveHeuristic,
    scratch: &mut HeuristicScratch,
) -> u8 {
    score_candidates(|f| &candidates[f as usize], &heuristic, scratch)
}

/// Score 5 filter candidates and return the best filter index.
///
/// The `get_candidate` closure returns the filtered row data for filter `f`.
/// This is shared between the Vec-based path (filter_image_heuristic) and
/// the precomputed path (filter_image_from_precomputed).
fn score_candidates<'a>(
    get_candidate: impl Fn(u8) -> &'a [u8],
    heuristic: &AdaptiveHeuristic,
    scratch: &mut HeuristicScratch,
) -> u8 {
    match heuristic {
        AdaptiveHeuristic::MinSum => {
            let mut best = 0u8;
            let mut best_score = u64::MAX;
            for f in 0..5u8 {
                let score = sav_score(get_candidate(f));
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
                let score = entropy_score(get_candidate(f));
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
                let score = bigrams_score(
                    get_candidate(f),
                    &mut scratch.bigram_seen,
                    &mut scratch.touched_words,
                );
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
                let score = bigram_entropy_score(
                    get_candidate(f),
                    &mut scratch.bigram_counts,
                    &mut scratch.nonzero_keys,
                );
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

fn bigrams_score(data: &[u8], seen: &mut [u64], touched: &mut Vec<u16>) -> usize {
    if data.len() < 2 {
        return 0;
    }
    // Sparse tracking: instead of seen.fill(0), we track which words were
    // modified and reset only those after scoring. Saves 8KB memset per call.
    touched.clear();
    let mut count = 0usize;
    for pair in data.windows(2) {
        let key = (pair[0] as usize) << 8 | pair[1] as usize;
        let word = key >> 6;
        let bit = 1u64 << (key & 63);
        if seen[word] & bit == 0 {
            if seen[word] == 0 {
                touched.push(word as u16);
            }
            seen[word] |= bit;
            count += 1;
        }
    }
    // Reset only the words we touched
    for &w in touched.iter() {
        seen[w as usize] = 0;
    }
    count
}

/// Shannon entropy of byte-pair bigrams.
///
/// Unlike `bigrams_score` which counts unique bigrams, this computes the
/// actual entropy of the bigram distribution. Better at distinguishing
/// between filtered rows that have similar unique-bigram counts but
/// different frequency distributions.
///
/// Uses sparse tracking: `nonzero` collects indices of entries set during
/// counting. Entropy is computed only over those entries, and they're reset
/// to 0 during the computation. This avoids both the 256KB `fill(0)` and
/// the 65536-entry iteration that made this function 30-170x slower than
/// MinSum.
fn bigram_entropy_score(data: &[u8], counts: &mut [u32], nonzero: &mut Vec<u16>) -> f64 {
    if data.len() < 2 {
        return 0.0;
    }
    nonzero.clear();
    let n = data.len() - 1;
    for pair in data.windows(2) {
        let key = (pair[0] as usize) << 8 | pair[1] as usize;
        if counts[key] == 0 {
            nonzero.push(key as u16);
        }
        counts[key] += 1;
    }
    let len = n as f64;
    let mut entropy = 0.0f64;
    for &key in nonzero.iter() {
        let c = counts[key as usize];
        let p = c as f64 / len;
        entropy -= p * p.log2();
        counts[key as usize] = 0; // Reset as we go — no fill(0) needed
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple test image: `height` rows × `row_bytes` bytes.
    /// Uses a gradient pattern so filters produce different outputs.
    fn make_test_image(width: usize, height: usize, bpp: usize) -> (Vec<u8>, usize) {
        let row_bytes = width * bpp;
        let mut data = vec![0u8; row_bytes * height];
        for y in 0..height {
            for x in 0..row_bytes {
                data[y * row_bytes + x] = ((x * 7 + y * 13) % 256) as u8;
            }
        }
        (data, row_bytes)
    }

    // ---- apply_filter tests ----

    #[test]
    fn apply_filter_none() {
        let row = [10, 20, 30, 40];
        let prev = [0, 0, 0, 0];
        let mut out = [0u8; 4];
        apply_filter(0, &row, &prev, 3, &mut out);
        assert_eq!(out, row);
    }

    #[test]
    fn apply_filter_sub() {
        let row = [10, 20, 30, 40, 50, 60];
        let prev = [0u8; 6];
        let mut out = [0u8; 6];
        apply_filter(1, &row, &prev, 3, &mut out);
        // First 3 bytes: same as input
        assert_eq!(out[..3], [10, 20, 30]);
        // Remaining: row[i] - row[i-3]
        assert_eq!(out[3], 40u8.wrapping_sub(10));
        assert_eq!(out[4], 50u8.wrapping_sub(20));
        assert_eq!(out[5], 60u8.wrapping_sub(30));
    }

    #[test]
    fn apply_filter_up() {
        let row = [10, 20, 30];
        let prev = [5, 10, 15];
        let mut out = [0u8; 3];
        apply_filter(2, &row, &prev, 3, &mut out);
        assert_eq!(out, [5, 10, 15]);
    }

    #[test]
    fn apply_filter_avg() {
        let row = [10, 20, 30, 40];
        let prev = [0, 0, 0, 0];
        let mut out = [0u8; 4];
        apply_filter(3, &row, &prev, 3, &mut out);
        // First bpp bytes: row[i] - prev[i]/2
        assert_eq!(out[0], 10);
        assert_eq!(out[1], 20);
        assert_eq!(out[2], 30);
        // After bpp: row[i] - floor((row[i-bpp] + prev[i]) / 2)
        assert_eq!(out[3], 40u8.wrapping_sub((10u16 >> 1) as u8));
    }

    #[test]
    fn apply_filter_paeth() {
        let row = [10, 20, 30, 40];
        let prev = [0, 0, 0, 0];
        let mut out = [0u8; 4];
        apply_filter(4, &row, &prev, 3, &mut out);
        // First bpp bytes: paeth(0, prev[i], 0)
        assert_eq!(out[0], 10u8.wrapping_sub(paeth_predictor(0, 0, 0)));
        assert_eq!(out[1], 20u8.wrapping_sub(paeth_predictor(0, 0, 0)));
        // After bpp: paeth(row[i-bpp], prev[i], prev[i-bpp])
        assert_eq!(out[3], 40u8.wrapping_sub(paeth_predictor(10, 0, 0)));
    }

    #[test]
    fn apply_filter_unknown_is_none() {
        let row = [10, 20, 30];
        let prev = [0u8; 3];
        let mut out = [0u8; 3];
        apply_filter(255, &row, &prev, 3, &mut out);
        assert_eq!(out, row);
    }

    // ---- Heuristic scoring tests ----

    #[test]
    fn sav_score_all_zeros() {
        assert_eq!(sav_score(&[0, 0, 0]), 0);
    }

    #[test]
    fn sav_score_mixed() {
        // Values <= 128 contribute value; > 128 contribute (256 - value)
        let score = sav_score(&[1, 255, 128, 129]);
        // 1 + 1 + 128 + 127 = 257
        assert_eq!(score, 257);
    }

    #[test]
    fn entropy_score_empty() {
        assert_eq!(entropy_score(&[]), 0.0);
    }

    #[test]
    fn entropy_score_uniform() {
        // All same byte → 0 entropy
        assert_eq!(entropy_score(&[42, 42, 42, 42]), 0.0);
    }

    #[test]
    fn entropy_score_two_symbols() {
        // Equal frequency of two symbols → 1.0 bit
        let e = entropy_score(&[0, 1, 0, 1]);
        assert!((e - 1.0).abs() < 0.01, "expected ~1.0, got {e}");
    }

    #[test]
    fn bigrams_score_short() {
        let mut seen = vec![0u64; 1024];
        let mut touched = Vec::new();
        assert_eq!(bigrams_score(&[42], &mut seen, &mut touched), 0);
    }

    #[test]
    fn bigrams_score_unique_pairs() {
        let mut seen = vec![0u64; 1024];
        let mut touched = Vec::new();
        // [0,1,2,3] has 3 unique bigrams: (0,1), (1,2), (2,3)
        let score = bigrams_score(&[0, 1, 2, 3], &mut seen, &mut touched);
        assert_eq!(score, 3);
        // Verify reset worked
        assert!(seen.iter().all(|&w| w == 0));
    }

    #[test]
    fn bigrams_score_repeated() {
        let mut seen = vec![0u64; 1024];
        let mut touched = Vec::new();
        // [0,1,0,1] has 2 unique bigrams: (0,1), (1,0)
        let score = bigrams_score(&[0, 1, 0, 1], &mut seen, &mut touched);
        assert_eq!(score, 2);
    }

    #[test]
    fn bigram_entropy_short() {
        let mut counts = vec![0u32; 65536];
        let mut nonzero = Vec::new();
        assert_eq!(bigram_entropy_score(&[42], &mut counts, &mut nonzero), 0.0);
    }

    #[test]
    fn bigram_entropy_resets_counts() {
        let mut counts = vec![0u32; 65536];
        let mut nonzero = Vec::new();
        bigram_entropy_score(&[0, 1, 2, 3], &mut counts, &mut nonzero);
        // All counts should be reset to 0
        assert!(counts.iter().all(|&c| c == 0));
    }

    // ---- Strategy: Single ----

    #[test]
    fn filter_image_single_none() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::Single(0),
            &Unstoppable,
            &mut out,
        );
        // Each row: 1 filter byte (0) + row_bytes data
        assert_eq!(out.len(), 3 * (1 + row_bytes));
        for y in 0..3 {
            let offset = y * (1 + row_bytes);
            assert_eq!(out[offset], 0); // filter byte
            // Filter 0 (None) → data is unchanged
            assert_eq!(
                &out[offset + 1..offset + 1 + row_bytes],
                &data[y * row_bytes..(y + 1) * row_bytes]
            );
        }
    }

    #[test]
    fn filter_image_single_paeth() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::Single(4),
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
        for y in 0..3 {
            assert_eq!(out[y * (1 + row_bytes)], 4);
        }
    }

    // ---- Strategy: Adaptive ----

    #[test]
    fn filter_image_adaptive_minsum() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::Adaptive(AdaptiveHeuristic::MinSum),
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
        // Each row's first byte should be a valid filter (0-4)
        for y in 0..3 {
            assert!(out[y * (1 + row_bytes)] <= 4);
        }
    }

    #[test]
    fn filter_image_adaptive_entropy() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::Adaptive(AdaptiveHeuristic::Entropy),
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
    }

    #[test]
    fn filter_image_adaptive_bigrams() {
        let (data, row_bytes) = make_test_image(8, 4, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            4,
            3,
            Strategy::Adaptive(AdaptiveHeuristic::Bigrams),
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 4 * (1 + row_bytes));
    }

    #[test]
    fn filter_image_adaptive_bigent() {
        let (data, row_bytes) = make_test_image(8, 4, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            4,
            3,
            Strategy::Adaptive(AdaptiveHeuristic::BigEnt),
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 4 * (1 + row_bytes));
    }

    // ---- Strategy: BruteForce ----

    #[test]
    fn filter_image_bruteforce() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::BruteForce {
                context_rows: 2,
                eval_level: 1,
            },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
        for y in 0..3 {
            assert!(out[y * (1 + row_bytes)] <= 4);
        }
    }

    // ---- Strategy: BruteForceBlock ----

    #[test]
    fn filter_image_blockwise_basic() {
        let (data, row_bytes) = make_test_image(4, 4, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            4,
            3,
            Strategy::BruteForceBlock {
                context_rows: 2,
                eval_level: 1,
            },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 4 * (1 + row_bytes));
        for y in 0..4 {
            assert!(out[y * (1 + row_bytes)] <= 4);
        }
    }

    #[test]
    fn filter_image_blockwise_zero_height() {
        let mut out = Vec::new();
        filter_image(
            &[],
            12,
            0,
            3,
            Strategy::BruteForceBlock {
                context_rows: 2,
                eval_level: 1,
            },
            &Unstoppable,
            &mut out,
        );
        assert!(out.is_empty());
    }

    // ---- Strategy: BruteForceFork ----

    #[test]
    fn filter_image_brute_fork_basic() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::BruteForceFork { eval_level: 1 },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
        for y in 0..3 {
            assert!(out[y * (1 + row_bytes)] <= 4);
        }
    }

    // ---- Strategy: AdaptiveFork ----

    #[test]
    fn filter_image_adaptive_fork_basic() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::AdaptiveFork {
                eval_level: 1,
                narrow_to: 2,
            },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
        for y in 0..3 {
            assert!(out[y * (1 + row_bytes)] <= 4);
        }
    }

    #[test]
    fn filter_image_adaptive_fork_narrow_to_1() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::AdaptiveFork {
                eval_level: 1,
                narrow_to: 1,
            },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
    }

    #[test]
    fn filter_image_adaptive_fork_many_rows() {
        // Test adaptive score decay + periodic reactivation (every 16 rows)
        let (data, row_bytes) = make_test_image(4, 32, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            32,
            3,
            Strategy::AdaptiveFork {
                eval_level: 1,
                narrow_to: 2,
            },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 32 * (1 + row_bytes));
    }

    // ---- Strategy: BruteForceBeam ----

    #[test]
    fn filter_image_beam_basic() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::BruteForceBeam {
                eval_level: 1,
                beam_width: 2,
            },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
        for y in 0..3 {
            assert!(out[y * (1 + row_bytes)] <= 4);
        }
    }

    #[test]
    fn filter_image_beam_zero_height() {
        let mut out = Vec::new();
        filter_image(
            &[],
            12,
            0,
            3,
            Strategy::BruteForceBeam {
                eval_level: 1,
                beam_width: 2,
            },
            &Unstoppable,
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn filter_image_beam_width_1() {
        // beam_width=1 should behave like BruteForceFork
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let mut out = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::BruteForceBeam {
                eval_level: 1,
                beam_width: 1,
            },
            &Unstoppable,
            &mut out,
        );
        assert_eq!(out.len(), 3 * (1 + row_bytes));
    }

    // ---- precompute_all_filters ----

    #[test]
    fn precompute_all_filters_basic() {
        let (data, row_bytes) = make_test_image(4, 2, 3);
        let pc = precompute_all_filters(&data, row_bytes, 2, 3);
        assert_eq!(pc.len(), 5 * 2 * row_bytes);

        // Filter 0 (None) should be identical to raw row
        for y in 0..2 {
            let row = &data[y * row_bytes..(y + 1) * row_bytes];
            let filtered = precomputed_row(&pc, row_bytes, y, 0);
            assert_eq!(filtered, row);
        }
    }

    // ---- filter_image_from_precomputed ----

    #[test]
    fn from_precomputed_single() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let pc = precompute_all_filters(&data, row_bytes, 3, 3);

        // Compare direct filter_image vs from_precomputed for Single(4)
        let mut direct = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::Single(4),
            &Unstoppable,
            &mut direct,
        );

        let mut precomp = Vec::new();
        let mut scratch = HeuristicScratch::new_universal();
        filter_image_from_precomputed(
            &pc,
            row_bytes,
            3,
            Strategy::Single(4),
            &mut scratch,
            &mut precomp,
        );

        assert_eq!(direct, precomp);
    }

    #[test]
    fn from_precomputed_adaptive() {
        let (data, row_bytes) = make_test_image(4, 3, 3);
        let pc = precompute_all_filters(&data, row_bytes, 3, 3);

        let mut direct = Vec::new();
        filter_image(
            &data,
            row_bytes,
            3,
            3,
            Strategy::Adaptive(AdaptiveHeuristic::MinSum),
            &Unstoppable,
            &mut direct,
        );

        let mut precomp = Vec::new();
        let mut scratch = HeuristicScratch::new_universal();
        filter_image_from_precomputed(
            &pc,
            row_bytes,
            3,
            Strategy::Adaptive(AdaptiveHeuristic::MinSum),
            &mut scratch,
            &mut precomp,
        );

        assert_eq!(direct, precomp);
    }

    // ---- learn_block_size ----

    #[test]
    fn learn_block_size_small() {
        // Small images (<=64 rows): budget 50K
        // B=5: 3125 * ceil(10/5) = 6250 ≤ 50K → should get 5
        assert!(learn_block_size(10) >= 2);
        assert!(learn_block_size(10) <= 5);
    }

    #[test]
    fn learn_block_size_large() {
        // Large: budget 200K
        // B=2: 25 * ceil(1000/2) = 12500 ≤ 200K → fits
        let b = learn_block_size(1000);
        assert!(b >= 2);
        assert!(b <= 5);
    }

    // ---- HeuristicScratch ----

    #[test]
    fn scratch_new_bigrams() {
        let s = HeuristicScratch::new(AdaptiveHeuristic::Bigrams);
        assert_eq!(s.bigram_seen.len(), 1024);
        assert!(s.bigram_counts.is_empty());
    }

    #[test]
    fn scratch_new_bigent() {
        let s = HeuristicScratch::new(AdaptiveHeuristic::BigEnt);
        assert!(s.bigram_seen.is_empty());
        assert_eq!(s.bigram_counts.len(), 65536);
    }

    #[test]
    fn scratch_new_minsum() {
        let s = HeuristicScratch::new(AdaptiveHeuristic::MinSum);
        assert!(s.bigram_seen.is_empty());
        assert!(s.bigram_counts.is_empty());
    }

    #[test]
    fn scratch_universal() {
        let s = HeuristicScratch::new_universal();
        assert_eq!(s.bigram_seen.len(), 1024);
        assert_eq!(s.bigram_counts.len(), 65536);
    }

    // ---- paeth_predictor ----

    #[test]
    fn paeth_predictor_basic() {
        // When all inputs are the same, p = a+b-c = a, so pa=0 → returns a
        assert_eq!(paeth_predictor(10, 10, 10), 10);
    }

    #[test]
    fn paeth_predictor_zero() {
        assert_eq!(paeth_predictor(0, 0, 0), 0);
    }

    #[test]
    fn paeth_predictor_left_wins() {
        // a=100, b=0, c=0: p=100, pa=0, pb=100, pc=100 → returns a=100
        assert_eq!(paeth_predictor(100, 0, 0), 100);
    }

    #[test]
    fn paeth_predictor_above_wins() {
        // a=0, b=100, c=0: p=100, pa=100, pb=0, pc=100 → returns b=100
        assert_eq!(paeth_predictor(0, 100, 0), 100);
    }

    // ---- Strategy constants ----

    #[test]
    fn strategy_constants_sizes() {
        assert_eq!(HEURISTIC_STRATEGIES.len(), 9);
        assert_eq!(FAST_STRATEGIES.len(), 5);
        assert_eq!(MINIMAL_STRATEGIES.len(), 3);
    }

    // ---- bpp=4 (RGBA) coverage ----

    #[test]
    fn filter_image_bpp4() {
        let (data, row_bytes) = make_test_image(4, 3, 4);
        for strategy in [
            Strategy::Single(0),
            Strategy::Single(4),
            Strategy::Adaptive(AdaptiveHeuristic::MinSum),
            Strategy::BruteForce {
                context_rows: 1,
                eval_level: 1,
            },
            Strategy::BruteForceFork { eval_level: 1 },
        ] {
            let mut out = Vec::new();
            filter_image(&data, row_bytes, 3, 4, strategy, &Unstoppable, &mut out);
            assert_eq!(out.len(), 3 * (1 + row_bytes));
        }
    }
}
