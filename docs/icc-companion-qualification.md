# Native ICC companion qualification

This separate review descendant starts from the qualified PNG facade source.
It binds descriptor/context CICP to both native cICP and a canonical ICC
compatibility companion. Gray/gray-alpha use GRAY ICCs. HLG retains native CICP
without inventing a target-dependent ICC. Supplied ICC bytes are retained;
exact canonical byte equality permits the corresponding known companion to
pass color-consistency checks. Hash recognition alone grants no such proof.

Non-D65 gray whites are retained even with the sRGB transfer. Linear metadata
profiles still refuse when f32 storage converts samples to sRGB. New profiles
being metadata-only does not disable this existing refusal.

The reviewed pixel source is `3ceb0500059456aacc16c7aecb43b2d167ce1535`;
core is `db6ec36d0072988888f6175695bc33c840bbcc93`. Root numerical patches
are garb `58ce9cad7b337418a7abe9b1f6e3e4bd1928b385` and linear-srgb
`52896f4b24664315586bd60e5f048253b1f69052`. The manifest-only zenresize
source `a886c8504092ae7d708ca051b4e24489e262a992` permits one pixel family
in the dev/example graph. SIMD resolves stable 0.9.31 with no beta ceilings.

`just icc-companion-check` runs the unchanged library, source-color and row
planning gates plus native HDR companion witnesses. The exact root cohort
passes 749 tests with 14 existing ignores. The library portion is 735 tests.
The witnesses cover RGB/RGBA/gray/gray-alpha, 8/16-bit strided samples, PQ/HLG,
descriptor/context declarations, one-shot/row encodes and non-D65 gray whites.
Every encoded sample and ICC payload is read independently with image-png.
No established pixel expectation is weakened. The final test wrapper records
158 seconds and 0.83 GiB peak RSS; this is not a performance comparison.

The frozen root lock is
[perf/icc-companion-validation/root-cohort.lock](../perf/icc-companion-validation/root-cohort.lock).
The standalone facade harness retains its own distinct zenflate source and
lock; do not substitute one graph's result for the other. Fuzz targets are
compiled as a separate graph; compilation is not a fuzz execution.

Original source-binding failure and disk-full receipts are retained outside
git. The first stale linear-profile regression is fixed rather than having
its assertion changed. A separate core ICC-authority guard is under review;
it is deliberately absent from this carrier-only source pin.

Final strict Clippy passes. The separate facade graph passes 23 controls;
all five fuzz binaries compile. Its compiled zenflate source remains distinct
from the root cohort. Cargo's unused optional core-family patch warnings do
not indicate duplicate selected types; the resolved roots use one pixel family.
The raw low-level API continues to write caller-selected metadata; the ICC
companion behavior here applies to the pixel-context trait route.

The explicit native cICP-only builder contract remains available and its
original assertion is unchanged. Automatic companions apply when the trait
route derives color from the pixels' descriptor/context. The full integration
suite now uses the public raw-row negotiation API instead of a private test
helper; all 52 integration controls pass without changing their expectations.
