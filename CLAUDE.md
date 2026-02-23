# zenpng

PNG encoder/decoder with SIMD-accelerated unfiltering and zenflate decompression.

## Architecture

- `src/chunk/` — PNG chunk parsing, iteration, writing
  - `mod.rs` — PNG_SIGNATURE, ChunkRef, ChunkIter (zero-copy chunk iteration)
  - `ihdr.rs` — Ihdr struct, parsing, validation
  - `ancillary.rs` — PngAncillary (PLTE, tRNS, gAMA, sRGB, cHRM, cICP, iCCP, eXIf, XMP, acTL)
  - `write.rs` — write_chunk() with CRC computation
- `src/decoder/` — PNG decode pipeline
  - `mod.rs` — decode orchestration (probe_png, decode_png, PngInfo construction) + all tests
  - `row.rs` — IdatSource, RowDecoder (streaming row-by-row decompress + unfilter)
  - `postprocess.rs` — post_process_row, OutputFormat, build_pixel_data, color conversion
  - `interlace.rs` — Adam7 pass constants, decode_interlaced
- `src/encoder/` — PNG encode pipeline
  - `mod.rs` — CompressOptions, PhaseStat/PhaseStats, write_indexed_png, write_truecolor_png
  - `filter.rs` — Filter strategies (Single, Adaptive, BruteForce, BruteForceBlock)
  - `compress.rs` — Progressive 4-phase compression engine
  - `metadata.rs` — PngWriteMetadata, chunk serialization (gAMA, sRGB, cHRM, iCCP, cICP, etc.)
- `src/simd/` — SIMD-accelerated unfiltering (Sub, Up, Avg, Paeth)
- `src/decode.rs` — Public decode API facade
- `src/encode.rs` — Public encode API facade
- `src/error.rs` — Error types
- `src/zencodec.rs` — zencodec-types trait integration

### Dependencies

- **zenflate** (`../zenflate`) — deflate decompression (port of libdeflate to safe Rust)
- **archmage** — SIMD dispatch framework (`#[arcane]` entry points, `#[rite]` inlined helpers, `incant!` tier dispatch)
- **safe_unaligned_simd** — Safe wrappers for unaligned SIMD loads/stores

## SIMD Unfilter (`src/simd/`)

### Dispatch

Filters are per-row in PNG. `incant!` dispatches once per row to the highest available SIMD tier. Inner loops process all pixels. No per-pixel dispatch overhead.

### Filter Performance (real images, isolated micro-benchmark)

| Filter | bpp=3 (RGB) | bpp=4 (RGBA) | Notes |
|--------|-------------|--------------|-------|
| Paeth  | 1.60x       | 2.12x        | Branchless i16 predictor (SSE4.2/V2) |
| Sub    | ~1.0x       | 1.20x        | Sequential dependency limits gains |
| Up     | ~1.0x       | ~1.0x        | LLVM auto-vectorizes scalar equivalently |
| Average| ~0.95x      | ~1.0x        | bpp=3 SIMD was slower, reverted to scalar |

### SIMD Tier Assignments

- **Paeth**: `[v2]` (SSE4.2) for both bpp=3 and bpp=4
- **Sub**: `[v1]` (SSE2) for both bpp=3 and bpp=4
- **Up**: `[v3, v1]` (AVX2 + SSE2 fallback)
- **Avg**: `[v1]` (SSE2) for bpp=4 only; all other bpp is scalar

### Codegen Patterns

For bpp=3 (3-byte pixels), use `copy_from_slice` for stores:
```rust
// GOOD: single bounds check, compiles to word+byte store
let val = (_mm_cvtsi128_si32(result) as u32).to_le_bytes();
row[i..i + 3].copy_from_slice(&val[..3]);

// BAD: 3 bounds checks + stack spill
row[i] = bytes[0];
row[i + 1] = bytes[1];
row[i + 2] = bytes[2];
```

AVX-512 V4 masked stores (`_mm_mask_storeu_epi8`) were tested for bpp=3 — no improvement over V2/V1 paths.

### Profiling Results

- **Callgrind**: Paeth scalar = 36.8% of instructions; SIMD Paeth = 15.0% (2.5x reduction). zenflate inflate = 0.5%.
- **Cachegrind**: L1 data miss rate 2.1%, LL near zero. Unfilter is cache-friendly.
- **Heaptrack**: ~7 heap allocations per decode call. No per-row allocations.
- **Buffer alignment**: Standard `Vec<u8>`, unaligned SIMD loads. Not worth aligned allocation (4-byte loads rarely cross cache lines, Up is 0.3% of instructions).

## Development

### Benchmarking

```bash
cargo run --release --example decode_bench --features _dev [-- /path/to/image.png]
```

Default test image: `frymire-srgb.png` (RGB, bpp=3). Also test with RGBA images for bpp=4 paths.

The `_dev` feature enables `archmage/testable_dispatch`, allowing `Sse2Token::dangerously_disable_token_process_wide(true)` to force scalar fallback even for compile-time guaranteed SSE2.

### Testing

```bash
cargo test --features _dev     # all tests including SIMD tier permutations
cargo test -- simd             # SIMD tests only
```

