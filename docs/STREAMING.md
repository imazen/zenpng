# Streaming in zenpng: what streams today

State on 2026-10-08 (main `92e2797` plus the threading-defaults PR, zenflate
`f041b61`). Measurements:
`benchmarks/stream_memory_x86_2026-10-07.txt` (heap / RSS / wall, 4096 px)
and the streaming time ratios in `benchmarks/streaming_timing_x86_2026-10-07.md`.
"PR #25" is imazen/zenpng#25 (`PngEncoderConfig::with_downcast` and
`with_parallel`), not merged yet.

## Encode

zencodec `Encoder::push_rows` + `finish` picks a mode on the first push:

| Mode | When | Streams? | Memory |
|---|---|---|---|
| Stored (`TrueStreaming`) | effort 0, canvas height set | yes: stored DEFLATE rows appended as they arrive | output (≈ raw size) |
| Strips | effort 1-15, canvas height set, **every downcast off**, near-lossless 0, no `iDOT` segments, image ≥ 2 strips (≥ ~1 MiB of filtered rows) | yes: each ~512 KiB strip is screened, refined and compressed when its last row arrives; with `parallel`, on a worker pool (≤ 2 strips per thread in flight) | one strip of rows + output (+ in-flight strips when parallel) |
| Pre-filtered | effort 1, canvas height set, strips not applicable | filter pass only; compresses in `finish()` | filtered image |
| Buffered | everything else: effort ≥ 16, no canvas height, downcasts on (the default), near-lossless, `iDOT`, small images | no: rows are copied, `finish()` runs the one-shot encoder | raw image copy + one-shot working set |

Why the conditions: downcasts (opaque RGBA → RGB, gray, palette, `tRNS`,
16 → 8 bit) need every pixel before the color type is known; efforts 16+
run brute-force and fork/beam searches over the whole image; `iDOT`
segments are planned from the whole stream. The one-shot encode uses the
same strips at every thread count but chooses filters over the whole image;
strip streaming can't see the strips to come and chooses per strip:
e2-e7 +0.01-0.02% geomean over the one-shot encode, e8-e15 +0.07-0.17%
(worst +6.8%, line art at e10;
`benchmarks/strip_layout_sizes_2026-10-08.md`).

What a caller can reach:

| | Today (main) | After PR #25 |
|---|---|---|
| zencodec `push_rows` | stored at e0, pre-filtered at e1, buffered otherwise (no public way to turn downcasts off) | `.with_downcast(DowncastFlags::none())` → strip streaming at e1-15; `.with_parallel(true)` → threaded strips |
| zencodec one-shot `encode` | single-threaded only (`parallel` has no zencodec builder; the threading policy only caps threads) | `.with_parallel(true)` → multi-threaded |
| `zenpng::encode_*` + `EncodeConfig` | one-shot only (no row API); `with_parallel`, `with_downcast` available | same |
| APNG (`AnimationFrameEncoder`) | buffers every frame | same |

Measured (4096x3072, i265 E-cores, heap peak including the 55.8 MB input
load for RGB8 / 68.9 MB for RGBA8; ST = 1 thread, MT = 4):

| effort | RGB8 one-shot ST | push_rows default ST | strip stream ST | one-shot MT | strip stream MT |
|---|---|---|---|---|---|
| 1 | 135 MB, 0.16 s | 114 MB, 0.15 s | 66 MB, 0.15 s | 76 MB, 0.12 s | 74 MB, 0.12 s |
| 2 | 205 MB, 0.35 s | 243 MB, 0.37 s | 68 MB, 0.31 s | 76 MB, 0.16 s | 89 MB, 0.16 s |
| 7 | 205 MB, 1.85 s | 243 MB, 1.88 s | 67 MB, 1.80 s | 87 MB, 0.62 s | 100 MB, 0.57 s |
| 13 | 208 MB, 10.4 s | 246 MB, 10.2 s | 76 MB, 9.9 s | 129 MB, 2.61 s | 151 MB, 2.60 s |
| 15 | 208 MB, 11.7 s | 246 MB, 11.8 s | 76 MB, 11.4 s | 129 MB, 3.03 s | 151 MB, 3.02 s |
| 16 | 209 MB, 25.0 s | 247 MB, 24.3 s | buffers: 247 MB, 24.5 s | 368 MB, 23.3 s | buffers: 406 MB, 23.2 s |
| 19 | 209 MB, 54.8 s | 247 MB, 54.8 s | buffers: 247 MB, 54.6 s | 368 MB, 53.3 s | buffers: 406 MB, 53.7 s |

