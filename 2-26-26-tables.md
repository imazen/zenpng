# zenpng vs ECT Algorithm Comparison Tables — 2026-02-26 Snapshot

## zenpng Effort Table (0-61+)

| Effort | Preset | Strategies | Screen | TopK | Refine | BruteForce | BFF | AdaptFork | Beam | Recompress | FullOpt |
|--------|--------|-----------|--------|------|--------|------------|-----|-----------|------|------------|---------|
| 0 | None | 1: None | e0 (final) | 1 | — | — | — | — | — | — | — |
| 1 | — | 1: Paeth | e1 (final) | 1 | — | — | — | — | — | — | — |
| 2 | Fastest | 1: Paeth | e2 (final) | 1 | — | — | — | — | — | — | — |
| 3 | — | 3: MINIMAL | e3 (final) | 1 | — | — | — | — | — | — | — |
| 4 | — | 3: MINIMAL | e4 (final) | 1 | — | — | — | — | — | — | — |
| 5 | — | 5: FAST | e5 (final) | 1 | — | — | — | — | — | — | — |
| 6 | Fast | 5: FAST | e6 (final) | 1 | — | — | — | — | — | — | — |
| 7 | — | 5: FAST | e7 (final) | 1 | — | — | — | — | — | — | — |
| 8 | — | 5: FAST | e7 | 3 | [8] | — | — | — | — | — | — |
| 9 | — | 5: FAST | e7 | 3 | [10] | — | — | — | — | — | — |
| 10 | Balanced | 9: HEURISTIC | e7 | 3 | [12] | — | — | — | — | — | — |
| 11 | — | 9: HEURISTIC | e7 | 3 | [14] | — | — | — | — | — | — |
| 12 | — | 9: HEURISTIC | e7 | 3 | [15] | — | — | — | — | — | — |
| 13 | Thorough | 9: HEURISTIC | e7 | 3 | [17] | — | — | — | — | — | — |
| 14 | — | 9: HEURISTIC | e7 | 3 | [18] | — | — | — | — | — | — |
| 15 | — | 9: HEURISTIC | e7 | 3 | [20] | — | — | — | — | — | — |
| 16 | High | 9: HEURISTIC | e7 | 3 | [20,22] | — | — | — | — | — | — |
| 17 | — | 9: HEURISTIC | e7 | 3 | [22] | — | — | — | — | — | — |
| 18 | — | 9: HEURISTIC | e7 | 3 | [22,24] | — | — | — | — | — | — |
| 19 | — | 9: HEURISTIC | e7 | 3 | [24] | — | — | — | — | — | — |
| 20 | Aggressive | 9: HEURISTIC | e7 | 3 | [24,26] | — | — | — | — | — | — |
| 21 | — | 9: HEURISTIC | e7 | 3 | [26,28] | — | — | — | — | — | — |
| 22 | — | 9: HEURISTIC | e7 | 3 | [28] | — | — | — | — | — | — |
| 23 | — | 9: HEURISTIC | e7 | 3 | [28,30] | — | — | — | — | — | — |
| 24 | Best | 9: HEURISTIC | e7 | 3 | [28,30] | (5,1) | — | — | — | — | — |
| 25 | — | 9: HEURISTIC | e7 | 3 | [28,30] | (5,1)(5,4) | — | — | — | — | — |
| 26 | — | 9: HEURISTIC | e7 | 3 | [30] | (5,1)(5,4) | [10] | (15,2) | — | — | — |
| 27 | — | 9: HEURISTIC | e7 | 3 | [30] | (5,1)(5,4) | [10,15] | (15,2)(22,2) | — | — | — |
| 28 | Crush | 9: HEURISTIC | e7 | 3 | [30] | full sweep | [10,15] | (15,2)(22,2) | — | Yes | — |
| 29 | — | 9: HEURISTIC | e7 | 3 | [30] | full sweep | [10,15] | (15,2)(22,2) | (10,3) | Yes | — |
| 30 | Maniac | 9: HEURISTIC | e7 | 3 | [30] | full sweep | [10,15] | (15,2)(22,2) | (10,3)(15,3) | Yes | — |
| 31-35 | — | none (lean) | — | 1 | — | — | [10] | — | — | Yes | E (FO only) |
| 36-45 | — | none (lean) | — | 1 | — | — | [10,15] | — | — | Yes | E (FO only) |
| 46-60 | — | 9: HEURISTIC | e7 | 3 | [30] | (1,1)(1,4)(3,1)(3,4) | [10,15] | — | — | Yes | E |
| 61+ | — | 9: HEURISTIC | e7 | 3 | [30] | full sweep | [10,15] | (15,2)(22,2) | (10,3)(15,3) | Yes | E |

