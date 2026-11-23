# chunk-source merge + wasm thread gating — 2026-11-23

Lane: `swe-zenpng` (agent `swe-zensim`), baseline `56a39aa7` on `main`.
Measurement host: i134 (i5-13400F), pinned cpu 2, `nice -n 19`, serialized
through `~/tmp/bench-lanes-2026-11-23/i134.lock` by `remote_bench.sh`.

## Change

1. **`IdatSource` + `FdatSource` → `ChunkSource<'a>`**
   (`src/decoder/row.rs`). One streaming `InputSource` impl covering both the
   image IDAT run and each APNG frame's fdAT run. An internal
   `ChunkMode::{Idat,Fdat}` resolves the 4-byte tag (`IDAT`/`fdAT`), payload
   offset (`0`/`4` — fdAT leads with a 4-byte sequence number) and
   mode-specific error literals **at construction**. Hot `fill_buf` reads the
   resolved `tag`/`payload_off` fields and carries no mode branch beyond the
   bounded-run `end_pos` compare IDAT already had. The current-chunk range is
   now `(start, end)` into the file buffer instead of a `&'a [u8]` — identical
   slice, identical consume semantics. `post_idat_pos`/`post_fdat_pos` merge
   into `post_chunk_pos` (read once by `decoder/apng.rs:454`). Callers:
   `decode_png`/`decoder/mod.rs` (IDAT), `interlace.rs` Adam7 (IDAT),
   `idot.rs::idat_bounded` (iDOT segments), `apng.rs` (fdAT per frame). All
   error strings preserved verbatim per mode. Before this change, every
   `StreamDecompressor<IdatSource>` and `StreamDecompressor<FdatSource>`
   monomorphized a separate copy of the zenflate inflate state machine; after,
   all chunk-stream decode funnels through a single
   `StreamDecompressor<ChunkSource>` instantiation.

