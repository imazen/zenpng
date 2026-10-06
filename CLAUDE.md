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
  - `postprocess.rs` — `RowExpander` (built once per image: palette/sub-byte lookup tables, tRNS, 16-bit byte swap; expands raw rows straight into the output buffer), `OutBuf`, build_pixel_buffer/build_pixel_data
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
- `src/zencodec.rs` — zencodec trait integration

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

(x86 figures above are old and predate the const-generic fixed kernels.)

**ARM (Neoverse-N1, `benches/unfilter_tiers.rs`, 2026-10-06, vs the fixed
kernels):** NEON wins Sub bpp=4 (2.06×), Avg bpp=4 (2.66×) and Paeth bpp=4
(1.31×); the fixed kernel wins Sub bpp=3 (1.26×) and Paeth bpp=3 (1.09×); Up is
autovectorised scalar (NEON kernel 0.66×). Production routing follows those
numbers. `benchmarks/unfilter_tiers_arm_2026-10-06.txt`. (The 2026-05-29 notes
measured NEON against the pre-fixed-kernel scalar path.)

### Pixel sizes without SIMD kernels

Every pixel size without a SIMD kernel (see tier table) uses the const-generic kernels in `src/simd/fixed.rs`
(`sub::<N>`, `avg::<N>`, `paeth::<N>` over `[u8; N]` chunks, dispatched by
`by_bpp!`). `paeth_branchless` is the stb formulation, checked exhaustively
against the spec predictor over all 2^24 inputs.

### SIMD Tier Assignments

- **Paeth**: bpp=4 `[v2, neon, wasm128]`; bpp=3 `[wasm128]`, else fixed kernel
- **Sub**: bpp=4 `[v1, neon, wasm128]`; bpp=3 `[wasm128]`, else fixed kernel
- **Up**: `[v3, v1, wasm128]`; aarch64 uses the autovectorised scalar loop
- **Avg**: bpp=4 `[v1, neon, wasm128]`; all other bpp use the fixed kernel

The x86/NEON bpp=3 Sub and Paeth kernels were deleted 2026-10-06: the fixed
kernel matched or beat them on both ISAs (`benchmarks/unfilter_tiers_*_2026-10-06`).

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

- **Callgrind** (old, single Paeth-heavy image): Paeth scalar = 36.8% of instructions; SIMD Paeth = 15.0% (2.5x reduction).
- **Callgrind, 2026-10-05** (imazen-26 image 9097 after the RowExpander/fixed-kernel work): zenflate
  `StreamDecompressor::fill` is ~70% of instructions. Inflate, not unfiltering, is now the decode
  bottleneck. `examples/inflate_bench.rs` measures fdeflate (what image-png uses) 1.04–1.6× faster than
  zenflate streaming on PNG IDAT data; closing that gap is a zenflate change. (An earlier note here
  claimed "zenflate inflate = 0.5%"; that was wrong for any compressed image.)
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

### Comparing against image-png

`benches/vs_png.rs` (zenbench) decodes and encodes against image-rs/image-png
main (pinned git rev in `[dev-dependencies] png_main`) and asserts both decoders
produce identical 8-bit output before timing. Inputs come from
`ZENPNG_BENCH_DIR`, named `<id>_<format>_<longedge>.png`:

```bash
ZENPNG_BENCH_DIR=~/tmp/mtpng/bench_in taskset -c 2 cargo bench --bench vs_png --features _dev -- --group=decode
```

Results: `benchmarks/vs_png_*.{txt,meta}`. ARM NEON vs scalar per filter:
`cargo bench --bench unfilter_tiers --features _dev` (aarch64 only).

### Testing

```bash
cargo test --features _dev     # all tests including SIMD tier permutations
cargo test -- simd             # SIMD tests only
```

Each filter has `for_each_token_permutation` tests that verify byte-exact match against scalar reference at all dispatch tiers.

**32-bit (`usize` = u32) coverage.** CI runs `cross test --target
i686-unknown-linux-gnu` and `cargo test --lib --target wasm32-wasip1`. Locally,
i686 *test* builds need a 32-bit C toolchain for the `libdeflater` dev-dep
(`cargo clippy --target i686-unknown-linux-gnu --lib` still typechecks the
library); the runnable 32-bit proxy is wasm32:
`CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime --dir ." cargo test --lib --target wasm32-wasip1`.
Size math derived from an IHDR must be checked and bounded by
`alloc_util::alloc_len` / `stream_capacity` (`isize::MAX`, the `Vec` ceiling):
a gray8 row at the PNG max width is exactly `isize::MAX` bytes on 32-bit, so
`stride * 2` wrapped there (2026-08-27, `huge-ihdr-gray8-2147483647sq.png`).
Gate width-specific expectations with `#[cfg(target_pointer_width = ...)]`
test pairs, not a runtime `cfg!()` branch — an out-of-range `usize` literal in
the dead branch is still a compile error.

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

With Adler-32 verification on, every row loop (serial, interlaced, APNG frame,
codec streaming) calls `RowDecoder::finish_stream` / `drain_stream` after the
last row so the zlib footer is reached even when the image ended earlier
(trailing data, large inflate buffer). Regression test:
`strict_decode_verifies_adler_after_last_row`.

