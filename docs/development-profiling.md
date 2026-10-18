# Development profiling

Resolve and retain `Cargo.lock`, then build the examples with `just coz-build`.
The ordinary workloads compile on Linux and Windows; Coz counters run on Linux.

Generate developer input fixtures into a new directory:

```sh
just coz-generate-inputs ~/tmp/NEW_INPUTS source1.png source2.png
just coz-run hash 7 ~/tmp/NEW_INPUTS
```

The generator uses image-png for independent decoder test fixtures. Its source
header describes size and format selections. It never upscales. Output files
and the output directory must be new; previous attempts are preserved.

Hash mode checks decoded shape, channels, depth and native byte order against
image-png, then independently reads our encoded output. It reports strict raw
samples, lossless format-reduced values and visible-alpha comparisons separately.
Only non-palindromic 16-bit samples discriminate byte order.

Coz workloads count complete image operations:

```sh
just coz-run encode --st 7 100 ~/tmp/NEW_INPUTS
just coz-run decode --st 100 ~/tmp/NEW_INPUTS
```

On Linux, collect a causal profile by launching the built workload through Coz:

```sh
coz run --output ~/tmp/NEW_encode.jsonl --- target/release/examples/coz_harness encode --st 7 100 ~/tmp/NEW_INPUTS
```

Use a new profile path for each attempt. `just coz-run` executes the workload;
the Coz runtime supplies the profiler. The progress points are `encode_unit`
and `decode_unit`, each incremented after a complete operation.

For an external zenbench comparison, `pair enc EFFORT ITERS NEW_RESULT INPUTS`
and `pair dec ITERS NEW_RESULT INPUTS` print total `elapsed_ms`. Timers include
the codec call, allocation and output destruction. Full-output checksums, input
loading, warmup and result-file writes happen outside timers. The result digest
includes the repetition count; decode also includes dimensions and sample format.
Compare both result files and retain the binaries, lockfile, inputs and commands.
This worker alone does not establish a performance improvement.
