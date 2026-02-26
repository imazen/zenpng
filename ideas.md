# Compression Improvement Ideas

## The Local Minima Problem

The core tension in our pipeline: **fast heuristics make "decent" guesses that
trap us in local minima at high effort levels.**

### How it manifests

At effort 0-30, the pipeline works well. Heuristic screening (Phase 1) picks
candidates, refine (Phase 2) compresses them properly, and BFF (Phase 3) does
streaming filter selection. Each phase produces genuinely better output than
the previous.

At effort 31+, everything changes. The lean pipeline (BFF + FullOptimal) skips
heuristic screening entirely — and for good reason. We discovered that zenflate
NearOptimal's ranking of filter candidates is **uncorrelated** with the ranking
that FullOptimal or zenzop would produce. The "best" candidate from screening
was often the wrong one for the final compressor.

But BFF has the same problem at a finer grain. BFF uses a Greedy (e10) or Lazy
(e15) eval compressor to score each filter per row. This eval compressor makes
**locally decent** choices that are invisible to the FullOptimal compressor that
runs afterward. The filter sequence that BFF selects is optimal for Greedy/Lazy
matching, not for forward-DP optimal parsing.

### The spectrum of correlation mismatch

```
Heuristic → FastHt:     moderate correlation (same greedy parse family)
FastHt → Lazy:          high correlation (lazy refines greedy's choices)
Lazy → NearOptimal:     moderate correlation (backward DP can diverge)
NearOptimal → FullOpt:  low-moderate correlation (different DP, different splits)
Any heuristic → zenzop: low correlation (completely different algorithm)
```

Each step up in compressor sophistication can invalidate the assumptions of the
previous step. This is why screening hurts at E31+ — the pipeline is:
1. Screen with FastHt → pick top-3 candidates
2. Recompress with FullOptimal → find different ranking
3. Net effect: wasted time on wrong candidates

### Possible solutions

**A. Match the eval compressor to the final compressor.**
Use FullOptimal at 1 iteration as the BFF eval. This is expensive (~10x slower
per row than Greedy) but produces a filter sequence that's directly optimal for
the final compressor. Could be a new tier at E60+.

**B. Multi-pass BFF with escalating eval.**
- Pass 1: BFF with Greedy eval → baseline filter sequence
- Pass 2: For each row, try alternatives using Lazy eval
- Pass 3: For rows where Lazy disagreed with Greedy, try NearOptimal eval
- Only re-evaluate rows where lower-quality evals disagreed

**C. Abolish filter selection; let the compressor decide.**
FullOptimal already considers all possible LZ77 parses. If we could teach it to
also consider filter alternatives per row, it would jointly optimize filter
selection and LZ77 parsing in a single DP pass. Conceptually: expand the state
space from (position, cost) to (position, filter_for_next_row, cost). This is
a 5x state expansion — expensive but clean.

**D. Iterative filter refinement (zenzop-style).**
After FullOptimal compression, analyze which rows contribute most to the output
size. For those rows, try alternative filters and re-run FullOptimal. Like the
genetic algorithm's whole-image fitness, but targeted to high-impact rows.

## Filter Coherence and Run-Length Stability

### Why consecutive same-filter rows compress better

Three mechanisms:

1. **Filter byte repetition.** Each row starts with a 1-byte filter indicator.
   Consecutive same-filter rows create a repeating pattern at stride=row_bytes
   that DEFLATE captures as trivial back-references (~3 bits each vs ~8 bits
   for varied literals). Small savings but real.

2. **Residual structure regularity.** The big one. Same filter on similar content
   → similar residual byte patterns → DEFLATE finds long cross-row matches
   (distance = k × row_bytes). Mixed filters destroy this regularity even when
   the underlying pixel content is similar.

3. **Huffman code concentration.** Within a DEFLATE block, mixed filters broaden
   the symbol distribution → longer Huffman codes for everyone. Uniform filters
   concentrate the distribution → shorter codes for common symbols.

### BFF's oscillation problem

BFF makes greedy per-row decisions. When two filters score within a few bytes of
each other, BFF can oscillate: `[Paeth, Sub, Paeth, Sub, ...]`. Each individual
decision is locally optimal, but the sequence is globally suboptimal because it
destroys cross-row match structure.

This is especially bad for photographic content where adjacent rows are very
similar — the "right" filter for each row is ambiguous, and the oscillation
penalty exceeds the per-row benefit of switching.

### Proposed: Sticky penalty

When evaluating filters in BFF, add a small byte penalty for switching away from
the current filter. The penalty represents the expected loss of cross-row match
coherence.

```
effective_size[filter] = compressed_size[filter] + (filter != current_filter ? PENALTY : 0)
```

**Calibration:** The penalty should be proportional to row_bytes. For a 3KB row
(1024×RGB), a penalty of 4-8 bytes is ~0.2%. This suppresses noise-level
oscillation while allowing genuine filter transitions (where the benefit exceeds
the coherence cost).

