# M4 Pro PNG unfilter tier audit, 2026-10-16

Host: `mac` (Apple M4 Pro, 12 logical CPUs). Baseline source: `2177458f`.
Command for both runs: `run-heavy --mem 12G --jobs 4 -- just arm-unfilters-macos`.
The bench builds with `_dev`, default target CPU, and compares production
dispatch, forced scalar, and direct NEON on seven row widths and both RGB8
and RGBA8. Cargo's local path patches resolved zencodec to a local checkout;
the resolved lock contents were the same for both runs. The 56 groups ran with
`--cpu-policy=unrestricted`: macOS does not enforce a performance-core process
boundary, so these are exploratory measurements.

The baseline showed a dispatch cost on a one-pixel RGBA8 row. Sub does no work
on the first pixel; Paeth uses only the pixel above it. The change returns
early for Sub and uses the scalar Paeth path at exactly one RGBA8 pixel.
Existing all-tier parity tests cover the 4-byte row.

| Group (production path) | Before median | After median | Change |
|---|---:|---:|---:|
| Sub, RGBA8, 1 pixel | 10.6 ns | 8.5 ns | −19.8% |
| Paeth, RGBA8, 1 pixel | 13.7 ns | 11.5 ns | −16.1% |
| Sub, RGBA8, 1920 pixels | 2.17 µs | 2.17 µs | 0.0% at shown precision |
| Paeth, RGBA8, 1920 pixels | 9.15 µs | 9.12 µs | −0.3% |

For the 1920-pixel row, forced scalar took 4.32 µs for Sub and 9.97 µs for
Paeth before the change. The production NEON routing remains useful at that
size. Across all 56 groups, no other production-path median moved by 7% or
more; this is an observational check under unrestricted scheduling, not a
zenbench confirmation. `cargo test -p zenpng --lib simd::` passed 55 tests.
The before run reported peak RSS 0.06 GiB and minimum available 5625 MiB;
the after run reported peak RSS 0.14 GiB and minimum available 5588 MiB.
Raw logs on the measured Mac are in `~/tmp/zenpng-unfilters-20261016T230205Z.log`
and `~/tmp/zenpng-unfilters-20261016T230314Z.log`.
