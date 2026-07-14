# Changelog

All notable changes to zenpng are documented here.

## [Unreleased]

### QUEUED BREAKING CHANGES
<!-- Breaking changes that will ship together in the next release. Do NOT
     ship these piecemeal — batch them. -->
- `Cargo.toml` version was **defensively pre-bumped 0.1.4 → 0.2.0** (no tag,
  no publish, no GitHub release yet — those steps still need explicit
  owner sign-off) because the breaking changes below already landed on
  `main` while the crate version stayed at 0.1.4, so a `cargo publish` run
  today would have shipped a semver break as a patch release.
- Every `zencodec` trait impl's associated `Error` type changed from
  `At<PngError>` to `At<CodecError>` (Pattern B, d6ff72d): source-breaking
  for any downstream code that names or matches on the associated type
  through `zencodec::encode::{EncoderConfig,EncodeJob,Encoder,
  AnimationFrameEncoder}` / `zencodec::decode::{DecoderConfig,Decode,
  StreamingDecode,AnimationFrameDecoder}` for `zenpng`'s types. `cargo
  semver-checks` (rustdoc-JSON diff against published 0.1.4) reports **0
  breaking findings** here — a known tool blind spot: it does not model
  associated-type-value changes inside impls of a *foreign* trait. Manually
  verified via direct diff against the published 0.1.4 source
  (`cargo read zenpng` → every `type Error = At<PngError>` site is now
  `type Error = At<CodecError>`).
- The `zencodec` cargo feature (a no-op stub in 0.1.4, gating nothing) was
  removed outright when `zencodec` became a required dependency (d6ff72d).
  Downstream `Cargo.toml`s pinning `zenpng = { features = ["zencodec"] }`
  (e.g. zenpipe/zencodecs) hard-error with "Package 'zenpng' does not have
  feature 'zencodec'" against that main commit. Restored as a deprecated
  no-op stub in the pre-bump commit (see "Added" below) so
  `--features zencodec` keeps resolving for one release cycle.

### Added
- **`zenpng` reference CLI (`src/bin/zenpng.rs`).** Three subcommands:
  `normalize <in> <out>` (decode + re-encode pixels-only, stripping all
  ancillary chunks), `crop <in> <out> <side>` (centered square crop, clamped),
  and `compare <a> <b>` (exact pixel-equality gate, prints `EXACT`/`DIFFER`).
  SDR/8-bit only (16-bit rejected loudly, never silently truncated). Built as
  the dogfood replacement for OpenCV/cv2 in the jxl-encoder codec scoreboard:
  zenpng decodes Display-P3 / EXIF captures that crash libjxl 0.12's PNG reader.
- **Codec-agnostic error taxonomy (`zencodec::CategorizedError`).** `PngError`
  and the caller-facing `detect::ProbeError` now implement
  `zencodec::CategorizedError` (`codec_name() = Some("zenpng")` + total `category()`), so
  a consumer can route on the coarse `ErrorCategory` (HTTP status, retry policy,
  logging) without matching the enum. The stringly variants were split into
  discrete, category-named ones: new `Truncated` (→ `UnexpectedEof`),
  `UnsupportedFeature` (→ `UnsupportedImageFeature`), `Unsupported(UnsupportedOperation)`
  (delegates — `PixelFormat` → `UnsupportedPixelFormat`), `Io` (→ `Io`), and
  `Limit(zencodec::LimitExceeded)` (delegates, **carries the `LimitKind`**). The
  16 truncation/EOF decode sites, the 3 output-sink sites, and every configured-
  limit site (encoder `check_*`, decoder pixel/memory caps, APNG cumulative-
  memory + `acTL` frame cap, the >u32 IDAT guard) were rewired to construct the
  precise variant. The kept variants narrowed in meaning: `Decode` →
  `MalformedImage`, `InvalidInput` → `InvalidParameters`, `LimitExceeded` →
  `OutOfMemory` (now only allocation-failure / address-space-overflow sites).
  `Quantize` maps to `Internal` (delegating would need zenquant to impl
  `CategorizedError` — a follow-up). All additive on `#[non_exhaustive]`; no
  public variant removed or renamed. (d6ff72d)
- **Taxonomy mapping corrections** (additive — 2 new `PngError` variants):
  the four "missing PNG signature" decode-entry checks (`decoder::mod::probe_png`,
  `decoder::interlace::decode_interlaced`, `decoder::row::IdatSource::new`,
  `decoder::apng::ApngDecoder::new`) previously raised `PngError::Decode`
  (→ `MalformedImage`); they now raise the new `PngError::NotPng` (→
  `UnsupportedImageType`), matching `detect::ProbeError::NotPng`'s existing
  mapping and zencodec's own doc example for the category ("e.g. 'not a
  PNG'"). The two decode-policy rejections (`animation_frame_decoder`'s
  animation-forbidden check, `check_progressive_policy`'s interlace-forbidden
  check) previously raised `PngError::InvalidInput` (→ `InvalidParameters`);
  they now raise the new `PngError::PolicyRejected` (→ `PolicyRejected`) —
  the request was understood and declined, not malformed input. Updated the
  `envelope_category_survives_dyn_erasure` regression test, which pinned the
  old (incorrect) `MalformedImage` mapping for the "not a PNG" probe path.
