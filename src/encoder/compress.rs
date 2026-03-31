//! Progressive compression engine with multi-strategy filter selection.

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use enough::Stop;
use zenflate::{CompressionLevel, Compressor};

use crate::error::PngError;
#[allow(unused_imports)]
use whereat::at;

use super::filter::{
    FAST_STRATEGIES, HEURISTIC_STRATEGIES, HeuristicScratch, MINIMAL_STRATEGIES, Strategy,
    filter_image, filter_image_from_precomputed, precompute_all_filters,
};
use super::{PhaseStat, PhaseStats};

/// Parameters derived from a single effort value (0-200).
///
/// Each field controls one axis of the compression pipeline:
/// - `zenflate_effort`: final zenflate effort for Phase 2 / screen-only
/// - `strategies`: which filter strategies to screen in Phase 1
/// - `screen_effort`: cheap zenflate effort for Phase 1 screening
/// - `screen_is_final`: if true, Phase 1 IS the final pass (no Phase 2)
/// - `top_k`: how many candidates advance from Phase 1 to Phase 2
/// - `refine_efforts`: zenflate efforts to try in Phase 2
/// - `brute_configs`: (context_rows, eval_effort) for Phase 3 brute-force
/// - `fork_brute_efforts`: eval efforts for forking brute-force
/// - `use_recompress`: whether to run Phase 4 (recompression with zenflate best + optional zopfli)
///
/// Monotonicity (higher effort never produces larger output) is enforced by
/// `zenflate::CompressionLevel::monotonicity_fallback()`, which the compression
/// helpers follow automatically. Screen effort stays at FastHt (≤9) to avoid
/// cross-strategy ranking divergence.
struct EffortParams {
    zenflate_effort: u32,
    strategies: &'static [Strategy],
    screen_effort: u32,
    screen_is_final: bool,
    top_k: usize,
    refine_efforts: &'static [u32],
    brute_configs: &'static [(usize, u32)],
    block_brute_configs: &'static [(usize, u32)],
    fork_brute_efforts: &'static [u32],
    adaptive_fork_configs: &'static [(u32, usize)], // (eval_level, narrow_to)
    beam_brute_configs: &'static [(u32, usize)],    // (eval_level, beam_width)
    use_recompress: bool,
    /// If set, Phase 4 also runs zenflate FullOptimal at this effort level.
    /// Effort 31+ maps to (effort-16) FullOptimal iterations.
    full_optimal_effort: Option<u32>,
    /// If true, Phase 4 runs ONLY FullOptimal (skips NearOptimal and zenzop).
    /// Used for E31-45 lean tier where FullOptimal is the sole recompressor.
    full_optimal_only: bool,
}

impl EffortParams {
    /// Map effort (0-30) to pipeline parameters.
    ///
    /// Monotonicity is enforced by `CompressionLevel::monotonicity_fallback()`:
    /// each refine/brute compression automatically follows the fallback chain,
    /// trying each previous strategy boundary's max effort. Screen effort stays
    /// at FastHt (≤9) for consistent candidate ranking.
    ///
    /// The `bpp` parameter enables content-aware optimization: for indexed images
    /// (bpp=1), brute-force filter evaluation is enabled at lower effort thresholds
    /// because filter selection has outsized impact on indexed/graphic content.
    fn from_effort_and_bpp(effort: u32, bpp: usize) -> Self {
        let mut params = Self::from_effort(effort);

        // Content-aware adjustment: narrow-row images (low bpp) benefit heavily
        // from brute-force filter selection while the overhead is small since
        // rows have fewer bytes per pixel.
        //
        // bpp=1 (indexed/gray8): 30%+ improvement for graphic content, cheap rows
        // bpp=2 (gray+alpha/gray16): similar pattern, still relatively narrow
        if params.brute_configs.is_empty()
            && ((bpp == 1 && (16..24).contains(&effort))
                || (bpp == 2 && (20..24).contains(&effort)))
        {
            params.brute_configs = &[(5, 1)];
        }

        params
    }

