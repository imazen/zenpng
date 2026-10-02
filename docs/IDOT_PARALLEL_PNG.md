# Apple `iDOT` parallel PNG: format, measurements, and a design for zenpng

Status: investigation (2026-10-02). Nothing here is implemented in the decoder or
encoder yet. The probe harness is `examples/idot_probe.rs`; raw results are in
`benchmarks/idot_parallel_probe_2026-10-02.{log,meta}`.

## TL;DR

- `iDOT` is an unregistered ancillary chunk that Apple software has written since
  about 2011. It splits the IDAT stream into independently inflatable horizontal
  strips so that two cores can decode one PNG. Every other decoder ignores it, and
  the strips are ordinary DEFLATE, so the files stay standard PNGs.
- The layout below is **verified on 8 real Apple-written PNGs**. It is a clean
  generalisation that also explains every published reverse-engineering note.
- **Decode:** with segments decoded on two P-cores, the probe's fused
  inflate+unfilter loop scales **1.69–1.86×** over the same loop run serially.
  It is **1.06–1.30×** faster than today's `zenpng::decode` on images of 4.5 MP and
  up, and **0.83–0.93×** (slower) at 0.6–1.1 MP. The probe loop carries an extra row
  copy that `RowDecoder` does not, so the real gain should sit between those two
  figures. That is untested until a RowDecoder-per-segment version exists. When the
  same run was unpinned on a hybrid CPU under load, it was **slower** than serial
  (0.46–0.62×), because the second thread landed on an E-core. Scheduling policy
  matters as much as the format.
- **Encode:** splitting into 2 independent segments costs **−0.17% to +0.23%**
  in size (median about +0.04%) and roughly halves deflate wall time at effort
  13–24. This parallel-encode benefit applies to every decoder, not only Apple's.
