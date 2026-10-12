# Typed PNG and exact animation API integration

The original integration combines the existing typed one-shot interface, exact APNG
timing/compositing, native color negotiation and strip-streaming configuration
PRs with main `1df6b2f8`. Current threading, strip compression and exact cLLI
metadata remain intact. The new `encode(PixelSlice)` returns the underlying
job's `At<CodecError>` so native causes and traces survive the wrapper.

PR18’s two convenience functions were subsequently reverted at user direction.
The validation below describes the original integration; the retained APIs are
exact animation/color and streaming configuration.

Runtime `88c325ab382eb7c1182e8ce3ca787d0c43459da2` passed release default
library/integration/native-color tests, minimal integration/native-color tests,
doctests and all-target/all-feature strict Clippy. These include 8/16-bit exact
animation timing/color and composition, retained checksum/truncation/resource
checks, and typed one-shot roundtrips. The 14 library and two doctest ignores are
inherited; none were added or changed. This is local x86 coverage, not native
ARM/Windows or a new speed/memory comparison.

Strict rustdoc then rejected stale links. The followup
`03376a23c9d95467730b0a30e47841786b4138f4` changes only source comments and
passes strict all-feature docs and mechanical public-API snapshot generation.
The snapshots are included with this report. Initial wrapper error-type and
doc failures remain in the evidence archive; no assertion was relaxed.

Whole-job resources on Ryzen 9 7900X: runtime/test/Clippy plus failing docs,
`run-heavy: done rc=101 53s | peak-RSS 0.77GiB | min-avail 22050MiB | peak-load 4.92`;
corrected docs/snapshots,
`run-heavy: done rc=0 21s | peak-RSS 0.53GiB | min-avail 22601MiB | peak-load 2.10`.
These are build/test job resources, not codec-only memory or throughput.

Shared unreleased API sources retain the animation PR's exact pins: zencodec
`20310944`, zenpixels/zenpixels-convert `789d69db`; zenflate retains main's
`9f8684bf`. Consuming workspaces must apply these patches themselves. No crate
publication is part of this integration.

Evidence key: `output/zenpng/api_integration_2026-10-12`, canonical and dev mirror
`/mnt/v/output/zenpng/api_integration_2026-10-12`. Both copies' six content hashes
verified. `SHA256SUMS` digest:
`c5b9aaa12dcdcab3eedb369471efcdbe4a7d37fbcdd093c94121cf20a91868fd`.

## PR18 revert

`ef31abe4107b70753d16c552a751aa5a28589d1e` removes only the two PR18
convenience functions and their documentation. PR22 remains merged, including
native animation color negotiation. `just test-api-integration` passed release
library tests, default/minimal integration and native-color tests, doctests,
all-target/all-feature strict Clippy, strict docs and API snapshot generation.
The regenerated snapshot removes exactly those two public functions.

Whole-job validation: `run-heavy: done rc=0 68s | peak-RSS 0.77GiB |
min-avail 22125MiB | peak-load 4.85`; this is build/test resource usage.
Full log: artifact key `output/zenpng/revert_pr18_2026-10-12/gate.log`.
