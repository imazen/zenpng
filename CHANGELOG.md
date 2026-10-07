# Changelog

All notable changes to zenpng are documented here.

## [Unreleased]

### Added

- **Multi-threaded decode of Apple `iDOT` PNGs** (`PngDecodeConfig::max_threads`,
  default 0 = automatic; 1 = single-threaded). Strips are decoded on a work
  queue straight into the output buffer. On hybrid CPUs (Linux), workers are
  pinned to the fastest core tier when the strips fill at most half of it.
  Output is byte-identical to the serial decode; any table that doesn't prove
  that (malformed, Buchanan's "ambiguous PNG", Up/Average/Paeth at a boundary,
  Adler-32 mismatch) falls back to the serial decoder. Apple's 2-strip files
  decode 1.57–1.71× faster at 1–8 MP; zenpng files with 4–16 strips decode
  2.05–3.99× faster (Core Ultra 7 265K, `docs/IDOT_PARALLEL_PNG.md`).
- **`EncodeConfig::with_decode_segments(n)`** writes an `iDOT` table and
  independently decodable strips (2 = Apple's layout). Off by default; output
  is unchanged unless it is set. Costs −0.09% to +0.40% in size for n ≤ 8, and
  1–8% in encode time at efforts 7–19. Images under ~2 MiB of row data get no
  strips.
- 44 tiny `iDOT` test fixtures (`tests/fixtures/idot/`, 53 KB) covering every
  color type and bit depth, with generator-computed pixel hashes and Apple
  ImageIO (macOS 27) decode hashes; a macOS CI job checks zenpng's `iDOT`
  output against ImageIO on every push.
- The encoder never writes `iDOT` for 1/2/4-bit grayscale, and at most 16
  segments: Apple ImageIO's parallel path mis-decodes sub-byte gray (its
  serial path is fine), and 16 is the largest count verified in ImageIO.
- New Linux-only dependency `rustix` (safe `sched_setaffinity` wrappers) for
  core-tier pinning.

### Changed

- `push_rows` at efforts 1-15 compresses strip by strip as rows arrive
  (about 512 KiB of filtered rows each) when the canvas height is known,
  every downcast and near-lossless are off, no `iDOT` segments are
  requested, and the image spans two or more strips. Output is
  byte-identical to the one-shot multi-threaded encode; memory is one strip
  plus the compressed output instead of the whole image. No public builder
  turns downcasts off on `PngEncoderConfig` yet, so zencodec callers can't
  reach this mode until one lands; otherwise `push_rows` buffers as before.
  The strip worker is shared with the multi-threaded encoder (byte-identical
  output, `tests/strip_encode.rs`, `tests/idot*.rs`).
- Lower fixed cost per decode: the zenflate stream decoder is boxed (its
  inline Huffman tables were copied on every move of the row decoder), and a
  whole-image inflate buffer gets 1 KiB of slack so zenflate doesn't grow and
  zero-fill it at end of stream. 64x64 RGB8 (9097): 444K -> 391K
  instructions per decode (image-png main: 406K; callgrind, i265).
- **Faster decode of non-RGBA8 formats.** Rows are expanded straight into the
  output buffer by a per-image `RowExpander` (precomputed palette and sub-byte
  lookup tables), 16-bit output is written without an intermediate copy
  (ed5646d), every pixel size gets a const-generic Sub/Avg/Paeth kernel with a
  branchless Paeth predictor (d183bd3), and 1/2/4-bit rows unpack a byte at a
  time (f638e30). Measured against image-rs/image-png main on a Core Ultra 7
  265K P-core: palette 2.68 → 1.58 ms, gray8 3.85 → 2.30 ms, RGB16 11.2 →
  6.35 ms, gray1 2.02 → 0.087 ms (`benches/vs_png.rs`,
  `benchmarks/vs_png_decode_*_2026-10-06.*`).
