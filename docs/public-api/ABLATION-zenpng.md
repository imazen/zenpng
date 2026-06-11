# zenpng Public API Ablation Report

**Date:** 2026-06-11  
**Snapshot commit:** da2cce6519dc8bef52bccee39ac77e395ab9b216  
**Snapshot:** 917 items (default), 983 items (all-except-_*)  
**Scan template:** `grep -r "<sym>" /home/lilith/work --include="*.rs" -l | grep -v zen/zenpng/ | grep -v zen-arm-src/ | grep -v pre-filter/ | grep -v .jplag/ | grep -v target/`  
**Consumers checked:** zenpipe/zencodecs, imageflow, zenmetrics, hdr-corpus-convert, zengif, zensim, BRAG  
**Mode:** REPORT ONLY — no source or manifest changes.

---

## Summary

| Category | Count |
|----------|-------|
| Total named items (unique, default section) | 458 |
| Flagged for action | 5 items / groups |
| % of total | ~1.1% |
| Clear mistakes confirmed | 3 (A/B) + 2 informational |
| Items kept (consumers found or structurally necessary) | 453+ |

Threshold: 10% to aggregate. At 1.1% flagged, individual item-level reporting is appropriate.

---

## Grep Evidence

All commands run 2026-06-11. Excludes: zen-arm-src (stale box snapshot), pre-filter (stale snapshot), .jplag (comparison copy), target/ (build output).

```
available_backends   → 0 files (excluding zenpng itself + archmage-practicum which has unrelated fn)
quantizer_by_name    → 0 files
PngStreamingDecoder  → 0 files
DowncastFlags        → 0 files (outside zenpng)
ZenquantQuantizer::config/config_mut → 0 files (outside zenpng)
ZenquantQuantizer (struct) → 4 files in zen/zengif/ (own impl, not importing from zenpng)
PngChromaticities    → 0 files (but structurally required as PngInfo field)
PngBackground        → 0 files (same)
SignificantBits      → 0 files (same)
PngTime              → 0 files (same)
PhysUnit             → 0 files (same)
TextChunk            → 0 files in zen/* (hits in resvg/zed/image-png are unrelated structs)
```

---

## Module Table

### `zenpng` (root)

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `available_backends()` | 0 | **B** (pub(crate) candidate) | Always-on (no feature gate). Only callers are within zenpng itself (quantize.rs self-test). Exposes backend enumeration that has no consumer outside this crate. Alternatively, make it `#[doc(hidden)]` if downstream CLI use is anticipated. |
| `quantizer_by_name()` | 0 | **B** (pub(crate) candidate) | Same situation as `available_backends`. The doc example shows a usage pattern but no real caller exists. |
| `ZenquantQuantizer::config()` | 0 | **A** (`#[doc(hidden)]`) | Returns `&zenquant::QuantizeConfig`, exposing the inner zenquant abstraction. No external consumer. Prefer the builder methods. `config_mut()` same. |
| `ZenquantQuantizer::config_mut()` | 0 | **A** (`#[doc(hidden)]`) | Same as above. |

### `zenpng::codec` (private module, `PngStreamingDecoder` leaks)

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `PngStreamingDecoder<'a>` | 0 | **Informational** | `mod codec` is private, but `PngStreamingDecoder` is `pub struct` inside it and leaks through the public type system via: `PngDecodeJob::StreamDec` (associated type) and `PngDecodeJob::streaming_decoder()` return type. This means rustdoc will surface it as `zenpng::codec::PngStreamingDecoder`. No external consumer as of this scan. Could be sealed behind `#[doc(hidden)]` on the struct, or the associated type alias could use an opaque wrapper. Not proposing a breaking change here — marking informational for next API review. |

### `zenpng::detect`

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `PngProbe::from_info()` | 0 | **A** (`#[doc(hidden)]`) | Takes `&zenpng::PngInfo` and constructs a `PngProbe`. Used internally in zenpng's `detect::probe()` function. No external consumer. Should be `pub(crate)` or `#[doc(hidden)]`. |

### Feature-gated quantizer structs (`imagequant` / `quantette` features)

| Item | External hits | Action | Notes |
|------|---------------|--------|-------|
| `ImagequantQuantizer` (struct + pub fields + builder) | 0 | KEEP | No external consumer but this is the deliberate API for the `imagequant` feature. Pub fields + builder methods is an intentional pattern (struct update syntax). |
| `QuantetteQuantizer` (struct + pub fields + builder) | 0 | KEEP | Same reasoning. |

---

## Items Confirmed Safe (Selected)

These looked suspicious but are structurally necessary or have confirmed consumers:

| Item | Reason to keep |
|------|----------------|
| `PngChromaticities`, `PngBackground`, `SignificantBits`, `PngTime`, `PhysUnit`, `TextChunk` | Fields of `PngInfo`, which is the return type of `probe()` — a widely-used function (imageflow, zenmetrics, zencodecs). Cannot remove without removing `PngInfo` fields. |
| `DowncastFlags` + all pub fields | Used by `EncodeConfig::downcast` field and `with_downcast()` builder. zenpipe/zencodecs chain uses `EncodeConfig`. |
| `PngLimits` type alias | Already `#[deprecated]` with correct note. No action needed. |
| `PngDecodeConfig` vs `PngDecoderConfig` | Both are real distinct types: `PngDecodeConfig` = low-level decode config (used by imageflow, zenmetrics); `PngDecoderConfig` = zencodec trait wrapper (used by zencodecs). Not a duplicate. |
| `ZenquantQuantizer` (struct itself) | Used within zenpng for `default_quantizer()` return type and internal indexed encoding. The `config()` / `config_mut()` methods are the only candidates for hiding. |
| `encode_apng_auto`, `QualityGate`, `ApngEncodeParams` | Used in hdr-corpus-convert HDR PNG pipeline. |
| `ApngEncodeConfig::encode` (pub field) | Struct-update syntax usage expected. Intentional. |
| `EncodeConfig::cicp`, `with_cicp`, `with_content_light_level`, `with_mastering_display` | HDR write API — recent deliberate addition (June 2026). Keep wholesale. |

---

## Top-5 Digest

1. **`available_backends()`** — B: pub(crate). Zero external consumers. Always-on with no feature gate. Bleeds quantizer implementation details.
2. **`quantizer_by_name()`** — B: pub(crate). Zero external consumers. Same issue.  
3. **`ZenquantQuantizer::config()` / `config_mut()`** — A: #[doc(hidden)]. Exposes inner zenquant abstraction. Zero external consumers. Builder methods are the correct surface.
4. **`PngProbe::from_info()`** — A: #[doc(hidden)]. Internal constructor with no external caller. Should be `pub(crate)`.
5. **`PngStreamingDecoder` (informational)** — Not proposing action but the struct in private `mod codec` leaks into rustdoc via `StreamDec` associated type. Candidate for sealing in a future API cleanup.

---

## Action Summary

| Action class | Items | Breaking? |
|-------------|-------|-----------|
| A — `#[doc(hidden)]` / `#[deprecated]` | 3 items (config/config_mut, from_info) | No |
| B — pub(crate) / remove | 2 functions (available_backends, quantizer_by_name) | **Yes** (semver-breaking) — queue in QUEUED BREAKING CHANGES |
| Informational | 1 (PngStreamingDecoder) | N/A |

**B items must wait for next breaking release.** Add to `## QUEUED BREAKING CHANGES` in CHANGELOG.md `[Unreleased]` section.
