# Stable SIMD facade cohort, 2026-11-07

This candidate descends from public-facade source review `eaa4e7ca` and removes
only the temporary SIMD upper caps. All runtime and existing test sources are
byte-identical to that review. Original review refs and main are untouched.

The committed independent validation root now uses registry archmage,
archmage-macros and magetypes 0.9.31, garb `8f9aa10d` (source-identical stable-cap
descendant of `605276d7`) and zenflate `a2c7d540` (source-identical stable-cap
descendant of `17233d1e`). Core `1fb997bb`, pixels `fde59b96` and linear-srgb
`52896f4b` remain unchanged.

`just public-icc-facade-check` passes all 23 tests with zero ignored/filtered
cases, formatting and strict PNG-library/all-validation-target Clippy.
The existing APNG hashes and pixel/assertion contracts remain unchanged.
Validation wrapper: 58 seconds, peak RSS 0.46 GiB, minimum available 15815 MiB,
peak load 1.73. These are process resources, not codec speed or image memory.

The inherited native-root core/pixel patch table still targets the older API
family. Full native-root and optional-backend qualification are not claimed
by this paired consumer-graph check. Gray API review remains separate.
