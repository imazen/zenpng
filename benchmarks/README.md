# zenpng benchmarks

Committed benchmark data and methodology. Each result is paired with the source
file in this directory, the git commit it came from, the host it ran on, and the
exact command. Numbers here are only those actually measured — nothing is
extrapolated or estimated.

Standard discipline (per the workspace benchmarking rules):

- Built **without** `-C target-cpu=native` — runtime SIMD dispatch (archmage) is
  what ships, so that is what's measured.
- Microbenchmarks use [zenbench](https://github.com/imazen/zenbench) (interleaved
  round-robin, paired statistics); inputs are loaded into RAM before the timed
  region.
- Corpus sweeps (`effort_curve`, `crusher_bench`) iterate a directory of images
  and report encoded **size** (IO-independent) plus wall time across the corpus.

## Environment

| Class | Host |
|-------|------|
| x86 microbenches + resource sweeps | AMD Ryzen 9 7950X (16-core), 59 GiB RAM, WSL2 |
| ARM unfilter | Hetzner CAX — Ampere Altra / Neoverse-N1, aarch64, 4 vCPU / ~8 GB, rustc 1.96.0 |

## Compression effort curve

`effort_curve_2026-02-26.csv` (`.meta`) — commit `0939481`. All 31 standard
effort levels (e0–e30), per-image encoded size and encode time over a 100-image
representative corpus. The aggregate size-vs-time curve is monotonic; ~5.7% of
per-image points show small non-monotonic wobble inherent to DEFLATE.

```sh
cargo run --release --features _dev --example effort_curve <corpus-dir>
```

Rendered charts (repo root): `effort_curve_fast.svg`, `effort_curve_detail.svg`
(generator: `benchmarks/gen_effort_charts.py`).

## Compression vs. other PNG optimizers

`crusher_bench_2026-02-26.csv` (`.meta`) — commit `0939481`. Maximum-effort
zenpng (effort 31, `zopfli` feature) against external PNG optimizers. Aggregate
encoded bytes on the **13 still images** of the corpus (APNG files excluded — ECT
does not optimize APNG):

| Encoder | Total bytes | vs zenpng |
|---------|-------------|-----------|
| zenpng E31 | 1,108,045 | — (smallest; wins 8/13 images) |
| ECT `-9` | 1,109,219 | +0.11% |
| zopflipng | 1,114,820 | +0.61% |
| oxipng `-omax` | 1,117,256 | +0.83% |

```sh
cargo run --release --features zopfli --example crusher_bench <corpus-dir>
```

The corpus is small (13 still images), so treat the margin as a rough indicator,
not a guarantee. **Reproduction limitation:** the corpus
(`/mnt/v/output/zenpng/test_corpus`) is not public, and the external CLI tool
versions (oxipng / ect / zopflipng / optipng / pngcrush) were not recorded in
the original run — so the cross-tool comparison is a historical snapshot, not
byte-for-byte reproducible from this file alone. The per-image CSV is committed
for inspection.

## SIMD unfiltering & scan predicates

`scalar_vs_simd_2026-05-01.log` (`.meta`) — Ryzen 9 7950X. Scalar vs. SIMD
(magetypes 512-bit generic) for the encoder's downcast scan predicates
(`is_grayscale`, `alpha_is_binary`, bit-replication, fused 3-in-1):

- In-cache (1 MP): 4–15× faster than scalar.
- DRAM-bound (16 MP): ~2× — at the memory-bandwidth ceiling; fusing the three
  checks recovers 4.3×.

Decision recorded in the meta: keep magetypes generic SIMD; no hand-written
per-arch intrinsics (no candidate showed the ≥10% gap that would justify them).

```sh
cargo bench --features _dev --bench scalar_vs_simd
```

Related predicate benches: `scan_predicates_2026-05-01.log`,
`fused_predicates_2026-05-01.log` (`.meta` each).

### ARM Sub unfilter

`zenpng_arm_sub_unfilter_2026-05-29.tsv` (`.meta`) — Neoverse-N1, rustc 1.96.0,
no `target-cpu=native`. NEON Sub-unfilter rewrite that keeps the running
reconstructed pixel in a NEON register across iterations (median of 5,
back-to-back A/B):

- Sub bpp=3: +68% (981 → 1649 MB/s)
- Sub bpp=4: +24% (1260 → 1563 MB/s)

```sh
cargo run --release --features _dev --example unfilter_bench
```

## Resource estimation (peak memory & time)

Calibration data behind `zenpng::heuristics::{estimate_encode, estimate_decode}`.
Each cell is one process; peak memory is VmHWM (corroborated against heaptrack on
sampled cells).

| File | Commit | What |
|------|--------|------|
| `png_resource_main_2026-06-14.tsv` (+ `higheffort`, `alphadepth`) | `975eebc` | VmHWM + wall vs size × effort × content class (single-thread) |
| `vcpu_resource_sweep_2026-06-20.tsv` | `7a6ee94` | peak mem / heap / wall vs **thread count** (1–28) |
| `zenpng_encode_mem_2026-06-23.tsv` | `246d7c2` | encode VmHWM admission-safety recalibration, 96-cell grid |

Key findings (recorded in the `.meta` files):

- **Threading:** zenpng parallelizes over filter *strategies*; wall-time speedup
  saturates at ~3× by ~2 threads (not linear in thread count). Peak heap **grows**
  with threads (≈ +90% from 1→8 threads at 2048² e13) — a real
  speed-for-memory trade.
- **Admission safety:** the 2026-06-23 recalibration raised the fixed overhead
  (6→8 MiB) and per-effort anchors so the `typical` peak-memory estimate
  (`EncodeEstimate::peak_memory_bytes`) never under-predicts measured VmHWM across
  the swept grid (0/96 cells under after fit).

```sh
cargo build --release --example png_probe
# vCPU sweep (one process per cell, capped via run-heavy):
run-heavy --mem 24G -- bash scripts/vcpu_resource_sweep.sh \
  target/release/examples/png_probe <images-dir> benchmarks/vcpu_resource_sweep_2026-06-20.tsv
```
