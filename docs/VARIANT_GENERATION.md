# Variant Generation: zenpng's adoption of the zenjpeg patterns

Written 2026-06-11. Codec-neutral patterns:
`zenjpeg/docs/VARIANT_GENERATION.md`. Code: `src/sweep.rs` (public —
zenpng has no `__expert` gate; all knobs are public),
`tests/sweep_validate.rs` (runs in the normal suite, ~2.5 s).

PNG is the degenerate-and-pleasant case: **the entire curated space is
trial-class** (lossless ⇒ decoded pixels identical across every cell ⇒
`min(bytes)` is exact), there is no quality grid, and the whole
`modes_full` is 9 cells.

- **Discrimination/exclusions** (module docs): `near_lossless_bits` is
  metric-class (changes pixels) — never in the trial axes;
  Crush/Maniac/Brag/Minutes are minutes-per-MP tiers (constructible,
  parseable, not curated); `Filter` has one variant; metadata chunks are
  orthogonal container bytes; `parallel` is **pinned off in every cell**
  (pattern 9 — thread-dependent chunking would make bytes
  machine-dependent) and hashed.
- **Resolution as identity** (patterns 3+4): `Compression::effort()` is
  the resolution function, and the fingerprint hashes the RESOLVED
  effort — `Effort(13)` fingerprint-aliases `Balanced` (test-pinned).
- **Id grammar** (pattern 7): `png-<preset>` / `png-e<n>`;
  `variant_from_cell_id` + grammar-totality roundtrip test.
- **Validation** (patterns 6/14/15): every cell decode-verified and
  EXACT-roundtripped on noise / checker / palette-bands / odd-509×381 /
  tiny content; every tier live vs `png-balanced`; extremes sane
  (`none` largest everywhere; `intense` ≥ `fastest` on palette-friendly
  content). First-run finding: the roundtrip gate tripped on
  checkerboard because the encoder auto-downcasts B/W RGB to grayscale
  PNG — a *format* change, not a pixel change; the harness pins
  `DowncastFlags::none()` and the lesson is recorded here: exactness
  gates must normalize decoded FORMAT before comparing, or pin the
  format negotiation off.
- **Exact trials** (pattern 2): the playbook's queued item — "trial
  zopfli iterations under a byte gate" — maps to the Effort(31+)
  region; the planner makes those cells constructible today and the
  trial helper is the follow-up.
- **Executor wiring** (step 8): zenmetrics plan-cell bridge (q=0
  sentinel everywhere — PNG has no quality).

## Known limits

- Alpha/16-bit/palette input legs (encode_rgba8/rgb16/…) aren't in the
  harness corpus yet; the sweep axes are input-format-agnostic so this
  is corpus work, not planner work.
- APNG (animation) is a separate variant space — unmodeled.