**Column key:**
- **BruteForce**: (context_rows, eval_effort) pairs. "full sweep" = (1,1)(1,4)(3,1)(3,4)(5,1)(5,4)(8,1)(8,4)
- **BFF**: BruteForceFork eval efforts — incremental DEFLATE filter selection
- **AdaptFork**: (eval_level, narrow_to) — adaptive fork with candidate narrowing
- **Beam**: (eval_level, beam_width) — beam search with incremental DEFLATE
- **FullOpt**: "E" = effort level passed to zenflate FullOptimal (iterations = effort-16). "FO only" = skip NearOptimal/zenzop, use FullOptimal exclusively.

## ECT Level Table (1-9)

| ECT Level | Iterations | searchext | filter_style | noblocksplit | noblocksplitlz | trystatic | skipdynamic | Filters |
|-----------|-----------|-----------|-------------|-------------|----------------|-----------|-------------|---------|
| 1 | — | — | — | — | — | — | — | optipng L1 only |
| 2 | 1 | 0 | 0 | 2000 | 800 | 0 | 180 | LFS_AVG + optipng |
| 3 | 1 | 1 | 0 | 2000 | 512 | 0 | 180 | LFS_AVG + optipng |
| 4 | 2 | 1 | 0 | 2000 | 512 | 0 | 180 | LFS_AVG + optipng |
| 5 | 3 | 1 | 1 | 2000 | 200 | 0 | 180 | LFS_AVG + optipng |
| 6 | 8 | 1 | 1 | 1300 | 200 | 800 | 80 | LFS_AVG + optipng |
| 7 | 13 | 1 | 1 | 1000 | 200 | 1800 | 80 | LFS_AVG + optipng |
| 8 | 40 | 1 | 2 | 800 | 120 | 2000 | 80 | LFS_AVG + optipng |
| 9 | 60 | 2 | 3 | 800 | 100 | 3000 | 80 | LFS_AVG + optipng |

**ECT --allfilters adds:** LFS_PREDEFINED, LFS_ZERO, LFS_BRUTE_FORCE, LFS_SUB, LFS_UP, LFS_AVG, LFS_PAETH, LFS_ENTROPY, LFS_DISTINCT_BIGRAMS, LFS_INCREMENTAL, LFS_INCREMENTAL2, LFS_INCREMENTAL3 (12 strategies)

**ECT --allfilters-b further adds:** LFS_DISTINCT_BYTES, LFS_MINSUM, LFS_GENETIC (15 strategies total)

## zenflate Compression Strategy Mapping

| Effort | Strategy | Description |
|--------|----------|-------------|
| 0 | Store | No compression, uncompressed DEFLATE blocks |
| 1-4 | Turbo | Single-entry hash table, limited hash updates during skips |
| 5-9 | FastHt | 2-entry hash table, limited hash updates during skips |
| 10 | Greedy | Hash chain greedy, best match at current position |
| 11-17 | Lazy | Hash chain lazy, single lookahead (check if next pos has better match) |
| 18-22 | Lazy2 | Hash chain double-lazy, two lookaheads |
| 23-30 | NearOptimal | Binary tree matchfinder, iterative backward DP parser |
| 31+ | FullOptimal | Zopfli-style forward DP, iterative cost model. Iters = effort - 16 |

