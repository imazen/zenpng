# Exact PNG transcode encoding

Runtime source `e495085c331734678f65899e4c59a552cd9ae280` passed
`just test-exact-transcode` on 2026-10-12. This runs retained default/minimal
PNG/APNG integration tests, library tests, doctests, strict all-feature Clippy,
docs and public API snapshots. The independent PNG decoder verifies buffered,
strided row, segmented and animation output, including hidden RGB under zero
alpha. ICC-only transcodes select both metadata preservation and the separate
`ColorEmitPolicy::Verbatim`; default color emission may derive matching CICP.
The original missing-policy test failure is retained; its assertions were not
weakened.

Whole validation job: 59 seconds, peak RSS 0.76 GiB, minimum available memory
22010 MiB, peak load 4.12, exit 0. These are validation-job resource figures,
not encoder throughput or decoder memory measurements.

Logs: `output/zenpng/exact_transcode_2026-10-12/`, canonical archive and dev mirror.

| File | SHA-256 |
|---|---|
| gate.log | 90a60042cff5708827ca2058fbaa1c7e3471e002780f0bc1c72ae468956d40b3 |
| initial-policy-test.log | b5bdd356d66fa219be76715d5aa318eb7d312aa9768a300e5d9b182d6c330b20 |

The preservation flag defaults off. Opt-in APNG uses byte-exact SOURCE/NONE
rectangles and bypasses the visible-pixel blend/dispose search. Size and speed
costs of that opt-in have not been measured. Explicit quantization and pixel
conversion controls remain independent.
