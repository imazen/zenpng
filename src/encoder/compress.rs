//! Progressive compression engine with multi-strategy filter selection.

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zenflate::{CompressionLevel, Compressor, Unstoppable};

use crate::error::PngError;

use super::filter::{HEURISTIC_STRATEGIES, Strategy, filter_image};
use super::{PhaseStat, PhaseStats};

/// Try compressing `filtered` data with all `compressors`, updating `best_compressed`
/// if a smaller result is found.
pub(crate) fn try_compress(
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
/// Only for Crush with the `zopfli` feature enabled.
///
/// Each phase checks the deadline before starting.
pub(crate) fn compress_filtered(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    compression_level: u8,
    opts: super::CompressOptions<'_>,
    mut stats: Option<&mut PhaseStats>,
) -> Result<Vec<u8>, PngError> {
    use std::time::Instant;

    let filtered_size = (row_bytes + 1) * height;
    let mut best_compressed: Option<Vec<u8>> = None;

    if let Some(s) = &mut stats {
        s.raw_size = filtered_size;
    }

    // Reusable buffers
    let mut filtered = Vec::with_capacity(filtered_size);
    let compress_bound = Compressor::zlib_compress_bound(filtered_size);
    let mut compress_buf = vec![0u8; compress_bound];
    let mut verify_buf = vec![0u8; filtered_size];

    // Internal zenflate level: levels 13-14 use L12 internally.
    let zenflate_level = compression_level.min(12);

    // For L0-1, screening IS the final compression — no Phase 2 needed.
    // For L2+, screen at L1 then refine the best candidates at the target level.
    // Level 14 (Maniac) screens at L6 for more accurate strategy ranking.
    let screen_level = match compression_level {
        0..=1 => compression_level as u32,
        14 => 6,
        _ => 1,
    };

    // Refinement tiers: which compression levels to try in Phase 2.
    // Deadline is checked between tiers for graceful early return.
    let refine_tiers: &[u32] = if zenflate_level >= 11 {
        &[10, 11, 12]
    } else {
        // Return a static slice for the target level. For levels 2-9
        // this is a single entry matching the requested compression.
        match zenflate_level {
            2 => &[2],
            3 => &[3],
            4 => &[4],
            5 => &[5],
            6 => &[6],
            7 => &[7],
            8 => &[8],
            9 => &[9],
            10 => &[10],
            _ => &[1], // L0-1 won't reach here (needs_refine is false)
        }
    };

    let needs_refine = compression_level >= 2;

    let can_brute_force = compression_level >= 12;

    // Brute-force configs: (context_rows, eval_level)
    //
    // Corpus analysis (gb82-sc 10 images, CID22-512, strategy_explorer) showed:
    // - On photographic images, BF saves <0.1% over best heuristic at L12
    // - On screenshots, BF saves 5-7% but heuristics at L6-L10 are "good enough"
    // - Context > 5 has diminishing/negative returns vs context=5
    // - eval=4 vs eval=1: only +0.02-0.06% for 1.5-2x filter time
    //
    // BF is only enabled at Best (L12) and above. At lower levels, the zenflate
    // compressor level overwhelms filter selection quality — a mediocre filter
    // at L12 beats a perfect filter at L9.
    //
    // Level 14 (Maniac) sweeps all context/eval combos exhaustively.
    let brute_configs: &[(usize, u32)] = match compression_level {
        14 => &[
            (1, 1),
            (1, 4),
            (3, 1),
            (3, 4),
            (5, 1),
            (5, 4),
            (8, 1),
            (8, 4),
        ],
        12 => &[(5, 1), (5, 4)],
        _ => &[],
    };

    // Block-wise brute-force: DISABLED.
    // Corpus analysis showed block brute is both slower AND larger than
    // per-row brute force (-0.03 to -0.06% vs per-row at same level).
    let block_brute_configs: &[(usize, u32)] = &[];

    // ---- Phase 1: Screen all heuristic strategies ----
    // Use a cheap compressor to rank strategies by compressed size.
    let phase_start = if stats.is_some() {
        Some(Instant::now())
    } else {
        None
    };
    let mut screen_compressor = Compressor::new(CompressionLevel::new(screen_level));
    // (screen_size, filtered_data) — sorted later to pick top candidates
    let mut screen_results: Vec<(usize, Vec<u8>)> = Vec::with_capacity(HEURISTIC_STRATEGIES.len());

    for (i, strategy) in HEURISTIC_STRATEGIES.iter().enumerate() {
        // Always try at least one strategy (even with zero budget),
        // but check budget before subsequent strategies.
        if i > 0 && opts.deadline.should_stop() {
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

    if let (Some(s), Some(t)) = (&mut stats, phase_start) {
        let tried = screen_results.len();
        s.phases.push(PhaseStat {
            name: alloc::format!("Screen ({tried}×L{screen_level})"),
            duration_ns: t.elapsed().as_nanos() as u64,
            best_size: best_compressed.as_ref().map_or(0, |b| b.len()),
            evaluations: tried as u32,
        });
    }

    // Early return: L0-1 don't need refinement, or deadline hit
    if !needs_refine || opts.deadline.should_stop() {
        return best_compressed
            .ok_or_else(|| PngError::InvalidInput("no filter strategies tried".to_string()));
    }

    // ---- Phase 2: Refine top 3 at target level(s) ----
    //
    // Iterate tier-by-tier so we can deadline-check between tiers.
    // Iterates tier-by-tier with deadline checks between tiers.
    let phase2_start = if stats.is_some() {
        Some(Instant::now())
    } else {
        None
    };
    let top_n = screen_results.len().min(3);

    // Track the best zenflate size per candidate for zopfli ranking later
    #[cfg(feature = "zopfli")]
    let mut zopfli_candidates: Vec<(usize, Vec<u8>)> = Vec::new();

    for &tier_level in refine_tiers {
        if opts.deadline.should_stop() {
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

    if let (Some(s), Some(t)) = (&mut stats, phase2_start) {
        let tiers_str = refine_tiers
            .iter()
            .map(|l| alloc::format!("L{l}"))
            .collect::<Vec<_>>()
            .join(",");
        s.phases.push(PhaseStat {
            name: alloc::format!("Refine ({top_n}×{tiers_str})"),
            duration_ns: t.elapsed().as_nanos() as u64,
            best_size: best_compressed.as_ref().map_or(0, |b| b.len()),
            evaluations: (top_n * refine_tiers.len()) as u32,
        });
    }

    // ---- Phase 3: Brute-force ----
    // Brute-force filtering is expensive (~3-4s per config), so compress at
    // the highest tiers only. We already have lower-tier results from Phase 2.
    let phase3_start = if stats.is_some() && can_brute_force {
        Some(Instant::now())
    } else {
        None
    };
    let mut brute_evals = 0u32;
    if can_brute_force && !opts.deadline.should_stop() {
        let brute_levels: &[u32] = if zenflate_level >= 11 {
            &[10, 11, 12]
        } else {
            &[zenflate_level as u32]
        };
        let mut brute_compressors: Vec<Compressor> = brute_levels
            .iter()
            .map(|&l| Compressor::new(CompressionLevel::new(l)))
            .collect();

        for &(context_rows, eval_level) in brute_configs {
            if opts.deadline.should_stop() {
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
            brute_evals += 1;

            #[cfg(feature = "zopfli")]
            if opts.use_zopfli && _best_size < usize::MAX {
                zopfli_candidates.push((_best_size, filtered.clone()));
            }
        }

        // Block-wise brute-force: runs after per-row so it has that result
        // as a baseline to beat.
        for &(context_rows, eval_level) in block_brute_configs {
            if opts.deadline.should_stop() {
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
            brute_evals += 1;

            #[cfg(feature = "zopfli")]
            if opts.use_zopfli && _best_size < usize::MAX {
                zopfli_candidates.push((_best_size, filtered.clone()));
            }
        }
    }

    if let (Some(s), Some(t)) = (&mut stats, phase3_start) {
        if brute_evals > 0 {
            let configs_desc = brute_configs
                .iter()
                .map(|(ctx, ev)| alloc::format!("ctx{ctx}/L{ev}"))
                .chain(
                    block_brute_configs
                        .iter()
                        .map(|(ctx, ev)| alloc::format!("blk-ctx{ctx}/L{ev}")),
                )
                .collect::<Vec<_>>()
                .join(",");
            s.phases.push(PhaseStat {
                name: alloc::format!("BruteForce ({configs_desc})"),
                duration_ns: t.elapsed().as_nanos() as u64,
                best_size: best_compressed.as_ref().map_or(0, |b| b.len()),
                evaluations: brute_evals,
            });
        }
    }

    // ---- Phase 4: Zopfli ----
    #[cfg(feature = "zopfli")]
    {
        let phase4_start = if stats.is_some() {
            Some(Instant::now())
        } else {
            None
        };
        if opts.use_zopfli && !zopfli_candidates.is_empty() && !opts.deadline.should_stop() {
            // Sort by zenflate size, take top 3
            zopfli_candidates.sort_by_key(|(size, _)| *size);
            zopfli_candidates.truncate(3);

            let n_candidates = zopfli_candidates.len();
            let best = zopfli_adaptive(
                &zopfli_candidates,
                opts.cancel,
                opts.deadline,
                opts.remaining_ns,
                &mut best_compressed,
            )?;
            if let Some(b) = best {
                best_compressed = Some(b);
            }

            if let (Some(s), Some(t)) = (&mut stats, phase4_start) {
                s.phases.push(PhaseStat {
                    name: alloc::format!("Zopfli ({n_candidates} candidates)"),
                    duration_ns: t.elapsed().as_nanos() as u64,
                    best_size: best_compressed.as_ref().map_or(0, |b| b.len()),
                    evaluations: n_candidates as u32,
                });
            }
        }
    }

    best_compressed.ok_or_else(|| PngError::InvalidInput("no filter strategies tried".to_string()))
}

/// Adaptive zopfli compression with time budgeting.
///
/// Strategy:
/// 1. Calibrate: compress top candidate with 5 iterations, measure wall time.
/// 2. From calibration, estimate iterations that fit in remaining time.
/// 3. If we can afford more iterations, run them in parallel on top candidates.
/// 4. Always keep the best result found at any stage.
#[cfg(feature = "zopfli")]
fn zopfli_adaptive(
    candidates: &[(usize, Vec<u8>)],
    cancel: &dyn Stop,
    deadline: &dyn Stop,
    remaining_ns: Option<&dyn Fn() -> Option<u64>>,
    current_best: &mut Option<Vec<u8>>,
) -> Result<Option<Vec<u8>>, PngError> {
    use std::time::Instant;

    // Combine cancel + deadline for zenzop — when deadline expires, zenzop
    // gracefully returns best-so-far instead of erroring.
    let combined = almost_enough::OrStop::new(cancel, deadline);

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
    let cal_result = compress_with_zopfli_n(&candidates[0].1, calibration_iters, &combined)?;
    let cal_elapsed = cal_start.elapsed();
    update_best(cal_result);

    // Estimate time per iteration from calibration.
    let ms_per_iter = cal_elapsed.as_secs_f64() * 1000.0 / calibration_iters as f64;

    // Phase 2: Determine max affordable iterations.
    // Calibration gives us a target, but the combined stop provides a hard backstop —
    // if calibration overestimates, zenzop will gracefully stop when deadline expires.
    let max_iters = match remaining_ns.and_then(|f| f()) {
        Some(ns) => {
            let remaining_ms = ns as f64 / 1_000_000.0;
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
        }
        None => 50u64,
    };

    if max_iters <= calibration_iters {
        return Ok(best);
    }

    // Phase 3: Run top candidates in parallel with calculated iterations.
    // All threads share the combined stop — deadline expiry gracefully stops
    // all threads, cancellation hard-aborts them.
    let zopfli_results: Vec<Result<Vec<u8>, PngError>> = std::thread::scope(|s| {
        let handles: Vec<_> = candidates
            .iter()
            .map(|(_size, data)| s.spawn(|| compress_with_zopfli_n(data, max_iters, &combined)))
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

#[cfg(all(test, feature = "zopfli"))]
mod zopfli_tests {
    use super::*;
    use core::sync::atomic::{AtomicI64, Ordering};

    /// Deadline stop that fires `TimedOut` after a fixed number of `check()` calls.
    /// Deterministic — no wall-clock dependency.
    struct CallCountDeadline(AtomicI64);

    impl CallCountDeadline {
        fn new(calls: i64) -> Self {
            Self(AtomicI64::new(calls))
        }
    }

    impl Stop for CallCountDeadline {
        fn check(&self) -> Result<(), enough::StopReason> {
            let prev = self.0.fetch_sub(1, Ordering::Relaxed);
            if prev <= 0 {
                Err(enough::StopReason::TimedOut)
            } else {
                Ok(())
            }
        }
    }

    /// Remaining-time callback that reports 1 second remaining for N calls,
    /// then 0 (expired). Deterministic.
    struct CallCountRemainingNs(AtomicI64);

    impl CallCountRemainingNs {
        fn new(calls: i64) -> Self {
            Self(AtomicI64::new(calls))
        }

        fn as_fn(&self) -> impl Fn() -> Option<u64> + '_ {
            move || {
                let prev = self.0.fetch_sub(1, Ordering::Relaxed);
                if prev <= 0 {
                    Some(0)
                } else {
                    Some(1_000_000_000)
                }
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
    fn zopfli_deadline_expiry_returns_valid_output() {
        // OrStop with a deadline that expires after 2 check() calls.
        // Zenzop should gracefully stop and return best-so-far.
        let data = test_data();
        let cancel = enough::Unstoppable;
        let deadline = CallCountDeadline::new(2);
        let stop = almost_enough::OrStop::new(&cancel, &deadline);
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
    fn or_stop_cancel_takes_priority() {
        // Both cancel and deadline would fire — cancel should win (checked first).
        let cancel = CallCountCancel::new(0); // fires immediately
        let deadline = CallCountDeadline::new(0); // also exhausted
        let stop = almost_enough::OrStop::new(&cancel, &deadline);
        let result = stop.check();
        assert!(matches!(result, Err(enough::StopReason::Cancelled)));
    }

    #[test]
    fn or_stop_deadline_fires_timed_out() {
        // Cancel is fine but deadline is exhausted — should get TimedOut.
        let cancel = enough::Unstoppable;
        let deadline = CallCountDeadline::new(0); // exhausted
        let stop = almost_enough::OrStop::new(&cancel, &deadline);
        let result = stop.check();
        assert!(matches!(result, Err(enough::StopReason::TimedOut)));
    }

    #[test]
    fn or_stop_neither_fires() {
        // Both cancel and deadline are fine — should get Ok.
        let cancel = enough::Unstoppable;
        let deadline = CallCountDeadline::new(100);
        let stop = almost_enough::OrStop::new(&cancel, &deadline);
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
        let deadline = enough::Unstoppable;
        let mut current_best = None;

        let result =
            zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best).unwrap();

        assert!(result.is_some(), "should find a result");
        verify_zlib(result.as_ref().unwrap(), &data);
    }

    #[test]
    fn zopfli_adaptive_deadline_expiry_returns_valid() {
        // Give enough calls for calibration then expire during Phase 3.
        let data = test_data();
        let compressed_size = {
            let c = compress_with_zopfli_n(&data, 5, &enough::Unstoppable).unwrap();
            c.len()
        };
        let candidates = vec![(compressed_size, data.clone())];
        let cancel = enough::Unstoppable;
        // Deadline expires after a few checks — zopfli_adaptive should
        // gracefully return with at least the calibration result.
        let deadline = CallCountDeadline::new(10);
        let remaining = CallCountRemainingNs::new(10);
        let remaining_fn = remaining.as_fn();
        let mut current_best = None;

        let result = zopfli_adaptive(
            &candidates,
            &cancel,
            &deadline,
            Some(&remaining_fn),
            &mut current_best,
        )
        .unwrap();

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
        let deadline = enough::Unstoppable;
        let mut current_best = None;

        let result = zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best);
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
    /// the zenflate baseline passed as current_best.
    #[test]
    fn zopfli_adaptive_never_regresses_vs_zenflate() {
        let data = test_data();
        let zenflate_result = zenflate_baseline(&data, 12);
        let zenflate_size = zenflate_result.len();
        let candidates = vec![(zenflate_size, data.clone())];

        // Test with no deadline (unlimited)
        let cancel = enough::Unstoppable;
        let deadline = enough::Unstoppable;
        let mut current_best = Some(zenflate_result.clone());

        let result =
            zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best).unwrap();

        if let Some(ref better) = result {
            assert!(
                better.len() < zenflate_size,
                "zopfli ({}) must be strictly smaller than zenflate ({zenflate_size})",
                better.len(),
            );
            verify_zlib(better, &data);
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

        let cancel = enough::Unstoppable;
        let deadline = enough::Unstoppable;
        let mut current_best = Some(zenflate_best.clone());

        let result =
            zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best).unwrap();

        if let Some(ref better) = result {
            assert!(
                better.len() < zenflate_best_size,
                "zopfli ({}) must be strictly smaller than zenflate ({zenflate_best_size})",
                better.len(),
            );
            // Verify it decompresses to one of the candidate patterns.
            let decompressed =
                miniz_oxide::inflate::decompress_to_vec_zlib(better).expect("invalid zlib");
            assert!(
                patterns.iter().any(|p| *p == decompressed),
                "decompressed data doesn't match any candidate",
            );
        }
    }
}
