# Public ICC facade migration

The canonical pixel cohort makes the old free ICC helpers private. The runtime
patch is `6b740345`, directly above manifest-only PR #55 (`fd4f7a56`). It replaces
three calls without changing their implementations or the surrounding decisions:

- ICC data model: `ColorModel::from_icc` delegates to the same header reader.
  It still recognizes the model of an unidentified or truncated profile;
  profile identification remains a separate check.
- RGB ICC synthesis: `SynthesizedIcc::for_cicp` delegates to the same synthesis
  implementation and preserves all outcome handling.

The test migration uses `IccIdentification::from_icc` and the public synthesis
facade. Pixel inputs, expected results, assertions and APNG golden hashes are
unchanged. Two Rec.2020/BT.709 refusal fixtures now come from public synthesis
instead of a private compact-profile constant: their ICC bytes differ, while
those tests retain the same intended color meaning and refusal expectations.
The test audit found 110/113 test/fixture files byte-identical; only the three
API-migrated test files differ. No fixture files were added or removed.

## Qualification graph

- Core family: `1fb997bb88e7405e6a315bf1f0b22f71e99be272`.
- Pixels and converter: `fde59b96632557592cffa9b7301acd8db96ed078`.
- Garb: `605276d717ea0b31a1cd2a36ae05301383c7b8b5`.
- Linear-srgb: `52896f4b24664315586bd60e5f048253b1f69052`.
- Zenflate: `17233d1edd6192df441413943dabc3afc5028c6d`.
- SIMD crates: `0.9.31-beta.2`.

The source branch inherits PR #55's manifest admission and leaves its existing
root patch table unchanged. That table predates the facade cohort, so this is
an explicitly paired consumer-graph qualification, not a standalone-root or
main qualification. PR #55's temporary upper cap excludes stable 0.9.31 and
needs a separate replacement after stable-cohort qualification. The gray API
review graph and its source-depth work are separate.

## Gates

The supplied shared incremental control first reproduces three E0603 errors
at the old ICC calls. After migration it passes for RGB8 and RGBA8 97x65 inputs.
An independent test root uses PNG with default features disabled and compiles
the original test files directly. Seven existing targets pass all 22 tests,
with no ignored or filtered tests:

- `source_color_binding` (5), `encode_carrier_context` (3),
  `encode_carrier_policy` (3).
- `animation_color_negotiation` (2), `apng_byte_identity` (1).
- `row_planning` (7), `significant_bits_details` (1).

Run `just public-icc-facade-check` to reproduce the independent test root with
its committed lock and exact source patches. It imports the existing tests by
path; their bodies are not copied or replaced. Together with the shared
control, 23 tests pass. Strict Clippy passes for the actual PNG library and
all targets of this test root; formatting also passes. The independent PNG
reader and APNG digest matrix retain their original assertions.

Latest wrapper: 62 seconds, peak RSS 0.44 GiB, minimum available 16952 MiB,
peak load 2.89. These are validation-process resources, not per-image memory
or speed. This does not assert a full native package or all-feature test pass.

Full native-root testing remains unqualified: published zenquant 0.1.3's
SIMD requirements reject beta.2, and a local manifest-only admission experiment
still encounters a resolver conflict in the full optional-backend graph.
The selected test root does not select those unrelated optional backends.

## JPEG consumer result

With JPEG PR #258 at `3a1627aae73de74025d09f72334c095c4d26f819`
and its PNG source changed from PR #55 to `6b740345`, locked full-feature
Clippy clears all PNG private-helper errors. It then reports five errors in
JPEG's own source: private ICC synthesis imports at `ultrahdr/encode.rs:707`
and `:929`, private orientation calls at `codec/decode.rs:2020` and `:2022`,
and a `Result<PixelBuffer, ConvertError>` versus `PixelBuffer` fallback mismatch
at `:2022`. That consumer gate remains failed; no JPEG source was changed here.