- Unfilter routing re-measured against the fixed kernels: bpp=3 Sub/Paeth use
  them on x86 and aarch64 (the x86/NEON bpp=3 kernels are removed), and
  aarch64 bpp=4 Sub/Paeth now use NEON (2.06× / 1.31× per row). ARM decode of
  RGBA8 and Sub-heavy RGB8 images is up to 1.21× faster (8f790fe).
- **Effort ladder rebuilt on zenflate's PNG levels (`png(n)`).** Efforts 2-7
  screen Paeth and MinSum at png(1) and recompress the winner at
  png(2..12); efforts 8-19 add filter None to the screen, screened at png(4)
  (e8-11) or png(10) (e12+), because unfiltered rows win by 4-21% on line
  art, documents and screenshots and png(1) can't rank them; efforts 15-19 add
  png(26/28/30) refinement and brute-force rows; e20-30 keep the heuristic
  screen, fork, block, beam and recompression searches. Each rung's search
  contains the one below (`ladder_searches_are_nested`); measured per-image
  inversions on 91 images: e7→e8 4 images (max +1.95%), e13→e14 1 image
  (+1.15%, a zenflate png(24) vs png(19) gap), all others ≤0.23%. Preset
  numbers are unchanged. Against 8bd0e2b (median, 25 images at 1024 px):
  `Fast` 1.7x faster and 6% smaller, `Balanced` 5% faster and 4.3% smaller,
  `High` 0.5% smaller but 1.5x slower; `Balanced` is 16% (line art) and 5.5%
  (documents) smaller than image-png's `High` (geomean).
  `benchmarks/pareto_ladder_x86_2026-10-06.md`.
- With `with_parallel(true)` and `with_decode_segments(n)`, the `iDOT` segments
  come straight from the parallel strip encoder (each segment's first row
  filtered with None or Sub) instead of a serial encode that is inflated again
  and re-split. Output decodes identically everywhere and in parallel in
  zenpng (`tests/idot_encode.rs`).
- **Two-thread decode for large ordinary PNGs.** When threads are allowed
  (`max_threads` 0, the default, or above 1) and the filtered stream is at
  least 512 KiB, a second thread inflates rows while the caller's thread
  unfilters and expands them. Output, warnings and errors (truncation, strict
  Adler-32) match the serial decoder (`tests/decode_pipeline.rs`). Single runs
  on i265: 1024 px RGB8/RGBA8 1.34-1.51x faster, 4096 px RGB8 1.70-1.93x.
- Paeth unfiltering for pixel sizes without a SIMD kernel (RGB8 included) uses
  a min-select predictor on aarch64: 1.35x faster on Neoverse-N1 (1920 px RGB8
  row 13.2 -> 9.8 us). x86 and wasm32 keep the stb form, which measured faster
  there.
- **Multi-threaded strip encode for efforts without brute-force or recompress
  phases (1-15).** With
  `with_parallel(true)`, the image is split into strips of about 512 KiB of
  filtered data; worker threads filter each strip with every screening
  strategy, recompress its best candidates at the refine levels, and compress
  it independently (`zenflate::png::StripCompressor`), keeping the smallest. The concatenation is one ordinary zlib stream, and the
  output does not depend on the thread count. Five 4096 px images on 8 cores:
  effort 1 184-303 ms -> 30-51 ms, effort 5 1.53-2.18 s -> 193-252 ms, sizes
  -0.37% to +0.25% (measured at c48cc26, before the ladder rework). Off by
  default (as `parallel` is).
- **Release builds no longer decompress every compressed candidate during
  encode.** The check was a decode-only workaround for a February 2026 zenflate
  bug, cost 17-23% of encode instructions at efforts 1-7, and dropped failing
  candidates silently. Debug builds (tests, `cargo fuzz`) now decode every
  candidate and panic unless the exact input comes back. Effort 1 on a
  1024x768 photo: 213.8M -> 146.2M instructions; effort 7: 1,395M -> 1,099M.
- Encode efforts 16-19 are 15-25% faster: refine tiers no longer recompress
  the fallback levels they share (byte-identical output, 1dbcc3c).