    fn from_effort(effort: u32) -> Self {
        if effort > 60 {
            // Effort 61+: Full Maniac pipeline + FullOptimal.
            // FullOptimal iterations = effort - 16 (e.g., effort 76 = 60i).
            return Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[30],
                brute_configs: &[
                    (1, 1),
                    (1, 4),
                    (3, 1),
                    (3, 4),
                    (5, 1),
                    (5, 4),
                    (8, 1),
                    (8, 4),
                ],
                block_brute_configs: &[(5, 1), (5, 4)],
                fork_brute_efforts: &[10, 15],
                adaptive_fork_configs: &[(15, 2), (22, 2)],
                beam_brute_configs: &[(10, 3), (15, 3)],
                use_recompress: true,
                full_optimal_effort: Some(effort),
                full_optimal_only: false,
            };
        }
        if effort > 45 {
            // Effort 46-60: 9 strategies + refine + moderate brute-force + FullOptimal.
            return Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[(1, 1), (1, 4), (3, 1), (3, 4)],
                block_brute_configs: &[],
                fork_brute_efforts: &[10, 15],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: true,
                full_optimal_effort: Some(effort),
                full_optimal_only: false,
            };
        }
        if effort > 30 {
            // Effort 31-45: Full pipeline + FullOptimal recompression.
            //
            // Includes the complete e30 pipeline (screening, refinement,
            // brute-force, BFF, beam) to guarantee monotonicity vs e30.
            // FullOptimal recompression adds iterative forward DP on top.
            //
            // Previously used a lean pipeline (BFF only + FullOptimal) which
            // regressed on 49% of images because BFF's Greedy-eval filter
            // selection produced suboptimal candidates for FullOptimal.
            // The full pipeline captures both screening-based and BFF-based
            // candidates, letting FullOptimal pick the best.
            //
            // FullOptimal iterations = effort - 16 (E31->15i, E36->20i, E45->29i).
            return Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[
                    (1, 1),
                    (1, 4),
                    (3, 1),
                    (3, 4),
                    (5, 1),
                    (5, 4),
                    (8, 1),
                    (8, 4),
                ],
                block_brute_configs: &[],
                fork_brute_efforts: &[10, 15],
                adaptive_fork_configs: &[(15, 2), (22, 2)],
                beam_brute_configs: &[(10, 3), (15, 3)],
                use_recompress: true,
                full_optimal_effort: Some(effort),
                full_optimal_only: false,
            };
        }
        match effort {
            // ── Low effort (0-7): screen IS final pass ──
            //
            // e0=Store, e1-4=Turbo, e5-9=FastHt.
            // Turbo→FastHt always improves (zenflate guarantee), no fallback needed.
            0 => Self {
                zenflate_effort: 0,
                strategies: &[Strategy::Single(0)],
                screen_effort: 0,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            1 => Self {
                zenflate_effort: 1,
                strategies: &[Strategy::Single(4)],
                screen_effort: 1,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            2 => Self {
                zenflate_effort: 2,
                strategies: MINIMAL_STRATEGIES,
                screen_effort: 2,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            3 => Self {
                zenflate_effort: 3,
                strategies: FAST_STRATEGIES,
                screen_effort: 3,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            4 => Self {
                zenflate_effort: 4,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 4,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            5 => Self {
                zenflate_effort: 5,
                strategies: FAST_STRATEGIES,
                screen_effort: 5,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            6 => Self {
                zenflate_effort: 6,
                strategies: FAST_STRATEGIES,
                screen_effort: 6,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            7 => Self {
                zenflate_effort: 7,
                strategies: FAST_STRATEGIES,
                screen_effort: 7,
                screen_is_final: true,
                top_k: 1,
                refine_efforts: &[],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            // ── Medium effort (8-15): screen + refine ──
            //
            // Screen at FastHt e7. Refine at target efforts; monotonicity
            // fallback chain (via zenflate) automatically tries previous
            // strategy boundaries (e.g., e12 → e10 → e9).
            8 => Self {
                zenflate_effort: 8,
                strategies: FAST_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[8],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            9 => Self {
                zenflate_effort: 10,
                strategies: FAST_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[10],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            10 => Self {
                zenflate_effort: 12,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[12],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            11 => Self {
                zenflate_effort: 14,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[14],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            12 => Self {
                zenflate_effort: 15,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[15],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            13 => Self {
                zenflate_effort: 17,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[17],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            14 => Self {
                zenflate_effort: 18,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[18],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            15 => Self {
                zenflate_effort: 20,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[20],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            // ── High effort (16-23): higher refine, multi-tier ──
            //
            // Screen at FastHt e7. Refine at target efforts; fallback chain
            // handles cross-strategy monotonicity automatically.
            16 => Self {
                zenflate_effort: 22,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[20, 22],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            17 => Self {
                zenflate_effort: 22,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[20, 22],
                brute_configs: &[(3, 1)],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            18 => Self {
                zenflate_effort: 24,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[22, 24],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            19 => Self {
                zenflate_effort: 24,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[22, 24],
                brute_configs: &[(3, 1)],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            20 => Self {
                zenflate_effort: 26,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[24, 26],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            21 => Self {
                zenflate_effort: 28,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[26, 28],
                brute_configs: &[],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            22 => Self {
                zenflate_effort: 28,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[26, 28],
                brute_configs: &[(5, 1)],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            23 => Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[(5, 1)],
                block_brute_configs: &[],
                fork_brute_efforts: &[],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            // ── Max effort (24-30): brute-force + zopfli ──
            24 => Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[(5, 1)],
                block_brute_configs: &[],
                fork_brute_efforts: &[10],
                adaptive_fork_configs: &[],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            25 => Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[(5, 1)],
                block_brute_configs: &[],
                fork_brute_efforts: &[10],
                adaptive_fork_configs: &[(15, 2)],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            26 => Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[(5, 1), (5, 4)],
                block_brute_configs: &[],
                fork_brute_efforts: &[10],
                adaptive_fork_configs: &[(15, 2)],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            27 => Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[(5, 1), (5, 4)],
                block_brute_configs: &[],
                fork_brute_efforts: &[10, 15],
                adaptive_fork_configs: &[(15, 2), (22, 2)],
                beam_brute_configs: &[],
                use_recompress: false,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            28 => Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[
                    (1, 1),
                    (1, 4),
                    (3, 1),
                    (3, 4),
                    (5, 1),
                    (5, 4),
                    (8, 1),
                    (8, 4),
                ],
                block_brute_configs: &[(5, 1)],
                fork_brute_efforts: &[10, 15],
                adaptive_fork_configs: &[(15, 2), (22, 2)],
                beam_brute_configs: &[],
                use_recompress: true,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            29 => Self {
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[
                    (1, 1),
                    (1, 4),
                    (3, 1),
                    (3, 4),
                    (5, 1),
                    (5, 4),
                    (8, 1),
                    (8, 4),
                ],
                block_brute_configs: &[(5, 1)],
                fork_brute_efforts: &[10, 15],
                adaptive_fork_configs: &[(15, 2), (22, 2)],
                beam_brute_configs: &[(10, 3)],
                use_recompress: true,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
            _ => Self {
                // effort 30
                zenflate_effort: 30,
                strategies: HEURISTIC_STRATEGIES,
                screen_effort: 7,
                screen_is_final: false,
                top_k: 3,
                refine_efforts: &[28, 30],
                brute_configs: &[
                    (1, 1),
                    (1, 4),
                    (3, 1),
                    (3, 4),
                    (5, 1),
                    (5, 4),
                    (8, 1),
                    (8, 4),
                ],
                block_brute_configs: &[(5, 1), (5, 4)],
                fork_brute_efforts: &[10, 15],
                adaptive_fork_configs: &[(15, 2), (22, 2)],
                beam_brute_configs: &[(10, 3), (15, 3)],
                use_recompress: true,
                full_optimal_effort: None,
                full_optimal_only: false,
            },
        }
    }
}

/// Try compressing `filtered` data with all `compressors`, updating `best_compressed`
/// if a smaller result is found.
pub(crate) fn try_compress(
    filtered: &[u8],
    compressors: &mut [Compressor],
    compress_buf: &mut [u8],
    verify_buf: &mut [u8],
    best_compressed: &mut Option<Vec<u8>>,
    cancel: &dyn Stop,
) -> crate::error::Result<usize> {
    let mut best_for_stream = usize::MAX;
    for compressor in compressors.iter_mut() {
        let compressed_len = match compressor.zlib_compress(filtered, compress_buf, cancel) {
            Ok(len) => len,
            Err(zenflate::CompressionError::Stopped(reason)) => {
                return Err(at!(PngError::from(reason)));
            }
            Err(e) => {
                return Err(at!(PngError::InvalidInput(alloc::format!(
                    "compression failed: {e}"
                ))));
            }
        };

        // Verify decompression roundtrip
        {
            let mut decompressor = zenflate::Decompressor::new();
            if decompressor
                .zlib_decompress(&compress_buf[..compressed_len], verify_buf, cancel)
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

/// Compress `filtered` data at the given effort, then follow zenflate's
/// monotonicity fallback chain (trying each previous strategy boundary's
/// max effort). Updates `best_compressed` if a smaller result is found.
fn try_compress_with_fallbacks(
    filtered: &[u8],
    effort: u32,
    compress_buf: &mut [u8],
    verify_buf: &mut [u8],
    best_compressed: &mut Option<Vec<u8>>,
    cancel: &dyn Stop,
) -> crate::error::Result<usize> {
    let mut best_size = usize::MAX;
    let mut level = CompressionLevel::new(effort);
    loop {
        let mut compressor = Compressor::new(level);
        let size = try_compress(
            filtered,
            core::slice::from_mut(&mut compressor),
            compress_buf,
            verify_buf,
            best_compressed,
            cancel,
        )?;
        best_size = best_size.min(size);
        match level.monotonicity_fallback() {
            Some(fb) => level = fb,
            None => break,
        }
    }
    Ok(best_size)
}

/// Progressive adaptive compression engine.
///
/// Instead of a flat loop over all strategies × all levels, works in phases:
///
/// **Phase 1 (Screen):** Try filter strategies with a cheap compressor to rank
/// them. Cost: ~1ms per strategy. This gets us a valid result immediately.
///
/// **Phase 2 (Refine):** Compress the top-K filtered streams at higher zenflate
/// effort(s). This is where 90%+ of final quality comes from.
///
/// **Phase 3 (Brute-force):** Per-row brute-force filter selection with DEFLATE
/// context evaluation. Expensive (~3-4s per config on 1024×1024) but can beat
/// heuristics on complex images.
///
/// **Phase 4 (Recompress):** Re-compress best candidates with zenflate effort 30 +
///   PNG cost biases, at effort >= 28. Optionally also tries zopfli (with `zopfli` feature).
///
/// Each phase checks the deadline before starting.
pub(crate) fn compress_filtered(
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
    bpp: usize,
    effort: u32,
    opts: super::CompressOptions<'_>,
    mut stats: Option<&mut PhaseStats>,
) -> crate::error::Result<Vec<u8>> {
    use std::time::Instant;

    let params = EffortParams::from_effort_and_bpp(effort, bpp);
    let filtered_size = (row_bytes + 1) * height;

    // ---- Effort 0 fast path: write zlib stored blocks directly ----
    // Bypasses the entire screening/compress/verify pipeline. No Compressor,
    // no Decompressor, no intermediate buffers. Just memcpy rows with filter
    // bytes into stored DEFLATE blocks with zlib header + Adler-32.
    if effort == 0 {
        if let Some(s) = &mut stats {
            s.raw_size = filtered_size;
        }
        let phase_start = if stats.is_some() {
            Some(Instant::now())
        } else {
            None
        };

        let result = zlib_store_unfiltered(packed_rows, row_bytes, height);

        if let (Some(s), Some(t)) = (&mut stats, phase_start) {
            s.phases.push(PhaseStat {
                name: "Store (effort 0)".to_string(),
                duration_ns: t.elapsed().as_nanos() as u64,
                best_size: result.len(),
                evaluations: 1,
            });
        }
        return Ok(result);
    }

    // Zero RGB channels of fully-transparent pixels (alpha == 0).
    // Invisible pixels with arbitrary RGB values create noise that defeats
    // PNG filtering and DEFLATE compression. Zeroing them produces runs of
    // identical bytes that compress significantly better.
    // Only for RGBA8 (bpp=4) where byte 3 of each pixel is alpha.
    let owned_rows;
    let packed_rows = if bpp == 4 && has_any_transparent_pixel(packed_rows) {
        owned_rows = zero_transparent_rgba8(packed_rows);
        &owned_rows
    } else {
        packed_rows
    };

    let mut best_compressed: Option<Vec<u8>> = None;

    if let Some(s) = &mut stats {
        s.raw_size = filtered_size;
    }

    // Reusable buffers
    let mut filtered = Vec::with_capacity(filtered_size);
    let compress_bound = Compressor::zlib_compress_bound(filtered_size);
    let mut compress_buf = vec![0u8; compress_bound];
    let mut verify_buf = vec![0u8; filtered_size];

    let strategies = params.strategies;

    let phase_start = if stats.is_some() {
        Some(Instant::now())
    } else {
        None
    };

    // (screen_size, filtered_data) — sorted later to pick top candidates
    let mut screen_results: Vec<(usize, Vec<u8>)> = Vec::with_capacity(strategies.len());

    let screen_effort = params.screen_effort;

    // Precompute all 5 filter variants once, shared across strategies.
    // This avoids redundant filter application: e.g. HEURISTIC_STRATEGIES has
    // 4 Adaptive strategies that each independently apply 5 filters → 20 passes.
    // With precomputation: 5 passes total, then each strategy just scores.
    // Cap at 64 MiB to avoid excessive memory on very large images.
    let precompute_size = 5 * height * row_bytes;
    let precomputed = if strategies.len() > 1 && precompute_size <= 64 * 1024 * 1024 {
        Some(precompute_all_filters(packed_rows, row_bytes, height, bpp))
    } else {
        None
    };

    if opts.parallel {
        // ── Parallel screening ──
        // Each thread gets its own filtered buffer, compressor, compress buffer,
        // verify buffer, and scratch. The precomputed filter data is shared
        // immutably across all threads.
        let precomputed_ref = precomputed.as_deref();
        #[allow(clippy::type_complexity)]
        let par_results: Vec<Option<(usize, Vec<u8>, Vec<u8>)>> = std::thread::scope(|s| {
            let handles: Vec<_> = strategies
                .iter()
                .map(|strategy| {
                    s.spawn(move || {
                        let mut t_filtered = Vec::with_capacity(filtered_size);
                        let mut t_compressor =
                            Compressor::new(CompressionLevel::new(screen_effort));
                        let mut t_compress_buf = vec![0u8; compress_bound];
                        let mut t_verify_buf = vec![0u8; filtered_size];

                        if let Some(pc) = precomputed_ref {
                            let mut t_scratch = HeuristicScratch::new_universal();
                            filter_image_from_precomputed(
                                pc,
                                row_bytes,
                                height,
                                *strategy,
                                &mut t_scratch,
                                &mut t_filtered,
                            );
                        } else {
                            filter_image(
                                packed_rows,
                                row_bytes,
                                height,
                                bpp,
                                *strategy,
                                opts.cancel,
                                &mut t_filtered,
                            );
                        }

                        let compressed_len = t_compressor
                            .zlib_compress(&t_filtered, &mut t_compress_buf, opts.cancel)
                            .ok()?;

                        // Verify decompression roundtrip
                        let mut decompressor = zenflate::Decompressor::new();
                        decompressor
                            .zlib_decompress(
                                &t_compress_buf[..compressed_len],
                                &mut t_verify_buf,
                                opts.cancel,
                            )
                            .ok()?;

                        Some((
                            compressed_len,
                            t_filtered,
                            t_compress_buf[..compressed_len].to_vec(),
                        ))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        for (compressed_len, filtered_data, compressed_data) in par_results.into_iter().flatten() {
            let dominated = best_compressed
                .as_ref()
                .is_some_and(|b| compressed_len >= b.len());
            if !dominated {
                best_compressed = Some(compressed_data);
            }
            screen_results.push((compressed_len, filtered_data));
        }
    } else {
        // ── Serial screening ──
        let mut screen_compressor = Compressor::new(CompressionLevel::new(screen_effort));
        let mut scratch = HeuristicScratch::new_universal();

        for (i, strategy) in strategies.iter().enumerate() {
            // Always try at least one strategy (even with zero budget),
            // but check budget before subsequent strategies.
            if i > 0 && opts.deadline.should_stop() {
                break;
            }

            filtered.clear();
            if let Some(ref pc) = precomputed {
                filter_image_from_precomputed(
                    pc,
                    row_bytes,
                    height,
                    *strategy,
                    &mut scratch,
                    &mut filtered,
                );
            } else {
                filter_image(
                    packed_rows,
                    row_bytes,
                    height,
                    bpp,
                    *strategy,
                    opts.cancel,
                    &mut filtered,
                );
            }

            let compressed_len =
                match screen_compressor.zlib_compress(&filtered, &mut compress_buf, opts.cancel) {
                    Ok(len) => len,
                    Err(zenflate::CompressionError::Stopped(reason)) => {
                        return Err(at!(PngError::from(reason)));
                    }
                    Err(e) => {
                        return Err(at!(PngError::InvalidInput(alloc::format!(
                            "compression failed: {e}"
                        ))));
                    }
                };

            // Verify decompression roundtrip
            let valid = {
                let mut decompressor = zenflate::Decompressor::new();
                decompressor
                    .zlib_decompress(
                        &compress_buf[..compressed_len],
                        &mut verify_buf,
                        opts.cancel,
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
    }

    // Sort by screen size — best first
    screen_results.sort_by_key(|(size, _)| *size);

    // Adaptive effort allocation: measure filter strategy variance.
    // If all strategies produce nearly identical sizes, filter choice barely
    // matters (common for photographic content) and brute-force Phase 3
    // would waste time without improving compression.
    let filter_variance_low = if screen_results.len() >= 3 {
        let best = screen_results[0].0;
        let worst = screen_results[screen_results.len() - 1].0;
        // <1% spread across all strategies → filter choice is unimportant
        best > 0 && (worst as f64 / best as f64) < 1.01
    } else {
        false
    };

    if let (Some(s), Some(t)) = (&mut stats, phase_start) {
        let tried = screen_results.len();
        s.phases.push(PhaseStat {
            name: alloc::format!("Screen ({tried}×E{screen_effort})"),
            duration_ns: t.elapsed().as_nanos() as u64,
            best_size: best_compressed.as_ref().map_or(0, |b| b.len()),
            evaluations: tried as u32,
        });
    }

    // Early return: screen-only modes don't need refinement, or deadline hit
    if params.screen_is_final || opts.deadline.should_stop() {
        return best_compressed.ok_or_else(|| {
            at!(PngError::InvalidInput(
                "no filter strategies tried".to_string()
            ))
        });
    }

    // ---- Phase 2: Refine top-K at target effort(s) ----
    let refine_tiers = params.refine_efforts;
    let phase2_start = if stats.is_some() {
        Some(Instant::now())
    } else {
        None
    };
    let top_n = screen_results.len().min(params.top_k);

    // Track the best zenflate size per candidate for Phase 4 recompression
    let mut recompress_candidates: Vec<(usize, Vec<u8>)> = Vec::new();

    // If no refine tiers but recompress is requested, pass screen results
    // directly to Phase 4. The screen compressed sizes are used for ranking.
    if refine_tiers.is_empty() && params.use_recompress {
        for (size, filtered_data) in &screen_results[..top_n] {
            recompress_candidates.push((*size, filtered_data.clone()));
        }
    }

    if opts.parallel && top_n > 1 {
        // ── Parallel refinement ──
        // Each candidate runs through all refine tiers in its own thread.
        // Each thread returns Option<(best_size, compressed_data, filtered_data_ref_index)>.
        let refine_results: Vec<Option<(usize, Vec<u8>)>> = std::thread::scope(|s| {
            let handles: Vec<_> = screen_results[..top_n]
                .iter()
                .map(|(_, filtered_data)| {
                    s.spawn(move || {
                        let mut t_compress_buf = vec![0u8; compress_bound];
                        let mut t_verify_buf = vec![0u8; filtered_size];
                        let mut t_best: Option<Vec<u8>> = None;

                        for &tier_level in refine_tiers {
                            // Follow zenflate's monotonicity fallback chain:
                            // compress at target, then at each previous strategy
                            // boundary's max effort, keeping the smallest.
                            let mut level = CompressionLevel::new(tier_level);
                            loop {
                                let mut compressor = Compressor::new(level);
                                if let Ok(len) = compressor.zlib_compress(
                                    filtered_data,
                                    &mut t_compress_buf,
                                    opts.cancel,
                                ) {
                                    let mut decompressor = zenflate::Decompressor::new();
                                    if decompressor
                                        .zlib_decompress(
                                            &t_compress_buf[..len],
                                            &mut t_verify_buf,
                                            opts.cancel,
                                        )
                                        .is_ok()
                                    {
                                        let dominated =
                                            t_best.as_ref().is_some_and(|b| len >= b.len());
                                        if !dominated {
                                            t_best = Some(t_compress_buf[..len].to_vec());
                                        }
                                    }
                                }
                                match level.monotonicity_fallback() {
                                    Some(fb) => level = fb,
                                    None => break,
                                }
                            }
                        }
                        t_best.map(|b| (b.len(), b))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        #[allow(unused_variables)]
        for (idx, result) in refine_results.into_iter().enumerate() {
            if let Some((size, compressed_data)) = result {
                let dominated = best_compressed.as_ref().is_some_and(|b| size >= b.len());
                if !dominated {
                    best_compressed = Some(compressed_data);
                }
                if params.use_recompress {
                    recompress_candidates.push((size, screen_results[idx].1.clone()));
                }
            }
        }
    } else {
        // ── Serial refinement ──
        // Iterate tier-by-tier with deadline checks between tiers.
        // Each tier follows zenflate's monotonicity fallback chain.
        for &tier_level in refine_tiers {
            if opts.deadline.should_stop() {
                break;
            }

            for (_, filtered_data) in &screen_results[..top_n] {
                let refine_size = try_compress_with_fallbacks(
                    filtered_data,
                    tier_level,
                    &mut compress_buf,
                    &mut verify_buf,
                    &mut best_compressed,
                    opts.cancel,
                )?;

                if params.use_recompress && refine_size < usize::MAX {
                    recompress_candidates.push((refine_size, filtered_data.clone()));
                }
            }
        }
    }

    if let (Some(s), Some(t)) = (&mut stats, phase2_start) {
        let tiers_str = refine_tiers
            .iter()
            .map(|l| alloc::format!("E{l}"))
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
    // the highest effort only. We already have lower-effort results from Phase 2.
    let brute_configs = params.brute_configs;
    let block_brute_configs = params.block_brute_configs;
    let fork_brute_levels = params.fork_brute_efforts;
    let beam_brute_configs = params.beam_brute_configs;
    let adaptive_fork_configs = params.adaptive_fork_configs;
    let can_brute_force = !brute_configs.is_empty()
        || !block_brute_configs.is_empty()
        || !fork_brute_levels.is_empty()
        || !beam_brute_configs.is_empty()
        || !adaptive_fork_configs.is_empty();

    let phase3_start = if stats.is_some() && can_brute_force {
        Some(Instant::now())
    } else {
        None
    };
    let mut brute_evals = 0u32;
    if can_brute_force && !opts.deadline.should_stop() && !filter_variance_low {
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

            let brute_size = try_compress_with_fallbacks(
                &filtered,
                params.zenflate_effort,
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;
            brute_evals += 1;

            if params.use_recompress && brute_size < usize::MAX {
                recompress_candidates.push((brute_size, filtered.clone()));
            }
        }

        // Block-wise brute-force: exhaustive search within multi-row blocks.
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

            let block_brute_size = try_compress_with_fallbacks(
                &filtered,
                params.zenflate_effort,
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;
            brute_evals += 1;

            if params.use_recompress && block_brute_size < usize::MAX {
                recompress_candidates.push((block_brute_size, filtered.clone()));
            }
        }

        // Forking brute-force: uses real DEFLATE state for filter evaluation
        // instead of a limited raw context window.
        for &eval_level in fork_brute_levels {
            if opts.deadline.should_stop() {
                break;
            }

            filtered.clear();
            filter_image(
                packed_rows,
                row_bytes,
                height,
                bpp,
                Strategy::BruteForceFork { eval_level },
                opts.cancel,
                &mut filtered,
            );

            let fork_brute_size = try_compress_with_fallbacks(
                &filtered,
                params.zenflate_effort,
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;
            brute_evals += 1;

            if params.use_recompress && fork_brute_size < usize::MAX {
                recompress_candidates.push((fork_brute_size, filtered.clone()));
            }
        }

        // Beam search: maintains K best partial filter sequences across rows.
        for &(eval_level, beam_width) in beam_brute_configs {
            if opts.deadline.should_stop() {
                break;
            }

            filtered.clear();
            filter_image(
                packed_rows,
                row_bytes,
                height,
                bpp,
                Strategy::BruteForceBeam {
                    eval_level,
                    beam_width,
                },
                opts.cancel,
                &mut filtered,
            );

            let beam_brute_size = try_compress_with_fallbacks(
                &filtered,
                params.zenflate_effort,
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;
            brute_evals += 1;

            if params.use_recompress && beam_brute_size < usize::MAX {
                recompress_candidates.push((beam_brute_size, filtered.clone()));
            }
        }

        // Adaptive narrowing fork: estimates all 5 filters cheaply, then fully
        // compresses only the top `narrow_to` candidates per row.
        for &(eval_level, narrow_to) in adaptive_fork_configs {
            if opts.deadline.should_stop() {
                break;
            }

            filtered.clear();
            filter_image(
                packed_rows,
                row_bytes,
                height,
                bpp,
                Strategy::AdaptiveFork {
                    eval_level,
                    narrow_to,
                },
                opts.cancel,
                &mut filtered,
            );

            let adaptive_size = try_compress_with_fallbacks(
                &filtered,
                params.zenflate_effort,
                &mut compress_buf,
                &mut verify_buf,
                &mut best_compressed,
                opts.cancel,
            )?;
            brute_evals += 1;

            if params.use_recompress && adaptive_size < usize::MAX {
                recompress_candidates.push((adaptive_size, filtered.clone()));
            }
        }
    }

    if let (Some(s), Some(t)) = (&mut stats, phase3_start)
        && brute_evals > 0
    {
        let configs_desc = brute_configs
            .iter()
            .map(|(ctx, ev)| alloc::format!("ctx{ctx}/E{ev}"))
            .chain(
                block_brute_configs
                    .iter()
                    .map(|(ctx, ev)| alloc::format!("blk-ctx{ctx}/E{ev}")),
            )
            .chain(
                fork_brute_levels
                    .iter()
                    .map(|l| alloc::format!("fork-E{l}")),
            )
            .chain(
                beam_brute_configs
                    .iter()
                    .map(|(ev, k)| alloc::format!("beam-E{ev}/K{k}")),
            )
            .chain(
                adaptive_fork_configs
                    .iter()
                    .map(|(ev, n)| alloc::format!("afork-E{ev}/N{n}")),
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

    // ---- Phase 4: Recompression ----
    // Re-compress top candidates with zenflate effort 30 (NearOptimal).
    // At effort 31+, also runs zenflate FullOptimal (Zopfli-style forward DP).
    // Optionally also tries zenzop (zopfli) if the `zopfli` feature is enabled.
    {
        let phase4_start = if stats.is_some() {
            Some(Instant::now())
        } else {
            None
        };
        if params.use_recompress
            && !recompress_candidates.is_empty()
            && !opts.deadline.should_stop()
        {
            // Sort by compression size, take top candidates.
            // Lean tier (full_optimal_only): 1 candidate for speed.
            // Normal tiers: up to 3 candidates.
            recompress_candidates.sort_by_key(|(size, _)| *size);
            if params.full_optimal_only {
                recompress_candidates.truncate(1);
            } else {
                recompress_candidates.truncate(3);
            }

            let n_candidates = recompress_candidates.len();

            if params.full_optimal_only {
                // Lean tier (E31-45): direct zenzop on top candidate.
                // zenzop enhanced is ~2.5x faster than zenflate FullOptimal
                // at equal iteration counts with equal or better compression.
                // Falls back to FullOptimal when zopfli feature isn't available.
                #[cfg(feature = "zopfli")]
                {
                    let iterations =
                        params.full_optimal_effort.unwrap_or(31).saturating_sub(16) as u64;
                    for (_screen_size, filtered_data) in &recompress_candidates {
                        if opts.cancel.should_stop() {
                            break;
                        }
                        let zopfli_result =
                            compress_with_zopfli_n(filtered_data, iterations.max(1), opts.cancel)?;
                        let dominated = best_compressed
                            .as_ref()
                            .is_some_and(|b| zopfli_result.len() >= b.len());
                        if !dominated {
                            best_compressed = Some(zopfli_result);
                        }
                    }
                }
                #[cfg(not(feature = "zopfli"))]
                {
                    if let Some(fo_effort) = params.full_optimal_effort {
                        let fo_best = zenflate_full_optimal_recompress(
                            &recompress_candidates,
                            fo_effort,
                            opts.cancel,
                            &mut best_compressed,
                            opts.max_threads,
                        )?;
                        if let Some(b) = fo_best {
                            best_compressed = Some(b);
                        }
                    }
                }
            } else {
                // Normal tier: NearOptimal + optional FullOptimal + optional zenzop
                // Zenflate NearOptimal recompression: effort 30 (always available)
                let zenflate_best = zenflate_recompress(
                    &recompress_candidates,
                    opts.cancel,
                    &mut best_compressed,
                    opts.max_threads,
                )?;
                if let Some(b) = zenflate_best {
                    best_compressed = Some(b);
                }

                // Zenflate FullOptimal recompression: effort 31+ (iterative forward DP)
                if let Some(fo_effort) = params.full_optimal_effort
                    && !opts.deadline.should_stop()
                {
                    let fo_best = zenflate_full_optimal_recompress(
                        &recompress_candidates,
                        fo_effort,
                        opts.cancel,
                        &mut best_compressed,
                        opts.max_threads,
                    )?;
                    if let Some(b) = fo_best {
                        best_compressed = Some(b);
                    }
                }

                // Optional zenzop (zopfli) recompression if feature is enabled
                #[cfg(feature = "zopfli")]
                {
                    if !opts.deadline.should_stop() {
                        let zopfli_best = zopfli_adaptive(
                            &recompress_candidates,
                            opts.cancel,
                            opts.deadline,
                            opts.remaining_ns,
                            &mut best_compressed,
                            opts.max_threads,
                        )?;
                        if let Some(b) = zopfli_best {
                            best_compressed = Some(b);
                        }
                    }
                }
            }

            if let (Some(s), Some(t)) = (&mut stats, phase4_start) {
                let mut label_parts = Vec::new();
                if params.full_optimal_only {
                    #[cfg(feature = "zopfli")]
                    label_parts.push("Zenzop".to_string());
                    #[cfg(not(feature = "zopfli"))]
                    label_parts.push("FullOpt".to_string());
                } else {
                    label_parts.push("NearOpt".to_string());
                    if params.full_optimal_effort.is_some() {
                        label_parts.push("FullOpt".to_string());
                    }
                    if cfg!(feature = "zopfli") {
                        label_parts.push("Zopfli".to_string());
                    }
                }
                let label = alloc::format!(
                    "Recompress[{}] ({n_candidates} candidates)",
                    label_parts.join("+")
                );
                s.phases.push(PhaseStat {
                    name: label,
                    duration_ns: t.elapsed().as_nanos() as u64,
                    best_size: best_compressed.as_ref().map_or(0, |b| b.len()),
                    evaluations: n_candidates as u32,
                });
            }
        }
    }

    best_compressed.ok_or_else(|| {
        at!(PngError::InvalidInput(
            "no filter strategies tried".to_string()
        ))
    })
}

/// Check if any RGBA8 pixel has alpha == 0.
///
/// Quick scan to avoid copying the entire image when there are no transparent
/// pixels (common for photos). Checks every 4th byte starting at offset 3.
fn has_any_transparent_pixel(data: &[u8]) -> bool {
    data.chunks_exact(4).any(|px| px[3] == 0)
}

/// Copy pixel data, zeroing RGB channels of fully-transparent pixels.
///
/// For each 4-byte RGBA8 pixel where alpha (byte 3) is 0, sets R, G, B
/// (bytes 0-2) to 0. This creates runs of [0,0,0,0] that PNG filters
/// and DEFLATE compress much better than random RGB + zero alpha.
fn zero_transparent_rgba8(data: &[u8]) -> Vec<u8> {
    let mut buf = data.to_vec();
    for px in buf.chunks_exact_mut(4) {
        if px[3] == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
        }
    }
    buf
}

/// Write zlib stored blocks directly from raw pixel rows, applying filter None.
///
/// Bypasses the entire Compressor/Decompressor pipeline for L0. Writes:
/// - 2-byte zlib header (CMF=0x78, FLG=0x01)
/// - Stored DEFLATE blocks containing [0x00 filter_byte, row_data] per row
/// - 4-byte Adler-32 checksum (big-endian)
///
/// Each stored block holds as many complete rows as fit in 65535 bytes.
/// Single rows exceeding 65535 bytes get their own block(s).
fn zlib_store_unfiltered(packed_rows: &[u8], row_bytes: usize, height: usize) -> Vec<u8> {
    let filtered_row = row_bytes + 1; // filter byte + row data
    let total_filtered = filtered_row * height;

    // Single-pass: write stored DEFLATE blocks directly from source rows,
    // computing Adler-32 incrementally per row. Each row feeds a single
    // adler32 call on (filter_byte ++ row_data) by using Adler32Hasher.
    let num_blocks = if total_filtered == 0 {
        1
    } else {
        total_filtered.div_ceil(65535)
    };
    let out_size = 2 + 5 * num_blocks + total_filtered + 4;
    let mut out = Vec::with_capacity(out_size);

    // zlib header: CM=8 (deflate), CINFO=7 (32K window), FCHECK
    out.push(0x78);
    out.push(0x01);

    if height == 0 {
        write_stored_block_header(&mut out, 0, true);
        out.extend_from_slice(&zenflate::adler32(1, &[]).to_be_bytes());
        return out;
    }

    let mut adler = 1u32;
    let mut block_remaining: usize = 0;
    let mut filtered_remaining = total_filtered;

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];

        // Write filter byte (0x00 = None) into the current stored block
        if block_remaining == 0 {
            let block_len = filtered_remaining.min(65535);
            let is_final = block_len >= filtered_remaining;
            write_stored_block_header(&mut out, block_len, is_final);
            block_remaining = block_len;
        }
        out.push(0u8);
        block_remaining -= 1;
        filtered_remaining -= 1;

        // Write row data, splitting across stored blocks as needed
        let mut data = row;
        while !data.is_empty() {
            if block_remaining == 0 {
                let block_len = filtered_remaining.min(65535);
                let is_final = block_len >= filtered_remaining;
                write_stored_block_header(&mut out, block_len, is_final);
                block_remaining = block_len;
            }
            let n = data.len().min(block_remaining);
            out.extend_from_slice(&data[..n]);
            data = &data[n..];
            block_remaining -= n;
            filtered_remaining -= n;
        }

        // Adler-32: feed filter byte (0x00) then row data.
        // For 0x00: s1 unchanged, s2 += s1, both mod 65521.
        let s1 = adler & 0xFFFF;
        let s2 = ((adler >> 16) + s1) % 65521;
        adler = (s2 << 16) | s1;
        adler = zenflate::adler32(adler, row);
    }

    out.extend_from_slice(&adler.to_be_bytes());
    out
}

/// Write zlib-wrapped stored DEFLATE blocks directly into the output Vec.
///
/// Used by the L0 fast path to write IDAT data directly into the PNG output,
/// avoiding a separate allocation. The caller handles the IDAT chunk framing
/// (length, type, CRC).
pub(crate) fn write_zlib_stored_inline(
    out: &mut Vec<u8>,
    packed_rows: &[u8],
    row_bytes: usize,
    height: usize,
) {
    let filtered_row = row_bytes + 1;
    let total_filtered = filtered_row * height;

    // zlib header
    out.push(0x78);
    out.push(0x01);

    if height == 0 {
        write_stored_block_header(out, 0, true);
        out.extend_from_slice(&zenflate::adler32(1, &[]).to_be_bytes());
        return;
    }

    let mut adler = 1u32;
    let mut block_remaining: usize = 0;
    let mut filtered_remaining = total_filtered;

    for y in 0..height {
        let row = &packed_rows[y * row_bytes..(y + 1) * row_bytes];

        // Write filter byte (0x00 = None)
        if block_remaining == 0 {
            let block_len = filtered_remaining.min(65535);
            let is_final = block_len >= filtered_remaining;
            write_stored_block_header(out, block_len, is_final);
            block_remaining = block_len;
        }
        out.push(0u8);
        block_remaining -= 1;
        filtered_remaining -= 1;

        // Write row data, splitting across stored blocks as needed
        let mut data = row;
        while !data.is_empty() {
            if block_remaining == 0 {
                let block_len = filtered_remaining.min(65535);
                let is_final = block_len >= filtered_remaining;
                write_stored_block_header(out, block_len, is_final);
                block_remaining = block_len;
            }
            let n = data.len().min(block_remaining);
            out.extend_from_slice(&data[..n]);
            data = &data[n..];
            block_remaining -= n;
            filtered_remaining -= n;
        }

        // Adler-32: 0x00 filter byte then row data
        let s1 = adler & 0xFFFF;
        let s2 = ((adler >> 16) + s1) % 65521;
        adler = (s2 << 16) | s1;
        adler = zenflate::adler32(adler, row);
    }

    out.extend_from_slice(&adler.to_be_bytes());
}

/// Write a stored DEFLATE block header (5 bytes).
fn write_stored_block_header(out: &mut Vec<u8>, len: usize, is_final: bool) {
    out.push(if is_final { 1 } else { 0 });
    out.push((len & 0xFF) as u8);
    out.push(((len >> 8) & 0xFF) as u8);
    let nlen = !len & 0xFFFF;
    out.push((nlen & 0xFF) as u8);
    out.push(((nlen >> 8) & 0xFF) as u8);
}

/// Recompress a single candidate with zenflate effort 30.
fn recompress_one(data: &[u8], cancel: &dyn Stop) -> crate::error::Result<Vec<u8>> {
    let mut compressor = Compressor::new(CompressionLevel::new(30));
    let bound = Compressor::zlib_compress_bound(data.len());
    let mut output = vec![0u8; bound];
    let len = compressor
        .zlib_compress(data, &mut output, cancel)
        .map_err(|e| match e {
            zenflate::CompressionError::Stopped(reason) => PngError::Stopped(reason),
            other => PngError::InvalidInput(alloc::format!("zenflate recompress failed: {other}")),
        })?;
    output.truncate(len);
    Ok(output)
}

/// Recompress top candidates with zenflate effort 30.
///
/// This re-compresses filtered data using zenflate's near-optimal parser
/// with ECT-derived optimizations. Runs candidates in parallel when
/// `max_threads != 1` and multiple candidates are available.
fn zenflate_recompress(
    candidates: &[(usize, Vec<u8>)],
    cancel: &dyn Stop,
    current_best: &mut Option<Vec<u8>>,
    max_threads: usize,
) -> crate::error::Result<Option<Vec<u8>>> {
    let mut best: Option<Vec<u8>> = None;

    let results: Vec<crate::error::Result<Vec<u8>>> = if max_threads == 1 || candidates.len() <= 1 {
        // Sequential
        candidates
            .iter()
            .map(|(_size, data)| recompress_one(data, cancel))
            .collect()
    } else {
        // Parallel
        std::thread::scope(|s| {
            let handles: Vec<_> = candidates
                .iter()
                .map(|(_size, data)| s.spawn(|| recompress_one(data, cancel)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
    };

    for result in results {
        let compressed = result?;
        let dominated = best.as_ref().is_some_and(|b| compressed.len() >= b.len())
            || current_best
                .as_ref()
                .is_some_and(|b| compressed.len() >= b.len());
        if !dominated {
            best = Some(compressed);
        }
    }

    Ok(best)
}

/// Recompress top candidates with zenflate FullOptimal (Zopfli-style forward DP).
///
/// Uses zenflate effort 31+ which maps to (effort-16) iterations of iterative
/// cost model refinement with forward DP parsing. Runs candidates in parallel.
/// Recompress a single candidate with zenflate FullOptimal at the given effort.
fn full_optimal_recompress_one(
    data: &[u8],
    effort: u32,
    cancel: &dyn Stop,
) -> crate::error::Result<Vec<u8>> {
    let mut compressor = Compressor::new(CompressionLevel::new(effort));
    let bound = Compressor::zlib_compress_bound(data.len());
    let mut output = vec![0u8; bound];
    let len = compressor
        .zlib_compress(data, &mut output, cancel)
        .map_err(|e| match e {
            zenflate::CompressionError::Stopped(reason) => PngError::Stopped(reason),
            other => PngError::InvalidInput(alloc::format!(
                "zenflate FullOptimal recompress failed: {other}"
            )),
        })?;
    output.truncate(len);
    Ok(output)
}

/// Recompress top candidates with zenflate FullOptimal (Zopfli-style forward DP).
///
/// Uses zenflate effort 31+ which maps to (effort-16) iterations of iterative
/// cost model refinement with forward DP parsing. Runs candidates in parallel
/// when `max_threads != 1` and multiple candidates are available.
fn zenflate_full_optimal_recompress(
    candidates: &[(usize, Vec<u8>)],
    effort: u32,
    cancel: &dyn Stop,
    current_best: &mut Option<Vec<u8>>,
    max_threads: usize,
) -> crate::error::Result<Option<Vec<u8>>> {
    let mut best: Option<Vec<u8>> = None;

    let results: Vec<crate::error::Result<Vec<u8>>> = if max_threads == 1 || candidates.len() <= 1 {
        // Sequential
        candidates
            .iter()
            .map(|(_size, data)| full_optimal_recompress_one(data, effort, cancel))
            .collect()
    } else {
        // Parallel
        std::thread::scope(|s| {
            let handles: Vec<_> = candidates
                .iter()
                .map(|(_size, data)| s.spawn(|| full_optimal_recompress_one(data, effort, cancel)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
    };

    for result in results {
        let compressed = result?;
        let dominated = best.as_ref().is_some_and(|b| compressed.len() >= b.len())
            || current_best
                .as_ref()
                .is_some_and(|b| compressed.len() >= b.len());
        if !dominated {
            best = Some(compressed);
        }
    }

    Ok(best)
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
    max_threads: usize,
) -> crate::error::Result<Option<Vec<u8>>> {
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
    let effective_parallelism = if max_threads == 1 {
        1.0
    } else {
        num_cpus() as f64
    };
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
            let parallel_factor = n_candidates.min(effective_parallelism);
            let ms_per_candidate = remaining_ms * parallel_factor / n_candidates;
            let iters = (ms_per_candidate / ms_per_iter).floor() as u64;
            iters.clamp(5, 100)
        }
        None => 50u64,
    };

    if max_iters <= calibration_iters {
        return Ok(best);
    }

    // Phase 3: Run top candidates in parallel (or sequentially if single-threaded).
    // All threads share the combined stop — deadline expiry gracefully stops
    // all threads, cancellation hard-aborts them.
    let zopfli_results: Vec<Result<Vec<u8>, PngError>> = if max_threads == 1
        || candidates.len() <= 1
    {
        candidates
            .iter()
            .map(|(_size, data)| compress_with_zopfli_n(data, max_iters, &combined))
            .collect()
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = candidates
                .iter()
                .map(|(_size, data)| s.spawn(|| compress_with_zopfli_n(data, max_iters, &combined)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
    };

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
) -> crate::error::Result<Vec<u8>> {
    use std::io::Write;
    let mut options = zenzop::Options::default();
    options.iteration_count = core::num::NonZeroU64::new(iterations.max(1)).unwrap();
    options.enhanced = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use enough::Unstoppable;

    // ---- EffortParams::from_effort property tests ----

    #[test]
    fn effort_0_is_store() {
        let p = EffortParams::from_effort(0);
        assert_eq!(p.zenflate_effort, 0);
        assert!(p.screen_is_final);
        assert_eq!(p.strategies.len(), 1);
        assert!(!p.use_recompress);
        assert!(p.refine_efforts.is_empty());
    }

    #[test]
    fn effort_1_single_paeth() {
        let p = EffortParams::from_effort(1);
        assert_eq!(p.zenflate_effort, 1);
        assert!(p.screen_is_final);
        assert_eq!(p.strategies.len(), 1);
    }

    #[test]
    fn effort_2_minimal_strategies() {
        let p = EffortParams::from_effort(2);
        assert_eq!(p.strategies.len(), MINIMAL_STRATEGIES.len());
        assert!(p.screen_is_final);
    }

    #[test]
    fn effort_3_7_screen_only() {
        for effort in 3..=7 {
            let p = EffortParams::from_effort(effort);
            assert!(p.screen_is_final, "effort {effort} should be screen-only");
            assert!(p.refine_efforts.is_empty());
            assert!(!p.use_recompress);
        }
    }

    #[test]
    fn effort_8_15_have_refinement() {
        for effort in 8..=15 {
            let p = EffortParams::from_effort(effort);
            assert!(
                !p.screen_is_final,
                "effort {effort} should not be screen-only"
            );
            assert!(
                !p.refine_efforts.is_empty(),
                "effort {effort} should have refine"
            );
            assert!(p.brute_configs.is_empty() || effort >= 10);
        }
    }

    #[test]
    fn effort_17_has_brute_force() {
        let p = EffortParams::from_effort(17);
        assert!(!p.brute_configs.is_empty());
    }

    #[test]
    fn effort_24_has_fork() {
        let p = EffortParams::from_effort(24);
        assert!(!p.fork_brute_efforts.is_empty());
    }

    #[test]
    fn effort_29_has_beam() {
        let p = EffortParams::from_effort(29);
        assert!(!p.beam_brute_configs.is_empty());
    }

    #[test]
    fn effort_30_maniac() {
        let p = EffortParams::from_effort(30);
        assert!(p.use_recompress);
        assert!(!p.brute_configs.is_empty());
        assert!(!p.fork_brute_efforts.is_empty());
        assert!(!p.beam_brute_configs.is_empty());
        assert!(p.full_optimal_effort.is_none());
    }

    #[test]
    fn effort_31_full_optimal() {
        let p = EffortParams::from_effort(31);
        assert!(p.full_optimal_effort.is_some());
        assert!(p.use_recompress);
    }

    #[test]
    fn effort_50_medium_tier() {
        let p = EffortParams::from_effort(50);
        assert!(p.full_optimal_effort.is_some());
        assert!(p.use_recompress);
    }

    #[test]
    fn effort_100_full_tier() {
        let p = EffortParams::from_effort(100);
        assert!(p.full_optimal_effort.is_some());
        assert!(p.use_recompress);
        assert!(!p.brute_configs.is_empty());
    }

    #[test]
    fn effort_monotonicity_zenflate_effort() {
        let mut prev = 0;
        for effort in 0..=30 {
            let p = EffortParams::from_effort(effort);
            assert!(
                p.zenflate_effort >= prev,
                "zenflate effort should be monotonic: e{effort} = {} < {prev}",
                p.zenflate_effort
            );
            prev = p.zenflate_effort;
        }
    }

    // ---- from_effort_and_bpp content-aware adjustment ----

    #[test]
    fn bpp1_gets_brute_at_lower_effort() {
        // For bpp=1, brute-force should be injected at effort 16-23
        let p = EffortParams::from_effort_and_bpp(20, 1);
        assert!(!p.brute_configs.is_empty());
    }

    #[test]
    fn bpp4_no_extra_brute() {
        // For bpp=4, no extra brute-force injection
        let p = EffortParams::from_effort_and_bpp(20, 4);
        // Should match the raw from_effort result
        let p_raw = EffortParams::from_effort(20);
        assert_eq!(p.brute_configs.len(), p_raw.brute_configs.len());
    }

    // ---- try_compress ----

    #[test]
    fn try_compress_basic() {
        let data: Vec<u8> = (0..=255).collect::<Vec<u8>>().repeat(4);
        let mut compressors = [Compressor::new(CompressionLevel::new(1))];
        let bound = Compressor::zlib_compress_bound(data.len());
        let mut compress_buf = vec![0u8; bound];
        let mut verify_buf = vec![0u8; data.len()];
        let mut best = None;

        let size = try_compress(
            &data,
            &mut compressors,
            &mut compress_buf,
            &mut verify_buf,
            &mut best,
            &Unstoppable,
        )
        .unwrap();

        assert!(size < data.len());
        assert!(best.is_some());

        // Verify decompression
        let compressed = best.unwrap();
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    // ---- zlib_store_unfiltered ----

    #[test]
    fn zlib_store_empty() {
        let result = zlib_store_unfiltered(&[], 4, 0);
        // Should be valid zlib with stored blocks
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        assert!(decompressed.is_empty());
    }

    #[test]
    fn zlib_store_small() {
        let row = [10u8, 20, 30];
        let result = zlib_store_unfiltered(&row, 3, 1);
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        // Should be: filter byte (0) + row data
        assert_eq!(decompressed.len(), 4); // 1 + 3
        assert_eq!(decompressed[0], 0); // filter byte
        assert_eq!(&decompressed[1..], &row);
    }

    #[test]
    fn zlib_store_multi_row() {
        let data = [1u8, 2, 3, 4, 5, 6];
        let result = zlib_store_unfiltered(&data, 3, 2);
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        // 2 rows × (filter_byte + 3 bytes) = 8 bytes
        assert_eq!(decompressed.len(), 8);
        assert_eq!(decompressed[0], 0); // filter byte
        assert_eq!(&decompressed[1..4], &[1, 2, 3]);
        assert_eq!(decompressed[4], 0); // filter byte
        assert_eq!(&decompressed[5..8], &[4, 5, 6]);
    }

    // ---- write_zlib_stored_inline ----

    #[test]
    fn write_stored_inline_matches_standalone() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let standalone = zlib_store_unfiltered(&data, 4, 3);

        let mut inline = Vec::new();
        write_zlib_stored_inline(&mut inline, &data, 4, 3);

        assert_eq!(standalone, inline);
    }

    #[test]
    fn write_stored_inline_zero_height() {
        let mut out = Vec::new();
        write_zlib_stored_inline(&mut out, &[], 4, 0);
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&out).unwrap();
        assert!(decompressed.is_empty());
    }

    // ---- write_stored_block_header ----

    #[test]
    fn stored_block_header_format() {
        let mut out = Vec::new();
        write_stored_block_header(&mut out, 100, false);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0], 0); // not final
        assert_eq!(out[1], 100); // len low
        assert_eq!(out[2], 0); // len high

        let mut out2 = Vec::new();
        write_stored_block_header(&mut out2, 100, true);
        assert_eq!(out2[0], 1); // final
    }

    // ---- compress_filtered integration ----

    #[test]
    fn compress_filtered_effort_0() {
        let data = vec![128u8; 12 * 4]; // 4 rows of 12 bytes
        let opts = super::super::CompressOptions {
            cancel: &Unstoppable,
            deadline: &Unstoppable,
            parallel: false,
            remaining_ns: None,
            max_threads: 0,
        };
        let result = compress_filtered(&data, 12, 4, 3, 0, opts, None).unwrap();
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        // 4 rows × (1 filter byte + 12 data bytes) = 52 bytes
        assert_eq!(decompressed.len(), 52);
    }

    #[test]
    fn compress_filtered_effort_1() {
        let data = vec![128u8; 12 * 4];
        let opts = super::super::CompressOptions {
            cancel: &Unstoppable,
            deadline: &Unstoppable,
            parallel: false,
            remaining_ns: None,
            max_threads: 0,
        };
        let result = compress_filtered(&data, 12, 4, 3, 1, opts, None).unwrap();
        // Should produce valid zlib
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        assert_eq!(decompressed.len(), 52);
    }

    #[test]
    fn compress_filtered_effort_10_with_stats() {
        let data = vec![128u8; 12 * 4];
        let opts = super::super::CompressOptions {
            cancel: &Unstoppable,
            deadline: &Unstoppable,
            parallel: false,
            remaining_ns: None,
            max_threads: 0,
        };
        let mut stats = PhaseStats::default();
        let result = compress_filtered(&data, 12, 4, 3, 10, opts, Some(&mut stats)).unwrap();
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        assert_eq!(decompressed.len(), 52);
        // Should have at least screen + refine phases
        assert!(
            stats.phases.len() >= 2,
            "should have ≥2 phases, got {}",
            stats.phases.len()
        );
        assert!(stats.raw_size > 0);
    }

    #[test]
    fn compress_filtered_parallel() {
        let data = vec![128u8; 12 * 4];
        let opts = super::super::CompressOptions {
            cancel: &Unstoppable,
            deadline: &Unstoppable,
            parallel: true,
            remaining_ns: None,
            max_threads: 0,
        };
        let result = compress_filtered(&data, 12, 4, 3, 7, opts, None).unwrap();
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        assert_eq!(decompressed.len(), 52);
    }

    #[test]
    fn compress_filtered_rgba_transparent() {
        // RGBA with transparent pixels should trigger zeroing
        let mut data = vec![0u8; 16 * 4]; // 4 rows of 4 RGBA pixels
        // Set some pixels with alpha=0 but non-zero RGB
        data[0] = 255; // R
        data[1] = 128; // G
        data[2] = 64; // B
        data[3] = 0; // A=0 (transparent)

        let opts = super::super::CompressOptions {
            cancel: &Unstoppable,
            deadline: &Unstoppable,
            parallel: false,
            remaining_ns: None,
            max_threads: 0,
        };
        let result = compress_filtered(&data, 16, 4, 4, 2, opts, None).unwrap();
        let decompressed = miniz_oxide::inflate::decompress_to_vec_zlib(&result).unwrap();
        assert_eq!(decompressed.len(), 4 * (1 + 16));
    }
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
            zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best, 1).unwrap();

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
            1,
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

        let result = zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best, 1);
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
            zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best, 1).unwrap();

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
            zopfli_adaptive(&candidates, &cancel, &deadline, None, &mut current_best, 1).unwrap();

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
                patterns.contains(&decompressed),
                "decompressed data doesn't match any candidate",
            );
        }
    }
}