### Monotonicity fallback chain

```
FullOptimal(31+) → NearOptimal max(e30) → Lazy2 max(e22) → Lazy max(e17) → Greedy max(e10) → FastHt max(e9) → [stop]
```

Turbo→FastHt always improves (guaranteed), so the chain terminates at FastHt.

## Filter Strategy Sets

### MINIMAL (3 strategies, effort 3-4)
| Strategy | Type | Description |
|----------|------|-------------|
| Single(0) | None | No filtering — best for flat/constant content |
| Single(4) | Paeth | Best single filter overall on most images |
| Adaptive(Bigrams) | Heuristic | Least distinct bigrams per row — cheap, effective |

### FAST (5 strategies, effort 5-9)
| Strategy | Type | Description |
|----------|------|-------------|
| Single(0) | None | No filtering |
| Single(4) | Paeth | Best single filter |
| Adaptive(MinSum) | Heuristic | Minimum sum of absolute values — classic PNG heuristic |
| Adaptive(Bigrams) | Heuristic | Least distinct bigrams — correlates with deflate |
| Adaptive(Entropy) | Heuristic | Minimum Shannon entropy per row |

### HEURISTIC (9 strategies, effort 10+)
| Strategy | Type | Description |
|----------|------|-------------|
| Single(0) | None | No filtering |
| Single(1) | Sub | Left-neighbor difference |
| Single(2) | Up | Above-neighbor difference |
| Single(3) | Average | Mean of left+above |
| Single(4) | Paeth | Paeth predictor |
| Adaptive(MinSum) | Heuristic | Minimum absolute sum |
| Adaptive(Entropy) | Heuristic | Minimum Shannon entropy |
| Adaptive(Bigrams) | Heuristic | Least distinct bigrams |
| Adaptive(BigEnt) | Heuristic | Combined bigrams+entropy (30-170x slower than MinSum) |

## Brute Force Variants

| Variant | How it works | Used at |
|---------|-------------|---------|
| BruteForce(ctx, eval) | For each row: try all 5 filters, compress row+context with zenflate at eval effort, pick smallest | e24+ |
| BruteForceBlock(ctx, eval) | Block-level: try all 5 filters for a block of rows | e28+ (disabled — slower AND larger) |
| BruteForceFork(eval) | Snapshot full DEFLATE compressor state, try all 5 filters, pick smallest cumulative output. True incremental evaluation | e26+ (e31+ lean) |
| AdaptiveFork(eval, narrow) | BFF but narrow to top-N filters per row before eval | e26+ |
| BeamSearch(eval, width) | K-wide beam of parallel DEFLATE states, each testing all 5 filters | e29+ |

## ECT Filter Strategy IDs (LodePNG)

| ID | Name | Description |
|----|------|-------------|
| 0 | LFS_ZERO | All rows use filter 0 (None) |
| 1 | LFS_SUB | All rows use filter 1 (Sub) |
| 2 | LFS_UP | All rows use filter 2 (Up) |
| 3 | LFS_AVG | All rows use filter 3 (Average) |
| 4 | LFS_PAETH | All rows use filter 4 (Paeth) |
| 5 | LFS_BRUTE_FORCE | Try all 5 filters per row, pick smallest compressed chunk |
| 6 | LFS_PREDEFINED | Reuse existing filter bytes from input PNG |
| 7 | LFS_ENTROPY | Shannon entropy heuristic per row |
| 8 | LFS_DISTINCT_BIGRAMS | Least distinct bigrams per row |
| 9 | LFS_DISTINCT_BYTES | Least distinct bytes per row |
| 10 | LFS_MINSUM | Minimum sum heuristic per row |
| 11 | LFS_INCREMENTAL | Incremental DEFLATE evaluation mode 1 |
| 12 | LFS_INCREMENTAL2 | Incremental DEFLATE evaluation mode 2 |
| 13 | LFS_INCREMENTAL3 | Incremental DEFLATE evaluation mode 3 |
| 14 | LFS_GENETIC | Genetic algorithm filter selection |
| 15 | LFS_ALL_CHEAP | All non-brute-force strategies |

