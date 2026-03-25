> **Historical.** The preset names and effort values below are from an earlier pipeline version. Presets have been renamed and effort mappings changed. Current presets: Fastest(1), Turbo(2), Fast(7), Balanced(13), Thorough(17), High(19), Aggressive(22), Intense(24).

# Compression Level Tuning Analysis

Empirical analysis of zenpng's compression pipeline to find where time goes
and what each phase actually delivers. Data from `phase_timing` and
`strategy_explorer` examples run across gb82-sc screenshots (10 images),
qoi-benchmark/screenshot_web (14 images), and CID22-512 validation (41 images).

## Pipeline architecture

Each compression level runs a 4-phase pipeline:

1. **Screen** — Try 9 heuristic strategies (5 single filters + 4 adaptive) with
   a cheap compressor (L1 by default). Rank by compressed size. ~500ms for
   1024×1024 images.

2. **Refine** — Take the top 3 strategies and recompress at the target zenflate
   level. This is where most compression quality comes from.

3. **Brute-force** — Per-row filter selection using DEFLATE context evaluation.
   Expensive (~3–8s per config). Tests different context window sizes and
   evaluation levels.

4. **Zopfli** — Adaptive-iteration zopfli on the top 3 candidates. Only at
   Crush and above. Very slow (~minutes).

## Phase value: where time goes

### gb82-sc screenshots (10 images, 1024×768 to 1920×1200)

| Level      | Screen | Refine       | BruteForce     | Zopfli       | Total    | Size  |
|------------|--------|--------------|----------------|--------------|----------|-------|
| Balanced   | 5.0s   | 0.7s (−315K) | 2.7s (−122K)   | —            | 8.5s     | 3.23M |
| Thorough   | 5.0s   | 1.6s (−363K) | 3.0s (−127K)   | —            | 9.7s     | 3.18M |
| High       | 5.0s   | 2.3s (−377K) | 3.2s (−127K)   | —            | 10.5s    | 3.17M |
| Aggressive | 4.9s   | 8.8s (−584K) | 5.8s (−128K)   | —            | 19.7s    | 2.97M |
| Best       | 5.0s   | 58.9s (−650K)| 44.8s (−134K)  | —            | 108.8s   | 2.90M |
| Crush      | 4.7s   | 61.0s (−650K)| 46.6s (−134K)  | 1323s (−17K) | 1436s    | 2.89M |
| Obsessive  | 4.5s   | 60.2s (−650K)| 110.9s (−134K) | 1269s (−17K) | 1445s    | 2.89M |
| Maniac     | 5.4s   | 61.2s (−337K)| 183.7s (−132K) | 1282s (−17K) | 1532s    | 2.88M |

(Sizes are totals across 10 images. Deltas show improvement beyond the previous phase.)

### Key observations

**The Refine phase delivers 70–85% of all compression gains.** Going from
Screen's L1 result to the target zenflate level is the single most impactful
step. The L9→L12 zenflate jump alone saves ~47K per image (3.1%).

**Brute-force delivers diminishing returns.** On screenshots, BF saves ~13K
per image total (−134K / 10 images). On photographic images, BF saves under
1K per image (<0.1%). The return per millisecond is poor: −3 bytes/ms at Best,
compared to −11 bytes/ms for Refine.

**Zopfli saves ~1.7K per image at enormous cost.** The zopfli phase takes
22 minutes for 10 screenshots and saves 17K total. That's 0.06% improvement
for 13× the time of the rest of the pipeline combined.

**Obsessive ≈ Crush.** The full BF config sweep (ctx 1–8 at multiple eval
levels) found the exact same optimum as Best's ctx=5 configs. More BF configs
don't help — ctx=5 already captures the filter optimum.

**Maniac's L6 screening is accurate but not useful.** Screening at L6 instead
of L1 means the Refine phase starts from a better baseline (hence smaller
Refine delta). But the final size is only ~10K smaller than Crush across 10
images. Not worth the extra BF time.

## Strategy explorer findings

### Screenshots: Heuristic vs brute-force gap (gb82-sc, 10 images)

| Level | Best Heuristic | Best BF | Gap | Gap % |
|------:|---------------:|--------:|----:|------:|
| 6 | 3,584,633 | 3,381,575 | 203,058 | 5.66% |
| 9 | 3,529,192 | 3,317,062 | 212,130 | 6.01% |
| 12 | 3,277,239 | 3,046,893 | 230,346 | 7.03% |

BF delivers 5–7% better compression than the best heuristic on screenshots.
This is much larger than the 0.07% gap seen on photographic images. However,
the efficiency (bytes/ms) is still lower than the Refine phase.

Adaptive(Bigrams) is the best heuristic at every level. Single(None) — no
filtering at all — wins 6 of 10 individual screenshots. Screenshots with
large flat regions compress best unfiltered.

### Photographic images: BF barely matters

At L12, the gap between the best heuristic and the best brute force is:
- **Photographic images**: 1,018 bytes average (0.07%)
- **Screenshots**: ~23K per image (7%)

The zenflate compressor level overwhelms filter selection quality on photos.
A mediocre filter at L12 beats a perfect filter at L9.

### Block brute-force is strictly dominated

Block-wise brute-force (selecting filters per block of rows) is both **slower
AND larger** than per-row brute-force at every zenflate level tested. It was
disabled based on this data.

### Context rows: 5 is sufficient

For per-row brute-force, increasing context_rows beyond 5 gives diminishing
or negative returns. The marginal savings going from ctx=1 to ctx=10 is only
926 bytes average at L12. ctx=5 is the sweet spot.

### eval=L4 vs eval=L1: barely matters

Evaluating brute-force candidates at zenflate L4 instead of L1 saves
+0.02–0.06% for 1.5–2× the filter time. Only justified at Best and above.

