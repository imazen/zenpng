# Metadata integration and the published PNG API

Baseline: [zenpng 0.1.4](https://crates.io/crates/zenpng/0.1.4), compared on
2026-10-12 against main `1db58217` plus the inventory integration. The checkout
already identifies itself as 0.2.0, so the check explicitly assumes a patch
release to expose differences instead of accepting them as a version bump.

## Existing breaks

The nine zencodec trait implementations in `src/codec.rs` changed their `Error`
associated type from `At<PngError>` to `At<CodecError>` in `d6ff72d`. These cover
encoder configuration, encode job, encoder, animation encoder, decoder
configuration, decode job, decoder, streaming decoder and animation decoder.
The native convenience methods retain their codec-specific errors. Manual
source comparison is necessary: cargo-semver-checks did not report these
associated-type changes across the dependency revisions.

The all-feature rustdoc comparison reports three lost auto traits:

| Type | Published trait | Existing change |
| --- | --- | --- |
| `PngEncoder` | `Sync` | `5035d47`, strip-streaming state |
| `PngAnimationFrameEncoder` | `UnwindSafe` | `70d320e`, cancellation state |
| `PngAnimationFrameEncoder` | `RefUnwindSafe` | `70d320e`, cancellation state |

Those fields are already present in main `1db58217`; the inventory change
adds neither encoder state nor an associated-type change. The automated
comparison passed 222 checks, failed one check covering those three traits,
and skipped 31 checks. A passing automated report alone would not establish
compatibility because of the manually found associated-type changes.

## Additive inventory contract

[PR32](https://github.com/imazen/zenpng/pull/32) supplies the existing walker,
review tests and fuzz checks. Its unreleased `Option<Inventory>` adapter is
reconciled with the shared always-callable operation: `Result<Inventory,
At<CodecError>>`. With the default-off `inventory` feature disabled, the
shared trait implementation returns `UnsupportedOperation::Inventory`.
With it enabled, the configured decode job supplies input limits, decode
policy and cancellation. Part-count failures retain `InventoryError` and
its resource category. Malformed content remains a description in the map.

The feature enables zenflate's default-off `scan` implementation. No zenmeta,
zencodecs or zenpipe dependency is needed for PNG chunk inventory. Baseline
color, orientation and HDR metadata remain available without inventory.

## Reproduction and validation

`just metadata-api-check` builds rustdoc JSON from this checkout and the
published 0.1.4 source, then compares them with an explicit patch release.
Generating the current JSON in place preserves the coordinated dependency
patches; a semver-checks throwaway workspace cannot resolve the unpublished
zencodec 0.1.27 dependency. Root patches also do not propagate to the separate
fuzz workspace, which carries matching pins. These pins must be replaced by
released versions as part of the coordinated release.

`just metadata-feature-check` checks disabled and enabled generic/dynamic
jobs, configured input limits, one-shot cancellation, existing fuzz seeds and
strict lints. `just metadata-inventory-check` retains the original walker
and review suites. `just metadata-inventory-oracle` compares ExifTool chunk
positions: the 2026-10-12 run covered 212 files and 1,277 chunks with no
differences; both tools rejected six bad signatures. This is structural
validation, not a pixel-decoding claim.

With user approval on 2026-10-12, the original PR test
`r4_max_input_bytes_rejection_is_reported` now asserts the typed input-limit
error, including its exact size and cap. It previously expected an annotated
map beyond that cap. The updated expectation matches the shared operation's
input admission contract; the test is not skipped.

The enabled inventory and all four review suites also passed on native x86-64
and i686 after that approved expectation change. The disabled feature, stable
fuzz replay, all-target clippy and existing encode/decode suites passed.

## Codec-owned rewrite planner

The additive `metadata-rewrite` feature exposes `metadata::rewrite` with
zencodec-owned Request, Plan and Error types. The planner and original PNG/APNG
assertions moved from zencodecs `aa21cba0`; zenmeta `a31a9b68` owns the bounded
EXIF/service/plan helpers. Existing decode APIs are unchanged.

`just metadata-rewrite-check` covers private chunks and trailers, SDR/16-bit
HDR sample preservation, CRC/order/truncation rejection, APNG frames and timing,
and timeouts at every observed CRC/container poll. Image-png independently
reads back the rewritten samples. No inventory feature or zenpipe dependency
is needed. The planner leaves encoded image streams opaque; it does not certify
every compressed sample or transform embedded rendering metadata.
