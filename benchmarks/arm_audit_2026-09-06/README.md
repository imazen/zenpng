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
