# Changelog

All notable changes to zenpng are documented here.

## [Unreleased]

## [0.1.4] - 2026-04-17

### Performance
- Skip the second full-file chunk scan that the zencodec decode path used
  to perform for `PngProbe` construction. `PngProbe::from_info` now builds
  the probe from decoder state in ~25 ns instead of re-parsing every chunk
  (~85 ns mean, ~635 ns on PNGs with many text chunks — ~17x speedup on
  that worst case). (85a8fdd)
- Use `memchr` to locate null-terminators in `tEXt`/`zTXt`/`iTXt` chunk
  keywords instead of byte-by-byte scans. (bb71c65)

### Added
- `PngProbe::from_info(&PngInfo)` constructor for building a probe from
  decoder-produced metadata with no extra I/O. (85a8fdd)
- `PngInfo::palette_size`, `PngInfo::compressed_data_size`, and
  `PngInfo::creating_tool` fields, populated as chunks are walked. (85a8fdd)
- Set `ColorAuthority::Cicp` on the output descriptor when a valid `cICP`
  chunk is present, so downstream consumers can prefer cICP over
  sRGB/gAMA/cHRM/iCCP signaling. (e8df40d)
- Accept `RGBX8_SRGB` and `BGRX8_SRGB` descriptors in encode dispatch; the
  padding byte is stripped and the pixels route through the 3-channel RGB
  encode path (one-shot and streaming `push_rows`). (40f9b13)
- Promote the output descriptor's `AlphaMode` from `Straight` to `Opaque`
  when decode synthesizes alpha for a source without an alpha channel
  (color_type 0/2, or color_type 3 without `tRNS`) in the
  `negotiate_and_convert` path. (a7f7649)

### Changed
- Migrate internal `ThreadingPolicy` usage to the `is_parallel()` helper
  from zencodec 0.1.18; use `Sequential`/`Parallel` in place of the
  deprecated `SingleThread`/`Unlimited` variants. (436445c)
- Refresh the fuzz lockfile to pull `zenpixels-convert` 0.2.8 (with
  `linear-srgb` 0.6.10), alongside `zencodec` 0.1.18 and `zenpixels` 0.2.8.
  (17abd2c)

### Fixed
- Silence i686 unused-import warnings emitted by `archmage`'s
  `#[autoversion]` proc-macro on `target_arch = "x86"`, while keeping
  x86_64/aarch64/wasm32 strict. (df7d745)

## [0.1.2] - 2026-04-01

### Streaming Encode (zencodec `Encoder` trait)

- **`push_rows()`/`finish()` streaming API** — encode PNG data incrementally
  without holding the entire decoded image in memory at once.
- **Effort 0: true streaming** — rows emit stored DEFLATE blocks on arrival.
  No intermediate pixel buffer. Peak memory ~1x output size.
- **Effort 1: pre-filtered streaming** — Paeth filter applied per-row on arrival,
  compressed in `finish()`. Peak memory ~2x image (filtered + compress_bound).
- **Effort 2+: buffered** — raw pixels accumulated, full encode in `finish()`.
  Equivalent to one-shot `encode()`.

### Encoding

- 32-effort compression pipeline (effort 0–200) with named presets from
  `None` through `Minutes`
- 4-phase progressive engine: screen → refine → brute-force → recompress
- 9 filter strategies (5 single + 4 adaptive heuristics)
- BruteForce and BruteForceFork per-row filter selection
- Beam search filter optimization
- Transparent pixel zeroing for RGBA
- Auto-indexed encoding via `encode_auto()` with pluggable quantizer backends
  (zenquant, imagequant, quantette) and perceptual quality gates (MaxDeltaE,
  MaxMpe, MinSsim2)
- APNG encoding with 6-way dispose/blend optimization and temporal palette
  consistency
- Metadata preservation (sRGB, gAMA, cHRM, cICP, iCCP, eXIf, XMP)
- 16-bit and float input via `bytemuck` + `linear-srgb` SIMD batch conversion

### Decoding

- Streaming row-by-row decode for non-interlaced PNGs
- Adam7 interlaced decode
- SIMD-accelerated unfiltering (Sub, Up, Avg, Paeth) via archmage dispatch
- APNG frame decode with `with_start_frame_index` support
- Configurable checksum verification (CRC-32, Adler-32) — skipped by default
- `PngProbe` with `SourceEncodingDetails` (compression analysis, creating tool
  detection, bits-per-pixel, palette size)
- `PngLimits` for pixel count, memory, output size, and frame count enforcement

### Robustness

- Fuzz targets for decoder
- Validated against 47,366-image corpus (zero pixel mismatches vs `png` crate)
- Overflow-safe IHDR computation (wasm32-safe)
- Non-panicking error paths (`Result` over `.expect()`)
- `ResourceLimits` enforcement for output size, input size, and APNG frames
