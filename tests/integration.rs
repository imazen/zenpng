//! Consolidated integration-test entry point for zenpng.
//!
//! Cargo compiles each top-level `tests/*.rs` file into its own test binary,
//! and every binary re-links the whole crate. Gathering the suites here as
//! submodules of one entry point builds and links them once instead of once
//! per file.
//!
//! The suite sources live under `tests/integration/` so Cargo's top-level
//! `tests/*.rs` auto-discovery does not also build them as separate binaries;
//! the `#[path]` attributes point this crate-root file at them. Add a new
//! suite as `tests/integration/<name>.rs` plus a `#[path] mod <name>;` line
//! here. Run one suite with e.g. `cargo test --test integration probe_parity::`.

#[path = "integration/probe_parity.rs"]
mod probe_parity;

#[path = "integration/simd_consistency.rs"]
mod simd_consistency;

#[path = "integration/cicp_chunk_emit.rs"]
mod cicp_chunk_emit;

#[path = "integration/truncation_series.rs"]
mod truncation_series;

#[path = "integration/apng_alloc.rs"]
mod apng_alloc;

#[path = "integration/sink_truncation.rs"]
mod sink_truncation;

#[path = "integration/content_light_raw.rs"]
mod content_light_raw;

#[path = "integration/animation_timing.rs"]
mod animation_timing;

#[path = "integration/animation_compositing.rs"]
mod animation_compositing;

#[path = "integration/mastering_raw.rs"]
mod mastering_raw;

#[path = "integration/exact_transcode.rs"]
mod exact_transcode;

#[path = "integration/checked_policy.rs"]
mod checked_policy;

#[cfg(feature = "controlled-decode")]
#[path = "integration/controlled_decode.rs"]
mod controlled_decode;