- The streaming inflate buffer is at least 256 KiB (capped at the image size),
  which cuts small-image decode time by 3–17% (9fc1a21).
- Large zeroed buffers (decode output) are now allocated zeroed by the
  allocator (fallible calloc via `bytemuck`) instead of reserve + fill, so page
  zeroing is no longer an up-front serial pass on the calling thread.
- zenflate requirement raised to 0.4.0, temporarily patched to
  imazen/zenflate#9 for the segment APIs. The patch must be replaced with a
  release before zenpng is published.

### Fixed

- `push_rows` at efforts 0 and 1 dropped the config's cICP, content light
  level and mastering display chunks that the one-shot encoder writes; every
  path now builds the header metadata in one place (`png_write_meta`).
- **The zencodec row-sink and streaming decoders no longer return a partial
  image as success.** When the image data ended early, `push_decoder` (for
  every format except RGB8/RGBA8) and the streaming decoder stopped quietly,
  leaving the remaining rows unwritten, while `decode()` reported "unexpected
  end of image data". Both now return that error. The streaming decoder also
  never reached the zlib footer, so a strict `DecodePolicy` did not verify its
  Adler-32; it does now. `push_decoder` expands rows straight into the sink
  buffer instead of through a temporary row. Tests:
  `tests/integration/sink_truncation.rs` (also checks `push_decoder` against
  `decode()` on all 44 tiny fixtures).
- Image data that ends early is now `PngError::Truncated` (category
  `UnexpectedEof`) on every decode path. It was `Decode` for non-interlaced
  and interlaced images and `Truncated` only for APNG frames. All row loops
  share one `fill_row` helper.
- **`PngDecodeConfig::strict()` now always verifies the Adler-32.** When the
  last image row was produced before the zlib footer was read (trailing data
  after the image, or a large inflate buffer), the checksum was never
  checked. The stream is now drained after the last row in strict mode, so a
  corrupt Adler-32 that was previously accepted is now an error (9fc1a21).
- **HDR and wide-gamut pixels are no longer silently mislabelled or rejected.**
  `ENCODE_DESCRIPTORS` / `DECODE_DESCRIPTORS` advertised only sRGB and linear
  forms, and the two failure modes that produced were opposite and both bad. A
  PQ buffer whose primaries happened to match an advertised entry was passed
  through by `adapt_for_encode_cow` — documented as the *permissive* negotiator
  — and written with **no colour chunk at all**: PQ samples in a file that reads
  back as sRGB, silently, unless the caller thought to pass `with_cicp` by hand.
  A buffer whose primaries did *not* match (Display-P3) instead attempted a real
  gamut conversion and failed outright for want of a peak luminance
  (`HdrSourceRequiresPeak`), so Display-P3 PQ could not be written at all. Both
  are fixed by advertising the forms PNG can already carry — BT.2100 PQ/HLG,
  Display-P3 PQ/HLG at 16-bit, Display-P3 sRGB at 8-bit — so negotiation finds
  an exact match and converts nothing. PNG 3rd ed. signals all of this with
  `cICP`, so no new pixel machinery was needed.
- **A non-sRGB descriptor now supplies its own `cICP` chunk.** A descriptor says
  what the samples are; without a colour chunk the file does not, and a PNG with
  no colour chunk is sRGB by convention. An explicit `with_cicp` still wins, and
  sRGB/BT.709 stays untagged (conventional, smallest, and unchanged for every
  existing caller). f32-linear inputs are excluded because this encoder converts
  them to sRGB on the way out, so a cICP taken from their descriptor would
  describe pixels the file does not contain — measured, that alone broke three
  f32 round-trip tests before the rule was narrowed.
  Tests: `hdr_and_wide_gamut_descriptors_round_trip_with_cicp` (pixels
  byte-identical, colour preserved) and `srgb_stays_untagged` (the converse).

### Changed