**Adaptive penalty:** Could make the penalty proportional to the current run
length — longer runs are more valuable to preserve. `penalty = base_penalty *
min(run_length, 8)`. A 1-row run gets 1× penalty; an 8-row run gets 8× penalty
to switch away from.

### Proposed: Run-length grouping

Instead of per-row BFF, decide in groups of K rows (K=4-8). Try all 5 filters
for the entire group. This naturally produces coherent runs.

Cost: 5 × group_compress per group (vs 5 × row_compress per row). For K=4 the
total cost is similar but each evaluation captures more cross-row context.

### Proposed: Two-pass smoothing

1. BFF picks filters greedily (current behavior)
2. Post-pass: identify short runs (< 3 rows). For each short run, try replacing
   with the adjacent longer run's filter. Accept if total compressed size
   improves after re-running FullOptimal.

## Match Cache Reuse for FullOptimal

### The opportunity

`get_best_lengths` (the forward DP core) is 46.5% of FullOptimal instructions.
Roughly half of that is matchfinding — for each position, the binary tree
matchfinder finds all matches up to length 258.

Match results depend only on the input data, not the cost model. Across all
iterations, the same positions produce the same matches. Currently we recompute
them every iteration.

### Implementation

Iteration 1: run matchfinder normally, cache results in a `Vec<Vec<LzMatch>>`.
Iterations 2+: replay cached matches, skip matchfinder entirely.

Memory cost: ~16 bytes per match × ~2 matches per position × input_length.
For a 1.4MB input: ~45 MB. Acceptable for E31+ where we're already spending
seconds on compression.

### Expected speedup

Matchfinding is ~50% of `get_best_lengths`, which is ~46% of total.
Eliminating it for iterations 2+ saves ~23% per additional iteration.
At 60 iterations: 1 full + 59 × 0.77 = ~46.4 equivalent iterations.
Expected speedup: 60/46.4 = 1.29x. From 3.31s to ~2.56s at 60i.

### Risks

- Memory pressure on very large images (10MB+ input → 150MB+ cache)
- Could add a size threshold: cache only if input < 2MB, fall back to
  recomputation for larger inputs
- The cache is read-only after iteration 1 — no thread safety concerns

## Better Initial Cost Model

### Current state

FullOptimal starts iteration 1 with a flat cost model (all symbols equal cost).
This produces a poor initial LZ77 parse, which then informs the cost model for
iteration 2. The first 2-3 iterations are essentially "warming up."

### Proposed: Seed from Lazy parse

Before starting FullOptimal, run a single Lazy (e15) compression pass. Extract
the symbol frequency distribution from that pass. Use it as the initial cost
model for FullOptimal iteration 1.

Cost: one Lazy pass (~50ms on our test image). Benefit: iterations 1-3 start
from a much better cost model, potentially saving 2-3 iterations worth of
convergence. At E31 (15i), this could make 12 iterations as good as the
current 15.

### Alternative: Seed from block statistics

After block splitting, compute per-block symbol frequencies from the initial
LZ77 output. Use these for the initial cost model. This is essentially free
(we already have the block split) and better than flat.

ECT's Zopfli does this — it uses the first block split's statistics to seed
the initial cost model.

## Block Splitting Improvements

### Current state

`SplitHistograms` + `block_cost_simple` reduced block splitting from 24.5% to
<0.5% of instructions. The split quality is good — `block_cost_simple` actually
produces better splits than `block_cost_best` (tested: +13 bytes worse with
the more sophisticated cost function).

### Iterated block splitting

Currently block splitting runs once per DP iteration. ECT's Zopfli has a `twice`
flag that re-runs block splitting after the first pass. The idea: after the DP
parser produces a new LZ77 output, the optimal split points may have changed.

Could implement as: after the final DP iteration, re-run block splitting on the
final LZ77 output, then re-encode the blocks with updated Huffman codes. This
is a single extra pass, not an iteration — very cheap.

### Adaptive block size

Current `MIN_BLOCK_LENGTH` is a constant. For highly variable content (e.g.,
images with both photographic regions and flat UI), smaller blocks could capture
distribution shifts better. For uniform content, larger blocks amortize the
Huffman table overhead.

Could analyze the variance of symbol frequencies across the SplitHistograms
chunks and adjust the minimum block size accordingly.

## ECT Feature Gap Analysis

### LFS_INCREMENTAL2/3

ECT has 3 incremental modes differing only in zlib tuning:
- INCREMENTAL: zlib L2, chain=200 (fast, loose)
- INCREMENTAL2: zlib L2, chain=1100 (thorough)
- INCREMENTAL3: zlib L1, no tune (fastest)

Our BFF uses zenflate Greedy (e10) or Lazy (e15). This is a single eval
compressor per BFF pass. ECT runs up to 3 incremental passes with different
eval quality.

**Opportunity:** We already support multiple BFF eval levels (`[10, 15]` at
E36+). Could add a third at e22 (Lazy2) for E46+. The marginal cost is one
more full-image BFF pass.

### LFS_GENETIC

Population-19, two-point crossover, 1% mutation, whole-image zlib L3 fitness.
Terminates after 500 generations without improvement.