RGBA8 follows the same pattern (e7: one-shot 313 MB / 2.53 s, strip stream
91 MB / 2.42 s, MT 151 / 124 MB at 0.74 s). Threads don't speed up efforts
16+ (brute force runs on one thread) but raise the heap. Multi-threaded
strip streaming keeps up to 2 strips per thread in flight: on RGB8 its heap
is 13-22 MB above the one-shot MT encode at e2-e15 (the input is only
36 MiB), on RGBA8 27 MB below it at e7.

Small images (under 512 KiB of filtered rows) encode single-threaded even
when threads are allowed (92e2797): thread spawns had made 64 px encodes up
to 5.7x slower.

## Decode

| API | Streams? | Threads |
|---|---|---|
| `zenpng::decode` | row by row internally into one output buffer | `max_threads` 0 (default): `iDOT` files decode strips in parallel; other non-palette, ≥ 8-bit images inflate on a second thread from a per-format size (below) |
| zencodec `push_decoder` | rows decoded straight into the sink's buffer (the sink provides the full height; interlaced images: full decode then copy) | same pipeline as `decode` when the limits' threading policy is parallel (the zencodec default); no `iDOT` parallelism |
| zencodec `streaming_decoder` | yes: batches of ~32 KiB of rows per `next_batch`, input held but output never whole; interlaced images rejected | pipelined like `decode` (borrowed input is copied once for the inflate thread); sequential policy keeps the input borrowed |

Palette and sub-byte gray images never pipeline: their row expansion is the
bottleneck and the handoff made pal8 1.32x slower. The others pipeline from
the size where two threads were measured at least 1.3x faster
(`benchmarks/decode_pipeline_crossover_2026-10-08.md`, `pipeline_min_bytes_for`):

| filtered data | gray8 | RGB8 | RGBA8 | RGB16 and other layouts |
|---|---|---|---|---|
| x86_64 | 1.69 MiB | 2.25 MiB | 3 MiB | 10.13 MiB |
| other targets (Neoverse-N1 data) | 12 MiB | 14.06 MiB | 6.75 MiB | 28.13 MiB |

Pipelined vs one thread (i265 P-cores,
`benchmarks/streaming_timing_x86_2026-10-07.md`, measured with the earlier
512 KiB threshold): 0.70-0.85x at 1024 px and 0.59-0.70x at 4096 px for
gray8/RGB8/RGBA8/RGB16, equal for all three APIs. On Neoverse-N1 the same
pipeline was 1.04-1.27x *slower* at 768-1024 px RGB8 and gray8, hence the
higher thresholds there.

`iDOT` files (independently decodable strips with a table) decode their
strips in parallel in `zenpng::decode` only: 0.18-0.32x of the serial time
at 4096 px with 8 segments (palette included). zenpng writes `iDOT` only
when asked (`EncodeConfig::with_decode_segments(n)`, n >= 2; no zencodec
builder): at most 16 segments and as many as its decoder would use (2 from
2 MiB of filtered rows, +1 per further 4 MiB; none below 2 MiB), never for
1/2/4-bit gray. Measured (4096x3072,
heap peak): RGB8 whole 55.8 MB / 0.10 s, whole MT 56.3 MB / 0.06 s, push
56.3 MB / 0.06 s, streaming 35.9 MB / 0.05 s; RGBA8 68.8 / 69.3 / 69.3 /
36.9 MB.
