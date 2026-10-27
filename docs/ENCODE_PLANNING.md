# Effective-job planning (zencodec adapter)

Experimental. The common zencodec planning hook is not settled, so these method
names may change to match it.

```rust
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zencodec::output_patch::OutputAccess;
use zenpng::PngEncoderConfig;

let job = PngEncoderConfig::new().job();
// The colour the rows will carry, declared before any header byte exists.
let context = pixels.color_context().map(|c| &**c);
let timing = job.plan_rows(width, height, pixels.descriptor(), context, OutputAccess::Forward)?;
// ... admit the plan; retain this exact job ...
let mut encoder = job.encoder()?;
let started = encoder.begin_declared(width, height, pixels.descriptor(), context, OutputAccess::Forward)?;
assert_eq!(started, timing); // begin_declared errors before handing out bytes if they differ
```

`plan_rows` and `begin_declared` run one preparation (`prepare_rows`): the
canvas, colour binding, limits, metadata policy and chunk validation, and the
row-mode decision. Planning allocates no rows and starts no strip workers.
`begin` and `begin_output` are `begin_declared` without a context. PNG output is
append-only, so seek-back access changes nothing. Input is one pass of rows in
every mode; there is no Adam7 encode.

The plan reports byte availability only. It is not a memory bound or permission
to apply backpressure.

## Row modes

`begin_declared` picks the first mode that applies:

| Mode | When | Output | Kept until `finish` |
|---|---|---|---|
| Stored | effort 0 | each row as it is pushed | one row of scratch and the output not yet taken |
| Strips | efforts 1-15, `DowncastFlags::none()`, no near-lossless or `iDOT` segments, at least two ~512 KiB strips | each strip once its last row arrives; the lag depends on the resolved worker count | one strip of rows, plus strips in flight with workers |
| Pre-filtered | effort 1 otherwise | at `finish` | the filtered image |
| Buffered | everything else | at `finish` | the image |

Downcast analysis is whole-image, so with default flags only effort 0 streams.
`tests/row_planning.rs` compares plan and startup over 1,728 cells.
[`STREAMING.md`](STREAMING.md) measures whole encodes per mode.

## Measured heap

`examples/encode_planning_probe.rs` under `scripts/profile-encode-planning.py`
(heaptrack event replay, cross-checked against `heaptrack_print`): a 2048×2048
RGB8 declaration, all jobs and encoders retained at once, dev profile, default
features, on a 24-thread host. Results:
[`benchmarks/encode_planning_alloc_2026-10-27.json`](../benchmarks/encode_planning_alloc_2026-10-27.json).

| Mode | Jobs | Prepared | Planned | Begun | Threads after `begin` |
|---|---:|---:|---:|---:|---:|
| Stored (effort 0) | 1 | 77,112 B | 77,109 B | 84,057 B | 2 |
| Stored (effort 0) | 4 | 78,792 B | 78,789 B | 106,578 B | 2 |
| Pre-filtered (effort 1) | 1 | 77,117 B | 77,114 B | 12,675,156 B | 2 |
| Strips (effort 5) | 1 | 77,112 B | 77,109 B | 1,829,430 B | 2 |
| Strips (effort 5) | 4 | 78,792 B | 78,789 B | 7,088,070 B | 2 |
| Strips (effort 5), parallel | 1 | 77,121 B | 77,118 B | 30,917,559 B | 26 |
| Strips (effort 5), parallel | 4 | 78,801 B | 78,798 B | 121,409,055 B | 98 |
| Buffered (effort 19) | 1 | 77,114 B | 77,111 B | 12,660,784 B | 2 |
| Buffered (effort 19) | 4 | 78,794 B | 78,791 B | 50,413,480 B | 2 |

These are whole-process heap peaks. They are not RSS, not native-only totals,
and not bounds for other sizes or configurations. Heaptrack adds one thread to
the probe's one (the same binary prints 1 without it): planning starts none, and
a parallel strip encoder starts 24 workers at `begin` on this host. Planning
added no allocation for stored, pre-filtered and buffered jobs and transient
strip-layout allocations for strip jobs; no planned peak exceeded the prepared
one. The Display P3 declarations (context ICC, descriptor only) show the same
planning deltas as sRGB. Parallel-strip peaks vary between declarations that
plan alike (30.9-31.0 MB for one job, 114.9-121.4 MB for four); the workers
start concurrently. They exit after the encoder drops without being joined, so
the probe saw their memory live at process exit.