- **`zencodec` / `zenpixels` / `zenpixels-convert` requirements now span the
  published minor and the next one**: `zencodec` `"0.1.26"` → `">=0.1.26,
  <0.3.0"`, `zenpixels` `">=0.2.11, <0.3"` → `">=0.2.11, <0.4.0"`,
  `zenpixels-convert` `">=0.2.16, <0.3"` → `">=0.2.16, <0.4.0"`, and `squintly`'s
  `zenpixels` `"0.2.11"` → `">=0.2.11, <0.4.0"`. For a `0.x` crate Cargo treats
  the minor as the major, so both the bare `"0.1.26"` and the `<0.3` ceiling stop
  at the next minor, and a `zencodec 0.2.0` / `zenpixels 0.3.0` release would
  have been invisible until this manifest was hand-edited. Floors are unchanged
  and nothing newer is published, so resolution is identical (`cargo metadata
  --all-features`: one copy of each). **This repo already documents why the range
  has to be uniform:** the `[patch.crates-io] zencodec` comment below describes
  exactly the two-copies failure — two instances of the same `0.x` crate make
  `DecoderConfig` a *different type* on each side and the trait bound fails to
  typecheck (E0277). A consumer left un-widened while its siblings move is how
  that second copy gets into the graph. The standing current-plus-next rule is
  documented in the zencodec repo's `CLAUDE.md`. The two `imazen/zencodec` git
  entries (the patch pin and the `zencodec-testkit` dev-dep) are untouched — a
  patch replaces the source regardless of the requirement, and a git dep has no
  registry requirement to widen. Note for whoever retires them: both comments
  say testkit is not yet published, but `zencodec-testkit 0.1.0` is on crates.io,
  so both can become plain registry deps whenever someone verifies the swap.