Each filter has `for_each_token_permutation` tests that verify byte-exact match against scalar reference at all dispatch tiers.

## Decode Checksum Options

Checksums are **skipped by default** for maximum decode speed.
- `PngDecodeConfig::default()` / `none()` / `lenient()` — skip both CRC-32 and Adler-32
- `PngDecodeConfig::strict()` — verify both CRC-32 and Adler-32
- `skip_decompression_checksum: bool` — skip Adler-32 verification (default: true)
- `skip_critical_chunk_crc: bool` — skip CRC-32 verification (default: true)

When CRC is skipped, computation is entirely elided. When Adler-32 is skipped
in the streaming path, zenflate still computes it but tolerates mismatches
(emits `DecompressionChecksumSkipped` warning). The stored-block fast path
skips Adler-32 computation entirely.

## Compression Effort Design

Unified 0-30 effort scale. Each ~3 effort points roughly doubles time.
`Compression::Effort(u32)` for fine-grained control, or named presets:

| Preset | Effort | Strategies | Pipeline | zenflate |
|---------|--------|------------|----------|----------|
| None | 0 | 1: None | store | Store |
| Fastest | 2 | 1: Paeth | screen-only | Turbo |
| Fast | 6 | 5: FAST | screen-only | FastHt |
| Balanced | 10 | 9: HEURISTIC | screen@7 + refine@12 | Lazy |
| Thorough | 13 | 9: HEURISTIC | screen@7 + refine@17 | Lazy |
| High | 16 | 9: HEURISTIC | screen@7 + refine@[20,22] | Lazy2 |
| Aggressive | 20 | 9: HEURISTIC | screen@7 + refine@[24,26] | NearOptimal |
| Best | 24 | 9: HEURISTIC | screen@7 + refine + BF(5,1) | NearOptimal |
| Crush | 28 | 9: HEURISTIC | screen@7 + refine + BF sweep + zopfli | NearOptimal |
| Maniac | 30 | 9: HEURISTIC | screen@7 + refine + BF sweep + zopfli max | NearOptimal |

### 4-phase pipeline (`src/encoder/compress.rs`)

`EffortParams::from_effort()` maps effort → all pipeline parameters:

1. **Phase 1 — Screen**: Apply filter strategies, compress at `screen_effort`.
   Low effort (0-7): screen IS final pass (no Phase 2).
2. **Phase 2 — Refine**: Top-K candidates re-compressed at `refine_efforts` via
   `try_compress_with_fallbacks()`, which follows zenflate's `monotonicity_fallback()`
   chain automatically.
3. **Phase 3 — BruteForce**: Per-row brute-force filter selection (effort 24+).
   BruteForceBlock permanently disabled (slower AND larger than per-row).
   BruteForceFork maintains actual DEFLATE state across rows (effort 26+).
4. **Phase 4 — Zopfli**: Zopfli adaptive with time budgeting (effort 28+).

### Filter strategy sets (`src/encoder/filter.rs`)

- **MINIMAL** (3): None, Paeth, Adaptive(Bigrams) — effort 3-4
- **FAST** (5): None, Paeth, Adaptive(MinSum, Bigrams, Entropy) — effort 5-9
- **HEURISTIC** (9): All 5 Singles + Adaptive(MinSum, Entropy, Bigrams, BigEnt) — effort 10+

BigEnt excluded from FAST — 30-170x slower than MinSum (256KB memset + 65536-entry
iteration per row). Only used in HEURISTIC tier where screen cost is dwarfed by refine.

### Filter precomputation optimization

When multiple strategies share the same 5 PNG filter variants (Single/Adaptive),
all 5 are computed once via `precompute_all_filters()` and shared across strategies
via `filter_image_from_precomputed()`. Capped at 64 MiB. Saves 5× filter passes
per additional adaptive strategy. Result: 2-3x screening speedup at effort 5+.

### Sparse heuristic tracking

`HeuristicScratch` tracks which buffer entries were modified:
- `bigrams_score`: sparse word tracking → reset only touched entries (no 8KB fill(0))
- `bigram_entropy_score`: sparse key tracking → compute entropy only on nonzero entries,
  reset during computation (no 256KB fill(0) or 65536-entry iteration)
- `new_universal()`: pre-allocates for BigEnt (the largest heuristic), reusable across all

### Monotonicity via zenflate fallback chain

Higher effort must never produce larger output. Enforced by
`zenflate::CompressionLevel::monotonicity_fallback()` — a caller-driven API where each
compression call follows the chain: NearOpt→Lazy2 max(e22)→Lazy max(e17)→Greedy max(e10)→
FastHt max(e9). `try_compress_with_fallbacks()` wraps this automatically.
Screen effort stays at FastHt (≤9) for consistent candidate ranking.
Turbo→FastHt always improves (zenflate guarantee), no fallback needed below e10.

### Filter performance (measured, effort_timing.rs)