- **Two-level origin-first `ErrorCategory` (zencodec PR #116).** Bumped the
  unpublished zencodec `[patch.crates-io]` rev from `c3220d51` to `2427387f`,
  which reshapes `ErrorCategory` from the flat 17-variant enum above into
  `Image(ImageError)` / `Request(RequestError)` / `Resource(ResourceError)` /
  `Policy(PolicyKind)` / `Lifecycle(enough::StopReason)` / `Io(CodecIoKind)` /
  `Internal(InternalKind)`. Neither shape has ever been published, so this
  isn't a break of released API. Rewired every `category()` arm and closed 3
  more audit findings: split `InvalidInput` (previously a catch-all → always
  `Request(Invalid(Parameters))`) by reading every construction site —
  new `InvalidBuffer` (pixel/palette/index buffer geometry) and `InvalidState`
  (streaming/animation call-sequence violations); the APNG pixel-format
  mismatch now routes through the existing `Unsupported(PixelFormat)` path;
  zenflate/zenzop/imagequant dependency failures and 2 broken-invariant sites
  (an internally-derived color type our own encoder can't handle; a
  decoder's own row-buffer construction) now route through a new
  `Internal(InternalKind)` variant (`Bug` vs `Dependency`); a CICP/ICC
  synthesis gap now routes through a new `CmsRequired` variant. Fixed a
  `Limit`/`LimitExceeded` name inversion from the prior entry above — the
  wrapper around `zencodec::LimitExceeded` (a configured cap) was named
  `Limit` while the allocator-failure/address-space-overflow variant was
  named `LimitExceeded`, backwards from what either name suggests; renamed
  to `LimitExceeded(zencodec::LimitExceeded)` and `OutOfMemory(String)` (the
  categories they map to are unchanged, only the names now match). Split
  `zenquant::QuantizeError`'s mapping instead of the acknowledged blanket
  `Internal`: `ZeroDimension`/`InvalidMaxColors` → `Parameters`,
  `DimensionMismatch` → `Buffer`, `QualityNotMet` → unclassified
  `Internal(Dependency)`. `PolicyRejected` now carries `zencodec::PolicyKind`
  (`Decode`/`Encode`) instead of one hardcoded category. All additive or
  renaming still-unreleased variants (none of `Limit`/`LimitExceeded`/
  `PolicyRejected` have ever been published); no released API broken.
  (f6f511f8)
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
  PNG picker needs to choose palette quantization. (a821f50)
- Depth-refined CART code-heuristic pickers for the zenpng lossless config
  space, codegen'd from the 2026-06-28 dual-model fan-out:
  `benchmarks/pickers/zenpng_lossless_cart_{zensim,ssim2}_2026-06-28.rs`
  (`pick_zenpng_lossless_heuristic(feats, zq) -> u16`, depth 8, 200/207
  leaves). Reference artifacts, not wired into the crate build; both files
  are >30 KB so per house rule 7b they are relocated to
  `/mnt/v/zen/picker-training/zenpng-2026-06-28/` with tracked
  `benchmarks/pickers/*.pointer.md` sidecars (path + sha256 + provenance).
  (61f1c10)
- Deprecated no-op `zencodec = []` cargo feature stub, restored for one
  release cycle so downstream `--features zencodec` (e.g. zenpipe/zencodecs)
  keeps resolving instead of hard-erroring after `zencodec` became a
  required dependency and the feature was dropped outright. See "QUEUED
  BREAKING CHANGES" above.

### Fixed
- Fuzz CI (red daily since 2026-07-02): `fuzz/Cargo.toml` is its own cargo
  workspace, so it did not inherit the root `[patch.crates-io]` zencodec
  git-branch override and its `fuzz/Cargo.lock` had drifted to registry
  zencodec 0.1.22 (missing the `CategorizedError`/`CodecError` taxonomy).
  Mirrored the same patch into `fuzz/Cargo.toml` (matching zenavif-parse's
  fix for the identical issue, commit 79551cf5) and regenerated
  `fuzz/Cargo.lock`.

### Changed
- **deps: migrate to published `zencodec 0.1.26`; drop the fuzz-crate git-rev
  patch.** `[dependencies] zencodec` is now `"0.1.26"` (was the unreleased-
  taxonomy `"0.1.25"` + git-rev patch). Removed `fuzz/Cargo.toml`'s
  `[patch.crates-io]` zencodec pin entirely and regenerated `fuzz/Cargo.lock`
  — the fuzz crate has no `zencodec-testkit` dependency, so nothing there
  needs source unification, and it now resolves `zencodec` straight from
  crates.io. The root `[patch.crates-io]` patch is **kept** rather than
  removed outright: it no longer exists for the taxonomy API (that shipped in
  0.1.26), but `zencodec-testkit` (dev-dependency, still unpublished)
  path-deps its own `zencodec` sibling from the same git checkout, and
  dropping the patch would leave two distinct `zencodec` instances in the
  graph (crates.io 0.1.26 direct vs. git via testkit) —
  `tests/integration/truncation_series.rs` passes zenpng's own
  `PngDecoderConfig` into testkit's `check_decode_truncation_series<D:
  DecoderConfig>`, which would fail to typecheck across two non-identical
  `DecoderConfig` traits (E0277). Both the patch and the `zencodec-testkit`
  dev-dep now pin via `tag = "v0.1.26"` (commit `998edf5`, byte-identical to
  the published crate) instead of a bare rev, for readability. (798e919)
- Docs: split README into a GitHub surface (`README.md`) and a generated
  crates.io surface (`README.crates.md`, no badges); refreshed for the
  `heuristics` resource-estimation, `detect` source-analysis, `cms`/`unchecked`,
  and native `zencodec` Fidelity APIs; added `benchmarks/README.md` and the
  canonical crosslink footer. (ad69bdb)
- **The `zencodec` trait impls now return the `At<zencodec::CodecError>`
  envelope (Pattern B), not `At<PngError>`.** Every encode/decode trait impl —
  `PngEncoderConfig` / `PngEncodeJob` / `PngEncoder` / `PngAnimationFrameEncoder`
  and `PngDecoderConfig` / `PngDecodeJob` / `PngDecoder` / `PngStreamingDecoder`
  / `PngAnimationFrameDecoder` — sets `type Error = At<CodecError>` and wraps its
  native error in the shared envelope (`CodecError::of` for already-located
  errors, the new `From<PngError> for At<CodecError>` bridge for bare ones). A
  generic consumer can now recover the `ErrorCategory` **and** the codec name
  (`"zenpng"`) *through `Dyn*` dispatch*: once the error is erased to
  `Box<dyn Error>`, the envelope downcasts back, where the previous
  `At<PngError>` left no shared concrete type to recover (both were lost). The
  internal logic is unchanged — each trait method delegates to a private
  `At<PngError>` body and converts once at the boundary. `PngError` (and
  `detect::ProbeError`) are untouched and remain the typed **detail** + category
  source inside the envelope. The codec's inherent rich-error API — the free
  `zenpng::decode` / `encode_*` functions and the `PngDecoderConfig::decode` /
  `probe` / `decode_into_*` + `PngEncoderConfig::encode_*` convenience methods —
  still returns `At<PngError>`, so direct PNG callers keep ergonomic enum
  matching. Regression gate: `codec::tests::envelope_category_survives_dyn_erasure`
  drives the decoder through `DynDecoderConfig` and asserts category + codec name
  survive `Box<dyn Error>` erasure. (d6ff72d)
- **`zencodec` is now a required (non-optional) dependency; the empty `zencodec`
  marker cargo feature is removed.** The trait integration
  (`PngEncoderConfig: EncoderConfig`, `PngDecoderConfig: DecoderConfig`, the
  `CategorizedError` impls on `PngError` / `ProbeError`, and the
  color-emit / orientation / metadata flow) was already compiled
  unconditionally — the `zencodec = []` feature gated nothing — so this drop
  only removes the no-op flag and the redundant `--features zencodec` /
  `zencodec`-only CI steps. The integration adds no `std`-only code (`zencodec`
  is `#![no_std] + alloc`), so the `wasm32-wasip1`, `wasm32-unknown-unknown`,
  and `--no-default-features` builds are unaffected. **Restored as a
  deprecated no-op stub** in a later commit (see "Added" above) after this
  broke downstream `--features zencodec` builds. (d6ff72d)

### Fixed
- **`sweep_cells_decode_exactly_and_steps_are_live` no longer panics on feature
  subsets.** The plan always carries every quantize cell (per
  `modes_full_has_all_eight_quantize_cells`), but a cell can only be *encoded*
  when its backend feature is compiled in; the test now filters cells to the
  available backends (gated by `cfg!(feature = ...)`, controlled by the CI
  feature matrix) instead of `.unwrap()`-ing the clean "needs the `imagequant`
  feature" error. Fixes the pre-existing red `cargo test` (default / no-default /
  `zencodec`-only) jobs on `main`. (a821f50)
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
  ceiling and the effort-anchor / alpha / 16-bit structure are unchanged. (26ca5d3)

### Changed
- **deps: migrate to published `zencodec 0.1.24` estimate API; drop the temporary
  git-rev patch.** Removed the `[patch.crates-io]` zencodec git-rev pin (0f71295)
  now that `zencodec 0.1.24` is on crates.io. Updated the
  `estimate_encode_resources` mapping for the refined `ResourceEstimate`:
  `new(peak, wall_ms: u64)` (was `f32`), `with_peak_max(max)` (the `min` arg is
  gone), dropped the removed `with_output_bytes`, and migrated
  `heuristics::encode_threading_info` to the new 1-arg
  `ThreadingInformation::parallel(max_efficient_threads)` (the `fraction` /
  `mem-per-thread` args are gone). (7bce0e3)

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