- **Conformance hazard:** a naive parallel decoder can produce a *different image*
  from a serial one (Buchanan's "ambiguous PNG"; its README says Apple appears to have patched it).
  zenpng must only take the parallel path when it can prove the result is
  byte-identical to the serial decode, and must fall back to serial otherwise.
  The probe reproduces both published attack shapes, and the validation rules
  below reject both.

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
   Paeth, it needs segment k-1's last row. The fallback is a deferred unfilter
   (see §3). Apple never emits this, but it is conformant PNG.
5. **Last segment:** must end with BFINAL. The bytes that follow are handled
   exactly as the serial path handles them (Adler-32 trailer, trailing data), so
   warnings and errors match.
6. **Checksums:** the CRC-32 check per chunk is unchanged. When Adler-32 is
   verified (`PngDecodeConfig::strict()`), each worker computes Adler-32 over its
   filtered bytes and the results are joined with `zenflate::adler32_combine`, so
   a mismatch raises the same error the serial path raises.

Failure of any check means discarding the parallel result and running the
unchanged serial decoder. Decoders must not reject a file for a bad `iDOT`: it
is ancillary, and the only consequence is losing parallelism. That differs from
Apple, which raises "iDOT doesn't point to valid IDAT chunk", and at least once
crashed on a bad iDOT (CVE-2016-1811).

## 3. Decoder design

Scope for the first version: full-frame `decode()` and the zencodec full-frame
path, non-interlaced, single image (not APNG `fdAT`). Streaming row-by-row
decode stays serial.

1. **Parse:** add `idot: Option<IdotTable>` to `PngAncillary` (crate-private),
   recording the chunk position and the validated `(first_row, row_count,
   chunk_pos)` triples. `ancillary.rs` currently drops unknown chunks
   (`_ => {}`), so nothing round-trips today. That is good: a stale `iDOT` must
   never be copied (its 4th letter is uppercase, meaning not safe to copy), and
   oxipng also strips it.
2. **Gate:** use the parallel path only when the static checks pass, the thread
   budget is above 1 (`max_threads`, or zencodec `ThreadingPolicy`), and the image
   is at least about 1 MP (to be set from the RowDecoder-based measurement; the
   probe breaks even somewhere between 1.1 and 4.5 MP).
3. **Execution:** allocate the output once, as today, and split it at segment row
   boundaries with `split_at_mut` (disjoint `&mut`, no unsafe code).
   **Segment 0 runs on the calling thread.** Segments 1..N run in
   `std::thread::scope`. Running segment 0 locally avoids a spawn and keeps half
   the work on the core the caller is already using, which matters on hybrid
   CPUs (§4). Each worker gets:
   - an `IdatSource` bounded to the chunk range `[offset[k], offset[k+1])`;
   - a `StreamDecompressor` (zlib for segment 0, raw DEFLATE after that);
   - a `RowDecoder` writing rows straight into its output slice, with a zeroed
     "previous row" for the segment's first row, which is valid for filter
     None or Sub.
   Per-row post-processing (the non-passthrough formats) is row-local, so it runs
   inside the worker.
4. **Deferred unfilter (obligation 4):** if segment k's first filter byte is Up,
   Average or Paeth, the worker inflates into a filtered scratch buffer. That
   costs `row_count · (stride+1)` extra bytes, counted against `max_memory_bytes`.
   It unfilters after joining segment k-1. Inflate, the larger share of the work,
   stays parallel.
5. **Fallback:** on any worker error or failed obligation, run the existing
   serial decode from the start. Segment 0 is byte-for-byte a prefix of the
   serial decode, so a later version could resume from it. That is not worth the
   complexity at first.

### zenflate API needed (additive, no semver break)

- A way to tell, once the source reports EOF, whether a `StreamDecompressor` is
  at a byte-aligned block boundary with no BFINAL seen. That could be a
  `segment` constructor/mode that returns `Ok` at a clean boundary instead of a
  truncation error, plus an accessor like `ended_at_block_boundary()`. Today an
  EOF mid-stream is treated as truncation.
- Optional: a per-segment Adler-32 value from the raw-mode decoder (it can also
  be computed in zenpng).

## 4. Measurements (decode)

From `benchmarks/idot_parallel_probe_2026-10-02.log`: `i265`, Core Ultra 7 265K
(8 P-cores plus 12 E-cores), a shared box with load average around 10, medians of
9 runs. "Fused" is the probe's streaming inflate+unfilter loop, the same shape
as `RowDecoder` but with one extra row copy.

| image | MP | zenpng::decode (ms) | fused serial (ms) | fused 2-seg on P-cores (ms) | scaling | vs zenpng::decode |
|---|---|---|---|---|---|---|
| w46 1160×556 | 0.64 | 1.33 | 3.00 | 1.61 | 1.86× | 0.83× |
| w42 1204×918 | 1.1 | 3.16 | 5.78 | 3.42 | 1.69× | 0.93× |
| w39 2784×1626 | 4.5 | 6.96 | 11.43 | 6.58 | 1.74× | 1.06× |
| w33 2880×1800 | 5.2 | 10.31 | 14.92 | 8.56 | 1.74× | 1.20× |
| w37 3350×2274 | 7.6 | 15.44 | 21.30 | 11.87 | 1.79× | 1.30× |
| w43 3080×2292 | 7.1 | 15.76 | 21.71 | 12.29 | 1.77× | 1.28× |

All parallel outputs were byte-identical to `zenpng::decode`.

**Hybrid-CPU result.** Unpinned (`taskset 0-19`), the 2-segment fused path ran at
**0.46–0.62×** of `zenpng::decode`, which is slower than serial. Pinned to E-cores, the
whole-image unfilter took 16.69 ms against 6.41 ms on a P-core (w33, log run 3). A 2-way split
can only finish when its slowest segment finishes, so one segment on an E-core
erases the gain. This is the reason for "segment 0 on the calling thread". It also
means parallel decode should be opt-in, or limited to callers that hand zenpng a
thread budget, rather than on by default. Not yet measured: the same experiment
on a homogeneous box (`dev`, Zen 5) and on ARM.

The probe's two-pass variant (inflate the whole segment, then unfilter) scaled
only 1.15–1.60×. The likely cause, not profiled, is the 10–16 MB intermediate
buffers falling out of cache. Either way, the per-row fused shape is the one to
build.

## 5. Encoder design

Parallel encoding is the larger win. Compression dominates encode time, the
output stays a standard PNG for every decoder, and Apple decoders also get
parallel decode.

1. **Option (needs API approval):** for example
   `EncodeConfig::with_parallel_segments(Segments::Off | Segments::Apple2)`.
   Default Off, so existing outputs stay byte-identical and the effort
   monotonicity tables stay valid.
2. **Layout: emit exactly what Apple emits.** `N = 2`, `first_row = [0, ⌈H/2⌉]`
   (or `⌊H/2⌋`; it is untested which one ImageIO prefers for odd H, so this needs
   a macOS check). `iDOT` goes immediately before the first IDAT, so
   `offset[0] = 40`. Segment 1 starts a new IDAT chunk. Do not emit `N > 2`: if
   ImageIO rejects it, Apple users get an error dialog rather than a slower
   decode. Skip the chunk entirely when H < 2, for interlaced images, and for APNG.
3. **Filtering:** pick filters per segment as usual, but restrict each
   non-first segment's first row to None or Sub. That costs one row per segment.
   Apple decoders may rely on it, and `mARK` requires it.
4. **Compression:** compress each segment as independent raw DEFLATE (no preset
   dictionary). Non-final segments end with BFINAL = 0 plus an empty stored block,
   so they finish byte-aligned. zenflate already has this, privately:
   `Compressor::deflate_compress_chunk` with `force_nonfinal`, used by
   `gzip_compress_parallel` (zenflate `src/compress/mod.rs`). It needs a public
   additive entry point with `chunk_start = 0`. The zlib header goes before
   segment 0, and the Adler-32 trailer is `adler32_combine` of the per-segment
   Adler values. FullOptimal/zenzop recompression runs per segment.
5. **Size cost**, measured by compressing Apple's own filtered bytes as N
   independent segments (`IDOT_ENC=1`, P-cores):

   | image | effort | N=2 size Δ | N=2 deflate wall | N=4 size Δ |
   |---|---|---|---|---|
   | w46 0.64 MP | 13 / 19 / 24 | +0.03% / +0.23% / +0.13% | 8.9→4.8 / 40.8→23.4 / 154→83 ms | −0.08% / +0.23% / +0.30% |
   | w39 4.5 MP | 13 / 19 / 24 | −0.08% / +0.07% / +0.09% | 34→18 / 95→50 / 585→309 ms | −0.13% / +0.03% / +0.15% |
   | w37 7.6 MP | 13 / 19 / 24 | +0.05% / +0.03% / +0.05% | 71→38 / 173→95 / 1281→708 ms | +0.08% / +0.08% / +0.10% |

   Caveats: this is one content class (screenshots) and three images. It does not
   include zenpng's own filter search or the effects of the effort pipeline.
   Before any default changes, the full sweep discipline applies: sizes from tiny
   to large, effort 0–200, and photo, screen and line-art content.
6. **Parallelism beyond N = 2 without iDOT.** For encode speed alone, segments
   could be split further (pigz style) without making the extra boundaries
   public. Only the boundary recorded in `iDOT` has to match Apple's layout.

## 6. Test plan

- **Fixtures (`codec-corpus`, not git):** several real Apple iDOT PNGs
  (Wikimedia Commons, freely licensed; see the `.meta` file),
  `mac_vs_ibm_output.png` and `race_condition.png` (MIT), plus synthetic edge
  cases:
  - offset pointing into the middle of a chunk, to a non-IDAT chunk, or past EOF;
  - N = 0, N = 1, N > H, overlapping or gapped row ranges;
  - an Up/Avg/Paeth first row in segment 1;
  - a back-reference across the boundary;
  - BFINAL inside segment 0;
  - `iDOT` after IDAT; interlaced with `iDOT`;
  - a valid layout with corrupt Adler-32 in strict mode.
- **Invariant tests:** for every fixture, parallel decode output, warnings and
  error equal the serial decode. Tests must not skip silently; the corpus is
  fetched explicitly.
- **Encoder round-trip:** zenpng decode (serial and parallel), plus a second
  decoder (`png` crate or libdeflater dev-dep) for standard-PNG validity, plus
  serial == parallel.
- **Apple acceptance (CI):** a `macos-latest` job that decodes zenpng `iDOT`
  outputs through ImageIO (a small Swift script using `CGImageSourceCreateImageAtIndex`,
  or `sips`). It compares pixels to the source and fails on any
  "iDOT doesn't point to valid IDAT" log line. Without it, the encoder half cannot
  be called conformant.
- **Fuzzing:** add an `iDOT`-aware fuzz target asserting parallel == serial on
  arbitrary inputs. This is the property that would have caught Apple's bug.

## 7. Order of work

1. zenflate: the segment-end API (decode) and a public non-final segment
   compressor (encode). Both are additive.
2. Decoder: parse + validate + fallback, passthrough path first (it covers every
   Apple sample), with the corpus fixtures and the parallel == serial invariant.
3. Measure the RowDecoder-based version on `i265` (hybrid), `dev` (Zen 5) and ARM,
   then set the size threshold and the default policy from those numbers.
4. Encoder `Apple2` mode with macOS CI acceptance.
5. Optional: `mARK` decode (same machinery), deferred unfilter, more than 2
   internal encode segments.
