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

## Compression Level Design

See `TUNING.md` for empirical analysis. Key decisions:
- Brute-force filter selection only at Best (L12) and above
- Obsessive level removed (identical output to Crush)
- Block-wise brute-force permanently disabled (slower AND larger than per-row)
- Zopfli adaptive with time budgeting at Crush and Maniac

## Known Issues

None currently.