## Compression Effort Design

Effort 0-30: standard pipeline. Effort 31+: full pipeline + FullOptimal.
`Compression::Effort(u32)` for fine-grained control, or named presets:

| Preset | Effort | Strategies | Pipeline | zenflate |
|---------|--------|------------|----------|----------|
| None | 0 | 1: None | store | Store |
| Fastest | 1 | 1: Paeth | screen-only | Turbo |
| Turbo | 2 | 3: MINIMAL | screen-only | Turbo |
| Fast | 7 | 5: FAST | screen-only | FastHt |
| Balanced | 13 | 9: HEURISTIC | screen@7 + refine@17 | Lazy |
| Thorough | 17 | 9: HEURISTIC | screen@7 + refine@[20,22] + BF(3,1) | Lazy2 |
| High | 19 | 9: HEURISTIC | screen@7 + refine@[22,24] + BF(3,1) | Lazy2 |
| Aggressive | 22 | 9: HEURISTIC | screen@7 + refine@[26,28] | NearOptimal |
| Intense | 24 | 9: HEURISTIC | screen@7 + refine + BF(5,1) + BFF[10] | NearOptimal |
| Crush | 27 | 9: HEURISTIC | screen@7 + refine + BF + BFF + AF + beam | NearOptimal |
| Maniac | 30 | 9: HEURISTIC | screen@7 + refine + full BF/BFF/AF/beam | NearOptimal |
| Brag | 31 | 9: HEURISTIC | full Maniac + FullOptimal 15i | FullOptimal |
| Minutes | 200 | 9: HEURISTIC | full Maniac + FullOptimal 184i | FullOptimal |

### Effort 31+ tiers

| Tier | Effort | Pipeline | Recompressor |
|------|--------|----------|-------------|
| Extended | 31-45 | Full e30 pipeline + FullOptimal | FullOptimal (effort-16)i |
| Medium | 46-60 | Full pipeline + BF + FullOptimal | FullOptimal + zenzop |
| Full | 61+ | Full Maniac + all BF variants | FullOptimal + zenzop |

E31-45 runs the complete e30 pipeline (screening, refinement, BF, BFF, beam)
plus FullOptimal recompression. This guarantees monotonicity vs e30 while
adding iterative forward DP for further compression. Previously used a
lean BFF-only pipeline, but benchmarking showed it regressed on 49% of images
due to correlation mismatch between BFF's Greedy-eval filter selection and
FullOptimal's compression.

### 4-phase pipeline (`src/encoder/compress.rs`)

`EffortParams::from_effort()` maps effort → all pipeline parameters:

1. **Phase 1 — Screen**: Apply filter strategies, compress at `screen_effort`.
   Low effort (0-7): screen IS final pass (no Phase 2).
2. **Phase 2 — Refine**: Top-K candidates re-compressed at `refine_efforts` via
   `try_compress_with_fallbacks()`, which follows zenflate's `monotonicity_fallback()`
   chain automatically.
3. **Phase 3 — BruteForce**: Per-row brute-force filter selection (effort 23+).
   BruteForceFork maintains actual DEFLATE state across rows (effort 24+).
4. **Phase 4 — Recompress**: At effort 28-30: zopfli adaptive with time budgeting.
   At effort 31+: NearOptimal + FullOptimal (+ optional zenzop).

### Filter strategy sets (`src/encoder/filter.rs`)

- **MINIMAL** (3): None, Paeth, Adaptive(Bigrams) — effort 2-3
- **FAST** (5): None, Paeth, Adaptive(MinSum, Bigrams, Entropy) — effort 4-9
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
Implemented in `compress_filtered()`. For 8-bit RGBA rows only, zeroes RGB channels of
fully-transparent pixels (`alpha == 0 → [0,0,0,0]`) before filtering/compression.
Quick `has_any_transparent_pixel()` scan avoids copying when no transparent pixels exist.
Creates runs of identical bytes that compress significantly better. No visible-pixel
impact, but the RGB bytes under `alpha == 0` are NOT preserved even in lossless mode
(documented on `encode_rgba8` and in the README; no `exact`-style opt-out exists yet).

Eligibility is keyed on `RowFormat { bpp, rgba8 }` (built via `RowFormat::from_png(
color_type, bit_depth)` / `::truecolor8(bpp)` / `::INDEXED`), NOT on `bpp == 4` —
GA16 is also 4 bytes/pixel and was corrupted by the old gate (see Known Issues).
Paths that skip it: effort 0 (stored blocks) and the zencodec `push_rows` effort-1
pre-filtered streaming path (`codec.rs` `PreFilteredState`), which store rows as
supplied — so buffered vs streaming encodes of the same RGBA8 input can differ in
the hidden RGB under `alpha == 0`.

### Auto-indexed encoding
`encode_auto()` and `encode_apng_auto()` quantize via any `Quantizer` backend and check a
`QualityGate` to decide indexed vs truecolor. Three gate types:

| Gate | Scale | Good default | Meaning |
|------|-------|-------------|---------|
| `MaxDeltaE(0.02)` | 0.0 – ∞ | 0.02 | Mean OKLab ΔE (lower = stricter) |
| `MaxMpe(0.008)` | 0.0 – ∞ | 0.008 | Masked perceptual error calibrated to butteraugli/SSIM2 |
| `MinSsim2(85.0)` | 0 – 100 | 85.0 | Estimated SSIMULACRA2 score (higher = stricter) |

`AutoEncodeResult` exposes `quality_loss` (OKLab ΔE), plus optional `mpe_score`,
`ssim2_estimate`, `butteraugli_estimate` (populated when `MaxMpe`/`MinSsim2` gate used).

APNG indexed path uses `build_palette_rgba()` for a shared palette across all frames,
then `remap_rgba_with_prev()` for temporal consistency (static pixels get identical
indices across frames, eliminating flicker). Delta regions are computed on index buffers
directly (1 byte/pixel) rather than RGBA pixels. `encode_apng_auto()` checks the quality
gate per frame and bails to truecolor if any frame fails, reporting worst-case metrics.

### 6-way APNG dispose/blend optimization
Implemented in `src/encoder/apng.rs`. Evaluates all 6 dispose/blend combinations
(3 dispose × 2 blend) per frame using greedy 1-step lookahead. For each frame:
1. Build SOURCE and OVER candidate subframes
2. Trial-compress both at effort 2 (Paeth + Turbo, ~5ms each)
3. For each of 3 dispose options, evaluate next frame's best candidate
4. Pick (dispose, blend) minimizing current_size + next_frame_best_size

Active when effort > 2 and >1 frame. Per-frame overhead: ~8 trial compressions × ~5ms.
Canvas state tracks compress_filtered's transparent pixel zeroing via
`zero_transparent_rgb_region()` to maintain decoder/optimizer consistency.
BLEND_OP_OVER safety: `can_use_over_truecolor/indexed()` verifies all changed pixels
have target_alpha==255 or canvas_alpha==0.

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

1. ~~**Transparent pixel zeroing**~~ — done (compress_filtered)
2. ~~**6-way dispose/blend optimization**~~ — done (optimize_apng_truecolor/indexed)
3. **Color type downconversion** — RGBA→RGB when opaque (25% savings), grayscale detection
4. **Duplicate frame merging** — combine identical consecutive frames
5. ~~**Exact-palette detection**~~ — done (try_build_exact_palette)

## Apple `iDOT` parallel PNG

Implemented decode + encode; see `docs/IDOT_PARALLEL_PNG.md`. Layout: `u32 N`
then N×`{first_row, row_count, offset}` (offset from the iDOT chunk's length
field). Decoder: `src/decoder/idot.rs` (validate, work-queue workers, fallback
to serial on anything unproven), `src/affinity.rs` (Linux core-tier pinning).
Encoder: `src/encoder/segments.rs`. The parallel path must stay
byte-identical to serial — `tests/idot.rs` runs the codec-corpus `png-idot`
set (real Apple, Buchanan adversarial, 26 generated). Thresholds:
`idot::workers_for_bytes` (2 workers ≥ 2 MiB, +1 per 4 MiB). Bench:
`examples/idot_bench.rs`, `examples/idot_encode.rs` (`--features _dev`;
`ZENPNG_PIN`, `ZENPNG_IDOT_MIN_BYTES`, `ZENPNG_IDOT_TRACE` overrides).
zenflate is `[patch.crates-io]`'d to imazen/zenflate#9 until released.
Apple ImageIO's own iDOT path is buggy (boundary Up/Avg/Paeth rows, gapped
tables, 1/2/4-bit gray); the encoder avoids all three. Mac tooling:
`tests/fixtures/idot/mac/` (`ssh mac`, macOS 27; `log` is a zsh builtin there,
use `/usr/bin/log`).

## Known Issues

- **Fixed 2026-08-27 — GA16 corrupted by RGBA8 transparent zeroing.** The
  `compress_filtered` zeroing gate was `bpp == 4`, which 16-bit gray+alpha also
  satisfies; a GA16 pixel `[G_hi, G_lo, A_hi, A_lo]` with alpha low byte 0 had
  `G_hi, G_lo, A_hi` wiped (gray 0x1234 / alpha 0xFF00 → gray 0 / alpha 0x0000).
  Unreachable from the public API (no GA16 encode entry; `optimize_16bit` never
  emits GA16) but live at `write_truecolor_png(4, 16)`. Now keyed on `RowFormat`;
  regression tests `truecolor_png_ga16_alpha_low_byte_zero_roundtrips_byte_exact`
  and `..._rgba16_...` in `src/encoder/mod.rs`. If a GA16 encode entry point or
  `GrayAlpha16` encode descriptor is ever added, those tests are the gate.
- **Open (design, not a defect):** buffered RGBA8 encodes zero RGB under
  `alpha == 0`; the `push_rows` effort-1 streaming path does not. Both decode to
  the same visible image. Resolving it either way needs the `exact` knob decision.