- **`miniz_oxide` dev-dependency `"0.8"` → `"0.9.1"`.** Third-party-only update.
  `miniz_oxide` is a dev-dep used two ways, neither of which reaches shipped
  output: as a *reference inflater* in the encoder unit tests
  (`decompress_to_vec_zlib`, ~12 call sites in `src/encoder/compress.rs`, which
  assert zenpng's IDAT round-trips through an independent decoder), and as a
  comparison compressor in `examples/corpus_bench.rs` / `examples/deflate_compare.rs`
  (benchmark numbers only). Both signatures are unchanged in 0.9
  (`decompress_to_vec_zlib(&[u8]) -> Result<Vec<u8>, DecompressError>`,
  `compress_to_vec_zlib(&[u8], u8) -> Vec<u8>`), and inflate output is
  spec-determined, so no test expectation moves. All 736 tests pass and the two
  byte-exact 16-bit regression tests
  (`truecolor_png_ga16_alpha_low_byte_zero_roundtrips_byte_exact`, `…_rgba16_…`)
  are unchanged.

- **`quantette` `"0.5"` → `"0.6.0"`, after measuring that it changes no pixels.**
  `quantette` is a palette quantizer — choosing output colors is its entire job —
  and 0.6.0 is a leading-digit bump, which for a `0.x` crate is the author
  signalling a break, so this could not be taken on faith. zenpng's quantette
  coverage is *structural*, not byte-golden (the tests assert only that the
  palette is non-empty, `len() <= 256`, and that the index count matches), so the
  suite cannot prove palette stability on its own.

  It was therefore measured directly: a harness depending on `quantette =0.5.1`
  and `=0.6.0` *simultaneously* (they are semver-incompatible, so Cargo will
  link both) replayed `QuantetteQuantizer::quantize_rgba`'s exact call sequence
  — `ImageBuf::new` → `Pipeline::palette_size().quantize_method()` →
  `output_srgb8_indexed_image()` → `palette()` / `indices()` — against both, and
  compared the emitted palette and index buffers byte for byte. **576
  configurations** (4 content classes: photo / screenshot / line-art / uniform
  noise × 3 sizes: 16², 64², 256² × Wu and k-means × palette sizes 2/16/64/256 ×
  dithering off and on × `sampling_factor` 1.0 — zenpng's default — 0.5 and
  0.25) produced **zero** differences: every palette and every index buffer was
  byte-identical. The comparison also spans a SIMD and RNG backend change
  (0.5.1 resolves `wide 0.8.3` + `rand 0.9`, 0.6.0 resolves `wide 1.7.0` +
  `rand 0.10`) and is still bit-exact. The API zenpng uses compiles unchanged,
  and 0.6.0's `rust-version` is 1.90, below the declared MSRV.

  The **root** `Cargo.lock` is not committed because zenpng gitignores it
  (`.gitignore:1`); a fresh resolve already picks the newest compatible
  third-party versions, which is what CI builds. The two **nested** lockfiles
  *are* tracked and were refreshed under the same third-party-only constraint:
  `fuzz/Cargo.lock` (21 packages — `getrandom` 0.3.4 → 0.4.3, which drops the
  `wasip2` / `wit-bindgen` subtree, plus `bytemuck` 1.25.2, `libc` 0.2.189,
  `cc` 1.4.4, `thiserror` 2.0.20) and `apidoc/Cargo.lock` (14 packages —
  `rustdoc-types` 0.57.4, `serde` 1.0.229, `syn` 3.0.4, `thiserror` 2.0.20).
  Both still `cargo check --all-targets` clean. For the record, a constrained
  refresh (`cargo update -p …` over all 129 third-party packages, every
  zen-family crate excluded) moves 24 packages, including `thiserror` 2.0.19 →
  2.0.20, `imgref` 1.12.2 → 1.12.3, `flate2` 1.1.9 → 1.1.10, `libdeflater`
  1.25.2 → 1.26.0, `crc32fast` 1.5.0 → 1.5.1, and `palette` 0.7.6 → 0.7.7
  (which swaps `fast-srgb8` for `palette_math`). `zenflate 0.3.6 → 0.4.0` was
  **not** taken — zen-family deps are out of scope for this pass.

### Fixed

- **Clippy on stable 1.98 failed the lib.** `chunks_exact_to_as_chunks` is new in
  Rust 1.98, and `scalar_bit_replication_lossless_be16` (`src/lib.rs`) used
  `be.chunks_exact(2)` with a constant chunk size — now `be.as_chunks::<2>().0`.
  The Clippy CI job pins `dtolnay/rust-toolchain@stable`, so this was going to
  turn the job red on its own the moment the runners picked up 1.98; it is not
  caused by any dependency change. `as_chunks` predates the declared MSRV, and
  `cargo hack check --rust-version` (1.93) still passes.

- **Pushes to `main` now cancel their superseded CI runs.** `ci.yml` keyed its
  concurrency group on `${{ github.head_ref || github.run_id }}`.
  `github.head_ref` is populated only for `pull_request` events, so on a push it
  was empty and the group fell through to `github.run_id` — unique per run, so no
  two pushes ever shared a group and `cancel-in-progress` could never fire. Every
  push started a full matrix that ran to completion even when several commits
  landed seconds apart. Now keyed on `${{ github.ref }}`, which is set for both
  event types, so PR cancellation is unchanged and consecutive pushes supersede
  each other.

- **The `Fuzz regression` CI job could not fail.** It ran
  `cargo test --test fuzz_regression 2>/dev/null || echo "No regression test
  found…"` inside an `if [ -d fuzz/regression ]` guard, so a genuinely failing
  suite, a missing corpus, and a missing harness all reported green.
  `tests/fuzz_regression.rs` has existed the whole time, so the fallback was
  masking real failures rather than covering a missing target. The step is now
  a bare `cargo test --test fuzz_regression`. The harness's `!seeds.is_empty()`
  check is now a pinned `>= MIN_SEEDS` (3) that also skips `README`/dotfiles, so
  a gutted or documentation-only corpus fails instead of replaying whatever
  survived. Mutation-verified: cutting the corpus to one seed plus a README, and
  a panic injected into the replay path, each exit 101.

- **32-bit decode panicked on a max-width IHDR instead of returning
  `OutOfMemory`.** The streaming inflate buffer was sized `stride * 2` in
  `usize`; for gray8 at width 2^31 − 1 the stride is 2^31, so the doubling
  overflowed on i686/wasm32 (`attempt to multiply with overflow` in debug, a
  wrapped capacity in release). All four sites (`RowDecoder::new`, Adam7,
  APNG IDAT/fdAT frames) now size the buffer through
  `alloc_util::stream_capacity`, which computes in `u64` and rejects anything
  that — together with zenflate's own 32 KiB lookback — cannot be one Rust
  allocation. The IHDR row-size check now bounds rows at `isize::MAX` (the
  ceiling every `Vec` enforces) rather than `usize::MAX`, the stored-block
  fast path uses checked `stride * height` / `raw_row_bytes * height`, and the
  two row scratches in `RowDecoder` honor an explicit `AllocPreference`. The
  `unbounded_strict_config_reaches_the_allocator_for_huge_ihdr` regression
  test now passes on every pointer width with the same `OutOfMemory`
  expectation.

- **Configured limits are checked before the platform row-size bound.**
  `Ihdr::parse` rejected a row wider than the address space (RGBA8 at width
  2^31 − 1 is 2^33 bytes) before the decoder ever consulted `max_pixels` /
  `max_memory_bytes`, so on 32-bit an over-cap image surfaced as
  `OutOfMemory("row size overflow")` where 64-bit reported `LimitExceeded`.
  Decoders now parse with `Ihdr::parse_fields`, run `PngDecodeConfig::validate`,
  then `Ihdr::check_row_fits_platform`, giving `LimitExceeded` on every width
  (`apng_huge_canvas_is_rejected_by_default_limits` on i686). `ApngDecoder::new`
  now performs that limit check itself, which also covers the zencodec
  animation-frame decoder — it passed the caps into its config but nothing
  enforced them. `probe` no longer applies the platform row bound either (it
  reads metadata only, and `decode_apng` probes before decoding), so a valid
  header probes on every pointer width. The output-row byte count (`width ×
  output bpp`, up to 32× the raw row for 1-bit indexed → RGBA8) is now
  checked on all four decode paths instead of relying on the raw-row check.

- **CI: wasm32 job installs wasmtime via `taiki-e/install-action`.** The
  `wasmtime.dev/install.sh` script resolves "latest" through the GitHub API;
  when that is rate-limited on a shared runner it tries to fetch version `{`
  and installs nothing, so the tests never executed (run 33103871877).

- **16-bit gray+alpha rows were corrupted by the RGBA8 transparent-pixel
  zeroing.** `compress_filtered` keyed the "zero RGB under `alpha == 0`"
  optimization on `bpp == 4`, which 16-bit gray+alpha also satisfies
  (2 channels × 2 bytes). A GA16 pixel `[G_hi, G_lo, A_hi, A_lo]` was read
  as `[R, G, B, A]`, so every pixel whose alpha *low* byte was 0 (alpha
  0x0100, 0x8000, 0xFF00, …) had its gray and alpha high byte wiped —
  e.g. gray 0x1234 / alpha 0xFF00 decoded as gray 0 / alpha 0x0000, an
  opaque pixel turned fully transparent black. Not reachable from the
  public API today (no GA16 encode entry point and the 16-bit downcast never
  emits GA16), but live at `write_truecolor_png(color_type = 4, bit_depth =
  16)`. The zeroing is now keyed on an explicit `RowFormat { bpp, rgba8 }`
  derived from the IHDR color type + bit depth, so only 8-bit RGBA rows are
  eligible. Regression tests round-trip GA16 and RGBA16 byte-exact at
  efforts 1/7/13 (mutation-verified: restoring the `bpp == 4` gate fails
  the GA16 test).

- **APNG decode buffers were panic-on-OOM** (issue #13, residual after
  f979f72 moved the still-image paths to `alloc_util`): the composited canvas
  (`vec![0u8; canvas_bytes]`), the per-frame pixel buffers in
  `decode_idat_frame` / `decode_fdat_frame`, and the `RestorePrevious` saved
  region still used infallible `vec!` / `Vec::with_capacity`, so an
  allocation the configured caps allowed but the machine could not satisfy
  aborted the whole process. All four sites now go through
  `alloc_util` (default fallible for the untrusted-sized buffers, infallible
  for the one-row scratch, honoring `AllocPreference` overrides) and return
  `PngError::OutOfMemory`; the frame-size products are `checked_mul`
  instead of bare `*`. Regression-tested with a 2^31-1 square RGBA8 APNG
  under `PngDecodeConfig::none()` (mutation-verified: the old canvas site
  panics with `capacity overflow`).
- **`fuzz_decode_strict` harness reached the allocator with exabyte requests**
  (issue #19, farm signature `68f9a17dcbfa8396`): the target started from
  `PngDecodeConfig::strict()`, which has no `max_pixels` / `max_memory_bytes`,
  so a 2^31-1 × 2^31-1 IHDR requested ~4.6 EB and ASan aborted with
  `allocation-size-too-big` before the decoder's fallible `try_reserve` could
  return `Err`. The harness now starts from `default()` (120 MP / 4 GiB) with
  both checksums enabled. Added `tests/fuzz_regression.rs` (the
  `cargo test --test fuzz_regression` gate `fuzz.yml` already invoked but which
  did not exist) plus a synthesized huge-IHDR seed under `fuzz/regression/`;
  the original repro is only in R2 / the WSL mirror and was not reproduced
  locally. `strict()` itself is unchanged and still documents "no limits".
- **Out-of-range palette indices were silently corrupted instead of rejected**
  (issue #20, 2026-08-26 ultracode sweep, adversarially verified):
  `write_indexed_png` validated palette size and buffer length but never that
  each index references an existing entry. Sub-byte depths masked bad indices
  onto arbitrary palette colors (`idx & mask`); depth 8 emitted them verbatim,
  producing a spec-invalid PNG whose IDAT references entries beyond PLTE —
  decoder-dependent garbage returned as success. Reachable through the public
  `Quantizer` trait. Both the still and APNG indexed paths now validate every
  index up front (`validate_palette_indices`), and `pack_all_rows` debug-asserts
  the mask never truncates. Regression-tested at depths 2 and 8.
- The SIMD tier-parity test now decodes its reference bytes (byte-identity
  across tiers passes even when every tier is corrupt).
- Cleared the new clippy `-D warnings` wall across all targets
  (`chunks_exact` → `as_chunks`, cfg-return unreachable patterns), verified on
  aarch64 and x86_64 (lib).

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
- **Documented that RGB under `alpha == 0` is zeroed for 8-bit RGBA output**
  even in lossless mode (README quick start, `encode_rgba8` rustdoc,
  `PngEncoderConfig::with_lossless`). Behaviour is unchanged; it was
  previously stated only in the APNG section. Effort 0 and the `push_rows`
  effort-1 streaming path store rows as supplied; no other layout is touched.
- **Encode memory pre-flight now gates on the calibrated peak estimate
  (db6c5b7a).** With `ResourceLimits::max_memory_bytes` set, encode admission
  compares the budget against `heuristics::estimate_encode(..)`'s
  `peak_memory_bytes` (fixed overhead + input + the effort-dependent working
  set, a measured safe upper bound over the default 4-thread peak) instead of
  just the `w*h*bpp` input buffer, which under-stated the real peak by an
  order of magnitude at high effort (512×512 RGB8 e13: 768 KiB claimed vs
  ~28 MiB measured). Encodes near a tight budget that previously slipped
  through now fail up front with `LimitExceeded::Memory`; raise
  `max_memory_bytes` if the new honest estimate rejects a budget you know is
  sufficient. Covers one-shot encode and the buffered `push_rows`/`finish`
  path. No thread-count cap from the memory budget: the calibrated model
  bakes thread cost into its flat B/px envelope (fit over the {1,4}-thread
  grid) and exposes no separable per-thread memory term, so `max_threads`
  reduction has no defensible basis in the current model — re-sweep with a
  thread axis before adding one.
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
