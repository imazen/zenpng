# Changelog

All notable changes to zenpng are documented here.

## [Unreleased]

### Added
- **One-shot convenience functions: `encode_rgba8_bytes` + `decode_rgba8`.**
  Purely-additive top-level free functions for the two most common jobs in a
  single call: `encode_rgba8_bytes(&rgba, width, height) -> Result<Vec<u8>>`
  (tightly-packed RGBA8 → PNG at the default `EncodeConfig`) and
  `decode_rgba8(&png) -> Result<(Vec<u8>, u32, u32)>` (any PNG → packed RGBA8 +
  dimensions, normalizing grayscale / indexed / RGB / 16-bit sources to 8-bit
  RGBA). The byte-slice encoder is named `_bytes` because the typed
  `encode_rgba8(ImgRef<Rgba<u8>>, …)` already occupies that name; both wrap the
  existing builder path, so the full power API (custom filter/effort, metadata,
  16-bit, cancellation) is unchanged. README quick-start now leads with these;
  each fn carries that round-trip as a runnable doctest.
- **Palette/quantize axis on the sweep plan (`sweep::QuantizeSpec` +
  `QuantBackend`).** `SweepVariant` gains an optional `quantize` stratum:
  `None` = truecolor lossless (unchanged), `Some(spec)` = palette-reduce to
  `max_colors` via `Imagequant` (feature `imagequant`) or `Zenquant` (feature
  `quantize`). The axis is a **union**, not a cross — `SweepAxes::modes_full`
  and `scalar_dense` now carry the lossless compression cells PLUS 8 mandatory
  quantize cells (both backends × `{256,128,64,32}` colors) at the default
  `Balanced` compression, so `modes_full` is **17 cells** (9 truecolor + 8
  palette). Cell ids suffix `-iq{N}` / `-zq{N}` (e.g. `png-balanced-iq256`),
  roundtrip through `variant_from_cell_id`, and fingerprint distinctly (backend
  + color count fold into the hash). New `SweepVariant::encode_png` performs the
  encode (truecolor or indexed); the quantize arms are feature-gated and error
  (never silently truecolor) when the backend feature is off. This is the data a
  PNG picker needs to choose palette quantization.

### Changed
- Docs: split README into a GitHub surface (`README.md`) and a generated
  crates.io surface (`README.crates.md`, no badges); refreshed for the
  `heuristics` resource-estimation, `detect` source-analysis, `cms`/`unchecked`,
  and native `zencodec` Fidelity APIs; added `benchmarks/README.md` and the
  canonical crosslink footer.

### Fixed
- **encode peak-memory estimate is now admission-gating-safe (never under-
  predicts).** Admission control gates on `EncodeEstimate::peak_memory_bytes`
  (the `typ` field), so it must be a safe upper bound. A VmHWM re-sweep
  (`mem_probe_encode`, sizes {256,512,1024,2048} × effort {1,6,13,19,24,30} ×
  {photo,screenshot} × {1,4} threads, RGB8) found the 2026-06-14 anchors under-
  predicted **29 / 96 cells** in two bands: (1) the **default 4-thread** filter-
  strategy screening added working set that `ResourceEstimate::at_cores` does NOT
  fold into peak memory (worst `256² e13 4-thread` only 71 % covered), and (2)
  **Maniac (e30)** zopfli/FullOptimal buffers under-predicted at every size, even
  single-thread (`2048² e30` 510→ needs 522 MiB). Raised `ENCODE_FIXED_OVERHEAD`
  6→8 MiB and `ENCODE_BPP_ANCHORS` to `(1,18)(6,57)(13,102)(19,124)(24,125)
  (30,180)` with ~10 % margin: post-fit worst safety ratio 1.04, **0 cells
  under-predicted**, loosest 2.26× (the est may be loose, never short). Added a
  `typ_never_under_predicts_measured_peak` regression test pinned to the measured
  VmHWM peaks. heaptrack corroborated peak-heap≈VmHWM on 3 cells. Provenance:
  `benchmarks/zenpng_encode_mem_2026-06-23.tsv`. Also commits the
  `examples/mem_probe_encode.rs` encode probe used for the sweep. `_max` (1.8×)
  ceiling and the effort-anchor / alpha / 16-bit structure are unchanged.

### Changed
- **deps: migrate to published `zencodec 0.1.24` estimate API; drop the temporary
  git-rev patch.** Removed the `[patch.crates-io]` zencodec git-rev pin (0f71295)
  now that `zencodec 0.1.24` is on crates.io. Updated the
  `estimate_encode_resources` mapping for the refined `ResourceEstimate`:
  `new(peak, wall_ms: u64)` (was `f32`), `with_peak_max(max)` (the `min` arg is
  gone), dropped the removed `with_output_bytes`, and migrated
  `heuristics::encode_threading_info` to the new 1-arg
  `ThreadingInformation::parallel(max_efficient_threads)` (the `fraction` /
  `mem-per-thread` args are gone).

