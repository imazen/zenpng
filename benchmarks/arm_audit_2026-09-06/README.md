# ARM unfilter audit

The initial [production run](production-before.log) was collected at
`31ec83987a5597d80100c2c10b2779690b60851f` on Apple M4 Pro, macOS Darwin
25.5.0, rustc 1.98.0, without target-cpu overrides. Its `neon` label means
NEON-enabled production dispatch. Source inspection shows only Sub RGB8
actually reaches explicit NEON: Up, Paeth, Sub RGBA8, and Average RGBA8
force scalar on ARM; Average RGB8 has no explicit NEON implementation.
The other seven groups are scalar-versus-scalar controls, not evidence of
NEON parity. Sub RGB8 measured 2.36 us versus 10.86 us scalar on a 1920-pixel
row. Shared-host variance is recorded in the full log.

The updated benchmark names production and forced-scalar arms explicitly
and adds direct NEON calls for the seven implemented filter/bpp pairs.
It verifies direct-kernel output against forced-scalar output before timing.
No production dispatch or arithmetic changes in this benchmark correction.
Private kernels become crate-visible only; no public API is added.

Reproduction: `just arm-unfilters-macos`. All heavy work is serialized,
niced, and capped to four build/Rayon/OpenMP threads. macOS `/usr/bin/time -l`
resource counters are retained in the log; no Linux cgroup cap is available.

## Verified Average improvement

`dfdea008` replaces widen/add/shift/narrow with exact unsigned NEON halving
add (`vhadd_u8`). Fixed four-byte row chunks eliminate the repeated indexed
slice checks. `4d6db0bd` enables that kernel in production for ARM RGBA rows
with a left-neighbor recurrence; zero/one-pixel rows use the scalar formula.
Other architectures and filter dispatch choices are unchanged.

The [direct-kernel baseline](png-direct.log) measured Average RGBA at
12.42 us against 5.95 us scalar for 1920 pixels. After the change the
[same comparison](png-avg-halving.log) is 4.30 us against 6.43 us scalar.
These are separate interleaved runs; exact before/after percentages are
subject to shared-host variance. The [width sweep](png-avg-widths.log)
identified one-pixel dispatch overhead, handled by the recurrence-free path.

Final [production-dispatch measurements](png-avg-production.log):

| Row pixels | Production | Forced scalar |
|---:|---:|---:|
| 1 | 16.3ns | 16.3ns |
| 17 | 51.7ns | 68.9ns |
| 64 | 156.1ns | 225.6ns |
| 256 | 599.5ns | 888.5ns |
| 1024 | 2.35µs | 3.54µs |
| 1920 | 4.31µs | 6.45µs |
| 4096 | 9.09µs | 13.71µs |

The production path wins on measured rows of 17 through 4096 pixels;
one-pixel production and scalar timings are equivalent. This is an unfilter
kernel result, not a whole PNG decode speedup. Palette scans, deflate, and
other filtering costs are outside this comparison. No quality parameters or
source-calibration tables are derived from the experiment.

[Assembly excerpt](average-halving.asm) contains `uhadd.8b` and `add.8b`,
with no widening/narrowing, no helper calls, and no bounds checks in the loop.
Generated using `otool -tvV target/release/deps/unfilter_tiers-4c154db44fe14d6d`
on the benchmark at `dfdea008`. archmage/magetypes are locked at 0.9.28.

[Native test summary](native-tests.txt): 703 pass, zero failures, 14 existing
ignored tests unchanged. The new direct-NEON oracle covers all 65536
left/above byte pairs (including wrapping addition), row widths 1 through
1920, offsets 0 through 15, and untouched buffer guards. Existing dispatch
permutation tests also pass after enabling the kernel. Scoped formatting and
[final clippy](png-final-clippy.log) pass with warnings denied. Full test log
is retained outside git because it exceeds 30 KB; its SHA256 is recorded.

All logs retain process resource counters. Neither WASM timing nor an
end-to-end corpus speedup is claimed. The other explicit NEON filter losses
are confirmed by the baseline; production continues to select their faster
scalar paths. Sub RGB8 continues to benefit from its existing NEON kernel.


## Encoder predicate scans

The predicate benchmark now supplies an actually opaque buffer to the opacity check (the previous first pixel had alpha zero), checks exact results before timing, and gives each operation/size its own paired scalar baseline. Labels now say runtime SIMD instead of implying 512-bit vectors on ARM.

All 28 SIMD/scalar comparisons across 24 groups favored the existing runtime path: five individual predicates plus two fused variants at 64², 256², 1024², and 4096². Paired runtime reductions span 71.53–87.69% for these full-scan fixtures. Several individual-predicate cells have high CV; the retained paired intervals remain below zero. The constant-generic fused implementation was compared against scalar, not statistically against the runtime fused implementation.

All untimed exact-result checks passed, and `cargo clippy --locked -p zenpng --bench scalar_vs_simd --features _dev -- -D warnings` passed. No production predicate change was needed. These are deliberately full-scan synthetic fixtures, not end-to-end encode timings or quality calibration. Run `just arm-scan-tiers-macos`; results and the full-log pointer are adjacent to this report.