**Not worth implementing.** Reasons:
- Fitness function (zlib L3) doesn't correlate with FullOptimal output
- Population of 19 is tiny relative to the 5^H search space
- Hundreds of full-image compressions for marginal gains
- Our beam search is more principled (exact DEFLATE context, K states)

However, the concept of **whole-image evaluation** is valuable. Post-BFF, we
could do a quick check: compress the full image with the BFF filter sequence,
then try 2-3 alternative sequences (all-Paeth, all-Sub, best heuristic) and
keep the best. This is 3-4 full compressions, not 500+.

### LFS_DISTINCT_BYTES / LFS_MINSUM

We have MinSum (Adaptive(MinSum)) but not distinct bytes. Distinct bytes counts
the number of unique byte values per filtered row — similar to entropy but
faster (no log computation). Worth benchmarking, but unlikely to beat our
existing heuristics since it ignores byte frequency.

### Block splitting parameters

ECT tunes 7+ parameters per effort level:
- noblocksplit, noblocksplitlz, trystatic, skipdynamic
- searchext, filter_style, greed, ultra, advanced

We have: `MIN_BLOCK_LENGTH` (constant) and `block_cost_simple` (single cost).
There may be compression gains from tuning these, especially `trystatic` (trying
static Huffman for small blocks) and `noblocksplitlz` (minimum LZ77 size for
splitting). These are low-risk tuning knobs.

## Whole-Pipeline Ideas

### Effort 31+ redesign

Current E31-45 lean pipeline: BFF@10 → FullOptimal.

Proposed E31-45: BFF@10 → FullOptimal → verify.
The "verify" step: take the FullOptimal output, extract the filter sequence,
run BFF@15 on it with a penalty for deviating from the existing sequence. If
BFF@15 finds a better filter for any row, re-run FullOptimal on the updated
stream. This is ~2 full compressions total — moderate cost increase for
potentially finding cases where BFF@10 made a bad choice.

### Effort 46+ pipeline

Current: 9 strategies + refine + BFF + FullOptimal.
The screening phase is back at E46+ but still has the correlation mismatch.
Consider: screen with FullOptimal@1i instead of FastHt. 9 strategies × 1i
FullOptimal is ~9 seconds on our test image. Expensive, but this is E46+ where
users expect multi-minute runtimes. The ranking would be directly meaningful for
the final compressor.

### Diminishing returns detection

After each DP iteration, track the improvement delta. If delta < 0.01% for
3 consecutive iterations, stop early. ECT runs a fixed number of iterations;
we could be smarter and converge faster on easy images while spending more time
on hard ones.

Caution: convergence isn't monotonic. Sometimes iteration N barely improves but
iteration N+1 finds a new block split that drops 0.1%. Need a patience parameter:
stop only after K consecutive below-threshold iterations.

### Static block optimization

ECT's `trystatic` parameter tries static Huffman codes for blocks smaller than
a threshold. Static blocks have zero per-block Huffman table overhead (the codes
are defined by the spec). For tiny blocks (< 200-300 bytes of LZ77), static
encoding can beat dynamic because the ~30-80 bytes of Huffman table overhead
dominate.

zenflate already tries static blocks in some code paths. Verify this is active
in FullOptimal and tune the threshold.

## Measurement Infrastructure

### Representative corpus

Need a curated subset (~100 images) that covers:
- Photographic RGB (smooth gradients, high entropy)
- Photographic RGBA (photo + alpha channel)
- Screenshot/UI (flat regions, sharp edges, repeated patterns)
- Indexed/palette (limited colors, pattern-heavy)
- Synthetic/gradient (mathematically regular patterns)
- Mixed (photo regions + text overlays)
- Small images (icons, sprites — high overhead ratio)
- Large images (4K+ — different block splitting behavior)
- APNG (temporal correlation, different optimization landscape)

Selection method: profile corpus at 6 effort levels, cluster on compression
ratio profiles, select representatives from each cluster.

### A/B testing framework

For any proposed change, need a quick way to measure:
1. Compression ratio delta across the corpus (must be net-positive)
2. Speed delta (acceptable if ≤ 10% slower for ≥ 0.1% smaller)
3. Per-image breakdown (identify regressions even if aggregate improves)

The crusher_bench example is close but slow (runs external tools). Need a
zenflate-only fast path that compares two code versions on the corpus in < 60s.

## Priority Ranking

By expected impact (compression improvement × feasibility):

1. **Match cache reuse** — 1.29x speedup enables more iterations in same time
2. **Sticky penalty for BFF** — eliminates oscillation, easy to implement
3. **Better initial cost model** — saves 2-3 convergence iterations
4. **BFF eval at e22 for E46+** — one more incremental quality tier
5. **Post-BFF whole-image verify** — catches globally suboptimal sequences
6. **Static block threshold tuning** — small but free compression gains
7. **Run-length grouping for BFF** — addresses coherence more directly
8. **Iterated block splitting** — small gains, clean implementation
9. **Diminishing returns early-exit** — efficiency, not compression
10. **FullOptimal-seeded screening for E46+** — correct but expensive