### Best strategy changes with zopfli

At zenflate L12, BruteForce wins most images. At zopfli-50, Adaptive(BigEnt)
wins most images. BruteForce evaluates candidates with zenflate, so its filter
choices are optimized for zenflate's compressor, not zopfli's. This means the
filter selected by BF may not be optimal for zopfli.

### Zopfli loses to zenflate-12 on screenshots

On the gb82-sc screenshot corpus, BruteForce+zopfli-50 produces 3,051,220
bytes vs BruteForce+zenflate-12's 3,046,893 bytes. Zenflate-12 wins by 4K
while being 19× faster. Zopfli's DEFLATE optimization cannot compensate for
zenflate-12's superior compression on screenshot content.

### Filter choice dwarfs zopfli improvement

The range from worst to best filter strategy is ~42K per image average. The
improvement from switching L12 to zopfli-50 is ~6.8K. Spending time on better
filter selection would be more productive than spending it on zopfli iterations.

### Zopfli iteration scaling: severe diminishing returns

| Iterations | Avg savings vs L12 | Marginal savings | Avg time |
|------------|-------------------|-----------------|----------|
| 5          | −3,890 bytes      | —               | 2.9s     |
| 15         | −9,570 bytes      | −5,680 bytes    | 5.7s     |
| 50         | −12,492 bytes     | −2,922 bytes    | 15.0s    |

The step from 15→50 iterations yields less than half the savings of 5→15
while taking 2.6× more time. The cost per byte saved gets 6.7× worse.

### Efficiency frontier (gb82-sc screenshots, sorted by total size)

| Strategy | Level | Total Size | Time | Bytes/ms |
|----------|------:|-----------:|-----:|---------:|
| Single(None) | 6 | 3,675,958 | 318ms | 1,173 |
| Adaptive(Bigrams) | 6 | 3,584,633 | 574ms | 809 |
| Adaptive(Bigrams) | 9 | 3,529,192 | 1,088ms | 478 |
| BruteForce(c1e1) | 9 | 3,369,404 | 2,436ms | 279 |
| Single(None) | 12 | 3,313,498 | 9,658ms | 76 |
| Adaptive(Bigrams) | 12 | 3,277,239 | 11,398ms | 68 |
| BruteForce(c8e4) | 12 | 3,049,045 | 16,309ms | 61 |

## Problems with the current levels

### 1. Thorough and High are nearly identical

Thorough (L8) and High (L9) produce almost the same output:
- gb82-sc: 3.18M vs 3.17M (0.3% difference)
- Same BF config (ctx=3, eval=1)
- Similar time (9.7s vs 10.5s)

Two levels that produce nearly identical results waste the user's mental model.

### 2. Brute-force at Balanced/Thorough/High is wasted time

At L6–L9, brute-force takes 2.7–3.2s per corpus but saves only ~127K across
10 images. That's ~45 bytes/ms. The Refine phase delivers 4–10× better
bytes/ms. For users who pick "Balanced", they want a balance of speed and
quality — spending 3s on brute-force that saves 12K per image is not balanced.

### 3. Obsessive ≈ Crush (no differentiation)

The full BF sweep finds the same optimum as the standard ctx=5 configs.
There's no room between Crush and Maniac for a meaningfully different level.

### 4. Zopfli dominates Crush/Obsessive/Maniac time

Zopfli takes 85–95% of the total time at these levels but delivers 0.06% of
the total compression. The time budget is badly allocated.

## Implemented level redesign

Based on the analysis above, the following changes were made:

### Brute-force removed from Balanced through Aggressive

On photographic images, heuristic strategies at L6–L10 are within 0.1% of
brute-force. On screenshots the gap is 5–7%, but the bytes/ms efficiency is
still lower than the Refine phase. BF now only runs at Best (L12) and above.

| Level      | Previous time | New time | Savings |
|------------|--------------|----------|---------|
| Balanced   | 8.5s         | 5.7s     | −33%    |
| Thorough   | 9.7s         | 6.6s     | −32%    |
| High       | 10.5s        | 7.3s     | −30%    |
| Aggressive | 19.7s        | 13.7s    | −30%    |

### Obsessive removed

The full BF sweep found the same optimum as the standard ctx=5 configs on
every corpus tested. Obsessive added nothing over Crush. Simplified to:
Best → Crush → Maniac.

### Current level table

| Level      | Zenflate | BF configs    | Zopfli    |
|------------|----------|---------------|-----------|
| None       | L0       | —             | —         |
| Fastest    | L1       | —             | —         |
| Fast       | L4       | —             | —         |
| Balanced   | L6       | —             | —         |
| Thorough   | L8       | —             | —         |
| High       | L9       | —             | —         |
| Aggressive | L10      | —             | —         |
| Best       | L12      | ctx5/L1+L4    | —         |
| Crush      | L12      | ctx5/L1+L4    | adaptive  |
| Maniac     | L12+L6sc | ctx1–8/L1+L4  | adaptive  |

Changes from original:
- **BF removed from Balanced/Thorough/High/Aggressive** — heuristics are close enough
- **BF only at Best and above** — where users explicitly asked for maximum quality
- **Obsessive removed** — no measurable benefit over Crush
- **Maniac = the "I don't care about time" option** — full sweep, zopfli, L6 screening

### Open question: Thorough vs High

Thorough (L8) and High (L9) produce nearly identical output:
- gb82-sc: 3.18M vs 3.17M (0.3% difference)
- Similar time (6.6s vs 7.3s without BF)

Options to resolve: (a) Remove High entirely, (b) Move High to L10 to
create a meaningful gap before Aggressive, (c) Keep both for API stability.