### Added
- honor `ResourceLimits::prefer_fallible_allocations` (`AllocPreference`, 3-mode
  per-site) at untrusted decode allocations. Big, untrusted-sized full-image
  buffers default to the fallible `try_reserve` path (graceful
  `PngError::LimitExceeded` on OOM); small bounded per-row scratch defaults to
  the faster infallible `vec!`. `Fallible`/`Infallible` force one path
  everywhere; `CodecDefault` (the default) keeps each site's own default, so the
  direct `decode()` API is unchanged. New internal `alloc_util` helpers
  (`resolve_fallible` / `alloc_zeroed` / `vec_with_capacity`).
- implement `estimate_decode_resources` on `PngDecoderConfig` (overrides the
  `zencodec::DecoderConfig` default) — maps `heuristics::estimate_decode` to a
  core-adjusted `ResourceEstimate` with `ThreadingInformation::SERIAL` (PNG
  decode is a serial DEFLATE inflate).
- vCPU-aware resource estimation via zencodec's unified `estimate` API:
  `PngEncoderConfig::estimate_encode_resources(&ImageCharacteristics, &ComputeEnvironment)`
  (overrides the `zencodec::EncoderConfig` default) returns a core-adjusted
  `ResourceEstimate`. `heuristics::encode_threading_info(effort)` now returns
  the shared `zencodec::estimate::ThreadingInformation` (replacing the
  short-lived local `ThreadingInfo` copy + `estimate_encode_threaded`).
- `InternalParams` cross-codec bundle (`__expert`). `zenpng::internal_params::InternalParams`
  (`compression` + `parallel`, both `Option<_>`) + `EncodeConfig::with_internal_params`,
  gated behind the new pure-visibility `__expert` feature — mirrors `zenjpeg`'s bundle so
  one picker model drives every zen codec with the same Option-bundle shape. No new tunables
  (fields route through existing public setters).
- `sweep`: trained-scalar-head + compute-budget surface (variant-generation
  playbook patterns 17–18). `sweep::compute_tier(&SweepVariant) -> u8` —
  ordinal compute-cost proxy (PNG's single dial is the compression effort, so
  the tier *is* `Compression::effort()` saturated into `u8`).
  `SweepAxes::scalar_dense()` — the densest principled effort ladder
  (default-first `Balanced`, then every standard tier plus the heavy `Crush`/
  `Maniac` tiers `modes_full` excludes) so a scalar head sees the full
  compute-vs-bytes curve. `sweep::plan_constrained(axes, compute_limit,
  max_deviations)` — `plan()` plus an optional compute-tier ceiling (dropped
  cells reported in the new `SweepPlan::compute_tier_skipped`, never silently
  capped) and a deviation-scope filter (single-axis on PNG; present for
  cross-codec API uniformity). `plan()` now delegates to
  `plan_constrained(axes, None, None)` — behavior unchanged. All additive.
- **Calibrated resource-estimation module (`heuristics`).** New
  `zenpng::heuristics` with `EncodeEstimate` (min/typical/max peak memory +
  `time_ms` + `output_bytes`), `DecodeEstimate`, and
  `estimate_encode(w,h,input_bpp,effort)` / `estimate_decode(w,h,output_bpp)`
  — mirrors the zen per-codec pattern (`zenwebp::heuristics`). Calibrated
  from real measurement: a new `examples/png_probe` measures the marginal
  working set (`VmHWM` delta) + wall + user/sys CPU (`/proc/self/stat`,
  `with_parallel(false)`), swept by `scripts/png_resource_calibrate.py` over
  5 content classes × 256–1024 px × effort {1,6,13,19,24,27,30} × rgb/rgba ×
  8/16-bit (`benchmarks/png_resource_*_2026-06-14.tsv`). The model captures
  that the **compression level dominates BOTH time and memory**: encode time
  spans 0.03 → ~125 µs/px (e1 → e30 Maniac, ~4000×) and working set 18 → 120
  B/px, while decode is a near-free DEFLATE inflate (~5 B/px, 0.006 µs/px).
  Alpha: +23 B/px, +35 % time. 16-bit: +16 B/px.

### Fixed

- docs(readme): document the `metadata: Option<&Metadata>` encode argument
  (the 2nd positional arg of `encode_rgba8`/`encode_rgb8`/…, 5th of
  `encode_apng`). Every example passed `None`, which silently writes no
  ICC/EXIF/XMP — contradicting the "full metadata roundtrip" headline.
  Added inline `None`-drops-metadata comments, a decode→encode
  metadata-preserving example, the `zencodec::Metadata` dependency, the
  `zenpng::PngError` import path, and the `At<PngError>: std::error::Error`
  (`?`-to-`main`) fact; fixed the non-compiling `At::location()` server
  snippet to `e.frames().next()…`. Found by an insulated external-developer
  usability test.