2. **wasm32: thread code compiled out, not just runtime-disabled**
   Every `std::thread::scope`/`spawn` path is now behind
   `#[cfg(not(target_arch = "wasm32"))]` so nothing thread-shaped is
   instantiated for wasm at all: the two-thread decode pipeline in
   `decoder/pipeline.rs` (`Piped`, `run`, `unfilter_into`, `PipedWorker`
   struct + `pipe_worker`), the iDOT parallel decode cluster
   (`idot.rs::plan_workers`/`decode_parallel`/`run_worker`, `mod affinity`,
   and its tests — `workers_for_bytes` stays, it is also the encoder's
   sizing heuristic), the pipeline invocation sites in `decoder/mod.rs`,
   `codec.rs`'s `Piped`/`StreamRows::Piped` arm, the now-unreferenced
   `RowDecoder::{idot_segments,next_filtered_row,stride,bpp}` helpers whose
   only callers were those thread paths, the parallel strip pool in
   `encoder/segments.rs` (`run_strips`' `std::thread::scope`), and the
   encode-side `par_map`/`parallel_strips`/strip-worker paths in
   `encoder/compress.rs` — including `StripPool` itself (struct, `StripJob`,
   `StripResult`, `impl`, `Drop`, the `StripStream::pool` field,
   `StripPlan::threads`, and its `new`/`push_row`/`finish`/`lag_rows` call
   sites; `StripStream::push_row`'s serial arm is extracted to
   `compress_strip_serial` so the `if let`/`else` structure survives the
   `cfg` without duplicating the serial block). On wasm every one of these
   now falls through to the exact same serial code the
   `max_threads == 1`/`!worth_it`/`plan_workers` reject paths already
   selected at runtime — same functions, same order, same bits. Behavior on non-wasm targets is unchanged (all gates were
   already runtime decisions; they are now compile-time on wasm only).

## Parity argument

- `ChunkSource::idat` byte-for-byte replays `IdatSource`: same range bounds,
  same `post_chunk_pos` update points, same `end_pos` early-exit, same error
  literals.
- `ChunkSource::fdat` byte-for-byte replays `FdatSource`: same
  sequence-number skip (`payload_off = 4`), same `chunk_pos` accounting
  (crc_end, not deflate end), same `post_chunk_pos` (past the first non-fdAT
  chunk), same empty-vs-nonempty current-range semantics, same error
  literals.
- Serial paths unchanged on wasm and `max_threads == 1`: identical code to
  what the runtime gates already selected.
- `png_conformance_corpus`, APNG, iDOT, interlace and streaming test suites
  pass unmodified (see §Verification).

## Measurements (i134)

Identical probe and bench sources both phases (measure script heredocs;
`zenpng_measure.sh before|after` in `~/tmp/bench-lanes-2026-11-23/`,
artifacts `artifacts-swe-zenpng/{before,after}/`).

### Wasm cdylib probe — `decode` + `decode_apng` + `encode_rgb8` + `encode_apng`

`RUSTFLAGS="-C target-feature=+simd128"`, `opt-level="s" lto cgu=1 strip`,
standalone `zenpng-wasm-probe` crate, `path` dep on the measured tree.

| metric | BEFORE | AFTER | Δ |
|---|---|---|---|
| probe .wasm (stripped) | 630,036 B | 586,606 B | **−43,430 B (−6.9%)** |
| probe .wasm gzip -9 | 240,402 B | 221,549 B | **−18,853 B (−7.8%)** |

The `StripPool` item-level gate changes nothing in the linked probe's
size — the after2 rebuild is again 586,606 B / gzip 221,549 (the two
binaries differ at 56 byte positions, layout noise from the
restructured source) because `pooled() == false` had already let LLVM
dead-strip the unreachable `StripPool::new`/spawn code. The `cfg`
makes the exclusion compile-time (`StripPool` is not instantiated for
wasm at all — no `StripPool`, `spawn`, or `JoinHandle` symbols in the
wasm rlib object members; the residual `thread` symbols are
`std::thread::local::LocalKey` users such as `LocalKey::with`
monomorphs for the scratch pools — TLS accessor functions, not thread
spawn/join code) instead of relying on a runtime-false gate plus DCE.

### Native test binary (debuginfo-free `size` on stripped elf)

| metric | BEFORE | AFTER | Δ |
|---|---|---|---|
| GNU `size` text | 5,324,085 B | 5,292,889 B | −31,196 B (−0.59%) |
| `StreamDecompressor<…>` monos, production | 11 (5 `FdatSource` + 6 `IdatSource`) | 6 (`ChunkSource`) | −45% |
| `IdatSource`/`FdatSource`/`ChunkSource` symbols | 23 | 12 | −48% |
| `std::thread`/`pipeline`/`par_map`-family symbols | 318 | 322 | +4 (layout noise; thread code is unchanged on native) |

### Timings (`benches/stream_and_apng.rs`, `ZENBENCH_CPU_POLICY=unrestricted`, cpu 2)

| arm | BEFORE | AFTER (after2 re-run) | Δ |
|---|---|---|---|
| `decode_png_256` | 10.83 ±0.2 µs | 10.98 µs | +1.4% — within run-to-run drift vs the first after run (10.89) |
| `decode_apng_128x3` | 160.35 ±1.4 µs | 156.19 µs | −2.6% |
| `encode_rgb8_256` | 15.28 ±0.16 ms | 14.89 ms | −2.6% |

`_dev` benches `unfilter_tiers` (production 347 MiB/s vs forced_scalar 343
MiB/s at baseline; arms unchanged — the refactor touches neither unfilter
kernels nor dispatch) and `scalar_vs_simd` (scalar 2.97→2.83 Gpx/s, runtime
SIMD unchanged) run on both trees; scalar arms are the bit-exact comparison
and sit inside run-to-run noise.

## Verification

- `cargo check -p zenpng --all-targets` — clean
- `cargo clippy -p zenpng --all-targets -- -D warnings` — clean
- `cargo test --release -p zenpng` — all pass (conformance corpus, APNG,
  iDOT, interlace, streaming, CRC/truncation)
- `cargo test --doc -p zenpng` — pass
- `cargo check --target wasm32-unknown-unknown -p zenpng` — clean
- `cargo check --target wasm32-unknown-unknown -p zenpng --features _dev` — clean
- `cargo fmt --check -p zenpng` — clean
- `cargo bench --bench stream_and_apng` — runs (new lane-attribution bench)
- `cargo bench --bench unfilter_tiers --features _dev` — runs
- `cargo bench --bench scalar_vs_simd --features _dev` — runs
