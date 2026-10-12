# Typed GrayAlpha encoding validation

Source `84fb628011b60447ebbe6a50abcadb0d0194bb48` passes
`just test-exact-transcode`: default/minimal integration, library tests,
doctests, strict Clippy, docs and API snapshots. Independent PNG decoding
checks GrayAlpha8/16, strided buffered/row output at efforts 0/1/7,
multiple strips and GA16 alpha with a zero low byte. No thresholds were changed.

Whole validation job: rc 0, 30 seconds, peak RSS 0.82 GiB, minimum available
22900 MiB, peak load 2.37. No throughput claim.

Canonical and dev mirror: `output/zenpng/gray_alpha_2026-10-12/gate.log`.
SHA-256: `300e7e293db0800f782381060a9b0530451b0bbbccd23bdb7e25f14abbeaf722`.
