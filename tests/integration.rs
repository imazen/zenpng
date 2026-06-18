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