## ECT ↔ zenpng Filter Equivalences

| ECT Filter | zenpng Equivalent | Notes |
|------------|------------------|-------|
| LFS_ZERO (0) | Single(0) | Identical — no filtering |
| LFS_SUB (1) | Single(1) | Identical |
| LFS_UP (2) | Single(2) | Identical |
| LFS_AVG (3) | Single(3) | Identical |
| LFS_PAETH (4) | Single(4) | Identical |
| LFS_BRUTE_FORCE (5) | BruteForce(ctx, eval) | Same concept, different eval compressors |
| LFS_PREDEFINED (6) | — | No equivalent (zenpng doesn't read input filters) |
| LFS_ENTROPY (7) | Adaptive(Entropy) | Same heuristic |
| LFS_DISTINCT_BIGRAMS (8) | Adaptive(Bigrams) | Same heuristic |
| LFS_DISTINCT_BYTES (9) | — | Not implemented in zenpng |
| LFS_MINSUM (10) | Adaptive(MinSum) | Same heuristic |
| LFS_INCREMENTAL (11) | BruteForceFork(eval) | Same concept — snapshot DEFLATE, try all filters |
| LFS_INCREMENTAL2 (12) | — | ECT has 3 incremental modes, zenpng has 1 |
| LFS_INCREMENTAL3 (13) | — | ECT has 3 incremental modes, zenpng has 1 |
| LFS_GENETIC (14) | — | Not implemented in zenpng |
| — | Adaptive(BigEnt) | zenpng-only combined bigrams+entropy heuristic |
| — | AdaptiveFork | zenpng-only: adaptive fork with candidate narrowing |
| — | BeamSearch | zenpng-only: K-wide parallel beam search |

## ECT Block Splitting Parameters

| Parameter | Description | ECT-9 value |
|-----------|-------------|-------------|
| noblocksplit | Min uncompressed size to attempt block splitting | 800 |
| noblocksplitlz | Min LZ77-encoded size to attempt block splitting | 100 |
| trystatic | Size threshold: try static Huffman if dynamic < this | 3000 |
| skipdynamic | Min block size to try dynamic Huffman encoding | 80 |
| searchext | Extended search parameter (higher = more thorough) | 2 |
| filter_style | Zopfli filter style parameter | 3 |
| num | Parallel block split searches | 9 (levels 6+) / 3 (levels 2-5) |
| greed | Greedy search threshold (PNG mode) | 50 (vs 258 generic) |
| ultra | Ultra mode | 1 (levels 5+), 2 (>60i), 3 (>90i) |
| advanced | Advanced Huffman optimizations | Yes (levels 5+) |

## Benchmark Results (single image — 0d154749 from CLIC 2025)

| Engine | Config | Time | Output | vs ECT-9 |
|--------|--------|------|--------|----------|
| zenflate | 15 iterations (E31) | 1.46s | 1,986,089 | -495 bytes |
| zenflate | 60 iterations (E76) | 3.31s | 1,984,665 | -1,913 bytes |
| zenzop | 15 iterations | 1.72s | 1,986,080 | -498 bytes |
| zenzop | 60 iterations | 4.10s | 1,985,771 | -807 bytes |
| ECT-9 | 60 iterations (reuse) | 2.67s | 1,990,905 | +4,327 bytes |
| ECT-9 | 60 iterations (full) | 3.60s | 1,986,578 | baseline |

## 10-Image Aggregate Benchmark

| Tool | Total bytes | vs ECT-9 |
|------|-------------|----------|
| ECT-9 | 14,296,222 | baseline (100.0%) |
| zenpng E31 | 14,323,573 | +0.19% |

zenpng wins on 3/10 images. Gap is primarily iteration count (15 vs 60) and ECT's
multiple filter strategy passes. At E76 (60 iterations) zenflate beats ECT-9 on
single-image tests, suggesting the gap at E31 is from fewer iterations, not
algorithmic weakness.
