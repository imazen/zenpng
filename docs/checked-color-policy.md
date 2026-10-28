# Checked color handoff

The `EncodeJob::try_set_policy` implementation accepts the all-default no-op
and `EncodePolicy::none().with_color(ColorEmitPolicy::Verbatim)`.
Other explicit fields currently return `UnsupportedOperation::EncodePolicy`
without mutating the job. This is deliberately narrower than the legacy
best-effort setter; acceptance qualifies an effect, not just a stored value.

This lets an adapter pass its already-prepared ICC signaling through PNG
without deriving a cICP carrier from that profile. Deriving cICP can change
which carrier readers treat as authoritative even when ICC bytes are retained.
The adapter still checks the resulting output against its complete request.

The independent `png` reader and native probe both confirm retained ICC,
absent derived cICP and exact supplied RGBA samples. Still, row and APNG
paths run at efforts 0, 1 and 7. Refusals followed by encoding the same job
prove rejection does not replace the previously accepted policy. Existing
strided GrayAlpha and transparent-RGB regressions also pass.

Validation (2026-10-25): `just checked-policy-check`, including formatting,
strict library/integration Clippy, every fuzz target's compilation and API
snapshots. Resource-wrapper receipt: rc=0, 32 s, peak RSS 0.51 GiB,
minimum available 16626 MiB, peak load 2.67. This describes the test process,
not a codec workspace bound. Shared types are pinned to the reviewed
zencodec operation and zenpixels conversion changes; no crate is published.