| Filter type | Screenshot (RGBA 1356×1132) | Photo (RGB 512×512) |
|------------|---------------------------|---------------------|
| Single (None/Sub/Up/Avg/Paeth) | 275-519 MP/s | 350-650 MP/s |
| Adaptive(MinSum) | 86-171 MP/s | 100-200 MP/s |
| Adaptive(Entropy) | ~80 MP/s | ~120 MP/s |
| Adaptive(Bigrams) | ~60 MP/s | ~90 MP/s |
| Adaptive(BigEnt) | **3 MP/s** | **1 MP/s** |

At low effort, filter cost dominates (89% of screening time on screenshots).
Turbo zenflate compress costs 1.6-3.4ms per strategy — negligible next to filters.

## Pending Encoder Optimizations

### Transparent pixel zeroing
Implemented in `compress_filtered()`. For RGBA8 (bpp=4), zeroes RGB channels of
fully-transparent pixels (`alpha == 0 → [0,0,0,0]`) before filtering/compression.
Quick `has_any_transparent_pixel()` scan avoids copying when no transparent pixels exist.
Creates runs of identical bytes that compress significantly better. No quality impact.

### Auto-indexed encoding via zenquant
Already implemented: `encode_rgba8_auto()` quantizes via zenquant, checks OKLab ΔE against
`max_loss` threshold, uses indexed PNG if quality permits, falls back to truecolor.
`encode_apng_auto()` does the same for APNG with a shared global palette.

zenquant also exposes SSIM2-estimated quality via `compute_quality_metric(true)` and
`min_ssim2()` / `target_ssim2()`. The quality gate could optionally use SSIM2 instead of
(or alongside) OKLab ΔE for tighter perceptual control.

### 6-way APNG dispose/blend optimization (not yet implemented)
zenpng's APNG encoder hardcodes `dispose_op=NONE` + `blend_op=SOURCE` on every frame
(`src/encoder/apng.rs` lines 192-193, 251-252). apngasm evaluates all 6 combinations per
frame transition:

- **3 dispose ops** × **2 blend ops** = 6 candidates per frame
  - Dispose None: previous frame stays as-is
  - Dispose Background: clear previous frame's rect to transparent before compositing
  - Dispose Previous: restore the frame before the previous one
  - Blend Source: overwrite pixels in the region
  - Blend Over: alpha-composite (unchanged pixels → fully transparent → compresses well)

apngasm trial-compresses each candidate at low quality (zlib L2), picks the smallest, then
re-compresses the winner at L9. The Blend Over path is particularly valuable: unchanged pixels
become `[0,0,0,0]` (transparent), creating long zero runs that deflate loves. Dispose
Background/Previous matter when animations cycle between states or have recurring backgrounds.

Implementation approach: for each frame i, build 6 candidate subframes from the 6
dispose/blend combos, trial-compress each at L1-L2 with zenflate, pick the winner, then
compress at the target level. Needs a "prev-prev" frame buffer for Dispose Previous. The
trial compression is cheap (same as Phase 1 screening in `compress.rs`). This is the single
biggest APNG compression win available — can reduce APNG file sizes by 20-50% on typical
animations.

### APNG color type downconversion (not yet implemented)
zenpng hardcodes RGBA8 (`color_type=6`) for all APNG truecolor output. apngasm's
`downconvertOptimizations()` analyzes all frames and reduces to the minimal color type:

- **RGBA → RGB** when all pixels are fully opaque (25% raw data reduction)
- **RGBA → Grayscale** when all pixels are gray + simple transparency
- **RGBA → GrayAlpha** when all pixels are gray but need alpha
- **RGBA/RGB → Palette** when ≤256 unique colors across ALL frames (no quantization)
- **Palette cleanup**: remove unused entries, sort by alpha then frequency

The RGBA→RGB case alone is significant — most animations are fully opaque, and dropping
the alpha channel saves 25% before compression even starts. Implementation: scan all frames
for `alpha < 255`, if none found emit as RGB (color_type=2). For grayscale detection, check
`r == g == b` on all pixels. For exact-palette, count unique colors across all frames.

### APNG duplicate frame merging (not yet implemented)
apngasm's `duplicateFramesOptimization()` detects consecutive identical frames and merges
them by summing delays (GCD-simplified fraction). Eliminates redundant frame data entirely.
Common in animations with "hold" frames. Simple pixel comparison + delay arithmetic.

### apngasm comparison (analyzed 2026-02-22)
apngasm uses zlib L9 (no zopfli/libdeflate), 2-strategy filter selection (DEFAULT vs FILTERED),
and no quantization (palette only when image already has ≤256 exact colors). zenpng already
dominates on per-frame compression: zenflate L12 > zlib L9, 9 heuristic + 3 brute-force
strategies, zenquant perceptual quantization. Optimizations worth adopting from apngasm:

1. **Transparent pixel zeroing** — zero RGB on alpha==0 pixels
2. **6-way dispose/blend optimization** — trial-compress all 6 combos per frame
3. **Color type downconversion** — RGBA→RGB when opaque (25% savings), grayscale detection
4. **Duplicate frame merging** — combine identical consecutive frames
5. **Exact-palette detection** — use indexed color when ≤256 unique colors across all frames (no quantization needed)

## Known Issues

None currently.