- `cicp_pq_without_cms_is_an_encode_error` →
  `cicp_pq_without_cms_synthesizes_icc_from_bundle`: zenpixels-convert
  0.2.13 made CICP→ICC synthesis feature-independent (bundled blob), so
  a no-`cms` build now embeds a real PQ profile instead of refusing —
  the refusal expectation was stale, not the gate lost (expectation
  updated with sign-off; matches zenjpeg 8447d4d4's call).

### Added

- `sweep` module: variant-generation playbook adoption — the entire
  curated space is trial-class (lossless), `Compression::effort()` is
  the fingerprint identity (`Effort(13)` aliases `Balanced`), `parallel`
  pinned off per pattern 9, `png-<preset>`/`png-e<n>` id grammar with
  parser + totality test. `tests/sweep_validate.rs` gates per-cell
  decodability + EXACT roundtrip + tier liveness on a 5-image synthetic
  corpus (first run caught the downcast-format comparison hazard —
  documented in `docs/VARIANT_GENERATION.md`).

### Added
- Versioned public-API surface snapshot at `docs/public-api/zenpng.txt`, regenerated by `tests/public_api_doc.rs` on every `cargo test` (`ZEN_API_DOC=check` verifies in CI, `=off` skips); justfile `api-doc` / `api-doc-check` recipes.
- `cms` feature: ICC synthesis for the color-emit path via `zenpixels-convert/icc-db` (a bundled LZ4 profile blob + pure-Rust lz4_flex decoder — **no moxcms**), covering the full ITU-T H.273 grid incl PQ/HLG. Requires `zenpixels-convert` 0.2.13 (unreleased — adds the `icc-db` feature). Without it only the bundled Display-P3 / SDR BT.2020 / AdobeRGB consts synthesize. Failing to synthesize a needed ICC is now an encode **error**, not a silent skip: PNG's cICP chunk (PNG 3.0) is too new to be the sole color carrier — most deployed decoders ignore it and would read the pixels as sRGB. The error names the `cms` feature and the supply-an-ICC / drop-the-CICP alternatives. CI tests `--features zencodec,cms`; tests `cicp_pq_without_cms_is_an_encode_error` / `cicp_pq_with_cms_synthesizes_icc`.
- zencodec 0.1.21 color-emit integration: encode-side ICC-vs-cICP reconciliation via `resolve_color_emit` under the caller's `ColorEmitPolicy`; CICP-only sources synthesize an ICC via zenpixels-convert `synthesize_icc_for_cicp`; decode surfaces the stored EXIF Orientation tag. Deps bumped to published zencodec 0.1.21 / zenpixels 0.2.11 / zenpixels-convert 0.2.12; CI now tests `--features zencodec` (560e793d).
- Native HDR decode signaling: the decode-side output descriptor (probe `output_info`, full decode, and the streaming/push paths) now carries the transfer function and color primaries from the cICP chunk — a BT.2100-PQ PNG decodes as a PQ/BT.2020-tagged buffer instead of claiming sRGB, so downstream conversion applies the right EOTF. Layout/depth negotiation preserves the tagging. Tests `decode_descriptor_carries_cicp_pq_hdr` / `decode_descriptor_without_cicp_stays_srgb`.
- PNG 3.0 HDR signaling through the public `EncodeConfig` API: `with_cicp` (cICP), `with_content_light_level` (cLLI), and `with_mastering_display` (mDCV). Set `Cicp::BT2100_PQ`/`BT2100_HLG` with 16-bit samples for HDR renditions. The chunk writers and decode-side parsing already existed; this wires them through the ergonomic encode builder (previously reachable only via the zencodec `Metadata` path). cICP matrix-coefficients are forced to 0 (PNG's RGB color model) and mDCV is emitted only alongside cICP per PNG-3 §11.3.2.7. Roundtrip test `png3_hdr_cicp_clli_mdcv_16bit_roundtrip`.

### Changed
- Exclude `tests/` from the published crate tarball; regression PNG fixtures and test source files were unnecessarily shipping to crates.io.

### Performance
- Faster NEON (aarch64) Sub unfilter. The previous loop reloaded the running
  reconstructed pixel from a scalar `u32` every step; the rewrite keeps it in a
  NEON register across iterations (bpp=4 resolves two pixels per iteration via an
  in-register prefix add). Measured on Ampere Altra / Neoverse-N1: Sub bpp=4
  +33% (3088 → 4117 MB/s), Sub bpp=3 +20% (2716 → 3258 MB/s). Decode output is
  byte-identical (verified by the `simd::sub` tier-permutation tests on aarch64).
  Benchmark: `benchmarks/zenpng_arm_sub_unfilter_2026-05-29.{tsv,meta}`.

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
