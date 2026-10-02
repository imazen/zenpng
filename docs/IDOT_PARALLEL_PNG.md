# Apple `iDOT` parallel PNG: format, implementation, measurements

Status (2026-10-02): **implemented** in zenpng for both decode and encode, on
top of zenflate's segment APIs ([imazen/zenflate#9](https://github.com/imazen/zenflate/pull/9),
patched in via `[patch.crates-io]` until released). Fixtures:
[codec-corpus `png-idot/`](https://github.com/imazen/codec-corpus/tree/main/png-idot).
Raw results: `benchmarks/idot_decode_2026-10-02.{log,meta}`,
`benchmarks/idot_encode_2026-10-02.{log,meta}`, and the original investigation in
`benchmarks/idot_parallel_probe_2026-10-02.{log,meta}`.

## TL;DR

- `iDOT` is an unregistered ancillary chunk that Apple software has written since
  about 2011. It splits the IDAT stream into independently inflatable horizontal
  strips so several cores can decode one PNG. Every other decoder ignores it, and
  the strips are ordinary DEFLATE, so the files stay standard PNGs.
- The layout (§1) is **verified on 8 real Apple-written PNGs**.
- **Decode** (Core Ultra 7 265K, 8P+12E, unpinned shell, median of 41): Apple's
  own 2-segment files decode **1.57–1.71×** faster than `max_threads = 1` at
  1.1–8 MP. zenpng-written files with more segments reach **2.05–2.48×** (4),
  **2.66–3.20×** (8) and **3.99×** (16, 7.6 MP). Every output was byte-identical
  to the serial decode.
- **Encode:** `EncodeConfig::with_decode_segments(n)` costs **−0.09% to +0.40%**
  in size for N ≤ 8 (+0.50% worst case at N = 16) and **+1–8%** encode time at
  efforts 7–19. It is off by default; unsegmented output is byte-identical to
  before.
- **Conformance:** the parallel path is only used when it provably matches the
  serial decode (§2); anything else falls back. A corpus of real, adversarial
  (Buchanan's "ambiguous PNG") and 26 generated cases checks this on every
  build, and mutation tests confirm the checks are load-bearing.

## 1. Wire format

All fields are big-endian `u32`. The chunk data is `4 + 12·N` bytes; Apple always
writes `N = 2`, which gives 28 bytes.

| word | field | Apple value |
|---|---|---|
| 0 | `N`, the segment count | 2 |
| 1 + 3k | `first_row[k]` | 0, H/2 |
| 2 + 3k | `row_count[k]` | H/2, H/2 |
| 3 + 3k | `offset[k]`: byte offset of segment k's first IDAT chunk, **measured from the first byte of the iDOT chunk's length field** | 40, … |

Verification: on all 8 sample files, `iDOT.pos + offset[k]` lands exactly on an
`IDAT` chunk header (`examples/idot_probe.rs`, and the dump script in the
investigation notes).

How this reconciles the published descriptions:

- [Hacker Factor, "Connecting the iDOTs" (2020)](https://hackerfactor.com/blog/index.php?/archives/895-Connecting-the-iDOTs.html)
  reads the seven words as "divisor, 0, divided height, 0x40?, first half, second
  half, restart offset". The `0` is `first_row[0]`, and the mystery word is
  `offset[0]`. It is 0x28 (40), not 0x40: 12 bytes of chunk framing plus 28 bytes
  of data, so the first IDAT immediately follows the iDOT. A commenter there
  who worked on it at Apple says it was added for launch-time PNG decoding on
  the first Retina iPads.
- [Buchanan's ambiguous-png-packer](https://github.com/DavidBuchanan314/ambiguous-png-packer)
  writes `2, 0, 1, 40, 1, h-1, off` (a 1-row first segment), which Apple accepted.
  That confirms the split need not be even.
- [Nedra1998/Novi](https://github.com/Nedra1998/Novi) parses it as a fixed header
  plus `N-1` records. That is byte-equivalent for N = 2.
- The W3C [PNG extensions registry](https://w3c.github.io/png/extensions/Overview.html)
  (v1.6.1, 2025-09-15) lists `iDOT` under "chunks not described here" and points to
  the Hacker Factor post.

**Unknown:** how Apple's ImageIO treats `N ≠ 2`, and which half it expects to get
the extra row when H is odd. Buchanan's file shows that an uneven `1 / h-1` split
is accepted. Nobody has published N > 2 behaviour.

### What Apple's encoder actually writes (8 files, macOS 11–14 screenshots)

- Chunk order: `IHDR iCCP eXIf pHYs iTXt iDOT IDAT… IEND`, with `iDOT`
  **immediately before the first IDAT**.
- IDAT payloads are 16384 bytes. Segment 0's last IDAT is short and ends with
  `00 00 ff ff`, an empty stored block from a full flush (BFINAL = 0, byte-aligned).
  Segment 1 starts a new IDAT chunk.
- Segment 1 inflates with an **empty window**: decoding it as a separate raw
  DEFLATE stream reproduced the serial bytes exactly in all 8 files. That means a
  full flush (no back-references across the boundary), not just a sync flush.
- **Filters: only None (0) and Sub (1)**, in every row of all 8 files. The first
  row of segment 1 is therefore independent of segment 0's last row. That is a
  property of Apple's encoder, not a rule the format states.
- The zlib Adler-32 trailer covers the whole filtered stream, not each segment.
- All 8 samples are RGBA8, non-interlaced, with no `tRNS`, so they take zenpng's
  passthrough decode path (`src/decoder/mod.rs`, `is_passthrough`).

### Related proposal: `mARK` (not standardised)

[w3c/png#60](https://github.com/w3c/png/issues/60) /
[libspng/png-restart-marker](https://github.com/libspng/png-restart-marker)
proposes `mARK`, a registered take on the same idea for the 5th edition. It has:
1-byte method, 1-byte type, `u32` segment count, optional offsets; it must appear
once, before IDAT; non-first segments must start with filter None or Sub; every
segment must start with an IDAT header and decompress independently; it is only
defined for interlace 0; and decoders must fall back to serial on any error. The
related threads are [w3c/png#54](https://github.com/w3c/png/issues/54) and the
[PNG WG charter draft](https://w3c.github.io/charter-drafts/2025/png-wg.html),
which lists parallel encode/decode as 5th-edition research. Because the
decoder machinery below would be shared, adding `mARK` later is cheap. Emitting
it is not worth doing until it is registered.

## 2. The serial-equivalence problem (why this needs care)

A PNG decoder that ignores `iDOT` reads one zlib stream. A parallel decoder
reads N streams starting at offsets the file itself claims. If the file lies,
the two produce different pixels.
[Buchanan, "ambiguous PNG"](https://www.da.vidbuchanan.co.uk/widgets/pngdiff/)
showed that Apple's parallel decoder did exactly that. The packer's samples are
MIT-licensed:

- `mac_vs_ibm_output.png`: segment 0 ends **inside** a stored block (the 5-byte
  stored header is in segment 0 and its payload in segment 1). A serial decoder
  treats segment 1's bytes as literal stored data, while a parallel decoder
  decodes them as fresh DEFLATE. The result is two different images. The probe
  shows the independent decode of segment 1 does **not** match the serial bytes.
- `race_condition.png`: segments overlap (`first_row` 0 and 0, `row_count` 960
  and 960, in a 960-row image). The probe panics on the out-of-range split, which
  is the point.

zenpng's rule, in line with the "two code paths for the same operation must
produce the same output" policy, is this: **the parallel path is an
optimisation of the serial decode, never a second interpretation.** Every input
either decodes to exactly the serial result or falls back to the serial path.

### Proof obligations, all checked before parallel output is accepted

1. **Static checks (before any work):** interlace 0; `iDOT` length is
   `4 + 12·N` with `2 ≤ N ≤ min(H, MAX_SEGMENTS)`; `first_row[0] = 0`;
   `first_row[k+1] = first_row[k] + row_count[k]`; `Σ row_count = H`; every
   `row_count ≥ 1`. `iDOT` must come before the first IDAT, and the offsets must be
   strictly increasing. Each `iDOT.pos + offset[k]` must be the header of an IDAT
   chunk inside the contiguous IDAT run, and `offset[0]` must point to the first
   IDAT. All arithmetic is checked (`alloc_util`, 32-bit clean).
2. **Segment k < N-1 ends cleanly:** after its input is exhausted, the inflater
   must sit in the between-blocks state, byte-aligned, with zero pending bits.
   It must not have seen BFINAL, and it must have produced exactly
   `row_count[k] · (stride + 1)` bytes. This is the check that defeats
   `mac_vs_ibm_output.png`. If it holds, a serial decoder's next block header
   starts at the same byte the parallel decoder starts segment k+1 from.
3. **Segment k > 0 is window-independent:** it is decoded with an empty window,
   and any back-reference before the segment start is an error. zenflate's
   `StreamDecompressor` already rejects these (`offset > lookback_valid`,
   `src/decompress/streaming.rs` in zenflate main 13401e9) instead of reading
   zero-fill. If segment k+1 makes no such reference, its output equals the
   serial output, because a serial decoder would compute the same bytes from the
   same symbols.
4. **Filter dependency:** if the first row of segment k > 0 uses Up, Average or
   Paeth, it needs segment k-1's last row. zenpng falls back to serial (a
   deferred unfilter is possible but not implemented). Apple never emits this,
   but it is conformant PNG.
5. **Last segment:** must end with BFINAL and produce no extra rows; anything
   else falls back.
6. **Checksums:** the CRC-32 check per chunk is unchanged (bounded IDAT sources
   check it per segment). The per-segment Adler-32s, which zenflate computes
   anyway, are joined with `zenflate::adler32_combine` and compared with the
   zlib footer on **every** parallel decode, strict or not. A mismatch falls
   back, so the serial decoder decides what to report (a warning by default,
   an error in strict mode).

Failure of any check means discarding the parallel result and running the
unchanged serial decoder. Decoders must not reject a file for a bad `iDOT`: it
is ancillary, and the only consequence is losing parallelism. That differs from
Apple, which raises "iDOT doesn't point to valid IDAT chunk", and at least once
crashed on a bad iDOT (CVE-2016-1811).

## 3. What zenpng implements

### Decode (`src/decoder/idot.rs`, `src/affinity.rs`)

- `RowDecoder::new` records the `iDOT` chunk (a second `iDOT` disables it) and
  `idot::validate` applies the static checks of §2 (1 < N ≤ 64, contiguous rows
  covering the image, offsets on IDAT chunk headers in increasing order, the
  first at the first IDAT, interlace 0).
- `decode_png` tries the parallel path on both the RGB8/RGBA8 passthrough path
  and the post-processed path (every other format), before the serial row loop.
  On `Outcome::Fallback` the untouched serial decoder runs.
- Each segment is a work unit: a bounded `IdatSource` over its IDAT chunks, a
  `StreamDecompressor` (`zlib` for segment 0, `zlib_continuation` after) with
  `with_segment_end(true)`, and rows unfiltered (and post-processed) straight
  into that segment's disjoint slice of the output buffer. Workers claim units
  from a shared queue, so a fast core takes more segments; the calling thread
  is one of the workers. Any failure sets an abort flag and falls back.
- Output buffers come from `alloc_util::alloc_zeroed`, which now uses
  `bytemuck::allocation::try_zeroed_vec` (fallible calloc) instead of
  `try_reserve_exact` + `resize`. The old path zero-filled the whole output on
  the calling thread before any worker started. On w33 (20 MB RGBA) a trace
  showed ~6 ms of a 10.9 ms parallel decode spent outside the workers; the
  calloc change cut that decode from 10.1 to 8.4 ms in the same benchmark
  (log section A predates it). Serial decode time is unchanged by it.

**API:** `PngDecodeConfig::max_threads` / `with_max_threads(n)`: 0 = automatic
(default), 1 = never parallel, N = cap. zencodec's `ThreadingPolicy::Sequential`
maps to 1. Files without `iDOT` are unaffected.

### Thresholds

`workers = min(threads, segments, workers_for_bytes(rows × stride))`, where
`workers_for_bytes` allows 2 workers from 2 MiB of filtered data and one more
per additional 4 MiB (`idot::MIN_BYTES_PER_WORKER`, `EXTRA_WORKER_BYTES`). From
the forced-worker sweep (benchmark log section B):

| filtered data | 2 workers | 4 workers | 8 workers |
|---|---|---|---|
| 1.0 MB (512² RGBA) | 1.19× / 1.02× | noisy | slower |
| 4.2 MB (1024² RGBA) | 1.43× / 1.33× | 1.20× | 1.04× |
| 8.4–8.6 MB (2048×1024, 1448² RGBA) | 1.21–1.73× | 1.46–1.64× | 1.41–1.58× |
| 13–30 MB (4.5–7.6 MP) | 1.6–1.8× | 1.86–2.47× | best only at 7.6 MP |

### Core tiers

On hybrid CPUs, an equal split finishes when its slowest strip does. Linux
workers can be pinned (via `rustix::thread::sched_setaffinity`, no `unsafe` in
zenpng) to the fastest tier, detected from sysfs `cpu_capacity` or
`cpuinfo_max_freq` (CPUs within 15% of the fastest; one tier = no pinning).
The calling thread is pinned for the duration and restored afterwards.

Policy, from measurement (log sections A, C, D):

- **Pin when every worker gets one segment and they fill at most half the fast
  tier** (Apple's N = 2; N = 4 on an 8-P-core part). Apple originals, unpinned
  shell: pinned 1.57–1.71× vs unpinned 1.31–1.48×.
- **Otherwise don't pin**: with more segments than workers the queue balances
  across all cores, and filling the whole P tier measured slower (w37 N = 8:
  2.49× pinned to 8 P-cores vs 3.20× spread over all 20).
- Not implemented: macOS (no affinity API; QoS classes would be the analogue)
  and Windows (CPU sets). Not measured: Zen 5 (`dev`), ARM big.LITTLE.

### Results (log section C, final policy, unpinned shell, median of 41)

| file | MP | segments | serial ms | auto ms | speedup |
|---|---|---|---|---|---|
| Apple w42 | 1.11 | 2 | 2.76 | 1.76 | 1.57× |
| Apple w33 | 5.18 | 2 | 8.57 | 5.02 | 1.71× |
| Apple w37 | 7.62 | 2 | 13.07 | 8.35 | 1.57× |
| zenpng w39 | 4.53 | 4 | 7.37 | 3.60 | 2.05× |
| zenpng w39 | 4.53 | 8 | 7.37 | 2.77 | 2.66× |
| zenpng w37 | 7.62 | 4 | 17.73 | 7.15 | 2.48× |
| zenpng w37 | 7.62 | 8 | 17.86 | 5.58 | 3.20× |
| zenpng w37 | 7.62 | 16 | 17.81 | 4.47 | 3.99× |

(zenpng's effort-7 re-encodes decode slower serially than Apple's originals,
probably because they use Up/Average/Paeth where Apple uses only None/Sub; not
profiled.)

### Encode (`src/encoder/segments.rs`)

`EncodeConfig::decode_segments` / `with_decode_segments(n)` (0/1 = off, the
default; 2 = Apple's layout; more = more strips, untested on Apple software).
After the normal pipeline picks filters and produces one zlib stream, the
encoder:

1. re-inflates it to recover the chosen filter bytes,
2. re-filters each non-first segment's first row as None or Sub (smaller
   absolute byte sum) if it used Up/Average/Paeth,
3. compresses the segments in parallel with
   `Compressor::deflate_compress_segment` at the pipeline's final level
   (`compress::segment_level`: FullOptimal at 31+, else the largest refine
   level),
4. writes `iDOT` immediately before the first IDAT (`offset[0] = 40`, as Apple
   does) and one IDAT chunk per segment.

The segment count follows the decoder's thresholds, so images under 2 MiB of
filtered data get no `iDOT`. It applies to non-interlaced still images at
effort ≥ 1 (truecolor, gray, sub-byte gray, indexed); APNG is unaffected.
zenzop (effort 46+) output is re-compressed with zenflate's FullOptimal
instead, because zenzop has no segment mode.

| image | effort | N=2 size | N=4 | N=8 | time N=1 → N=2 |
|---|---|---|---|---|---|
| w39 4.5 MP | 7 | −0.02% | +0.05% | +0.28% | 288 → 293 ms |
| w37 7.6 MP | 7 | +0.06% | +0.11% | +0.17% | 544 → 564 ms |
| w39 4.5 MP | 13 | −0.09% | +0.12% | +0.28% | 733 → 745 ms |
| w37 7.6 MP | 13 | +0.09% | +0.10% | +0.24% | 1441 → 1546 ms |
| w39 4.5 MP | 19 | +0.11% | +0.40% | +0.39% | 3989 → 4282 ms |
| w37 7.6 MP | 19 | +0.06% | +0.12% | +0.22% | 8452 → 9134 ms |

The re-segment pass costs extra time rather than saving it, because the
pipeline still compresses the whole stream first. Folding segments into the
pipeline's final compression would turn this into an encode-time *speedup*
(the probe measured 1.87–1.89× on the deflate phase at efforts 13–24 with
N = 2); not implemented.

## 4. Tests

- `tests/idot.rs`: every file in codec-corpus `png-idot/` (3 Apple, 2
  adversarial, 26 generated) decodes identically with `max_threads` 1, 0, 2, 3
  and 8, under default and strict configs (pixels, warnings, or error). With
  `--features _dev`, it also asserts which files completed in parallel (all
  Apple files and manifest entries marked `parallel_usable`) and that the rest
  never did. Mutation-checked: disabling the boundary-filter check fails on
  `boundary_row_average.png`, and disabling the segment-boundary check fails on
  `complete_stream_in_segment0.png`.
- `tests/idot_encode.rs`: the size rule; `decode_segments` 0/1 byte-identical
  to the default; round-trips at efforts 1/7/13 × N = 2/3/8 through the `png`
  crate and through zenpng serially and in parallel.
- zenflate `tests/segments.rs`: segment round-trips at efforts 0–31,
  continuation checksums, and the adversarial stream shapes.
- CI runs the iDOT tests with `_dev` on every platform; the i686 `cross` job
  fetches the corpus on the host.

## 5. Not done

- **Apple acceptance of zenpng output.** Nothing here has been decoded by
  ImageIO. A macOS CI job that decodes zenpng `iDOT` files through
  `CGImageSource` and checks for "iDOT doesn't point to valid IDAT chunk" is
  required before calling the encoder Apple-conformant, and before claiming
  N > 2 is safe for Apple software.
- Deferred unfilter for segments starting with Up/Average/Paeth (falls back).
- Pinning on macOS/Windows; measurements on Zen 5 and ARM.
- Segment-aware final compression in the encode pipeline (an encode speedup
  instead of a +1–8% cost).
- `mARK` (the W3C restart-marker proposal) decode would reuse this machinery.
- zenflate#9 must be released, and the `[patch.crates-io]` entry removed,
  before a zenpng release.
